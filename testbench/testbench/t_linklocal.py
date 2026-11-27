"""T-A1-70 to T-A1-73: the IPv4 link-local address, its probe, announcements and conflicts (A1.11,
A1.6 C-4; 0048, 0049; [RFC3927] sections 2 and 9). Emulator: eth.log, arp.peer, ip.conflict,
sock.log, dhcp.server. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import time

from .core import Ctx, PartSkip, test
from .mgmt import config_tlv, read_log
from .p2 import PORT_DDC, Host, wait_reachable

# [RFC3927] section 9
PROBE_WAIT, PROBE_NUM, PROBE_MIN, PROBE_MAX = 1.0, 3, 1.0, 2.0
ANNOUNCE_WAIT, ANNOUNCE_NUM, ANNOUNCE_INTERVAL = 2.0, 2, 2.0
MAX_CONFLICTS, RATE_LIMIT_INTERVAL = 10, 60.0
SLACK = 0.05  # s: the emulator's frame stamps against the RFC's bounds
START_BOUND_S, CLOSED_S = 17.0, 10.0  # A1.11: start-up with no DHCP answer; a probe while running
ZERO = "0.0.0.0"


# ---------------- judgments on the frames (pure: unit-tested) ----------------
def arps(entries: list[dict]) -> list[tuple[float, int, str, str]]:
    """(t s, op, sender IP, target IP) of the ARP frames the radio sent, in order."""
    out = []
    for e in entries:
        a = e.get("arp")
        if e.get("dir") == "from_radio" and isinstance(a, dict):
            out.append((e["t_us"] / 1e6, int(a.get("op", 0)), a.get("spa", ""), a.get("tpa", "")))
    return out


def probes(frames, addr: str | None = None) -> list[tuple[float, str]]:
    """RFC 3927 2.2.1: ARP requests with sender IP 0.0.0.0; (t, target)."""
    return [(t, tpa) for t, op, spa, tpa in frames if op == 1 and spa == ZERO and (addr is None or tpa == addr)]


def announcements(frames, addr: str) -> list[float]:
    """RFC 3927 2.3: ARP requests with sender and target IP the address."""
    return [t for t, op, spa, tpa in frames if op == 1 and spa == addr and tpa == addr]


def judge_probe(ctx, label: str, ts: list[float]) -> bool:
    """PROBE_NUM probes, PROBE_MIN to PROBE_MAX apart."""
    gaps = [round(b - a, 3) for a, b in zip(ts, ts[1:])]
    return ctx.check(f"{label}: {PROBE_NUM} probes, {PROBE_MIN:g} to {PROBE_MAX:g} s apart ([RFC3927] section 9)",
                     len(ts) == PROBE_NUM and all(PROBE_MIN - SLACK <= g <= PROBE_MAX + SLACK for g in gaps),
                     f"{len(ts)} probes, gaps {gaps} s")


def judge_announce(ctx, label: str, last_probe: float, ts: list[float]) -> bool:
    """ANNOUNCE_NUM announcements, the first ANNOUNCE_WAIT after the last probe, ANNOUNCE_INTERVAL apart."""
    ok = len(ts) == ANNOUNCE_NUM and ts[0] - last_probe >= ANNOUNCE_WAIT - SLACK and \
        all(abs((b - a) - ANNOUNCE_INTERVAL) <= SLACK for a, b in zip(ts, ts[1:]))
    return ctx.check(f"{label}: {ANNOUNCE_NUM} announcements, the first {ANNOUNCE_WAIT:g} s after the last probe, "
                     f"{ANNOUNCE_INTERVAL:g} s apart ([RFC3927] section 9)", ok,
                     f"after the last probe {[round(t - last_probe, 3) for t in ts]} s")


def no_defence(frames, addr: str, after: float) -> list[float]:
    """Frames that defend `addr` after a conflict ([RFC3927] 2.5 option (a): none): an ARP with it as sender."""
    return [t for t, op, spa, tpa in frames if t > after and spa == addr]


NO_FRAMES = ("not judged: eth.log holds no ARP frame from the radio (the emulator does not model the frames of its "
             "link-local probe), so this says nothing about the image")


def frames_or_part(ctx, label: str, entries: list[dict]):
    """The radio's ARP frames, or None with a part and no pass for the test when eth.log holds none."""
    f = arps(entries)
    if f:
        return f
    ctx.part(f"{label}: {NO_FRAMES}")
    ctx.no_pass = True
    return None


# ---------------- fixtures ----------------
def need(ctx, *fx):
    for f in fx:
        ctx.need(f)


def eth_start(ctx):
    ctx.fixture("eth.log", action="start")


def eth_read(ctx) -> list[dict]:
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("eth.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            ctx.check("eth.log: no entries dropped", not r.get("dropped", 0), f"dropped {r.get('dropped', 0)}")
            return out
    return out


def socks(ctx) -> list[dict] | None:
    if "sock.log" not in ctx.endpoint.get("fixtures", []):
        return None
    from .t_mgmt import sock_log_read
    return sock_log_read(ctx)


def opened(sock: list[dict], port: int, after_s: float = 0.0) -> float | None:
    """The first OPEN of a socket on `port` in the last boot after `after_s`, in s."""
    boot = max((e.get("boot", 0) for e in sock), default=0)
    t = [e["t_us"] / 1e6 for e in sock if e.get("boot", 0) == boot and e.get("action") == "OPEN" and e.get("port") == port
         and e["t_us"] / 1e6 >= after_s]
    return min(t) if t else None


def no_dhcp(ctx):
    if "dhcp.server" in ctx.endpoint.get("fixtures", []) or "dhcp.stop" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("dhcp.stop")


def stored_mode(ctx) -> bytes | None:
    """The stored C-1 address mode (GET_CONFIG tag 1), read without the secret where the radio answers so,
    else with it; None where it cannot be read."""
    from .core import Skip
    from .mgmt import Mgmt
    from .t_store import get_config
    with Mgmt(ctx.net) as c:
        mode = get_config(c).get(1)
    if mode is not None:
        return mode
    from .t_dhcpmode import address_mode
    from .t_mgmt import secret_of
    try:
        return address_mode(ctx, secret_of(ctx))
    except Skip:
        return None


def dhcp_mode_stored(ctx, label: str) -> bool:
    """Before a power cycle that expects a DHCP start: the stored C-1 address mode set to DHCP. A start brings up
    the stored configuration (A1.6, "survive a restart"), so a static address an earlier test left stored would
    be in force. False (a part, no pass) where the mode cannot be read or set."""
    from .core import Skip
    mode = stored_mode(ctx)
    if mode == b"\x00":
        return True
    if mode is not None:
        from .t_dhcpmode import set_config
        from .t_mgmt import secret_of
        try:
            st = set_config(ctx, secret_of(ctx), (1, b"\x00"))
        except Skip:
            st = None
        if st == 0:
            ctx.notes.append(f"{label}: the stored address mode was {mode.hex()} at the start (left by an earlier test): "
                             "set back to DHCP before the power cycle")
            return True
    ctx.part(f"{label}: the stored address mode is {'unreadable' if mode is None else mode.hex()} and could not be set to "
             "DHCP: the start may bring up a static configuration (A1.6)")
    ctx.no_pass = True
    return False


def link_local_start(ctx, label: str) -> tuple[list, list | None]:
    """A power cycle with no DHCP server: the radio's ARP frames and the socket log of that start."""
    dhcp_mode_stored(ctx, label)
    no_dhcp(ctx)
    eth_start(ctx)
    if "sock.log" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("sock.log", action="start")
    ctx.fixture("power.cycle")
    wait_reachable(ctx, label, 40)
    entries = eth_read(ctx)
    return entries, socks(ctx)


def current_ip(ctx) -> str:
    return ctx.endpoint["radio"]["ip"]


def is_link_local(ip: str) -> bool:
    a = [int(x) for x in ip.split(".")]
    return a[0] == 169 and a[1] == 254 and 1 <= a[2] <= 254


# ---------------- the tests ----------------
@test("T-A1-70", "T-A1-71", own_boot=True)
def t_a1_70(ctx: Ctx):
    """A1.11 (0048, 0049): the probe at start-up with no DHCP server, the announcements, the sockets after
    them within the start-up bound; a conflict during the probe makes the radio choose another address."""
    need(ctx, "eth.log", "arp.peer", "power.cycle")
    entries, sock = link_local_start(ctx, "a power cycle with no DHCP server")
    frames = frames_or_part(ctx, "the probe at start-up", entries)
    if frames is None:
        return
    p = probes(frames)
    if not ctx.check("the radio probes a link-local address", bool(p) and is_link_local(p[0][1]), f"{p[:3]}"):
        return
    addr = p[0][1]
    pa = [t for t, _ in probes(frames, addr)]
    judge_probe(ctx, f"the probe of {addr}", pa)
    an = announcements(frames, addr)
    judge_announce(ctx, f"the announcements of {addr}", pa[-1], an)
    ctx.eq("the address in use is the one probed and announced", current_ip(ctx), addr)
    if sock is None:
        ctx.part("'only then answers discovery and IDENTIFY', and the start-up bound: not judged, no sock.log")
    elif an:
        disc, mg = opened(sock, 1024), opened(sock, 7251)
        ctx.check("discovery (port 1024) and IDENTIFY (port 7251) open only after the last announcement (sock.log)",
                  disc is not None and mg is not None and min(disc, mg) >= an[-1], f"1024 at {disc}, 7251 at {mg}, last announcement {an[-1]}")
        first = min([e["t_us"] / 1e6 for e in entries if e.get("dir") == "from_radio"], default=None)
        # link up comes before the radio's first frame, so this is never stricter than "from link up"
        ctx.check(f"the sockets open within {START_BOUND_S:g} s of the radio's first frame (A1.11, from link up)",
                  first is not None and mg is not None and max(disc or 0, mg) - first <= START_BOUND_S,
                  f"first frame {first}, sockets {disc}, {mg}")
    # a conflict during the probe: an ARP reply claiming that address
    ctx.fixture("arp.peer", action="set", ip=addr, answer_probe=True, defend=True)
    try:
        entries, _ = link_local_start(ctx, "a power cycle with the probed address taken")
    finally:
        ctx.fixture("arp.peer", action="clear")
    frames = frames_or_part(ctx, "the probe with the address taken", entries)
    if frames is None:
        return
    other = [tpa for _, tpa in probes(frames) if tpa != addr]
    if not ctx.check(f"an ARP reply claiming {addr} during its probe: the radio chooses another address and probes it",
                     bool(other) and is_link_local(other[0]), f"probed {[tpa for _, tpa in probes(frames)][:6]}"):
        return
    new = other[0]
    judge_probe(ctx, f"the probe of {new}", [t for t, _ in probes(frames, new)])
    ctx.eq("and uses it", current_ip(ctx), new)
    ctx.check(f"{addr} is never announced", not announcements(frames, addr), "announced")


def session(ctx) -> Host:
    h = Host(ctx.net)
    h.start()
    h.recv_alive(0.3)
    return h


REACT_S = 10.0  # radio s: the image's reaction to a conflict looked for (it gives the address up at once, A1.11)


def reaction(sock: list[dict] | None, frames: list | None, t0_s: float, old_ip: str, ip_now: str) -> str | None:
    """How the image's reaction to a conflict raised at t0_s shows, or None (pure): the CLOSE of discovery (port
    1024) in sock.log, else a probe in eth.log (sender 0.0.0.0), else the endpoint's address no longer the old
    one."""
    if sock:
        boot = max((e.get("boot", 0) for e in sock), default=0)
        t = [e["t_us"] / 1e6 for e in sock if e.get("boot", 0) == boot and e.get("action") == "CLOSE"
             and e.get("port") == 1024 and e["t_us"] / 1e6 >= t0_s]
        if t:
            return f"port 1024 closed at {min(t):.3f} s (sock.log)"
    if frames:
        p = [t for t, _ in probes(frames) if t >= t0_s]
        if p:
            return f"a probe at {p[0]:.3f} s (eth.log)"
    if ip_now != old_ip:
        return f"the address {ip_now} instead of {old_ip}"
    return None


def wait_reaction(ctx, t0_s: float | None, old_ip: str) -> str | None:
    """Wait, in radio time, for the image's reaction to a conflict raised at t0_s (reaction()); None if none came
    within REACT_S. Raising the next conflict before it would find the chip's conflict still set, and the two
    would be one (emu 398f93d: 'already_set')."""
    from .core import RadioClock
    from .p2 import reread_endpoint
    clock, wall0 = RadioClock(ctx), time.monotonic()
    r0 = clock.now()
    fx = ctx.endpoint.get("fixtures", [])
    while True:
        if "net.endpoint" in fx:
            reread_endpoint(ctx)
        sock = socks(ctx) if t0_s is not None else None
        frames = arps(eth_read_quiet(ctx)) if t0_s is not None and "eth.log" in fx else None
        why = reaction(sock, frames, t0_s if t0_s is not None else 0.0, old_ip, current_ip(ctx))
        if why or clock.now() - r0 >= REACT_S or time.monotonic() - wall0 > 12 * REACT_S + 60:
            return why
        time.sleep(0.2)


def eth_read_quiet(ctx) -> list[dict]:
    """eth.log to its end, not judged (a poll; the reading the test judges comes after)."""
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("eth.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            return out
    return out


def reachable_moved(ctx, label: str, old_ip: str, limit_s: float = 40.0):
    """Reachable on an address other than old_ip (an address given up after a conflict), bounded in radio time;
    reachable on old_ip at the end is left to the test's own checks."""
    from .core import RadioClock
    clock, wall0 = RadioClock(ctx), time.monotonic()
    r0 = clock.now()
    while True:
        wait_reachable(ctx, label, limit_s)
        if current_ip(ctx) != old_ip or clock.now() - r0 >= limit_s or time.monotonic() - wall0 > 12 * limit_s + 60:
            return
        time.sleep(0.5)


def conflict(ctx, label: str, moves: bool = True):
    """ip.conflict raise; the frames and socket log until the image has reacted and the radio is reachable again
    (on another address where `moves`): (eth.log, sock.log, raise time s, the reaction or None)."""
    old_ip = current_ip(ctx)
    eth_start(ctx)
    if "sock.log" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("sock.log", action="start")
    t0 = (ctx.fixture("hw.observe") or {}).get("t_us") if "hw.observe" in ctx.endpoint.get("fixtures", []) else None
    t0_s = t0 / 1e6 if t0 is not None else None
    ctx.fixture("ip.conflict", action="raise")
    why = wait_reaction(ctx, t0_s, old_ip)
    if moves:
        reachable_moved(ctx, label, old_ip)
    else:
        wait_reachable(ctx, label, 40)
    return eth_read(ctx), socks(ctx), t0_s, why


def conflict_logged(ctx, label: str, s: bytes, n0: int):
    from .t_update import updating
    with updating(ctx, s, restart_at_end=False) as u:
        texts = [e[2] for e in read_log(u.c) if e[0] > n0]
    ctx.check(f"{label}: logged", any("conflict" in x.lower() for x in texts), f"{texts[-4:]}")


def last_seq(ctx, s: bytes) -> int:
    from .t_update import last_log_seq, updating
    with updating(ctx, s, restart_at_end=False) as u:
        return last_log_seq(u)


@test("T-A1-72", own_boot=True)
def t_a1_72(ctx: Ctx):
    """A1.11 (0049): a conflict while running on a link-local address: given up at once, no defending ARP,
    the session ended as an address change (A1.11a; B5.4 item 3), a new probe with discovery and management
    closed for at most 10 s, logged; after 10 conflicts 60 s before the next probe. The session is observed by
    a continuous record (t_dhcpmode.Stream), not by a probe whose RUN=1 would start a new one (A2.3)."""
    need(ctx, "eth.log", "ip.conflict", "power.cycle")
    from .t_dhcpmode import Case, former_net
    from .t_mgmt import secret_of
    link_local_start(ctx, "a power cycle with no DHCP server")
    addr = current_ip(ctx)
    if not ctx.check("running on a link-local address", is_link_local(addr), addr):
        return
    s = secret_of(ctx)
    n0 = last_seq(ctx, s)
    label = "a conflict on the link-local address"
    c = Case.observing(ctx, label)
    try:
        c.preset_tx()
        entries, sock, t0, why = conflict(ctx, label)
        reacted(ctx, label, why)
        c.ended(ctx.net, c.stream.now(), former_net(ctx))
    finally:
        c.close()
    frames = frames_or_part(ctx, "no defending ARP, and the new probe", entries)
    if frames is not None:
        after = t0 if t0 is not None else frames[0][0]
        ctx.eq(f"no defending ARP for {addr} after the conflict ([RFC3927] 2.5 option (a))", no_defence(frames, addr, after), [])
        p = probes(frames)
        new = [tpa for _, tpa in p if tpa != addr]
        if ctx.check("a new address probed", bool(new), f"{p[:4]}"):
            judge_probe(ctx, f"the probe of {new[0]}", [t for t, _ in probes(frames, new[0])])
            ctx.eq("the new address in use", current_ip(ctx), new[0])
    if sock is None:
        ctx.part("'discovery and management closed for at most 10 s': not judged, no sock.log")
    else:
        closes = [e["t_us"] / 1e6 for e in sock if e.get("action") == "CLOSE" and e.get("port") in (1024, 7251)]
        back = opened(sock, 1024, min(closes) if closes else 0.0)
        ctx.check(f"discovery and management closed during the probe for at most {CLOSED_S:g} s (sock.log)",
                  bool(closes) and back is not None and back - min(closes) <= CLOSED_S, f"closed at {closes[:2]}, port 1024 again at {back}")
    conflict_logged(ctx, "the conflict", secret_of(ctx), n0)
    # MAX_CONFLICTS: after the 10th conflict, 60 s before the next probe. Each raise waits for the image's
    # reaction to the one before: raised earlier, it finds the chip's conflict still set and the two are one.
    taken = 1 if why else 0
    for i in range(2, MAX_CONFLICTS):
        _, _, _, w = conflict(ctx, f"conflict {i}")
        taken += reacted(ctx, f"conflict {i}", w)
    old_ip = current_ip(ctx)
    eth_start(ctx)
    t_raise = (ctx.fixture("hw.observe") or {}).get("t_us") if "hw.observe" in ctx.endpoint.get("fixtures", []) else None
    ctx.fixture("ip.conflict", action="raise")
    reachable_moved(ctx, f"conflict {MAX_CONFLICTS}", old_ip, 120)
    frames = frames_or_part(ctx, f"after {MAX_CONFLICTS} conflicts, 60 s before the next probe", eth_read(ctx))
    if frames is None:
        return
    p = probes(frames)
    if taken < MAX_CONFLICTS - 1:
        raise PartSkip(f"after {MAX_CONFLICTS} conflicts, 60 s before the next probe: not judged, the image was seen to take "
                       f"only {taken} of the first {MAX_CONFLICTS - 1}")
    if t_raise is None or not p:
        raise PartSkip(f"after {MAX_CONFLICTS} conflicts, 60 s before the next probe: not judged (hw.observe {t_raise}, probes {len(p)})")
    ctx.check(f"after {MAX_CONFLICTS} conflicts the next probe waits {RATE_LIMIT_INTERVAL:g} s ([RFC3927] section 9)",
              p[0][0] - t_raise / 1e6 >= RATE_LIMIT_INTERVAL - SLACK, f"first probe {p[0][0] - t_raise / 1e6:.2f} s after the conflict")


def reacted(ctx, label: str, why: str | None) -> bool:
    """'Gives the address up at once' (A1.11), seen within REACT_S of radio time; the evidence as a note."""
    if why:
        ctx.notes.append(f"{label}: the image reacted: {why}")
    return ctx.check(f"{label}: the address given up (seen within {REACT_S:g} s of radio time)", why is not None,
                     "no CLOSE of port 1024, no probe, the same address")


DHCP_AGAIN_S = 30.0  # radio s: [RFC2131] 3.1 item 5 (10 s after a DECLINE) + the probe (10 s) + the first retry (4 s) + slack


def discover_after_decline(kinds: list[str]) -> bool:
    d = kinds.index("DECLINE") if "DECLINE" in kinds else None
    return d is not None and "DISCOVER" in kinds[d + 1:]


def wait_dhcp_again(ctx, before: int, limit_s: float = DHCP_AGAIN_S) -> list[str]:
    """The kinds of the DHCP messages since `before`, read until a DISCOVER follows the DECLINE or limit_s of radio
    time have passed."""
    from .core import RadioClock
    from .t_mgmt import dhcp_messages, mtype
    clock, wall0 = RadioClock(ctx), time.monotonic()
    r0 = clock.now()
    while True:
        kinds = [mtype(x) for x in dhcp_messages(ctx)[before:]]
        if discover_after_decline(kinds) or clock.now() - r0 >= limit_s or time.monotonic() - wall0 > 12 * limit_s + 60:
            return kinds
        time.sleep(0.5)


def static_back(ctx, s: bytes, mode0: bytes | None):
    """The stored address mode put back to mode0 (DHCP where unknown), so that the next start is as before."""
    want = mode0 if mode0 is not None else b"\x00"
    if stored_mode(ctx) != want:
        from .t_dhcpmode import set_config
        st = set_config(ctx, s, (1, want))
        if st != 0:
            ctx.part(f"the stored address mode not put back to {want.hex()} (status {st}): the next start may be static")


@test("T-A1-73", own_boot=True)
def t_a1_73(ctx: Ctx):
    """A1.6 C-4, A1.11 (0049): a conflict on a DHCP-leased address: DHCPDECLINE, link-local, DHCP again; on a
    static address: the previous configuration, or link-local; each logged."""
    need(ctx, "eth.log", "ip.conflict", "power.cycle", "dhcp.server")
    from .t_mgmt import dhcp_messages, secret_of
    from .t_store import get_config
    from .t_update import updating
    dhcp_mode_stored(ctx, "a power cycle with a DHCP server")
    ctx.fixture("dhcp.start")
    try:
        eth_start(ctx)
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "a power cycle with a DHCP server", 40)
        s = secret_of(ctx)
        n0 = last_seq(ctx, s)
        before = len(dhcp_messages(ctx))
        entries, _, _, _ = conflict(ctx, "a conflict on the leased address")
        kinds = wait_dhcp_again(ctx, before)
        ctx.check("a conflict on the leased address: DHCPDECLINE sent", "DECLINE" in kinds, f"{kinds}")
        frames = frames_or_part(ctx, "the link-local probe after the DECLINE", entries)
        if frames is not None:
            p = probes(frames)
            ctx.check("then a link-local address probed", bool(p) and is_link_local(p[0][1]), f"{p[:3]}")
        if "DECLINE" in kinds and not discover_after_decline(kinds):
            # A1.11 (0049) bounds no time for "DHCP again": not seen within the bench's window is no fail
            ctx.part(f"and DHCP restarted (a DISCOVER after the DECLINE): none within {DHCP_AGAIN_S:g} s of radio time "
                     f"after the radio was reachable again ({kinds}); the spec gives no bound, so not judged")
            ctx.no_pass = True
        else:
            ctx.check("and DHCP restarted (a DISCOVER after the DECLINE)", discover_after_decline(kinds), f"{kinds}")
        conflict_logged(ctx, "the conflict on the leased address", s, n0)
    finally:
        ctx.fixture("dhcp.stop")
    # a static address: the previous configuration (DHCP here), else link-local
    link_local_start(ctx, "a power cycle with no DHCP server")
    s = secret_of(ctx)
    ip = current_ip(ctx)
    mode0 = None
    try:
        with updating(ctx, s, restart_at_end=False) as u:
            conf0 = get_config(u.c)
            mode0 = conf0.get(1)
            static = bytes([169, 254, 77, 77])
            r = u.req("SET_CONFIG", config_tlv((1, b"\x01"), (2, static), (3, bytes([255, 255, 0, 0])), (4, bytes(4))))
            if not ctx.eq("a static address set: 0", r["status"] if r else None, 0):
                return
        time.sleep(0.5)
        wait_reachable(ctx, "the static address", 40)
        n0 = last_seq(ctx, s)
        conflict(ctx, "a conflict on the static address")
        with updating(ctx, s, restart_at_end=False) as u:
            conf = get_config(u.c)
            ctx.eq("a conflict on the static address: back to the previous configuration (C-4: address mode as before)",
                   conf.get(1), conf0.get(1))
        conflict_logged(ctx, "the conflict on the static address", s, n0)
    finally:
        static_back(ctx, s, mode0)  # also after an error: a static mode left stored is in force at the next start
    ctx.part("a conflict on a static address with no previous configuration (link-local): not run, the bench cannot "
             "remove the previous configuration")
    ctx.notes.append(f"the radio started on {ip}; the static address used was 169.254.77.77")
