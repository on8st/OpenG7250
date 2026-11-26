"""T-A1-100: a running SET_CONFIG that sets the C-1 address mode to DHCP (A1.11 "Change into DHCP mode
while running", "DHCP messages while link-local", "Bound on a borrow"; 0082). Emulator: dhcp.server,
sock.log, eth.log, arp.peer, ip.conflict, power.cycle, net.endpoint. What a runner does not offer is a
part, and the test no pass. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import threading
import time
from contextlib import contextmanager

from .core import Ctx, test
from .mgmt import config_tlv
from .p2 import (ARM_MAGIC, KEYED_TIMEOUT, PORT_DDC, PORT_DDC0, PORT_DISC, PORT_HP, STREAM_STATUS, TX_ARM, TX_ARM_STATE,
                 TX_ARM_TOKEN, TX_MODE_CLASS, TX_STATE, general, value, wait_reachable)
from .t_linklocal import (CLOSED_S, SLACK, current_ip, eth_read, eth_start, is_link_local, judge_probe, probes,
                          session, socks)

NET = "192.168.73."  # the subnet of dhcp.start's defaults (testlist-format, dhcp.server)
MASK = bytes([255, 255, 255, 0])
STATIC, STATIC_B, LEASED = NET + "50", NET + "51", NET + "10"
ATTEMPTS_S, BORROW_S = 6.0, 2.0  # 0082 point 3: at most 6 s of attempts; A1.11 "Bound on a borrow"
CLIENT = ("DISCOVER", "REQUEST")
NEEDS = ("dhcp.server", "power.cycle", "net.endpoint")


# ---------------- judgments (pure: unit-tested) ----------------
def mtype(m: dict) -> str:
    return str(m.get("type", "")).upper().replace("DHCP", "")


def src_ip(m: dict) -> str:
    return str(m.get("src", "")).split(":")[0]


def client_msgs(msgs: list[dict]) -> list[dict]:
    return [m for m in msgs if mtype(m) in CLIENT]


def bad_client_msgs(msgs: list[dict], held: str) -> list[str]:
    """The radio's DISCOVER and REQUEST that do not carry ciaddr 0.0.0.0, the BROADCAST flag and the
    address held as IP source (0082 point 3; 0052 widened)."""
    from .t_mgmt import bflag
    return [f"{mtype(m)} src {src_ip(m)} ciaddr {m.get('ciaddr')} broadcast {bflag(m)}" for m in client_msgs(msgs)
            if (src_ip(m), m.get("ciaddr"), bflag(m)) != (held, "0.0.0.0", True)]


def long_borrows(borrows: list[tuple[int, int]], after_us: int, until_us: int | None = None) -> list[float]:
    """The borrows of the management socket in [after_us, until_us) longer than 2 s (+ slack), in s."""
    return [round((b - a) / 1e6, 3) for a, b in borrows
            if a >= after_us and (until_us is None or a < until_us) and (b - a) / 1e6 > BORROW_S + SLACK]


def closed_span(sock: list[dict], after_s: float) -> tuple[float | None, float | None]:
    """(first CLOSE of discovery or management after `after_s`, the next OPEN of port 1024), in s."""
    from .t_linklocal import opened
    closes = [e["t_us"] / 1e6 for e in sock if e.get("action") == "CLOSE" and e.get("port") in (1024, 7251)
              and e["t_us"] / 1e6 >= after_s]
    first = min(closes) if closes else None
    return first, (opened(sock, 1024, first) if first is not None else None)


# ---------------- helpers ----------------
def has(ctx, name: str) -> bool:
    return name in ctx.endpoint.get("fixtures", [])


def ip4(s: str) -> bytes:
    return bytes(int(x) for x in s.split("."))


def radio_us(ctx) -> int | None:
    return (ctx.fixture("hw.observe") or {}).get("t_us") if has(ctx, "hw.observe") else None


@contextmanager
def kept(h):
    """The P2 session kept alive (C&C every 100 ms, B5.6) while the bench does other work."""
    stop = threading.Event()

    def run():
        while not stop.is_set():
            try:
                h.keepalive()
            except OSError:
                pass
            stop.wait(0.1)
    t = threading.Thread(target=run, daemon=True)
    t.start()
    try:
        yield
    finally:
        stop.set()
        t.join(1.0)


# ---------------- the session, observed without restarting it ----------------
# A probe that sends RUN=1 while it listens restarts an ended session itself (A2.3; B5.4 "Restart"), so it
# cannot see an end. The session is observed instead by a continuous record of what the radio sends, with
# each datagram's source and each bench packet's destination, and by the state a new session starts in.
GAP_S = 0.2  # status every 50 ms in RUN (B9.4): a gap of more than four periods is a stop
JITTER_S = 0.1  # the bench's own receive thread (host clock)
QUIET_S = 0.5  # after the change, the last stretch in which the old address must be silent
MIN_WINDOW_S = 1.0  # a "does not end" window at least this long (radio time): twenty status periods (B9.4)
OLD_ONLY_S = 0.6  # RUN=1 to the old address only: shorter than the 1 s watchdog (B5.6), so a session the
#                   bench's keepalives started at the new address since the move is still the one judged
PRESET = (0, 1, 300)  # TX_STATE bit 4 PA disabled, TX_MODE_CLASS, KEYED_TIMEOUT set by the bench
DEFAULTS = (1, 0, 500)  # the same after a session end (A3.6a Table A3-4, A3.6; B8-2)
TX_NAMES = ("PA enable cleared (TX_STATE bit 4)", "TX_MODE_CLASS", "KEYED_TIMEOUT")


class Stream:
    """The radio's datagrams to one host, recorded continuously as (time, radio port, radio address),
    while the host keeps its session alive (RUN=1 every 100 ms, B5.6); every datagram the host sends is
    recorded as (time, port, the radio address meant, destination). Times on the host clock; `marks`
    pair it with radio time. `manual` (unit tests): no thread, `wait` steps."""

    def __init__(self, ctx, h, now=time.time, radio_now=None, every: float = 0.1, manual: bool = False):
        self.ctx, self.h, self.now, self.every, self.manual = ctx, h, now, every, manual
        self._radio_now = radio_now
        self.got, self.sent, self.marks, self.nets = [], [], [], [h.net]
        self.pauses = []  # (from, to) host times the record was paused: datagrams then went to the register read
        self.lock = threading.Lock()
        self._t = self._stop = None
        send = h.send

        def logged(port, data, kind="unicast"):
            with self.lock:
                self.sent.append((self.now(), port, h.net.radio_ip, h.net.dest(port, kind)))
            send(port, data, kind)
        h.send = logged

    def radio_now(self) -> float:
        if self._radio_now is None:
            from .core import RadioClock
            self._radio_now = RadioClock(self.ctx).now
        return self._radio_now()

    def mark(self):
        self.marks.append((self.now(), self.radio_now()))

    def ratio(self) -> float:
        """Host seconds per radio second over the record (the emulator may run slower); at least 1, so
        a gap is never judged longer than the host saw it."""
        if len(self.marks) < 2 or self.marks[-1][1] <= self.marks[0][1]:
            return 1.0
        return max(1.0, (self.marks[-1][0] - self.marks[0][0]) / (self.marks[-1][1] - self.marks[0][1]))

    def know(self, net):
        """A further address mapping, for telling sources apart (the radio's address after a change)."""
        if all(n is not net for n in self.nets):
            self.nets.append(net)

    def retarget(self, net):
        """Send from the same socket (the same host and port) to the radio at `net`."""
        self.know(net)
        self.h.net, self.h.radio = net, net.radio_ip

    def radio_of(self, src):
        """The radio address a datagram came from; None where two known mappings share the source."""
        if not any(n.back for n in self.nets):
            return src[0]
        ips = {n.radio_ip for n in self.nets if tuple(src) in n.back}
        return ips.pop() if len(ips) == 1 else None

    def port_of(self, src) -> int:
        for n in self.nets:
            if tuple(src) in n.back:
                return n.back[tuple(src)]
        return src[1]

    def step(self):
        try:
            self.h.keepalive()
        except OSError:
            pass
        try:
            got = self.h.recv_raw(self.every)
        except (OSError, ValueError):
            return
        with self.lock:
            self.got += [(t, self.port_of(src), self.radio_of(src)) for _, src, t in got]

    def snapshot(self) -> list:
        with self.lock:
            return list(self.got)

    def start(self):
        self.mark()
        if self.manual or self._t:
            return
        stop = self._stop = threading.Event()

        def run():
            while not stop.is_set():
                self.step()
        self._t = threading.Thread(target=run, daemon=True)
        self._t.start()

    def stop(self):
        if self._t:
            self._stop.set()
            self._t.join(1.0)
            self._t = None
        self.mark()

    @contextmanager
    def paused(self):
        """The record stopped while the bench reads registers on the same socket."""
        running = self._t is not None
        self.stop()
        t0 = self.now()
        try:
            yield
        finally:
            self.pauses.append((t0, self.now()))
            if running:
                self.start()

    def wait(self, s: float):
        """s seconds of radio time, the record going on."""
        if self.manual:
            for _ in range(int(round(s / self.every))):
                self.step()
        else:
            from .t_a3 import wait_radio
            wait_radio(self.ctx, s * 1000)
        self.mark()


def status(rec: list, t0: float | None = None, t1: float | None = None) -> list[tuple[float, str | None]]:
    return [(t, r) for t, p, r in rec if p == PORT_DDC and (t0 is None or t >= t0) and (t1 is None or t <= t1)]


def gaps(ts: list[float], ratio: float = 1.0) -> list[tuple[float, float]]:
    """(start, radio seconds) of each gap between consecutive times longer than GAP_S + JITTER_S."""
    return [(a, round((b - a) / ratio, 3)) for a, b in zip(ts, ts[1:]) if (b - a) / ratio > GAP_S + JITTER_S]


def kept_faults(rec: list, t0: float, t1: float, ratio: float = 1.0, pauses=()) -> list[str]:
    """A session that does not end in [t0, t1]: status packets from one radio address, with no gap. A gap
    that overlaps a pause of the record (the bench reading registers on the socket) is the bench's, not
    the radio's: the datagrams then went to the read."""
    st = status(rec, t0, t1)
    if not st:
        return [f"no status packets in {t1 - t0:.1f} s"]
    out = []
    srcs = sorted({str(r) for _, r in st})
    if len(srcs) > 1:
        out.append(f"status packets from {srcs}")
    ts = [t0] + [t for t, _ in st] + [t1]
    held = [(a, b) for a, b in zip(ts, ts[1:]) if not any(a < pe and b > ps for ps, pe in pauses)]
    out += [f"a gap of {(b - a) / ratio:.2f} s of radio time {a - t0:.2f} s into the window" for a, b in held
            if (b - a) / ratio > GAP_S + JITTER_S]
    return out


def end_faults(rec: list, t_change: float, t_switch: float, old_ip: str, new_ip: str, ratio: float = 1.0) -> dict:
    """From the change (applied: the radio answers on the new address) to the switch the bench sent only to
    the old address. `old`: status packets from the old address after the change, which A1.11a forbids
    whether the session went on or a packet to the old address started one ("stop using the old address");
    `gap`: the longest gap before them, radio seconds (a restart under the bench's 100 ms RUN=1 can leave
    none longer than a status period, so it only describes); `new_early`: from the new address, where the
    bench sent nothing."""
    after = t_change + (GAP_S + JITTER_S) * ratio
    olds = [t for t, r in status(rec, None, t_switch) if r == old_ip]
    late = [t for t in olds if t >= after]
    gap = 0.0
    if late:
        before = [t for t in olds if t < late[0]][-1:] or [t_change]
        gap = round((late[0] - before[0]) / ratio, 3)
    return {"old": late, "gap": gap, "new_early": [t for t, r in status(rec, t_change, t_switch) if r == new_ip]}


NOT_OURS = "not our address"  # net.log drop reason of the chip's unicast filter (minor 10)


def former_net(ctx):
    """The mapping of the radio's address before the last move (endpoint radio.former, minor 10), or None."""
    from .net import Net
    f = (ctx.endpoint.get("radio") or {}).get("former")
    if not (isinstance(f, dict) and f.get("ip") and f.get("connect")):
        return None
    return Net(dict(ctx.endpoint, radio=dict(ip=f["ip"], connect=f["connect"])))


def netlog_entries(ctx) -> list[dict]:
    from .t_a2 import netlog_read
    out = netlog_read(ctx)
    try:
        ctx.fixture("net.log", action="stop")
    except Exception:  # noqa: BLE001  (a stop that fails loses nothing already read)
        pass
    return out


def drops_of(entries: list[dict], port: int | None) -> tuple[list[dict], list[dict]]:
    """(the bench host's datagrams the chip dropped as 'not our address', those the radio took), by net.log."""
    mine = [e for e in entries if port is None or str(e.get("peer", "")).rsplit(":", 1)[-1] == str(port)]
    return ([e for e in mine if e.get("dir") == "dropped" and e.get("reason") == NOT_OURS],
            [e for e in mine if e.get("dir") == "to_radio"])


def judge_drops(ctx, label: str, old_ip: str, entries: list[dict], port: int | None):
    """While the bench sent only to the old address: its datagrams reach the chip and are dropped as 'not our
    address', and none is taken (A1.11a, stop using the old address; net.log, minor 10)."""
    dropped, taken = drops_of(entries, port)
    ctx.check(f"{label}: the bench's packets to {old_ip} reach the chip and are dropped as '{NOT_OURS}' (net.log)",
              bool(dropped), f"{len(dropped)} dropped; reasons seen: {sorted({str(e.get('reason')) for e in entries if e.get('dir') == 'dropped'})}")
    ctx.eq(f"{label}: none of the bench's packets to {old_ip} taken by the radio (net.log to_radio)", len(taken), 0)


def judged_session(rec: list, new_ip: str, t_change: float, ratio: float = 1.0, pauses=()) -> str:
    """Which session at the new address the B5.4 item 3 checks see: the first one after the move (status packets
    from the new address with no gap since their first, which may lie before t_change where the bench's
    keepalives reached it through radio.connect), or a restart after a gap (by the watchdog, B5.6)."""
    ts = [t for t, r in status(rec) if r == new_ip]
    if not ts:
        return "none seen"
    held = [(a, b) for a, b in zip(ts, ts[1:]) if not any(a < pe and b > ps for ps, pe in pauses)
            and (b - a) / ratio > GAP_S + JITTER_S]
    since = f"{(t_change - ts[0]) / ratio:.2f} s of radio time before the change was seen" if ts[0] < t_change else "after the change was seen"
    if not held:
        return f"the first at {new_ip} after the move, its status packets without a gap since their first, {since}"
    a, b = held[-1]
    return (f"a restart at {new_ip} after a gap of {(b - a) / ratio:.2f} s of radio time (the first session there began "
            f"{since}): weaker than the first session after the move")


def tx_settings(h) -> tuple:
    """(TX_STATE bit 4, TX_MODE_CLASS, KEYED_TIMEOUT) read without changing them: TX_STATE is read-only, and a
    rejected write is answered with the value in force (B8.4, B8.4a)."""
    ts = value(h.write([(TX_STATE, 0)]), TX_STATE)
    mc = value(h.write([(TX_MODE_CLASS, 0xFF)]), TX_MODE_CLASS)
    kt = value(h.write([(KEYED_TIMEOUT, 50)]), KEYED_TIMEOUT)
    return (None if ts is None else (ts >> 4) & 1, mc, kt)


def tx_preset(ctx, h, label: str) -> bool:
    """PA enable set (General byte 58 bit [0]), TX_MODE_CLASS 1, KEYED_TIMEOUT 300: values a session end
    resets, so that a reset can be seen."""
    h.send(PORT_DISC, general(pa=True))
    h.keepalive()
    h.write([(TX_MODE_CLASS, PRESET[1])])
    h.write([(KEYED_TIMEOUT, PRESET[2])])
    got = tx_settings(h)
    if got != PRESET:
        ctx.part(f"{label}: the TX settings not taken beforehand ({got}, wanted {PRESET}): their reset not judged")
        ctx.no_pass = True
        return False
    return True


def judge_new_session(ctx, label: str, h, ddc0: int):
    """The session that the bench's RUN=1 starts after an address change carries nothing over (B5.4 "Restart"
    item 3, as T-B5-16): DDC0 reason 1 and no DDC0 I&Q, the TX settings of Table A3-4 (A3.6a) and of A3.6 at
    their defaults, and an arm refused with WATCHDOG_DISABLED until a General packet of the new session."""
    from .t_tx_locked import arm_reason
    ss = value(h.write([(STREAM_STATUS, 0)]), STREAM_STATUS)
    ctx.eq(f"{label}: the new session: DDC0 STREAM_STATUS reason 1 (B5.4 item 3)", None if ss is None else ss & 0x7F, 1)
    ctx.eq(f"{label}: the new session: no DDC0 I&Q (B5.4 item 3)", ddc0, 0)
    for name, got, want in zip(TX_NAMES, tx_settings(h), DEFAULTS):
        ctx.eq(f"{label}: the new session: {name} at its default (A3.6a, A3.6)", got, want)
    ctx.notes.append(f"{label}: the DUC-specific settings of Table A3-4 have no register to read (Table B8-2): not judged")
    tok = value(h.write([(TX_ARM_TOKEN, 0)]), TX_ARM_TOKEN)
    if not ctx.check(f"{label}: the new session: TX_ARM_TOKEN read", tok is not None, "no answer to a write of 0x0200"):
        return
    st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
    ctx.eq(f"{label}: the new session: an arm before its General packet refused with WATCHDOG_DISABLED (B5.4 item 2)",
           arm_reason(st), "WATCHDOG_DISABLED")
    h.send(PORT_DISC, general())
    h.keepalive()
    st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
    ctx.check(f"{label}: the new session: after its General packet the arm no longer refused with WATCHDOG_DISABLED",
              st is not None and arm_reason(st) != "WATCHDOG_DISABLED", f"{arm_reason(st)}")


def set_config(ctx, s: bytes, *items) -> int | None:
    from .t_update import updating
    with updating(ctx, s, restart_at_end=False) as u:
        r = u.req("SET_CONFIG", config_tlv(*items))
    return r["status"] if r else None


def go_static(ctx, s: bytes, ip: str, label: str) -> bool:
    st = set_config(ctx, s, (1, b"\x01"), (2, ip4(ip)), (3, MASK), (4, bytes(4)))
    if not ctx.eq(f"{label}: the static address {ip} set: 0", st, 0):
        return False
    time.sleep(0.5)
    wait_reachable(ctx, f"{label}: the static address", 40)
    return ctx.eq(f"{label}: running on {ip}", current_ip(ctx), ip)


def address_mode(ctx, s: bytes) -> bytes | None:
    from .t_store import get_config
    from .t_update import updating
    with updating(ctx, s, restart_at_end=False) as u:
        return get_config(u.c).get(1)


def wait_for(ctx, h, pred, limit_s: float) -> bool:
    """Poll `pred` for limit_s s of radio time (the radio's clock; wall time at most 12 times that plus
    60 s), the session kept alive meanwhile."""
    from .core import RadioClock
    clock, wall0 = RadioClock(ctx), time.monotonic()
    t0 = clock.now()
    with kept(h):
        while clock.now() - t0 < limit_s and time.monotonic() - wall0 < 12 * limit_s + 60:
            if pred():
                return True
            time.sleep(0.5)
    return bool(pred())


class Case:
    """One change into DHCP mode: the logs started, the request sent with the session kept alive."""

    def __init__(self, ctx, s: bytes, label: str, preset_before: bool = False):
        """`preset_before`: the TX settings set before the request (a case whose lease may come at once);
        otherwise after the reply, since the request's flash work may end the session by its watchdog and
        the host restarts it (B2.3 "Flash work"), which resets them (A3.6a)."""
        from .t_mgmt import dhcp_messages
        self.ctx, self.s, self.label = ctx, s, label
        self.held = current_ip(ctx)
        self.h = session(ctx)
        self.stream = Stream(ctx, self.h)
        self.preset = tx_preset(ctx, self.h, label) if preset_before else None
        if has(ctx, "sock.log"):
            ctx.fixture("sock.log", action="start")
        if has(ctx, "eth.log"):
            eth_start(ctx)
        self.n0 = len(dhcp_messages(ctx))
        self.t0 = radio_us(ctx)
        self.stream.start()  # keeps the session alive meanwhile
        self.st = set_config(ctx, s, (1, b"\x00"))
        self.t_reply = self.t_from = self.stream.now()
        self.t_reply_us = radio_us(ctx)

    def preset_tx(self):
        with self.stream.paused():
            self.preset = tx_preset(self.ctx, self.h, self.label)
        self.t_from = self.stream.now()

    def kept(self, what: str, t0: float | None = None, t1: float | None = None, tx: bool = True, moved=None) -> bool:
        """'The session does not end' in [t0, t1] (default: since the reply or the preset, until now):
        status packets from one address with no gap, and (tx) the TX settings not reset (A3.6a). `moved`:
        a callable, true when the address changed meanwhile, so the settings are not judged."""
        ctx, st = self.ctx, self.stream
        t0 = self.t_from if t0 is None else t0
        if t1 is None:  # until now, but never less than MIN_WINDOW_S of radio time
            st.mark()
            short = MIN_WINDOW_S - (st.now() - t0) / st.ratio()
            if short > 0:
                st.wait(short)
            t1 = st.now()
        st.mark()
        faults = kept_faults(st.snapshot(), t0, t1, st.ratio(), st.pauses)
        if tx and self.preset:
            with st.paused():
                got = tx_settings(self.h)
            if moved and moved():
                ctx.part(f"{self.label}: {what}: the TX settings not judged, the address changed meanwhile")
                ctx.no_pass = True
            else:
                faults += [f"{n} {g} (set {w}): reset" if g == d else f"{n} {g} (set {w})"
                           for n, g, w, d in zip(TX_NAMES, got, PRESET, DEFAULTS) if g != w]
        return ctx.check(f"{self.label}: {what}", not faults, "; ".join(faults))

    def ended(self, new, t_change: float, former=None):
        """An address change applied at t_change (the radio answers on the new address, `new`): the
        session ends, a packet to the old address starts none, and the bench's RUN=1 to the new address
        starts a new session that carries nothing over (A1.11a; A2.3; B5.4 item 3). `former`: the mapping
        of the address before the move (endpoint radio.former, minor 10); without it the bench's own
        mapping from before the move stands for the old address."""
        ctx, st, label = self.ctx, self.stream, self.label
        # where the bench's own mapping already reaches the new address (radio.connect follows the radio),
        # its RUN=1 since the move may have started a session there, rightly (A2.3)
        reached_new = self.h.net.dest(PORT_HP) == new.dest(PORT_HP)
        if former is not None:
            old = former
            st.nets = [former, new]  # from now on radio.connect reaches the new address
            st.retarget(former)  # the same socket, to the old address
        else:
            old = self.h.net
            st.know(new)
        apart = old.dest(PORT_HP) != new.dest(PORT_HP)
        from .t_a2 import netlog_start
        netlog = apart and netlog_start(ctx)
        st.wait(OLD_ONLY_S)  # RUN=1 to the old address only
        t_switch = st.now()
        sent = sorted({ip for t, p, ip, _ in st.sent if t >= t_change})
        ctx.notes.append(f"{label}: from the change on the bench sent to {sent} until {t_switch - t_change:.1f} s, "
                         f"then to {new.radio_ip}")
        if apart:
            f = end_faults(st.snapshot(), t_change, t_switch, old.radio_ip, new.radio_ip, st.ratio())
            ctx.check(f"{label}: the session ends and a P2 packet to the old address {old.radio_ip} starts none (A1.11a): "
                      "no status packets from it after the change", not f["old"],
                      f"{len(f['old'])} status packets from it after the change, the first {f['gap']:.2f} s of radio time "
                      "after the one before (" + ("a session again on the old address" if f["gap"] > GAP_S + JITTER_S else
                                                  "no gap longer than the bound: went on, or restarted at once") + ")")
            if not reached_new:
                ctx.check(f"{label}: no status packets from {new.radio_ip} before the bench sent there", not f["new_early"],
                          f"{len(f['new_early'])} packets")
            else:
                ctx.notes.append(f"{label}: the bench's keepalives already reached {new.radio_ip}, so 'no status packets "
                                 f"from {new.radio_ip} before the bench sent there' is not judged")
            if netlog:
                judge_drops(ctx, label, old.radio_ip, netlog_entries(ctx), getattr(self.h, "port", None))
            else:
                ctx.notes.append(f"{label}: no net.log: that the packets to {old.radio_ip} reached the chip and were "
                                 "dropped is not shown; 'starts none' rests on the absence of status packets")
        else:
            ctx.part(f"{label}: the endpoint maps the old address {old.radio_ip} and the new {new.radio_ip} to the same "
                     "local addresses (no radio.former): 'the status packets from the old address stop' and 'a packet "
                     "to the old address starts no session' not judged; the end judged by the new session's state "
                     "(B5.4 item 3)")
            ctx.no_pass = True
        st.retarget(new)
        st.wait(1.0)
        rec = st.snapshot()
        restart = [t for t, r in status(rec, t_switch) if r == new.radio_ip or not apart]
        if not ctx.check(f"{label}: the bench's RUN=1 to {new.radio_ip} starts a new session (A2.3)", bool(restart),
                         "no status packets within 1 s"):
            return
        ddc0 = sum(1 for t, p, _ in rec if p == PORT_DDC0 and t >= restart[0])
        if apart:
            ctx.notes.append(f"{label}: the session judged against B5.4 item 3: " + judged_session(rec, new.radio_ip, t_change, st.ratio(), st.pauses))
        with st.paused():
            judge_new_session(ctx, label, self.h, ddc0)

    def answered(self) -> bool:
        return self.ctx.eq(f"{self.label}: SET_CONFIG of address mode DHCP answered 0", self.st, 0)

    def msgs(self) -> list[dict]:
        from .t_mgmt import dhcp_messages
        return dhcp_messages(self.ctx)[self.n0:]

    def acked(self) -> bool:
        return any(mtype(m) == "ACK" for m in self.msgs())

    def borrows(self, until_us: int | None = None):
        ctx = self.ctx
        if not has(ctx, "sock.log"):
            ctx.part(f"{self.label}: 'never closed for more than 2 s at a time': not judged, no sock.log")
            ctx.no_pass = True
            return
        from .t_mgmt import mgmt_borrows
        _, borrows = mgmt_borrows(socks(ctx) or [])
        ctx.eq(f"{self.label}: the management socket never borrowed for more than 2 s at a time (sock.log)",
               long_borrows(borrows, self.t0 or 0, until_us), [])

    def close(self):
        self.stream.stop()
        self.h.close()


LINK_LOCAL_BY_S = ATTEMPTS_S + CLOSED_S + 1.0  # 0082 point 3, then "Link-local address"; 1 s for the bench's polls


def wait_link_local(ctx, h, limit_s: float) -> float:
    """Wait until the radio is on a link-local address (net.endpoint follows it), for at most limit_s of
    radio time (the radio's clock: the emulator may run slower than the host), the session kept alive;
    then until it answers discovery. The radio seconds waited."""
    from .core import RadioClock
    from .p2 import reread_endpoint
    clock, wall0 = RadioClock(ctx), time.monotonic()
    t0 = clock.now()
    with kept(h):
        while clock.now() - t0 < limit_s and time.monotonic() - wall0 < 12 * limit_s + 60:
            reread_endpoint(ctx)
            if is_link_local(current_ip(ctx)):
                break
            time.sleep(0.5)
    took = clock.now() - t0
    if is_link_local(current_ip(ctx)):
        wait_reachable(ctx, "the link-local address", 40)
    return took


def moved(ctx, ip: str) -> bool:
    """The radio's address is no longer `ip` (net.endpoint follows it)."""
    from .p2 import reread_endpoint
    reread_endpoint(ctx)
    return current_ip(ctx) != ip


def lease_bound(c, ack: dict, t_seen: float) -> float | None:
    """The host time up to which the session must run in the moving case: the DHCPACK's t_wall where it is on
    the host clock, else the radio's last REQUEST before it (its exact t_us, the clock of net.log) placed
    after the reply by the radio time between them; None without either."""
    t = ack_wall(ack, c.t_reply, t_seen)
    if t is not None:
        return t
    reqs = [m for m in c.msgs() if mtype(m) == "REQUEST" and not m.get("t_us_approx")
            and isinstance(m.get("t_us"), (int, float)) and m["t_us"] <= ack.get("t_us", float("inf"))]
    if not reqs or c.t_reply_us is None:
        return None
    return c.t_reply + (reqs[-1]["t_us"] - c.t_reply_us) / 1e6 * c.stream.ratio()


def ack_wall(ack: dict, t_reply: float, t_seen: float) -> float | None:
    """The DHCPACK's time on the host clock: its t_wall where that lies between the reply and the moment
    the bench saw it (the same clock), else None."""
    t = ack.get("t_wall")
    return t if isinstance(t, (int, float)) and t_reply - 1 <= t <= t_seen else None


def needs(ctx, label: str, *fx) -> bool:
    missing = [f for f in fx if not has(ctx, f)]
    if missing:
        ctx.part(f"{label}: not run, the runner does not offer {', '.join(missing)}")
        ctx.no_pass = True
    return not missing


# ---------------- the cases ----------------
def no_server(ctx, s: bytes):
    """From a static address with no server: at most 6 s of attempts, then a link-local address chosen and
    probed with discovery and management closed for at most 10 s, and DHCP retries go on."""
    label = "from a static address, no DHCP server"
    if not go_static(ctx, s, STATIC, label):
        return
    c = Case(ctx, s, label)
    try:
        if not c.answered():
            return
        c.preset_tx()
        c.stream.wait(1.0)
        c.kept("the session does not end for the mode change", moved=lambda: moved(ctx, c.held))
        took = wait_link_local(ctx, c.h, LINK_LOCAL_BY_S)
        ip = current_ip(ctx)
        if not ctx.check(f"{label}: a link-local address chosen within {LINK_LOCAL_BY_S:g} s of radio time (the "
                         f"{ATTEMPTS_S:g} s of attempts, then the probe of at most {CLOSED_S:g} s)", is_link_local(ip),
                         f"{ip} after {took:.1f} s of radio time"):
            return
        disc = [m for m in c.msgs() if mtype(m) == "DISCOVER" and not m.get("t_us_approx")]
        from_static = [m for m in disc if src_ip(m) == STATIC]
        ctx.eq(f"{label}: the attempts carry ciaddr 0.0.0.0, the BROADCAST flag and the static address as source",
               bad_client_msgs([m for m in c.msgs() if src_ip(m) != ip], STATIC), [])
        if has(ctx, "eth.log"):
            p = probes([(e["t_us"] / 1e6, int(e["arp"].get("op", 0)), e["arp"].get("spa", ""), e["arp"].get("tpa", ""))
                        for e in eth_read(ctx) if e.get("dir") == "from_radio" and isinstance(e.get("arp"), dict)], ip)
            if not p:
                ctx.part(f"{label}: the probe not judged: eth.log holds no probe of {ip} (the emulator may not model it)")
                ctx.no_pass = True
            else:
                judge_probe(ctx, f"{label}: the probe of {ip}", [t for t, _ in p])
                if from_static:
                    first = from_static[0]["t_us"] / 1e6
                    ctx.check(f"{label}: the static address kept for at most {ATTEMPTS_S:g} s of attempts (first DISCOVER "
                              "to the first probe)", p[0][0] - first <= ATTEMPTS_S + SLACK, f"{p[0][0] - first:.2f} s")
                else:
                    ctx.check(f"{label}: DHCP attempts from the static address before the link-local address", False,
                              f"DISCOVERs from {[src_ip(m) for m in disc][:4]}")
        else:
            ctx.part(f"{label}: the probe and the 6 s not judged: no eth.log")
            ctx.no_pass = True
        sock = socks(ctx)
        if sock is None:
            ctx.part(f"{label}: 'closed for at most 10 s' not judged: no sock.log")
            ctx.no_pass = True
        else:
            first, back = closed_span(sock, (c.t0 or 0) / 1e6)
            ctx.check(f"{label}: discovery and management closed for at most {CLOSED_S:g} s (sock.log)",
                      first is not None and back is not None and back - first <= CLOSED_S + SLACK, f"closed {first}, open {back}")
        retry = wait_for(ctx, c.h, lambda: any(mtype(m) == "DISCOVER" and src_ip(m) == ip for m in c.msgs()), 30)
        ctx.check(f"{label}: DHCP retries go on from the link-local address", retry, "no DISCOVER from it within 30 s")
    finally:
        c.close()


def from_link_local(ctx, s: bytes):
    """From a link-local address in static mode (a conflict on a static address whose previous
    configuration conflicts too, A1.11): no probe, no session end until a lease, then a lease is an
    address change."""
    label = "from a link-local address in static mode"
    if not needs(ctx, label, "arp.peer", "ip.conflict", "eth.log"):
        return
    if not (go_static(ctx, s, STATIC, f"{label} (the previous configuration)") and go_static(ctx, s, STATIC_B, label)):
        return
    ctx.fixture("arp.peer", action="set", ip=STATIC, answer_probe=True, defend=True)
    ctx.fixture("ip.conflict", action="auto", on=True)
    try:
        ctx.fixture("ip.conflict", action="raise")  # on STATIC_B: back to the previous configuration, STATIC
        time.sleep(0.5)
        wait_reachable(ctx, f"{label}: the conflict on {STATIC_B}", 60)
        if current_ip(ctx) == STATIC:  # the other host announces STATIC: it conflicts too
            ctx.fixture("arp.peer", action="announce", ip=STATIC, count=2)
            time.sleep(0.5)
            wait_reachable(ctx, f"{label}: the conflict on {STATIC}", 60)
        ip, mode = current_ip(ctx), address_mode(ctx, s)
        if not (is_link_local(ip) and mode == b"\x01"):
            ctx.part(f"{label}: not reached (after a conflict on {STATIC_B} with {STATIC} taken: {ip}, address mode {mode!r})")
            ctx.no_pass = True
            return
    finally:
        ctx.fixture("ip.conflict", action="auto", on=False)
        ctx.fixture("arp.peer", action="clear")
    c = Case(ctx, s, label)
    try:
        if not c.answered():
            return
        c.preset_tx()
        c.stream.wait(10.0)  # longer than the 6 s of attempts from a static address
        c.kept("no session end before a lease (10 s)", moved=lambda: moved(ctx, ip))
        ctx.eq(f"{label}: the link-local address kept", current_ip(ctx), ip)
        p = probes([(e["t_us"] / 1e6, int(e["arp"].get("op", 0)), e["arp"].get("spa", ""), e["arp"].get("tpa", ""))
                    for e in eth_read(ctx) if e.get("dir") == "from_radio" and isinstance(e.get("arp"), dict)])
        ctx.eq(f"{label}: no probe (eth.log)", p, [])
        ctx.eq(f"{label}: the attempts carry the link-local address as source, ciaddr 0.0.0.0, the BROADCAST flag",
               bad_client_msgs(c.msgs(), ip), [])
        c.borrows()
        ctx.fixture("dhcp.start")
        wait_for(ctx, c.h, c.acked, 90)  # the next retry: from 4 s doubling, at most 64 s
        wait_reachable(ctx, f"{label}: the lease", 40)
        ctx.check(f"{label}: the lease is an address change", not is_link_local(current_ip(ctx)), current_ip(ctx))
    finally:
        c.close()
        ctx.fixture("dhcp.stop")


def with_server(ctx, s: bytes):
    """From a static address with a server: answered, the session kept, borrows of at most 2 s, the
    messages' fields; a lease with another address ends the session, one with the same address and
    netmask does not; then DHCP set again starts no exchange."""
    ctx.fixture("dhcp.start")  # offers LEASED, netmask 255.255.255.0
    for label, static, moves in (("from a static address, a server offering another address", STATIC, True),
                                 ("from a static address, a server offering the same address and netmask", LEASED, False)):
        if not go_static(ctx, s, static, label):
            return
        c = Case(ctx, s, label, preset_before=moves)  # another address may come at once
        try:
            if not c.answered():
                return
            if not moves:
                c.preset_tx()
            if not ctx.check(f"{label}: a lease (DHCPACK in dhcp.log)", wait_for(ctx, c.h, c.acked, 30), f"{[mtype(m) for m in c.msgs()]}"):
                return
            t_seen = c.stream.now()
            ack = next(m for m in c.msgs() if mtype(m) == "ACK")
            if moves:  # up to the lease, which ends it; the TX settings cannot be read before it
                what = "the session does not end for the mode change (until the lease)"
                t1 = lease_bound(c, ack, t_seen)
                span = None if t1 is None else (t1 - c.t_reply) / c.stream.ratio()
                if span is None or span < GAP_S + JITTER_S:
                    ctx.notes.append(f"{label}: '{what}' not judged: " + (
                        "no time for the lease on either clock" if span is None else
                        f"the lease came {max(span, 0) * 1000:.0f} ms of radio time after the reply, less than the "
                        f"{(GAP_S + JITTER_S) * 1000:.0f} ms a gap needs to be seen"))
                else:
                    c.kept(what, t1=t1, tx=False)
            else:
                c.kept("the session does not end for the mode change")
            ctx.eq(f"{label}: the DHCP messages carry ciaddr 0.0.0.0, the BROADCAST flag and the static address as source",
                   bad_client_msgs([m for m in c.msgs() if m.get("t_us", 0) <= ack.get("t_us", 0) or m.get("t_us_approx")], static), [])
            if moves:
                c.borrows(None if ack.get("t_us_approx") else ack.get("t_us"))
                wait_reachable(ctx, f"{label}: the lease", 40)
                if ctx.eq(f"{label}: the radio moves to the leased address", current_ip(ctx), LEASED):
                    c.ended(ctx.net, c.stream.now(), former_net(ctx))
            else:
                c.borrows()
                c.kept("the session does not end (a lease with the same address and netmask)")
                ctx.eq(f"{label}: the address kept", current_ip(ctx), LEASED)
        finally:
            c.close()
    label = "in DHCP mode already, DHCP set again"
    if address_mode(ctx, s) != b"\x00":
        ctx.part(f"{label}: not run, the radio is not in DHCP mode")
        return
    c = Case(ctx, s, label)
    try:
        if not c.answered():
            return
        with kept(c.h):
            from .t_a3 import wait_radio
            wait_radio(ctx, ATTEMPTS_S * 1000 + 2000)
        ctx.eq(f"{label}: no DHCP exchange started (dhcp.log)", [mtype(m) for m in client_msgs(c.msgs())], [])
        if has(ctx, "sock.log"):
            from .t_mgmt import mgmt_borrows
            ctx.eq(f"{label}: no borrow of the management socket (sock.log)", mgmt_borrows(socks(ctx) or [])[1], [])
    finally:
        c.close()


@test("T-A1-100", own_boot=True)
def t_a1_100(ctx: Ctx):
    """A1.11 "Change into DHCP mode while running" (0082), emulator."""
    for f in NEEDS:
        ctx.need(f)
    from .t_mgmt import secret_of
    ctx.fixture("dhcp.stop")
    ctx.fixture("power.cycle")
    wait_reachable(ctx, "a power cycle with no DHCP server", 40)
    s = secret_of(ctx)
    try:
        no_server(ctx, s)
        from_link_local(ctx, s)
        with_server(ctx, s)
    finally:
        try:
            if address_mode(ctx, s) != b"\x00":
                set_config(ctx, s, (1, b"\x00"))
        finally:
            ctx.fixture("dhcp.stop")
            ctx.fixture("power.cycle")  # back to DHCP mode on a link-local address, as the test began
            wait_reachable(ctx, "the end of T-A1-100", 40)
