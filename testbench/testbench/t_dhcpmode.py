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
from .p2 import PORT_DDC, wait_reachable
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


def alive(h) -> bool:
    return any(p == PORT_DDC for p, _, _ in h.recv_alive(0.5))


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

    def __init__(self, ctx, s: bytes, label: str):
        from .t_mgmt import dhcp_messages
        self.ctx, self.s, self.label = ctx, s, label
        self.held = current_ip(ctx)
        self.h = session(ctx)
        if has(ctx, "sock.log"):
            ctx.fixture("sock.log", action="start")
        if has(ctx, "eth.log"):
            eth_start(ctx)
        self.n0 = len(dhcp_messages(ctx))
        self.t0 = radio_us(ctx)
        with kept(self.h):
            self.st = set_config(ctx, s, (1, b"\x00"))

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
        ctx.check(f"{label}: the session does not end for the mode change", alive(c.h), "no status packets after the reply")
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
        with kept(c.h):
            from .t_a3 import wait_radio
            wait_radio(ctx, 10_000)  # longer than the 6 s of attempts from a static address
        ctx.check(f"{label}: no session end before a lease", alive(c.h), "no status packets 10 s after the reply")
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
        c = Case(ctx, s, label)
        try:
            if not c.answered():
                return
            ctx.check(f"{label}: the session does not end for the mode change", alive(c.h), "no status packets after the reply")
            if not ctx.check(f"{label}: a lease (DHCPACK in dhcp.log)", wait_for(ctx, c.h, c.acked, 30), f"{[mtype(m) for m in c.msgs()]}"):
                return
            ack = next(m for m in c.msgs() if mtype(m) == "ACK")
            ctx.eq(f"{label}: the DHCP messages carry ciaddr 0.0.0.0, the BROADCAST flag and the static address as source",
                   bad_client_msgs([m for m in c.msgs() if m.get("t_us", 0) <= ack.get("t_us", 0) or m.get("t_us_approx")], static), [])
            if moves:
                c.borrows(None if ack.get("t_us_approx") else ack.get("t_us"))
                wait_reachable(ctx, f"{label}: the lease", 40)
                ctx.eq(f"{label}: the radio moves to the leased address", current_ip(ctx), LEASED)
                ctx.check(f"{label}: the session ends (an address change, A1.11a)", not alive(c.h), "status packets go on")
            else:
                c.borrows()
                ctx.check(f"{label}: the session does not end", alive(c.h), "no status packets after the lease")
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
