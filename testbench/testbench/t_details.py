"""The management details of 0073 to 0078: the write session (T-A1-15), limits that fall due during
flash work (T-A1-94), GET_STATS (T-A1-95), the calibration set's absence (T-A1-96), READ_LOG's flag
(T-A1-97), an address change (T-A1-98), what of the unexpected stop can be shown without a fault
fixture (T-A3-62), and the gaps of ports 1027 and 1029 (T-B11-04).

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import os
import struct
import time

from .core import Ctx, NotShown, PartSkip, RadioClock, Skip, test
from .mgmt import (FLASH_WINDOW_S, SavedCalibration, Mgmt, build, CMD, config_tlv, parse_log, parse_stats, read_log, req_retransmit,
                   send_retransmit, signed, stats, tlv)
from .p2 import (PORT_DDC, PORT_DISC, PORT_DUC, PORT_DUCIQ, PORT_HP, Host, ddc_specific, duc_specific, general, high_priority,
                 reread_endpoint, wait_reachable)
from .t_mgmt import burst, dhcp_messages, m, mtype, secret_of, simulated_set
from .t_store import get_config, has

MORE = 0x02  # reply flag bit 1, "more blocks follow" (Table A1-1)

# Table M-6, in its order (46 names, 0080)
COUNTERS = """mg_not_request mg_rate_drop mg_cap_drop mg_auth_required disc_rate_drop disc_src1024 setip_ignored gen1024_ignored
cap_rate_drop notowner_drop reg7250_drop takeovers icmp_unreach hp_rx hp_stale hp_resync hp_release gap_1027 gap_1029
duc_foreign duc_not_tx duc_cw_drop duc_fifo_drop cw_speed_clamped cw_hang_clamped ll_unprobed drop_1024 drop_1025
drop_1026 drop_1027 drop_1029 drop_7250 drop_7251 fault_2 fault_3 fault_9 fault_10 fault_16 fault_17 fault_18 fault_19
reset_unknown reset_software reset_watchdog reset_pin trip_count""".split()
# what reaching the radio after a power-on may itself have counted, so 0 is not asked of it
MAY_COUNT = {"mg_rate_drop", "mg_cap_drop", "disc_rate_drop", "drop_1024", "drop_7251", "ll_unprobed", "icmp_unreach"}


class Clock:
    """Seconds of radio time, and the wait that passes them (time.advance where offered). On the
    reference model RadioClock is the wall clock, which time.advance does not move."""

    def __init__(self, ctx):
        self.ctx, self.c, self.add = ctx, RadioClock(ctx), 0.0

    def now(self) -> float:
        return self.c.now() + self.add

    def wait(self, s: float):
        if s <= 0:
            return
        if has(self.ctx, "time.advance"):
            from .t_update import advance
            advance(self.ctx, s)
            if not self.c.emu:
                self.add += s
            return
        t0 = self.now()
        while self.now() - t0 < s:
            time.sleep(min(0.2, max(0.01, s - (self.now() - t0))))


def counter_list(payload: bytes) -> list[tuple[str, int]]:
    """GET_STATS in the order of the reply (a name listed twice stays twice)."""
    out, i = [], 0
    while i < len(payload):
        n = payload[i]
        out.append((payload[i + 1:i + 1 + n].decode("ascii", "replace"), int.from_bytes(payload[i + 1 + n:i + 9 + n], "big")))
        i += 9 + n
    return out


def judge_counters(ctx, payload: bytes):
    """Table M-1, Table M-6: the names, their order and the size of the reply."""
    got = counter_list(payload)
    names = [n for n, _ in got]
    ctx.check("one reply of at most 1400 bytes", 18 + len(payload) <= 1400, f"{18 + len(payload)} bytes")
    missing = [n for n in COUNTERS if n not in names]
    ctx.check("every name of Table M-6 is listed", not missing, f"missing {missing}")
    ctx.eq("they come first, in the order of the table", names[:len(COUNTERS)], COUNTERS)
    ctx.all_of("every name has at most 16 ASCII characters", names, lambda n: 1 <= len(n) <= 16 and n.isascii() and "�" not in n)
    ctx.eq("no name listed twice", len(names), len(set(names)))
    return dict(got)


# ---------------- T-A1-15 ----------------
@test("T-A1-15", whole=("flash.timing", "flash.log"))
def t_a1_15(ctx: Ctx):
    """A1.10 "One writer at a time" (0022 §75, 0074, 0077). A wait is judged only where the radio
    time that passed shows the case: "still held" under 60 s from before the holder's last request,
    "ended" more than 60 s and the 100 ms of 0073 after its reply."""
    s = secret_of(ctx)
    ck = Clock(ctx)
    with m(ctx) as a, m(ctx) as b:
        a.pace = b.pace = 18
        # the allow-list (C-7, tag 11) as it is: a write with no C-3 item, which needs no compaction (0072)
        allow = get_config(a).get(11)
        if not ctx.check("GET_CONFIG tag 11 (the value the writes repeat)", allow is not None, f"tags {sorted(get_config(a))}"):
            return

        def st(c, cmd, payload=b"", **kw):
            r = c.authed(cmd, s, payload, **kw)
            return r["status"] if r else None

        wr = lambda c: st(c, "SET_CONFIG", config_tlv((11, allow)))
        end = lambda c: st(c, "END_WRITE_SESSION")

        def held(label: str, t_sent: float):
            got, dt = wr(b), ck.now() - t_sent
            if dt < 60:
                ctx.eq(f"{label}: host B's write gets 35", got, 35)
            else:
                ctx.part(f"{label}: not judged, {dt:.1f} s of radio time passed")

        if wr(a) == 35:  # another test's session: it ends by itself
            ck.wait(61)
        t = ck.now()
        if not ctx.eq("host A's write: 0", wr(a), 0):
            return
        held("A holds the session", t)
        for cmd in ("GET_STATS", "IDENTIFY", "GET_CONFIG"):
            r = req_retransmit(b, cmd)
            ctx.eq(f"host B's {cmd} while A holds the session: 0", r["status"] if r else None, 0)
        # A's verified request refreshes
        ck.wait(45)
        t = ck.now()
        ctx.eq("A's write 45 s later: 0", wr(a), 0)
        ck.wait(45)
        held("A's verified request 45 s ago, its first write more than 90 s ago", t)
        # nothing else does
        t = ck.now()
        ctx.eq("A's write: 0", wr(a), 0)
        t_reply = ck.now()
        for i, at in enumerate((28, 55)):
            ck.wait(at - (ck.now() - t))
            for cmd in ("GET_STATS", "IDENTIFY", "GET_CHALLENGE", "UPDATE_STATUS"):
                req_retransmit(a, cmd)
            ctx.eq(f"A's request with a wrong HMAC ({i + 1}): 33", st(a, "SET_CONFIG", config_tlv((11, allow)), mangle=True), 33)
            if i == 0:
                held("28 s after A's write", t)
        ck.wait(60.5 - (ck.now() - t_reply))
        ctx.eq("more than 60 s after A's last verified request, its reads and its wrong HMAC in between: B's write 0", wr(b), 0)
        ctx.eq("B's END_WRITE_SESSION: 0", end(b), 0)
        # 0074: opening, END_WRITE_SESSION
        t = ck.now()
        ctx.eq("A's SET_CONFIG with an invalid address mode: 48", st(a, "SET_CONFIG", config_tlv((1, b"\x02"))), 48)
        held("after A's 48", t)
        t = ck.now()
        ctx.eq("A's END_WRITE_SESSION: 0", end(a), 0)
        ctx.eq("then B's write: 0", wr(b), 0)
        ctx.eq("B's END_WRITE_SESSION: 0", end(b), 0)
        ctx.eq("END_WRITE_SESSION with no session held: 0", end(a), 0)
        t = ck.now()
        ctx.eq("it opened none: another host's write straight after it: 0", wr(b), 0)
        ctx.eq("END_WRITE_SESSION from a host that does not hold the session: 35", end(a), 35)
        got, dt = wr(a), ck.now() - t
        if dt < 60:
            ctx.eq("and the holder's session stays: A's write 35", got, 35)
        ctx.eq("the holder's END_WRITE_SESSION: 0", end(b), 0)
        idle_pause(ctx, ck, a, b, s, end, config_tlv((11, allow)))


def c3_rewrite(conf: dict) -> bytes | None:
    """A SET_CONFIG payload of the TX time (a C-3 item) at its current value: by 0072 it is written by a compaction
    whether or not the value changes (A1.6 C-3), so it is a request with one store erase. None without tag 7."""
    tx = conf.get(7)
    return config_tlv((7, tx)) if tx is not None and len(tx) == 4 else None


def over_60(ctx, ck, a, b, s: bytes, value: bytes, label: str, wait_s: float, want: int, prepare, hit) -> bool:
    """0077, 0078: the holder's SET_CONFIG `wait_s` after its last request, its flash work over the 60 s:
    it is answered `want`, and host B's write straight after the reply gets 35. `prepare` runs before
    it; False when `hit(status)` says that it did not do the flash work meant."""
    t0 = ck.now()
    got = (a.authed("SET_CONFIG", s, config_tlv((1, b"\x02"))) or {}).get("status")
    t1 = ck.now()
    if not ctx.eq(f"{label}: host A's request before it, refused, holds the session: 48", got, 48):
        return True
    ck.wait(wait_s - (ck.now() - t0))
    prepare()
    data = signed(a, "SET_CONFIG", s, value)
    if data is None:
        ctx.part(f"{label}: not sent: {a.why}")
        return True
    t2 = ck.now()
    r = send_retransmit(a, data, FLASH_WINDOW_S, "SET_CONFIG")
    t3 = ck.now()
    other = (b.authed("SET_CONFIG", s, value) or {}).get("status")
    st = r["status"] if r else None
    if not hit(st):
        return False
    ctx.eq(f"{label}: host A's SET_CONFIG", st, want)
    if t2 - t0 < 60 < t3 - t1:
        ctx.check(f"{label}: host B's write straight after the reply gets 35", other == 35,
                  f"got {other}; sent {t2 - t0:.1f} s after the last request, answered {t3 - t1:.1f} s after its reply")
    else:
        ctx.part(f"{label}: the 60 s not inside the flash work: sent {t2 - t0:.1f} s after the last request, answered "
                 f"{t3 - t1:.1f} s after its reply")
    return True


def idle_pause(ctx, ck, a, b, s, end, tx):
    """The pause of the 60 s idle time during the holder's own request (A1.10; 0077, 0078)."""
    if has(ctx, "flash.delay"):  # the reference model: the flash work of one request takes 8 s
        over_60(ctx, ck, a, b, s, tx, "8 s of flash work from 57 s", 57, 0,
                lambda: ctx.fixture("flash.delay", us=8_000_000, count=1), lambda st: True)
        ctx.eq("A's END_WRITE_SESSION: 0", end(a), 0)
        ctx.part("the stretched compaction and the erase that ends late (emulator) not run on the model: flash.delay stands for it")
        return
    if not (has(ctx, "flash.timing") and has(ctx, "flash.log")):
        ctx.part("the stretched compaction (0077) not run: no flash.timing or flash.log")
        return
    from .t_flash import PROGRAM_MAX_US, in_range, is_erase, flog, flog_start, store_range, stretch
    from .t_update import restart
    from .update import Updater
    ft = ctx.endpoint.get("flash_timing") or {}
    if not ft.get("t_erase_max_us"):
        ctx.part("the stretched compaction (0077) not run: the endpoint gives no flash_timing.t_erase_max_us")
        return
    u = Updater(a, s)
    lo, hi = store_range(u)
    write = c3_rewrite(get_config(a))
    if write is None:
        ctx.part("the flash-work parts: not run, GET_CONFIG gives no TX time")
        return

    def case(label: str, wait_s: float, want: int, prepare, hit) -> bool:
        """A C-3 write (always one compaction, one store erase; 0072), up to 5 tries until that erase was in
        the request."""
        back = stretch(ctx, ft["t_erase_max_us"], ft.get("t_program_max_us") or PROGRAM_MAX_US)
        try:
            for _ in range(5):
                if over_60(ctx, ck, a, b, s, write, label, wait_s, want, prepare, hit):
                    return True
                prepare = flog_start_only
        finally:
            back()
        ctx.part(f"{label}: not shown, no store erase in 5 C-3 writes")
        return False

    def flog_start_only():
        flog_start(ctx)

    def hang():
        from .t_flash import late_erase
        flog_start(ctx)
        if not late_erase(ctx):  # 0081: an erase that ends without a flag 100 ms after the give-up
            raise PartSkip("an erase that ends late, from 55 s: not run (flash.fault end_after_us not offered)")

    failed = False
    try:
        label = "a compaction at the maximum from 57 s"
        case(label, 57, 0, flog_start_only, lambda st: any(is_erase(e) and in_range(e, lo, hi) for e in flog(ctx)))
        label = "an erase that ends late (100 ms after the give-up), from 55 s"
        if not has(ctx, "flash.fault"):
            ctx.part(f"{label}: not run: no flash.fault")
            return
        failed = True  # the fault is armed, and INTERNAL_ERROR holds until a reset
        case(label, 55, 55, hang, lambda st: st != 0)
    finally:
        if failed:
            restart(ctx, u)
        end(a)


# ---------------- T-A1-94 ----------------
def mgmt_times(log: list[dict], port: int, d: str) -> list[int]:
    """net.log: emulated times of the datagrams of one direction between port 7251 and the bench's port."""
    return [e["t_us"] for e in log if e.get("dir") == d and e.get("radio_port") == 7251 and str(e.get("peer", "")).endswith(f":{port}")]


def due_during(arrive: int | None, reply: int | None, boot: int, due: int):
    """What a limit that fell due at `due` shows around one request with flash work (all in us, one
    clock): (shown, kept, detail). Shown: the request came before it and the reply after it. Kept:
    the reset began no more than 100 ms after the reply (0073)."""
    if arrive is None or reply is None:
        return False, False, f"request {arrive}, reply {reply} in net.log"
    detail = (f"due {(due - arrive) / 1e6:.3f} s after the request arrived, the reply {(reply - arrive) / 1e6:.3f} s after it, "
              f"the reset {(boot - reply) / 1e6:.3f} s after the reply")
    return arrive <= due <= reply, 0 <= boot - reply <= 101_000, detail


def send_judged(ctx, u, data: bytes, what: str, due: int):
    """Send the signed request once, wait for its reply (or the reset), and judge the limit at `due`;
    the reply's status or None."""
    from .t_a2 import netlog_read
    from .t_a3 import boot_t_us
    before = boot_t_us(ctx)
    ctx.fixture("net.log", action="start")
    u.c.post(data)
    got, w0, t0 = [], time.monotonic(), u.c.clock.now()
    while not got and time.monotonic() - w0 < 180 and u.c.clock.now() - t0 < 20:
        got = u.c.listen(data, 0.5)
    log = netlog_read(ctx)
    arrive, reply = mgmt_times(log, u.c.port, "to_radio"), mgmt_times(log, u.c.port, "from_radio")
    time.sleep(0.3)
    wait_reachable(ctx, what)
    boot = boot_t_us(ctx)
    if not ctx.check(f"{what}: a reset followed", boot > before, f"boot event at {boot} us, {before} us before"):
        return got[0]["status"] if got else None
    shown, kept, detail = due_during(arrive[0] if arrive else None, reply[0] if reply else None, boot, due)
    if shown:
        ctx.check(f"{what}: the reset no later than 100 ms after the reply", kept, detail)
    else:
        ctx.part(f"{what}: the limit did not fall due between the request and its reply: {detail}")
    return got[0]["status"] if got else None


@test("T-A1-94", whole=True)
def t_a1_94(ctx: Ctx):
    """B2.3 "time over flash work" (0073), with every erase stretched to t_erase_max."""
    for fx in ("flash.timing", "flash.log", "flash.fault", "time.advance", "net.log", "gpio.force", "hw.observe"):
        ctx.need(fx)
    t_max = (ctx.endpoint.get("flash_timing") or {}).get("t_erase_max_us")
    if not t_max:
        raise Skip("the endpoint gives no flash_timing.t_erase_max_us")
    if not ctx.trace_file:
        raise Skip("needs hw.trace (the boot event times the reset)")
    s = secret_of(ctx)
    due_trial(ctx, s, t_max)
    due_abandon(ctx, s, t_max)


def due_trial(ctx, s: bytes, t_max: int):
    from .t_a3 import boot_t_us
    from .t_flash import PROGRAM_MAX_US, flog, flog_start, in_range, is_erase, store_range, stretch
    from .t_log import entries, judge_run, letter, newest
    from .t_update import activate_new, advance, updating
    with updating(ctx, s, restart_at_end=False) as u:
        lo, hi = store_range(u)
        write = c3_rewrite(get_config(u.c))  # a C-3 write: always one compaction (0072)
    if write is None:
        ctx.part("trial: not run, GET_CONFIG gives no TX time")
        return
    a = activate_new(ctx, s, 940)
    if not ctx.check("trial: an update activated", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    start = boot_t_us(ctx)
    ctx.fixture("gpio.force", pin="PF4", level=0, target="idr")  # a class 2 fault: no confirmation (0044)
    back = stretch(ctx, t_max, (ctx.endpoint.get("flash_timing") or {}).get("t_program_max_us") or PROGRAM_MAX_US)
    try:
        with updating(ctx, s, restart_at_end=False) as u:
            n0 = newest(ctx)
            now = (ctx.fixture("hw.observe") or {}).get("t_us", 0)
            at = start + 60_000_000 - 2_000_000  # so that the limit falls due during its flash work (0078)
            if now > at - 8_000_000:
                ctx.part(f"trial: the compaction could not be placed at the limit: {(at - now) / 1e6:.1f} s left before it")
            else:
                advance(ctx, (at - now) / 1e6 - 5)
                ctx.eq("trial: not confirmed (tag 10 = 1)", u.identify().get(10), b"\x01")
                logged = has(ctx, "flash.log")
                if logged:
                    flog_start(ctx)
                data = signed(u.c, "SET_CONFIG", s, write)
                now = (ctx.fixture("hw.observe") or {}).get("t_us", 0)
                if data is not None and now < at:
                    ctx.fixture("time.wait", _timeout=120, us=at - now) if has(ctx, "time.wait") else advance(ctx, (at - now) / 1e6)
                st = send_judged(ctx, u, data, "trial: a SET_CONFIG with a stretched compaction at the 60 s limit", start + 60_000_000)
                ctx.eq("trial: the SET_CONFIG is answered 0", st, 0)
                if logged:
                    ctx.check("trial: that SET_CONFIG erased a store sector (its compaction, flash.log)",
                              any(is_erase(e) and in_range(e, lo, hi) for e in flog(ctx)), "no store erase")
                t = u.identify()
                ctx.eq("trial: the next start rolls back (tag 10 = 2, the previous slot)", (t.get(10), t.get(23, b"")[:1]),
                       (b"\x02", bytes([a[0]])))
                judge_run(ctx, "trial", entries(ctx), n0, "restart: trial-unconfirmed",
                          boot=(f"boot: rollback to slot {letter(a[0])} (trial not confirmed)",))
    finally:
        back()
        ctx.fixture("gpio.force", pin="PF4", level=None, target="idr")
        with updating(ctx, s) as u:  # the RESTART also clears the class 2 fault
            pass


def due_abandon(ctx, s: bytes, t_max: int):
    """An UPDATE_BLOCK whose erase ends without a flag 100 ms after the give-up at two t_erase_max (0081), sent 9 min 55 s
    after the last update command. The second sector's erase: the block that begins it."""
    from .t_a2 import netlog_read
    from .t_flash import stretch
    from .t_log import entries, judge_run, newest
    from .t_update import advance, facts, newer, pkg, slot_bytes, updating
    from .update import SECTOR, block_payload, image
    back = stretch(ctx, t_max)
    try:
        with updating(ctx, s, restart_at_end=False) as u:
            run, other, ver, lay = facts(ctx, u)
            if slot_bytes(lay, other) < 2 * SECTOR:
                ctx.part("abandonment: the slot has one sector only, no block of a transfer begins another")
                return
            img = image(SECTOR + 1024, seed=941)
            r = u.begin(pkg(img, newer(ver), other))
            if not ctx.eq("abandonment: UPDATE_BEGIN accepted", r["status"] if r else None, 0):
                return
            st, n = u.blocks(img[:SECTOR - 512])
            if not ctx.eq("abandonment: the first blocks", st, 0):
                return
            ctx.fixture("net.log", action="start")
            k = SECTOR // 512 - 1
            r = u.req("UPDATE_BLOCK", block_payload(k, img[512 * k:512 * k + 512]))
            if not ctx.eq("abandonment: the last block of the first sector", r["status"] if r else None, 0):
                return
            last = mgmt_times(netlog_read(ctx), u.c.port, "from_radio")
            if not last:
                ctx.part("abandonment: the last reply is not in net.log")
                return
            n0 = newest(ctx)
            at = last[-1] + 595_000_000
            now = (ctx.fixture("hw.observe") or {}).get("t_us", 0)
            advance(ctx, (at - now) / 1e6 - 5)
            from .t_flash import late_erase
            if not late_erase(ctx):  # 0081: the erase ends without a flag 100 ms after the give-up
                return
            k += 1
            data = signed(u.c, "UPDATE_BLOCK", s, block_payload(k, img[512 * k:512 * k + 512]))
            now = (ctx.fixture("hw.observe") or {}).get("t_us", 0)
            if data is None or now > at + 4_000_000:
                ctx.part(f"abandonment: the block could not be placed: {u.c.why}")
                return
            if now < at:
                advance(ctx, (at - now) / 1e6)
            st = send_judged(ctx, u, data, "abandonment: an UPDATE_BLOCK whose erase ends late", last[-1] + 600_000_000)
            if st == 0:
                ctx.part("abandonment: the block was answered 0, its request did no erase")
                return
            ctx.eq("abandonment: the block gets 55", st, 55)
            judge_run(ctx, "abandonment", entries(ctx), n0, "restart: update-abandoned")
    finally:
        back()


# ---------------- T-A1-95 ----------------
@test("T-A1-95", whole=True)
def t_a1_95(ctx: Ctx):
    """GET_STATS (Table M-1, Table M-6; 0074)."""
    cycled = has(ctx, "power.cycle")
    if cycled:  # every counter from 0, those kept over a reset too (A3.5)
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "a power cycle")
    with m(ctx) as c:
        r = ctx.got(req_retransmit(c, "GET_STATS"), "GET_STATS", c.why)
        got = judge_counters(ctx, r["payload"])
        if cycled:
            from .t_flash import fault_flags
            flags = fault_flags(ctx)
            no_fault = flags == 0
            up = [n for n in COUNTERS if got.get(n) and n not in MAY_COUNT and (no_fault or not n.startswith("fault_"))]
            ctx.check("after a power-on each is 0 where nothing has been counted", not up, f"{ {n: got[n] for n in up} }")
            if not no_fault:
                ctx.notes.append(f"the fault counters not judged at 0: FAULT_FLAGS {flags}")
        else:
            ctx.part("'each at 0 where nothing has been counted' not judged: no power.cycle")
        with Host(ctx.net) as h:
            before = got.get("hp_rx", 0)
            h.stop()  # one High Priority packet, run = 0
        after, w0, t0 = None, time.monotonic(), c.clock.now()
        while time.monotonic() - w0 < 60 and c.clock.now() - t0 < 3:
            after = (stats(c) or {}).get("hp_rx")
            if after is not None and after != before:
                break
        ctx.eq(f"after one High Priority packet: hp_rx {'= 1' if cycled else '+1'}", after, 1 if cycled else before + 1)
        foreign_iq(ctx, c)
    burst(ctx, 25)


def counted(c, done, limit_s: float = 3):
    """GET_STATS until done(counters) or limit_s of radio time: the last one read."""
    got, w0, t0 = {}, time.monotonic(), c.clock.now()
    while time.monotonic() - w0 < 60 and c.clock.now() - t0 < limit_s:
        got = stats(c) or got
        if done(got):
            break
    return got


def foreign_iq(ctx, c):
    """One DUC I&Q packet from a source that does not own the session: counted twice (B11.1; 0078)."""
    with Host(ctx.net) as h, Host(ctx.net) as other:
        h.start()
        before = ctx.got(stats(c), "GET_STATS")
        other.send(PORT_DUCIQ, bytes(1444))
        after = counted(c, lambda g: g.get("duc_foreign", 0) > before.get("duc_foreign", 0))
        h.stop()
    for n in ("duc_foreign", "drop_1029"):
        ctx.eq(f"one DUC I&Q packet from another source: {n} rises by 1", after.get(n, 0) - before.get(n, 0), 1)


# ---------------- T-A1-96 ----------------
def tlv_tags(p: bytes) -> list[int]:
    """The tags of a Table M-3 TLV list, in order."""
    tags, i = [], 0
    while i + 2 <= len(p):
        tags.append(p[i])
        i += 2 + p[i + 1]
    return tags


@test("T-A1-96", whole=True)
def t_a1_96(ctx: Ctx):
    """GET_CONFIG and GET_CALIBRATION without and with a set that holds S2 and T3 (Table M-1, M-3; 0074)."""
    s = secret_of(ctx)
    with m(ctx) as c:
        saved = SavedCalibration(ctx, c)
        try:
            if not saved.empty:
                r = c.authed("CLEAR_CALIBRATION", s)
                if not ctx.eq("CLEAR_CALIBRATION: 0", r["status"] if r else None, 0):
                    return
            conf = get_config(c)
            ctx.check("GET_CONFIG answered", 7 in conf, f"tags {sorted(conf)}")
            ctx.check("with no calibration set GET_CONFIG holds no tag 9 or 10", 9 not in conf and 10 not in conf, f"tags {sorted(conf)}")
            if 7 in conf:  # 0079: the reply of a SET_CONFIG as GET_CONFIG gives it
                r = c.authed("SET_CONFIG", s, config_tlv((7, conf[7])))
                if ctx.eq("SET_CONFIG of the TX time as it is: 0", r["status"] if r else None, 0):
                    tags = tlv_tags(r["payload"])
                    ctx.check("with no calibration set the SET_CONFIG reply holds no tag 9 or 10 either (Table M-1; 0079)",
                              9 not in tags and 10 not in tags and 7 in tags, f"tags {tags}")
            g = ctx.got(c.req("GET_CALIBRATION"), "GET_CALIBRATION")
            ctx.eq("GET_CALIBRATION: 37 bytes, id zeros, date 0, source length 0, no entries", (g["status"], g["payload"]), (0, bytes(37)))
            r = c.authed("SET_CALIBRATION", s, simulated_set())
            if not r or r["status"] != 0:
                ctx.part(f"with a set holding S2 and T3: the simulated set is refused ({r['status'] if r else None})")
                return
            conf = get_config(c)
            ctx.check("with a set holding S2 and T3 both tags are there, 2 bytes each, not 0",
                      all(len(conf.get(t, b"")) == 2 and conf[t] != b"\0\0" for t in (9, 10)), f"{conf.get(9)!r}, {conf.get(10)!r}")
        finally:
            saved.restore(s)


# ---------------- T-A1-97 ----------------
def judge_pages(pages: list[tuple[int, list]]) -> tuple[bool, str]:
    """(flags, entries) of successive READ_LOG replies, the last one empty: bit 1 set on every page
    with entries but the last such page, clear on that one and on the empty page."""
    full = [f for f, e in pages[:-1]]
    want = [True] * (len(full) - 1) + [False] * min(1, len(full))
    got = [bool(f & MORE) for f in full]
    ok = got == want and not pages[-1][0] & MORE and not pages[-1][1]
    return ok, f"bit 1 on the pages with entries {got}, on the empty page {bool(pages[-1][0] & MORE)}"


@test("T-A1-97", whole=True)
def t_a1_97(ctx: Ctx):
    """A1.7 (0074): READ_LOG sets flag bit 1 while entries after the page exist."""
    s = secret_of(ctx)
    with m(ctx) as c:
        c.pace = 18
        tx0 = int.from_bytes(get_config(c).get(7, b""), "big")
        wrote = [0]

        def write(v):
            wrote[0] += 1
            r = c.authed("SET_CONFIG", s, config_tlv((7, struct.pack(">I", v))))
            return r["status"] if r else None

        def page(first: int):
            r = ctx.got(req_retransmit(c, "READ_LOG", first.to_bytes(4, "big")), "READ_LOG", c.why)
            return r["flags"], parse_log(r["payload"])

        try:
            for i in range(120):  # a log of more than one page: each write changes the TX time and is logged
                first = page(0)
                if first[1] and page(first[1][-1][0] + 1)[1]:
                    break
                if write(200_000 + i % 2) != 0:
                    break
            for _ in range(3):  # an entry logged by the radio meanwhile: read again
                pages, at = [], 0
                while True:
                    f, e = page(at)
                    pages.append((f, e))
                    if not e:
                        break
                    at = e[-1][0] + 1
                ok, detail = judge_pages(pages)
                if ok:
                    break
            if len(pages) < 3:
                raise NotShown(f"the log is one page only after {wrote[0]} writes")
            ctx.check("flag bit 1 set on every page but the last, clear on the last and on an empty page", ok, f"{detail}; {wrote[0]} writes")
            if ctx.eq("a new entry made by a write: 0", write(200_002), 0):
                f, e = page(at)
                ctx.check("asking from the last sequence number + 1 returns it", bool(e) and e[0][0] == at, f"asked from {at}, got {e[:2]}")
            replies = {cmd: req_retransmit(c, cmd) for cmd in ("IDENTIFY", "GET_STATS", "GET_CONFIG", "GET_CALIBRATION", "GET_CHALLENGE",
                                                               "UPDATE_STATUS")}
            replies["SET_CONFIG"] = c.authed("SET_CONFIG", s, config_tlv((7, struct.pack(">I", tx0))))
            replies["SET_CONFIG without HMAC"] = c.req("SET_CONFIG", b"")
            replies["END_WRITE_SESSION"] = c.authed("END_WRITE_SESSION", s)
            none = [k for k, r in replies.items() if r is None]
            ctx.check("every one of the other requests answered", not none, f"no reply to {none}")
            setb = [k for k, r in replies.items() if r and r["flags"] & MORE]
            ctx.check("bit 1 is 0 in every other reply", not setb, f"set in {setb}")
        finally:
            if tx0:
                write(tx0)
            c.authed("END_WRITE_SESSION", s)


# ---------------- T-A1-98 ----------------
def other_address(ep: dict, ip: str, mask: str) -> bytes:
    """Another host address in the radio's subnet that no host of the endpoint has."""
    n = lambda a: int.from_bytes(bytes(int(x) for x in a.split(".")), "big")
    used = {n(ip)} | {n(h["ip"]) for h in ep.get("hosts", [])}
    net, mk = n(ip) & n(mask), n(mask)
    for k in range(1, 255):
        a = (n(ip) & 0xFFFFFF00 | (n(ip) + k) & 0xFF)
        if a not in used and a & mk == net and a != net and a != net | (~mk & 0xFFFFFFFF) and a & 0xFF not in (0, 1, 254, 255):
            return a.to_bytes(4, "big")
    raise Skip("no free address in the radio's subnet")


@test("T-A1-98", whole=True)
def t_a1_98(ctx: Ctx):
    """A1.10 (0074): an address change ends the write session and discards challenges and stored
    replies; a renewal with the same address changes nothing."""
    if ctx.kind not in ("refmodel", "emulator"):
        raise Skip("moves the radio to another address: only on the reference model or an emulator")
    model = ctx.kind == "refmodel"
    if not model:
        ctx.need("net.endpoint")  # hosts[] follow the radio
    s = secret_of(ctx)
    ip, mask = ctx.endpoint["radio"]["ip"], ctx.endpoint["radio"].get("netmask", "255.255.0.0")
    b4 = lambda a: bytes(int(x) for x in a.split("."))
    new = other_address(ctx.endpoint, ip, mask)
    ck = Clock(ctx)

    def moved(what):
        if not model:  # the model's sockets stay; its management channel sees the new address
            time.sleep(0.3)
            wait_reachable(ctx, what)
            ctx.eq(f"{what}: the radio's address", ctx.endpoint["radio"]["ip"], ".".join(str(x) for x in want[0]))

    with m(ctx) as a, m(ctx) as b:
        pa, pb = a.port, b.port
        conf = get_config(a)
        tx = conf.get(7, b"")
        old_net = [(t, conf[t]) for t in (1, 2, 3, 4) if t in conf]
        wr = lambda c, ch=None: (c.authed("SET_CONFIG", s, config_tlv((7, tx)), challenge=ch) or {}).get("status")
        if wr(a) == 35:
            ck.wait(61)
        if not ctx.eq("host A's write opens the session: 0", wr(a), 0):
            return
        n0 = max([e[0] for e in read_log(a)], default=-1)
        ctx.eq("host B's write: 35", wr(b), 35)
        unused = b.challenge()
        ctx.check("a challenge for B, not used", unused is not None, b.why)
        ch = a.challenge()
        want = [new]
        t0 = ck.now()
        r = a.authed("SET_CONFIG", s, config_tlv((1, b"\x01"), (2, new), (3, b4(mask)), (4, b4(ip))), challenge=ch)
        ctx.eq("SET_CONFIG with a new static address: answered 0 first", r["status"] if r else None, 0)
        if not r or r["status"] != 0:
            return
    moved("the address change")
    try:
        with rebound(ctx, pa) as a, rebound(ctx, pb) as b:
            if (a.port, b.port) != (pa, pb):
                ctx.notes.append("the hosts have other source ports after the change: 'the former holder' is judged by its address only")
            ctx.eq("the former holder's write with the old challenge: 33", wr(a, ch), 33)
            if unused is not None:
                ctx.eq("host B's write with its challenge from before the change: 33", wr(b, unused), 33)
                if ck.now() - t0 > 9:
                    ctx.notes.append(f"{ck.now() - t0:.1f} s of radio time passed: B's challenge was over by its 10 s as well")
            ctx.check("the change of address is logged", any(e[0] > n0 for e in read_log(b)), "no new entry in READ_LOG")
            ctx.eq("a write from another host is not refused with 35: 0", wr(b), 0)
            ctx.eq("END_WRITE_SESSION: 0", (b.authed("END_WRITE_SESSION", s) or {}).get("status"), 0)
            ctx.eq("the former holder with a new challenge: 0", wr(a), 0)
            ctx.eq("and that opened a new session: B's write 35", wr(b), 35)
            want = [b4(ip)]
            r = a.authed("SET_CONFIG", s, config_tlv(*old_net))
            ctx.eq("the network items put back: 0", r["status"] if r else None, 0)
    finally:
        if not model:
            time.sleep(0.3)
            wait_reachable(ctx, "the address put back")
    with m(ctx) as a:
        ctx.eq("GET_CONFIG as before (tags 1 to 4)", [(t, get_config(a).get(t)) for t, _ in old_net], old_net)
        a.authed("END_WRITE_SESSION", s)
    if not has(ctx, "dhcp.server"):
        raise PartSkip("a DHCP renewal with the same address: needs dhcp.server (the reference model has no DHCP)")
    renewal(ctx, s)


def rebound(ctx, port: int) -> Mgmt:
    """A management socket on the same source port where that is possible."""
    try:
        c = Mgmt(ctx.net, port=port)
    except Skip:
        c = Mgmt(ctx.net)
    c.clock = RadioClock(ctx)
    return c


def renewals(log: list[dict], ip: str) -> list[int]:
    """t_us of the DHCPREQUESTs the radio sends from its lease (a renewal, RFC 2131 4.3.2)."""
    return sorted(x["t_us"] for x in log if mtype(x) == "REQUEST" and str(x.get("src", "")).split(":")[0] == ip
                  and not x.get("t_us_approx"))


def renewal(ctx, s: bytes):
    """Lease time 16 s (renewal after 8 s, RFC 2131 4.4.5): a session, a challenge and a stored reply from
    just before a renewal are as they were after it."""
    ctx.need("power.cycle")
    ctx.fixture("dhcp.start", lease_s=16)
    try:
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "a lease of 16 s", 20)
        ip = ctx.endpoint["radio"]["ip"]
        clock = RadioClock(ctx)
        now = lambda: (ctx.fixture("hw.observe") or {}).get("t_us", 0)

        def after(t: int, limit_s: float):
            """The first renewal later than t, or None."""
            t0, w0 = clock.now(), time.monotonic()
            while clock.now() - t0 < limit_s and time.monotonic() - w0 < 12 * limit_s + 60:
                r = [x for x in renewals(dhcp_messages(ctx), ip) if x > t]
                if r:
                    return r[0]
                time.sleep(0.1)
            return None

        r1 = after(0, 20)
        r2 = after(r1, 20) if r1 is not None else None
        if r2 is None:
            raise PartSkip(f"a DHCP renewal with the same address: no two renewals from {ip} in dhcp.log")
        with m(ctx) as a, m(ctx) as b:
            tx = get_config(a).get(7, b"")
            wr = lambda c, ch=None: (c.authed("SET_CONFIG", s, config_tlv((7, tx)), challenge=ch) or {}).get("status")
            due = r2 + (r2 - r1)
            while now() < due - 4_000_000:
                time.sleep(0.05)
            ctx.eq("renewal: A's write opens the session: 0", wr(a), 0)
            req = build(CMD["GET_CHALLENGE"], b"", a.next_id())
            while now() < due - 1_500_000:
                time.sleep(0.05)
            t_ch = now()
            got = a.raw(req, 1.0)
            r3 = after(t_ch, 8)
            time.sleep(0.2)
            again = a.raw(req, 1.0)
            t_again = now()
            if not got or r3 is None or t_again - t_ch > 4_800_000:
                raise PartSkip(f"a DHCP renewal with the same address: none seen within 5 s of a stored reply (challenge at {t_ch}, "
                               f"renewal {r3}, asked again at {t_again} us)")
            ctx.eq("renewal: the stored reply is as it was (the same challenge)", again[0]["payload"] if again else None, got[0]["payload"])
            ctx.eq("renewal: the write session is as it was: B's write 35", wr(b), 35)
            ctx.eq("renewal: the challenge from before it is good: 0", wr(a, got[0]["payload"]), 0)
            ctx.eq("the address did not change", ctx.endpoint["radio"]["ip"], ip)
            a.authed("END_WRITE_SESSION", s)
    finally:
        ctx.fixture("dhcp.stop")


# ---------------- T-A3-62 ----------------
STOP_LINE = "start: unexpected stop"


REASON = "restart: restart"


def until_dropped(ctx, u, n_max: int | None = None):
    """CLEAR_LOG, then entries (UPDATE_ABORT with no update state, 0074) until the oldest one of the ring
    has been dropped, or `n_max` of them: (entries made, oldest dropped), None with the reason in ctx._fill."""
    ctx.__dict__["_fill"] = ""
    r = u.req("CLEAR_LOG")
    if not r or r["status"] != 0:
        ctx.__dict__["_fill"] = f"CLEAR_LOG answered {r['status'] if r else None}"
        return None
    with m(ctx) as c:  # another source port, with a limit of its own
        c.pace = u.c.pace = 18

        def oldest():
            r = req_retransmit(c, "READ_LOG", b"\0\0\0\0")
            e = parse_log(r["payload"]) if r and r["status"] == 0 else []
            return e[0][0] if e else None

        u.req("UPDATE_ABORT")
        first, made, w0 = oldest(), 1, time.monotonic()
        if first is None:
            ctx.__dict__["_fill"] = "no entry in READ_LOG after CLEAR_LOG and an UPDATE_ABORT"
            return None
        while n_max is None or made < n_max:
            if made >= 900 or time.monotonic() - w0 > 600:
                ctx.__dict__["_fill"] = f"the ring was not full after {made} entries"
                return None
            r = u.req("UPDATE_ABORT")
            if not r or r["status"] != 0:
                ctx.__dict__["_fill"] = f"UPDATE_ABORT answered {r['status'] if r else None} after {made} entries"
                return None
            made += 1
            if n_max is None or made == n_max:
                now = oldest()
                if now is None:
                    ctx.__dict__["_fill"] = "READ_LOG not answered"
                    return None
                if now != first:
                    return made, True
        return made, False


FAULT_KINDS = ("hard", "bus", "usage", "memmanage", "other")  # A3.6 "Unexpected stop of the firmware"


def fault_line_ok(text: str | None) -> bool:
    """T-A3-62 (3): a line beginning `restart: processor fault` with a kind of A3.6; judged on that prefix and
    the kind only, never on a fault address."""
    p = "restart: processor fault "
    return bool(text) and text.startswith(p) and text[len(p):].split(" ")[0] in FAULT_KINDS


def rec_get(rec: dict, name: str):
    """A field of the cpu.fault record, at its top or in its entry (value.entry), in any case."""
    for d in (rec, rec.get("entry") if isinstance(rec.get("entry"), dict) else {}):
        for k in (name, name.lower(), name.upper()):
            if k in d:
                return d[k]
    return None


def entry_t_us(rec: dict) -> int | None:
    """value.entry.t_us: the emulated time of the fault handler's first instruction (us since power-on, the clock
    of at_us and the hw.trace events)."""
    e = rec.get("entry")
    t = e.get("t_us") if isinstance(e, dict) else None
    return int(t) if t is not None else None


def in_mode(rec: dict, mode: str) -> bool | None:
    """The cpu.fault record's IPSR: 0 in thread mode, the exception number in a handler; None if not given."""
    ipsr = rec_get(rec, "ipsr")
    if ipsr is None:
        return None
    ipsr = int(ipsr, 0) if isinstance(ipsr, str) else int(ipsr)
    return (ipsr & 0x1FF == 0) if mode == "thread" else (ipsr & 0x1FF != 0)


def fault_case(ctx, s: bytes, mode: str, judge):
    """T-A3-62, unarmed with a P2 session and RX running: an undefined-instruction fault (cpu.fault) in
    thread mode or inside an interrupt handler, and points (1) to (4)."""
    from .p2 import value
    from .t_a3 import boot_t_us
    from .t_log import entries, last_start
    from .t_tx_locked import safe
    from .t_update import rx_works
    label = f"a processor fault in {mode} mode (unarmed, RX running)"
    with Host(ctx.net) as h:
        h.start()
        if not ctx.at_least(f"{label}: RX running before it", len(h.recv(0.3, PORT_DDC)), what="status packets in 0.3 s"):
            return
        ctx.fixture("cpu.fault", in_us=300_000, mode=mode, within_us=5_000_000)
        h.recv_alive(2.0)  # the session goes on until the fault and the reset
        h.stop(settle=0)
    time.sleep(0.3)
    wait_reachable(ctx, label, 40)
    rec = ctx.fixture("cpu.fault", read=True) or {}
    if rec.get("missed") or rec.get("dropped"):
        ctx.part(f"{label}: not judged, the fault was {'missed' if rec.get('missed') else 'dropped by a reset first'} (cpu.fault read)")
        return
    m = in_mode(rec, mode)
    ctx.check(f"{label}: the fault came in {mode} mode (cpu.fault IPSR)", m is not False, f"{rec}")
    # (2) a software reset within 10 ms of the handler's entry
    entry, boot = entry_t_us(rec), boot_t_us(ctx)  # the reset: the hw.trace boot event after the fault
    if entry is None or not boot:
        ctx.part(f"{label}: '(2) a reset within 10 ms of the handler's entry' not judged: "
                 + ("cpu.fault's record gives no entry.t_us" if entry is None else "no hw.trace boot event"))
    else:
        ctx.check(f"{label}: (2) the reset within 10 ms of the handler's entry (emulated time: boot event - entry.t_us)",
                  0 < boot - entry <= 10_000, f"{(boot - entry) / 1000:.3f} ms (boot {boot}, entry {entry} us)")
    with Mgmt(ctx.net) as c:
        r = req_retransmit(c, "IDENTIFY")
        cause = tlv(r["payload"]).get(17) if r and r["status"] == 0 else None
    ctx.eq(f"{label}: (2) the reset cause a software reset, not the watchdog (IDENTIFY tag 17)", cause, b"\x02")
    # (3) the fault line before the start entry
    log = entries(ctx)
    k = last_start(log)
    before = log[k - 1][2] if k else None
    ctx.check(f"{label}: (3) before the start entry a line 'restart: processor fault <kind>' (A3.6)", fault_line_ok(before), f"{before!r}")
    # (4) the next start: the unexpected stop, WDT_RESET_AT_START, drop reason 5, RX and management working
    judge(label, True)
    with Host(ctx.net) as h:
        h.start()
        h.recv_alive(0.2)
        st = value(h.write([(0x0202, 0)]), 0x0202)  # TX_ARM_STATE
        h.stop()
    ctx.eq(f"{label}: (4) TX_ARM_STATE drop reason 5 RESET", None if st is None else (st >> 16) & 0xFF, 5)
    ctx.check(f"{label}: (4) RX and the management channel work after it", rx_works(ctx) and bool(entries(ctx)), "no status packet or no log")
    # (1) every pin of Table A3-6b at its safe level, no PA bias write: from the test's start through the next start
    safe(ctx, f"{label}: (1)")


@test("T-A3-62")
def t_a3_62(ctx: Ctx):
    """A3.6 "Evidence at the next start" (0075, 0078), the parts that need no processor fault: after a
    commanded reset no unexpected stop; after a software reset whose reason line or start entry was
    lost one. The fault itself, and the cut writes of the commanded-reset word, wait for an emulator
    fixture, the keyed part for TX: never a pass."""
    from .t_flash import WDT_RESET_AT_START, fault_flags
    from .t_log import entries, last_start, ram_kept
    from .t_update import activate_new, rx_works, trial_rolls_back, updating
    ctx.no_pass = True
    s = secret_of(ctx)
    model = "model" in ctx.endpoint  # the model has the log line; it models no faults

    def judge(label: str, want: bool, reason: str | None = None):
        log = entries(ctx)
        k = last_start(log)
        lines = [e[2] for e in log[k:]] if k is not None else []
        ctx.check(f"{label}: {'' if want else 'no '}'{STOP_LINE}' after the start entry", bool(lines) and (STOP_LINE in lines) == want,
                  f"{lines[:5]}")
        if reason:
            before = log[k - 1][2] if k else None
            ctx.check(f"{label}: READ_LOG still holds '{reason}', right before the start entry", before is not None and
                      before.startswith(reason), f"{before!r}")
        if not model:
            ff = fault_flags(ctx)
            ctx.check(f"{label}: WDT_RESET_AT_START {'set' if want else 'not set'} in FAULT_FLAGS",
                      ff is not None and bool(ff >> WDT_RESET_AT_START & 1) == want, f"FAULT_FLAGS {ff}")

    def commanded(label: str):
        with updating(ctx, s):
            pass
        judge(label, False, REASON)

    def start_entry_lost() -> bool:
        """The start entry of this run is made invalid: the last entry kept is then the RESTART line before it."""
        log = entries(ctx)
        k = last_start(log)
        if not k or log[k - 1][2] != REASON:
            return False
        ctx.fixture("log.corrupt", seq=log[k][0])
        return True

    if not ram_kept(ctx):
        raise Skip("the log may be lost over a reset until P-SYS-9 is set: a RESTART then counts as an unexpected stop")
    commanded("after a RESTART")
    commanded("after a second RESTART in a row")
    if has(ctx, "log.corrupt"):
        # 0078: the next RESTART's word is valid but names an entry that is no longer kept; the last one
        # kept is a reason line, so that the log alone would read as a commanded reset (0075)
        label = "after a RESTART, the run's start entry made invalid, and another RESTART (its word names another entry)"
        if start_entry_lost():
            with updating(ctx, s):
                pass
            judge(label, True, REASON)
            ctx.check("RX and the management channel work after it", rx_works(ctx) and bool(entries(ctx)), "no status packet or no log")
            commanded("after the next RESTART")
        else:
            ctx.part(f"{label}: not run, the run before did not end with '{REASON}'")
        # the last kept entry no reason line: an entry of this run after its start entry made invalid
        label = "after a RESTART whose reason line is lost with the kept log"
        with updating(ctx, s, restart_at_end=False) as u:
            u.req("UPDATE_ABORT")  # with no update state: logged, changes nothing (0074)
        log = entries(ctx)
        if log and log[-1] != log[last_start(log) or 0]:
            ctx.fixture("log.corrupt", seq=log[-1][0])  # it and all after it are dropped, the reason line too
            with updating(ctx, s):
                pass
            judge(label, True)
            commanded("after the next RESTART, again")
        else:
            ctx.part(f"{label}: not run, no entry after the start entry")
    else:
        ctx.part("a software reset with a kept entry made invalid, and a word that names another entry, not run: no log.corrupt")
    if has(ctx, "stop.sim"):  # the reference model: its stop path (A3.6), with the word cleared
        def stop(label: str, log: bool, reason: str):
            ctx.fixture("stop.sim", log=log)
            time.sleep(0.3)
            wait_reachable(ctx, label)
            judge(label, True, reason)
            commanded(f"a RESTART {label}")
        stop("after a stop", True, "restart: stop ")
        for log in (True, False):
            label = f"after a RESTART, the start entry made invalid, and a stop that {'logs its line' if log else 'leaves no line'}"
            if start_entry_lost():
                stop(label, log, REASON)
            else:
                ctx.part(f"{label}: not run, the run before did not end with '{REASON}'")
    with updating(ctx, s) as u:  # its RESTART follows the last entry made
        made = until_dropped(ctx, u)
    if made is None:
        ctx.part(f"a RESTART with the log ring just wrapped, and full: not run: {ctx.__dict__['_fill']}")
    else:
        judge(f"after a RESTART with the ring just wrapped ({made[0]} entries after CLEAR_LOG)", False, REASON)
        with updating(ctx, s) as u:
            full = until_dropped(ctx, u, made[0] - 1)
        if full and not full[1]:
            judge(f"after a RESTART with the ring full ({full[0]} entries after CLEAR_LOG, one fewer)", False, REASON)
        else:
            ctx.part(f"a RESTART with the log ring full: not shown: {ctx.__dict__['_fill'] or 'it had wrapped already'}")
    with updating(ctx, s) as u:  # a short log again for the tests after this one
        u.req("CLEAR_LOG")
    if has(ctx, "trial.sim") or has(ctx, "ch394.silent"):
        a = activate_new(ctx, s, 362, net_down=True)
        if ctx.check("an update activated (the trial's 60 s limit)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
            trial_rolls_back(ctx)
            judge("after the trial's restart: trial-unconfirmed", False, "restart: trial-unconfirmed")
    else:
        ctx.part("the trial's restart not run: needs trial.sim or ch394.silent")
    if has(ctx, "cpu.fault"):
        for mode in ("thread", "handler"):
            fault_case(ctx, s, mode, judge)
        pending = "a fault in the boot stage before and right after its clear of the word (the bench cannot place that clear), " \
                  "a fault during a stretched erase"
    else:
        pending = "the unexpected stop itself, also in the boot stage before and after its clear of the word (cpu.fault not offered)"
    raise PartSkip(f"pending: {pending}, the cut writes of the word (the emulator cannot cut a write to kept RAM), and the keyed "
                   "part (TX)")


# ---------------- T-B11-04 ----------------
def gaps(port: int, seqs: list[int]) -> int:
    """B11.6 (0078): the gaps in the numbers received on one port of one session."""
    top, n = None, 0
    for s in seqs:
        if port == PORT_HP and s == 0:
            continue
        if top is None:
            top = s
            continue
        d = 1 if port == PORT_HP and (top, s) == (0xFFFFFFFF, 1) else (s - top) & 0xFFFFFFFF
        if 1 <= d < 1 << 31:
            top, n = s, n + (d > 1)
    return n


# no three numbers of port 1027 in a row engage numbered mode (B11.6a), in which old ones are discarded
GAP_CASES = ((PORT_DUCIQ, (10, 11, 14, 13, 15), 1), (PORT_DUCIQ, (0xFFFFFFFF, 0), 0), (PORT_DUCIQ, (5, 5, 5), 0),
             (PORT_HP, (0xFFFFFFFF, 1), 0), (PORT_HP, (0, 0, 0), 0), (PORT_HP, (20, 22), 1), (PORT_HP, (7, 0, 8), 0))


def numbered(data: bytes, n: int) -> bytes:
    return n.to_bytes(4, "big") + data[4:]


def status_after(c, h, s: float, port: int, data: bytes, seqs) -> int:
    """Status packets in the second half of s seconds of radio time, with a packet sent every 100 ms."""
    t0, w0, n, i = c.clock.now(), time.monotonic(), 0, 0
    while (dt := c.clock.now() - t0) < s and time.monotonic() - w0 < 60:
        h.send(port, numbered(data, seqs[i % len(seqs)]))
        i += 1
        got = sum(p == PORT_DDC for p, _, _ in h.recv(0.1))
        if dt > s / 2:
            n += got
    return n


@test("T-B11-04")
def t_b11_04(ctx: Ctx):
    """B2.7, B11.6 (0078)."""
    with m(ctx) as c:
        for stop in (900, 3):  # any numbers on every C&C port, then only repeated and older ones
            with Host(ctx.net) as h:
                for port, data, n in ((PORT_DISC, general(), 77), (PORT_DDC, ddc_specific(), 5), (PORT_DUC, duc_specific(), 0xFFFFFFFF),
                                      (PORT_HP, high_priority(), 900)):
                    h.send(port, numbered(data, n))
                n = status_after(c, h, 3, PORT_HP, high_priority(), (900, 900, 3, 900, 2, 2))
                ctx.check("a session goes on with repeated and older numbers only", n > 0, f"{n} status packets")
                # a session that ended and began again looks the same, so run is cleared with such a number: the
                # DUC-specific packets would keep a session that goes on from the watchdog
                h.send(PORT_HP, numbered(high_priority(run=False), stop))
                n = status_after(c, h, 2, PORT_DUC, duc_specific(), (5, 5, 4))
                ctx.check(f"run cleared by a packet with the number {stop} after 900 ends it", n == 0, f"{n} status packets")
                h.stop()
        with Host(ctx.net) as h:
            for port, seqs, want in GAP_CASES:
                name, arrived = (("gap_1027", "hp_rx") if port == PORT_HP else ("gap_1029", "drop_1029"))
                label = f"port {port}, numbers {', '.join(f'0x{s:X}' if s > 999 else str(s) for s in seqs)}: {want} gaps"
                for _ in range(3):  # a datagram lost on the way would itself be a gap
                    h.start()
                    h.recv(0.1)
                    before = ctx.got(stats(c), "GET_STATS")
                    for s in seqs:
                        h.send(port, numbered(high_priority() if port == PORT_HP else bytes(1444), s))
                        h.recv(0.02)
                    need = before.get(arrived, 0) + len(seqs)
                    after = counted(c, lambda g: g.get(arrived, 0) >= need, 2)
                    h.stop()
                    if after.get(arrived) == need:
                        ctx.eq(f"{label}: {name} of the session", after.get(name), want)
                        break
                else:
                    ctx.part(f"{label}: not judged, {arrived} never rose by the {len(seqs)} packets sent")
