"""CWX keyer tests: B10.5a with its keyer timing (0046, spec/draft 7da25a9), T-B10-33 and
T-B10-43..46.

The keyer is judged from the key.log fixture (testlist-format v1.5): the key-on and key-off
times of the keyed state and the arrival of every High Priority packet, both in the radio's
time. Marks, spaces and latencies are measured between those events, so the host's own pacing
does not enter the verdict. The bench only plans its paddle edges well away from the keyer's
decision points and checks that they arrived there; an edge too close to one gives an error
("pacing"), not a verdict.

A test needs TX unlocked (record `tx` bit 4 = 1): before that it is blocked on the calibration
set of the freeze rule (Annex P). Every element keys under all conditions of A3.1a: the bench
sets PA enable, mode class CW, an arm, a TX frequency inside Table A3-0 and break-in, so that
CWX keys by itself (B10.5b).

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import time
from contextlib import contextmanager

from .core import Blocked, Ctx, RunnerFault, Skip, test
from .mgmt import FREEZE_SET, Mgmt, parse_calibration, read_log, stats
from .p2 import (ARM_MAGIC, PORT_DDC, PORT_DISC, PORT_DUC, PORT_HP, PORT_REG, TX_ARM, TX_ARM_STATE,
                 TX_MODE_CLASS, TX_STATE, Host, ddc_specific, duc_specific, general, high_priority,
                 need_record, need_token, parse_regs, value)

CWX, DOT, DASH = 0x01, 0x02, 0x04  # High Priority byte 5
REVERSE, IAMBIC, MODE_B, STRICT, BREAK_IN = 0x04, 0x08, 0x20, 0x40, 0x80  # DUC-specific byte 5
F_TX = 14.05e6  # inside Table A3-0
KEEPALIVE_MS = 20
LATENCY_US = 5_000  # B10.5a item 4 (SF-22): key-on within 5 ms of the packet with the edge
GUARD = 0.2  # a planned edge must arrive at least 0.2 T from a decision point


def tol(nominal_us: float) -> float:
    """B10.5a item 6: each mark and space within +-1 ms or +-2 %, whichever is larger."""
    return max(1000.0, 0.02 * nominal_us)


def char_tol(nominal_us: float) -> float:
    """B10.5a item 6: a whole character within +-2 ms or +-2 %."""
    return max(2000.0, 0.02 * nominal_us)


def unit_us(speed: int) -> float:
    return 1_200_000 / min(speed, 60)


def weight_in_force(w: int) -> int:
    return 50 if w == 0 else min(max(w, 33), 66)


def lengths(speed: int, weight: int) -> tuple[float, float, float]:
    """(dot mark, dash mark, space after a mark) in us (items 1-3)."""
    t = unit_us(speed)
    d = t * (weight_in_force(weight) - 50) / 50
    return t + d, 3 * t + d, t - d


RX_TR = {"PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0}  # T/R lines at their RX levels (HIS 85)


class Log:
    def __init__(self, rep: dict):
        self.key = [(e["t_us"], e["on"]) for e in rep["key"]]
        # v1.5 entries may carry the other RF signals at the key change: "tr" (the four T/R
        # levels) and "bias_nonzero". At a key-on both must already be in the transmit state
        # (A3.9a: relays switch before RF); at a key-off they are not judged, since the relays
        # return only after RF is gone (A3.9a item 1) and stay between CW elements (A3.4d).
        self.disagree = [e for e in rep["key"] if e.get("on") and (
            ("tr" in e and all(e["tr"].get(k) == v for k, v in RX_TR.items()))
            or ("bias_nonzero" in e and not e["bias_nonzero"]))]
        self.carries_rf = any("tr" in e or "bias_nonzero" in e for e in rep["key"])
        self.marks: list[tuple[float, float]] = []
        on = None
        for t, v in self.key:
            if v and on is None:
                on = t
            elif not v and on is not None:
                self.marks.append((on, t))
                on = None
        self.open = on  # keyed at the end of the log
        # paddle edges as the radio received them: (t_us, dot, dash), the CWX bit included
        self.edges: list[tuple[float, bool, bool]] = []
        prev = None
        for e in rep["hp"]:
            b5 = e["b5"]
            st = (bool(b5 & CWX and b5 & DOT), bool(b5 & CWX and b5 & DASH))
            if st != prev:
                self.edges.append((e["t_us"], *st))
                prev = st
        self.start_t = rep.get("start_t_us")

    def mark_lens(self):
        return [b - a for a, b in self.marks]

    def space_lens(self):
        return [self.marks[i + 1][0] - self.marks[i][1] for i in range(len(self.marks) - 1)]

    def edge(self, n: int) -> float:
        """Arrival time of the n-th change of the paddle bits (0 = the first press)."""
        if n >= len(self.edges):
            raise RunnerFault(f"pacing: paddle change {n} not in key.log hp ({len(self.edges)} changes)")
        return self.edges[n][0]


class Cw:
    """One owner session set up for CWX keying."""

    def __init__(self, ctx: Ctx, h: Host):
        self.ctx, self.h = ctx, h

    def duc(self, opts: int, speed: int, weight: int = 50):
        self.opts, self.speed, self.weight = opts, speed, weight
        self.h.send(PORT_DUC, duc_specific(cw=opts | BREAK_IN, speed=speed, weight=weight))
        self.h.drain(0.03)

    def setup(self, opts: int, speed: int, weight: int = 50):
        ctx, h = self.ctx, self.h
        ctx.need("key.log")
        rec = need_record(ctx, h)
        if not (rec[0x20] >> 4) & 1:
            raise Blocked(pending_reason(ctx))
        self.opts, self.speed, self.weight = opts, speed, weight
        h.send(PORT_DISC, general(pa=True))
        h.send(PORT_DDC, ddc_specific(192, True))
        h.send(PORT_DUC, duc_specific(cw=opts | BREAK_IN, speed=speed, weight=weight))
        h.send(PORT_HP, high_priority(run=True, duc0_hz=F_TX))
        h.running = True
        pairs = sum((parse_regs(d) for _, d, _ in h.recv(0.3, PORT_REG)), [])
        tok = need_token(ctx, h, pairs)
        h.write([(TX_MODE_CLASS, 2)])  # CW (Table B8-2)
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        if not ctx.check("armed for CWX (TX_ARM_STATE [3:0] = 1)", st is not None and st & 0xF == 1, f"TX_ARM_STATE {st}"):
            raise Skip("arm not accepted: the keyer tests need an armed session")
        self.idle(0.05)  # the first CWX edge comes >= 10 ms after the arm confirmation (B11.6a)

    def idle(self, s: float):
        end = time.monotonic() + s
        while time.monotonic() < end:
            self.h.send(PORT_HP, high_priority(run=True, duc0_hz=F_TX))
            time.sleep(min(KEEPALIVE_MS / 1000, max(0.0, end - time.monotonic())))

    def play(self, sched: list[tuple[int, int]], end_ms: int) -> Log:
        """Send the paddle bits of `sched` [(t_ms, DOT|DASH)] at their times, with keepalives
        every 20 ms, until end_ms; return the key.log of that time."""
        ctx, h = self.ctx, self.h
        start = ctx.fixture("key.log", action="start") or {}
        t0 = time.monotonic()
        bits, last_send, i = 0, -1e9, 0
        while True:
            now = (time.monotonic() - t0) * 1000
            changed = False
            while i < len(sched) and sched[i][0] <= now:
                bits = sched[i][1]
                i += 1
                changed = True
            if changed or now - last_send >= KEEPALIVE_MS:
                h.send(PORT_HP, high_priority(run=True, duc0_hz=F_TX, cwx=(CWX | bits) if bits else 0))
                last_send = now
            if now >= end_ms:
                break
            time.sleep(0.0005)
        rep = read_all(ctx)
        rep["start_t_us"] = start.get("t_us")
        log = Log(rep)
        if log.carries_rf:
            ctx.check("key.log: at every key-on the T/R lines have left their RX levels and the bias is non-zero "
                      "(chain A bit 7 taken as the key signal; T/R and bias must agree)", not log.disagree,
                      f"{len(log.disagree)} key-on entries disagree, first {log.disagree[0]}" if log.disagree else "agree")
        return log


def read_all(ctx: Ctx) -> dict:
    """key.log read with paging (v1.5), then stop; dropped entries fail the test."""
    key, hp = [], []
    for _ in range(1000):
        r = ctx.fixture("key.log", action="read", from_key=len(key), from_hp=len(hp))
        key += r.get("key", [])
        hp += r.get("hp", [])
        if len(key) >= r.get("count_key", 0) and len(hp) >= r.get("count_hp", 0):
            break
    r = ctx.fixture("key.log", action="stop", from_key=len(key), from_hp=len(hp)) or {}
    key += r.get("key", [])
    hp += r.get("hp", [])
    ctx.check("key.log: no entries dropped", not r.get("dropped", 0), f"dropped {r.get('dropped', 0)}")
    return dict(key=key, hp=hp)


def pending_reason(ctx) -> str:
    try:
        with Mgmt(ctx.net) as c:
            g = c.req("GET_CALIBRATION")
        have = set(parse_calibration(g["payload"])[3]) if g and g.get("status") == 0 else set()
    except Exception:
        have = set()
    missing = [p for p in FREEZE_SET if p not in have]
    return "pending:" + ",".join(missing or FREEZE_SET) + " (record tx bit 4 = 0: TX locked until the calibration set is complete, B10; Annex P)"


@contextmanager
def owner(ctx: Ctx):
    with Host(ctx.net) as h:
        try:
            yield Cw(ctx, h)
        finally:
            h.stop()


def guard(ctx, what: str, t_edge: float, points: list[float], unit: float):
    """A planned edge must not lie within GUARD * T of a decision point."""
    near = [p for p in points if abs(p - t_edge) < GUARD * unit]
    if near:
        raise RunnerFault(f"pacing: {what} arrived {abs(near[0] - t_edge) / 1000:.1f} ms from a decision point "
                          f"(needs >= {GUARD * unit / 1000:.1f} ms); emulated time too far from wall time?")


def check_lengths(ctx, label: str, got: list[float], want: float):
    bad = [g for g in got if abs(g - want) > tol(want)]
    ctx.check(f"{label}: every one {want / 1000:.1f} ms +-{tol(want) / 1000:.1f} ms (item 6)", bool(got) and not bad,
              f"{len(got)} measured, {[round(g / 1000, 2) for g in got]}")


def repeated(ctx, cw: Cw, label: str, paddle: int, hold_ms: int, dash: bool):
    """A single paddle held: n equal elements, each mark and space at its length; no element
    starts after the release (mode A or B: nothing else pressed)."""
    dot_m, dash_m, sp = lengths(cw.speed, cw.weight)
    mark = dash_m if dash else dot_m
    period = mark + sp
    log = cw.play([(0, paddle), (hold_ms, 0)], hold_ms + int(4 * period / 1000) + 100)
    p, r = log.edge(0), log.edge(1)
    if not ctx.check(f"{label}: keys", bool(log.marks),
                     f"{len(log.marks)} marks" if log.marks else "no key-on in key.log"):
        return log
    m0 = log.marks[0][0]
    ctx.check(f"{label}: first key-on within 5 ms of the press (item 4)", 0 <= m0 - p <= LATENCY_US,
              f"{(m0 - p) / 1000:.2f} ms after the packet's arrival")
    points = [m0 + k * period for k in range(1, 40)]
    guard(ctx, f"{label}: release", r, points, unit_us(cw.speed))
    n = int((r - m0) // period) + 1
    ctx.eq(f"{label}: elements until the release", len(log.marks), n)
    check_lengths(ctx, f"{label}: marks", log.mark_lens(), mark)
    check_lengths(ctx, f"{label}: spaces", log.space_lens(), sp)
    pairs = [log.marks[i + 1][0] - log.marks[i][0] for i in range(len(log.marks) - 1)]
    bad = [x for x in pairs if abs(x - period) > char_tol(period)]
    ctx.check(f"{label}: mark plus space {period / 1000:.1f} ms", not bad, f"{[round(x / 1000, 2) for x in pairs]}")
    return log


def kinds(cw: Cw, log: Log) -> str:
    dot_m, dash_m, _ = lengths(cw.speed, cw.weight)
    return "".join("." if abs(m - dot_m) < abs(m - dash_m) else "-" for m in log.mark_lens())


@test("T-B10-43")
def t_b10_43(ctx):
    with owner(ctx) as cw:
        cw.setup(IAMBIC, 20)
        repeated(ctx, cw, "S 20, dot held", DOT, 690, dash=False)
        cw.idle(0.2)
        repeated(ctx, cw, "S 20, dash held", DASH, 850, dash=True)
        cw.idle(0.2)
        cw.duc(IAMBIC, 60)
        repeated(ctx, cw, "S 60 (unit 20 ms), dot held", DOT, 190, dash=False)
        cw.idle(0.2)
        with Mgmt(ctx.net) as c:
            before = stats(c) or {}
            n0 = max([e[0] for e in read_log(c)], default=-1)
            cw.duc(IAMBIC, 70)  # exactly one DUC-specific packet with S = 70
            cw.speed = 70
            repeated(ctx, cw, "S 70 used as 60, dot held", DOT, 190, dash=False)
            after = stats(c) or {}
            texts = [e[2] for e in read_log(c, n0 + 1)]
        ctx.eq("GET_STATS cw_speed_clamped +1 for one packet with S 70 (item 1)",
               after.get("cw_speed_clamped", 0) - before.get("cw_speed_clamped", 0), 1)
        ctx.check("log: 'CW speed 70 used as 60' (item 1)", any("CW speed 70 used as 60" in t for t in texts),
                  f"{len(texts)} new entries" + (f", e.g. {texts[-1]!r}" if texts else ""))


@test("T-B10-44")
def t_b10_44(ctx):
    with owner(ctx) as cw:
        cw.setup(IAMBIC, 20, 50)
        for w, want in ((66, (79_200, 40_800)), (33, (39_600, 80_400)), (80, (79_200, 40_800)), (0, (60_000, 60_000))):
            cw.duc(IAMBIC, 20, w)
            log = repeated(ctx, cw, f"W {w}", DOT, 500, dash=False)
            mk, sp = lengths(20, w)[0], lengths(20, w)[2]
            ctx.eq(f"W {w}: nominal mark and space from the spec text (us)", (round(mk), round(sp)), want)
            cw.idle(0.2)


def one_dot_then(ctx, cw: Cw, label: str, press_ms: int, release_ms: int):
    """One dot (press 0, release 30: inside the mark), then the dot paddle again from press_ms
    to release_ms."""
    return cw.play([(0, DOT), (30, 0), (press_ms, DOT), (release_ms, 0)], release_ms + 500)


@test("T-B10-45")
def t_b10_45(ctx):
    t = unit_us(20)
    with owner(ctx) as cw:
        cw.setup(IAMBIC | STRICT, 20)
        # strict, pressed at 1.5 T after the end of the first mark and held into the element
        log = one_dot_then(ctx, cw, "strict", 150, 290)
        if ctx.check("strict: two elements", len(log.marks) == 2, f"{len(log.marks)} marks"):
            e1, p2 = log.marks[0][1], log.edge(2)
            guard(ctx, "strict: second press", p2, [e1 + t, e1 + 3 * t], t)
            ctx.check("strict: pressed between 1 T and 3 T after the end of the mark (as planned)", t < p2 - e1 < 3 * t,
                      f"{(p2 - e1) / 1000:.1f} ms")
            gap = log.marks[1][0] - e1
            ctx.check("strict: the element starts 3 T after the end of the mark (item 5)", abs(gap - 3 * t) <= tol(3 * t),
                      f"{gap / 1000:.2f} ms, want {3 * t / 1000:.0f} ms +-{tol(3 * t) / 1000:.1f}")
        cw.idle(0.2)
        # strict, pressed at 1.5 T and released at 2.5 T: inside the wait, nothing starts
        log = one_dot_then(ctx, cw, "strict, released in the wait", 150, 210)
        e1 = log.marks[0][1] if log.marks else 0
        guard(ctx, "strict: release inside the wait", log.edge(3), [e1 + 3 * t], t)
        ctx.eq("strict: pressed and released inside the 3 T wait starts nothing (item 5)", len(log.marks), 1)
        cw.idle(0.2)
        # clear: pressed at 1.5 T, starts at once
        cw.duc(IAMBIC, 20)
        log = one_dot_then(ctx, cw, "clear", 150, 180)
        if ctx.check("clear: two elements", len(log.marks) == 2, f"{len(log.marks)} marks"):
            d = log.marks[1][0] - log.edge(2)
            ctx.check("clear: the element starts at once, within 5 ms of the press (items 4, 5)", 0 <= d <= LATENCY_US,
                      f"{d / 1000:.2f} ms")


@test("T-B10-46")
def t_b10_46(ctx):
    t = unit_us(20)
    with owner(ctx) as cw:
        cw.setup(IAMBIC, 20)
        for mode_b in (False, True):
            m = "B" if mode_b else "A"
            cw.duc(IAMBIC | (MODE_B if mode_b else 0), 20)
            # (a) dash held, dot pressed in the dash mark, both released before its end
            log = cw.play([(0, DASH), (30, DOT | DASH), (120, 0)], 900)
            if log.marks:
                e1 = log.marks[0][1]
                guard(ctx, f"(a) mode {m}: release", log.edge(2), [e1], t)
                ctx.check(f"(a) mode {m}: dot pressed and both released inside the dash mark (as planned)",
                          log.marks[0][0] < log.edge(1) < log.edge(2) < e1, f"edges {[round(e[0]) for e in log.edges]}, mark {log.marks[0]}")
            ctx.eq(f"(a) mode {m}: elements (item 4)", kinds(cw, log), "-." if mode_b else "-")
            if mode_b and len(log.marks) == 2:
                gap = log.marks[1][0] - log.marks[0][1]
                ctx.check("(a) mode B: the dot after the 1 T space", abs(gap - t) <= tol(t), f"{gap / 1000:.2f} ms")
            cw.idle(0.2)
            # (b) dot pressed and released inside the dash mark, dash held to 700 ms
            log = cw.play([(0, DASH), (60, DOT | DASH), (100, DASH), (700, 0)], 1400)
            if log.marks:
                e1 = log.marks[0][1]
                ctx.check(f"(b) mode {m}: dot pressed and released inside the dash mark (as planned)",
                          log.marks[0][0] < log.edge(1) < log.edge(2) < e1, f"edges {[round(e[0]) for e in log.edges]}")
                pts = [e1 + t + k * 4 * t for k in range(4)] + [e1 + t + 2 * t + k * 4 * t for k in range(4)]
                guard(ctx, f"(b) mode {m}: dash release", log.edge(3), pts, t)
            ctx.eq(f"(b) mode {m}: elements (item 4: mode A no memory, mode B the dot from memory)",
                   kinds(cw, log), "-.--" if mode_b else "---")
            cw.idle(0.2)
            # (c) dash released in its mark; dot pressed and released only in the space after it
            log = cw.play([(0, DASH), (100, 0), (195, DOT), (225, 0)], 900)
            if log.marks:
                e1 = log.marks[0][1]
                ctx.check(f"(c) mode {m}: dot pressed and released inside the space (as planned)",
                          e1 < log.edge(2) < log.edge(3) < e1 + t, f"edges {[round(e[0]) for e in log.edges]}, mark end {e1}")
            ctx.eq(f"(c) mode {m}: no dot from a press in the space (item 4)", kinds(cw, log), "-")
            cw.idle(0.2)


@test("T-B10-33")
def t_b10_33(ctx):
    t = unit_us(20)
    with owner(ctx) as cw:
        cw.setup(IAMBIC, 20)
        # iambic, both held (dot first): alternating dots and dashes
        log = cw.play([(0, DOT), (30, DOT | DASH), (800, 0)], 1500)
        k = kinds(cw, log)
        ctx.check("iambic, both held: alternating, dot first", len(k) >= 3 and all(k[i] != k[i + 1] for i in range(len(k) - 1)) and k[0] == ".", k)
        cw.idle(0.2)
        # mode B: the opposite element after a squeeze release
        cw.duc(IAMBIC | MODE_B, 20)
        log = cw.play([(0, DASH), (30, DOT | DASH), (120, 0)], 900)
        ctx.eq("mode B: squeeze released in a dash gives one dot after it", kinds(cw, log), "-.")
        cw.idle(0.2)
        # reverse keys set: changes nothing for CWX (dot bit gives dots)
        cw.duc(IAMBIC | REVERSE, 20)
        log = cw.play([(0, DOT), (290, 0)], 800)
        ctx.check("reverse keys set: the dot bit still gives dots", bool(log.marks) and set(kinds(cw, log)) == {"."}, kinds(cw, log))
        cw.idle(0.2)
        # iambic clear: a straight key, one key-down for as long as held
        cw.duc(0, 20)
        log = cw.play([(0, DOT), (300, 0)], 800)
        if ctx.eq("iambic clear, dot held 300 ms: one key-down", len(log.marks), 1):
            want = log.edge(1) - log.edge(0)
            got = log.mark_lens()[0]
            ctx.check("iambic clear: key-down as long as the bit is held (no element timing)",
                      abs(got - want) <= LATENCY_US, f"key-down {got / 1000:.1f} ms, held {want / 1000:.1f} ms")
        cw.idle(0.2)
        # held over 1 s: unkeyed between 1000 and 1010 ms, reason 9
        log = cw.play([(0, DOT), (1300, 0)], 1600)
        if ctx.eq("dot held 1.3 s: one key-down", len(log.marks), 1):
            d = log.marks[0][1] - log.edge(0)
            ctx.check("cut by the 1 s cap: not before 1000 ms, by 1010 ms (B10.5a)", 1_000_000 <= d <= 1_010_000,
                      f"{d / 1000:.1f} ms after the press")
        ts = value(cw.h.write([(TX_STATE, 0)]), TX_STATE)
        ctx.eq("TX_STATE unkey reason 9 (CW key-down over 1 s)", None if ts is None else (ts >> 16) & 0xFF, 9)
        cw.idle(0.2)
        # speed 0: CWX keys nothing, iambic set or clear
        for opts, name in ((IAMBIC, "iambic set"), (0, "iambic clear")):
            cw.duc(opts, 0)
            log = cw.play([(0, DOT), (300, 0)], 700)
            ctx.eq(f"speed 0, {name}: no key-down", len(log.marks) + (1 if log.open else 0), 0)
            cw.idle(0.1)
