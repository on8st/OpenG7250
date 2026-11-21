"""Takeover and its notice to the old owner: A2.4 step 1, "Repeated notice", "Notice
destination", "Notice to every old owner" (0022 §29, §48, §58, §77 item 2, §79, §87 item 2).
T-A2-10, -11, -13, -15, -16, -17.

Hosts: two sockets of hosts[0] are two hosts, since a host is its source IP address and UDP
port (A2.2). A register destination (General bytes 31-32) is a third socket of the same host
entry, so that a runner maps it; the spec's example ports (50000, 6000) stand for any non-zero
port. The tests run with TX locked: the old owner is not armed, which A2.4 step 1 covers in
full (drop reason 4 TAKEOVER, a new token, the three copies and the repeated notice).

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import select
import struct
import time

from .core import Ctx, test
from .p2 import (PORT_DISC, PORT_DUCIQ, PORT_REG, TX_ARM_STATE, TX_ARM_TOKEN, Host, general,
                 need_token, parse_regs, value, write_status)

DROP_TAKEOVER = 4  # TX_ARM_STATE [23:16] (Table B8-3)
NOTICE_S = 5.0  # the repeated notice lasts 5 s
NOTICE_MIN_S = 0.1  # at most one notice per 100 ms
COPY_GAP_MS = (5, 25)  # "about 10 ms apart", where the host clock is the radio's clock
COPY_GAP_LOOSE_MS = (1, 50)  # on a host clock that is not the radio's (an emulator before v1.6)


def same_clock(ctx) -> bool:
    """The host's wall clock is the radio's clock: the reference model (its endpoint has a
    "model" entry). On an emulator emulated time can run slower than wall time, so 10 ms gaps
    measured on the host are not the radio's gaps (until v1.6 stamps sends in emulated time)."""
    return "model" in ctx.endpoint


def netlog_start(ctx) -> bool:
    """net.log (format v1.6): the radio's sends and arrivals stamped in emulated time."""
    if "net.log" not in ctx.endpoint.get("fixtures", []):
        return False
    ctx.fixture("net.log", action="start")
    ctx.__dict__["_netlog"] = True
    return True


def netlog_read(ctx) -> list[dict]:
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("net.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            ctx.check("net.log: no entries dropped", not r.get("dropped", 0), f"dropped {r.get('dropped', 0)}")
            return out
    return out


def netlog_copies(ctx, port: int) -> list[float] | None:
    """Emulated send times of the TX_ARM_STATE packets from radio port 7250 to a host port."""
    if not ctx.__dict__.get("_netlog"):
        return None
    t = []
    for e in netlog_read(ctx):
        head = bytes.fromhex(e.get("head", ""))
        if e.get("dir") == "from_radio" and e.get("radio_port") == PORT_REG and str(e.get("peer", "")).endswith(f":{port}") \
                and len(head) >= 6 and struct.unpack_from(">H", head, 4)[0] == TX_ARM_STATE:
            t.append(e["t_us"])
    return t


def emu_now(ctx) -> float | None:
    """The radio's time in seconds, from hw.observe t_us (emulated on an emulator)."""
    if "hw.observe" not in ctx.endpoint.get("fixtures", []):
        return None
    o = ctx.fixture("hw.observe") or {}
    return o["t_us"] / 1e6 if "t_us" in o else None


def wait_radio_time(ctx, h_keep, t0: float | None, wall0: float, at: float):
    """Wait until `at` seconds after t0 on the radio's clock (emulated where offered, else the
    wall clock from wall0), keeping host h_keep's session alive."""
    while True:
        now = emu_now(ctx) if t0 is not None else None
        el = (now - t0) if now is not None else time.monotonic() - wall0
        if el >= at:
            return el
        h_keep.keepalive()
        time.sleep(min(0.2, max(0.01, (at - el) / 2)))


def seq_of(data: bytes) -> int:
    return struct.unpack_from(">I", data, 0)[0]


def notices(pkts):
    """(t, seq, pairs) of every register packet carrying TX_ARM_STATE."""
    out = []
    for p, d, t in pkts:
        if p != PORT_REG:
            continue
        pairs = parse_regs(d)
        if value(pairs, TX_ARM_STATE) is not None:
            out.append((t, seq_of(d), pairs))
    return out


def recv_multi(hosts: dict, timeout: float, tick=None, every: float = 0.0) -> dict:
    """Datagrams of several hosts at once, each stamped when it arrives (one select over all
    sockets, so reading one host never delays the stamps of another): {name: [(port, data, t)]}.
    `tick()` is called every `every` seconds (to send while receiving)."""
    out = {k: [] for k in hosts}
    by_sock = {h.s: k for k, h in hosts.items()}
    end = time.monotonic() + timeout
    next_tick = time.monotonic()
    while (now := time.monotonic()) < end:
        if tick and now >= next_tick:
            tick()
            next_tick = now + every
        wait = min(end, next_tick) - now if tick else end - now
        r, _, _ = select.select(list(by_sock), [], [], max(0.0, wait))
        t = time.monotonic()
        for sk in r:
            data, src = sk.recvfrom(4096)
            h = hosts[by_sock[sk]]
            out[by_sock[sk]].append((h.net.radio_port(src), data, t))
    return out


def duc_iq() -> bytes:
    return bytes(1444)


def begin(h: Host, reg_dest: int = 0):
    """A session of host h; its packets of the first 0.3 s."""
    h.start(reg_dest=reg_dest)
    got = []
    for _ in range(3):
        h.keepalive()
        got += h.recv(0.1)
    return got


def keep(h: Host, s: float, every: float = 0.2):
    """The owner keeps its session alive for s seconds; what it received."""
    got, end = [], time.monotonic() + s
    while time.monotonic() < end:
        h.keepalive()
        got += h.recv(min(every, max(0.0, end - time.monotonic())))
    return got


def check_copies(ctx, label: str, got, last_seq: int | None, port: int | None = None):
    """The three copies of the final TX_ARM_STATE (A2.4 step 1)."""
    n = notices(got)
    ctx.eq(f"{label}: TX_ARM_STATE copies", len(n), 3)
    if not n:
        return n
    st = value(n[0][2], TX_ARM_STATE)
    ctx.eq(f"{label}: disarmed, drop reason 4 TAKEOVER", (st & 0xF, (st >> 16) & 0xFF), (0, DROP_TAKEOVER))
    ctx.check(f"{label}: identical register pairs", all(x[2] == n[0][2] for x in n), str([x[2] for x in n]))
    seqs = [x[1] for x in n]
    ctx.check(f"{label}: consecutive sequence numbers of port 7250", all(seqs[i + 1] == seqs[i] + 1 for i in range(len(seqs) - 1)), str(seqs))
    if last_seq is not None:
        ctx.eq(f"{label}: the first copy continues the old session's port 7250 counter (B2.7)", seqs[0], last_seq + 1)
    gaps = [round((n[i + 1][0] - n[i][0]) * 1000, 1) for i in range(len(n) - 1)]
    lo, hi = COPY_GAP_MS if same_clock(ctx) else COPY_GAP_LOOSE_MS
    where = "host clock = radio clock" if same_clock(ctx) else "host wall clock, loose: not the emulated clock"
    sent = netlog_copies(ctx, port) if port else None
    if sent and len(sent) >= 3:
        gaps = [round((sent[i + 1] - sent[i]) / 1000, 1) for i in range(2)]
        lo, hi, where = COPY_GAP_MS[0], COPY_GAP_MS[1], "net.log send times, emulated clock"
    ctx.check(f"{label}: about 10 ms apart ({lo}-{hi} ms, {where})", all(lo <= g <= hi for g in gaps), f"{gaps} ms")
    return n


def last_reg_seq(got) -> int | None:
    s = [seq_of(d) for p, d, _ in got if p == PORT_REG]
    return s[-1] if s else None


@test("T-A2-10")
def t_a2_10(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        before = begin(a)
        last = last_reg_seq(before)
        b.keepalive()  # the takeover: B's first C&C packet
        got = a.recv(1.0)  # A sends nothing now: any C&C from A would take the radio back
        keep(b, 0.0)
        n = check_copies(ctx, "old owner A", got, last, a.port)
        # a stream packet the radio sent before it handled the takeover can still arrive after
        # the first copy (P2 orders nothing between ports); one arriving after the second copy,
        # about 10 ms later, was sent after the takeover
        after = n[1][0] if len(n) >= 2 else 0.0
        other = [(p, len(d)) for p, d, t in got if t > after
                 and not (p == PORT_REG and value(parse_regs(d), TX_ARM_STATE) is not None)]
        ctx.eq("no other stream packet or register push to A after the takeover (from the second copy on)", other, [])
        d = a.discover(0.5)
        ctx.check("a discovery from A is still answered (B3.1)", bool(d), f"{len(d)} replies" if d else "no discovery reply")
        b.stop()


@test("T-A2-11")
def t_a2_11(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        begin(a)
        b.keepalive()
        a.recv(0.1)  # the three copies
        # A sends DUC I&Q every 30 ms for 1 s; B keeps its session
        t0, got, last_b = time.monotonic(), [], 0.0
        while time.monotonic() - t0 < 1.0:
            a.send(PORT_DUCIQ, duc_iq())
            if time.monotonic() - last_b > 0.2:
                b.keepalive()
                last_b = time.monotonic()
            got += a.recv(0.03)
        n = notices(got)
        gaps = [n[i + 1][0] - n[i][0] for i in range(len(n) - 1)]
        ctx.check("DUC I&Q from A within 5 s: the TAKEOVER notice repeated", len(n) >= 5, f"{len(n)} notices in 1 s")
        ctx.check("at most one notice per 100 ms", all(g >= NOTICE_MIN_S - 0.01 for g in gaps),
                  f"smallest gap {min(gaps) * 1000:.0f} ms" if gaps else "fewer than 2 notices")
        if n:
            st = value(n[0][2], TX_ARM_STATE)
            ctx.eq("the notice is the final TX_ARM_STATE (TAKEOVER)", (st >> 16) & 0xFF, DROP_TAKEOVER)
        # a register packet from A also gets the notice (and its NOT_OWNER answer); first let
        # 100 ms pass since the last notice
        a.recv(0.15)
        a.send(PORT_REG, b"\0\0\0\0" + struct.pack(">HI", TX_ARM_TOKEN, 0))
        got = a.recv(0.3)
        ctx.check("a register packet from A within 5 s: a notice", bool(notices(got)), f"{len(got)} packets")
        # a C&C packet from A takes the radio back: B gets three notices that continue B's
        # session counter; the NOT_OWNER answers to A while B was owner did not advance it (0051)
        before_b = keep(b, 0.3)
        b.send(PORT_REG, b"\0\0\0\0" + struct.pack(">HI", TX_ARM_TOKEN, 0))  # B's own read
        before_b += b.recv(0.2)
        last_b_seq = last_reg_seq(before_b)
        a.keepalive()
        got_b = b.recv(0.3)
        b_seqs = [seq_of(d) for p, d, _ in before_b + got_b if p == PORT_REG]
        ctx.check("B's port 7250 stream has no gap while A got answers (0051)",
                  bool(b_seqs) and all(b_seqs[i + 1] == b_seqs[i] + 1 for i in range(len(b_seqs) - 1)), str(b_seqs))
        got_a = a.recv(0.3)
        a.keepalive()
        got_a += a.recv(0.2)
        check_copies(ctx, "C&C from A: B displaced", got_b, last_b_seq, b.port)
        hp = [seq_of(d) for p, d, _ in got_a if p == 1025]
        ctx.check("streams to A restart at sequence 0 (B2.7)", bool(hp) and hp[0] == 0, f"port 1025 sequence numbers {hp[:5]}")
        a.stop()


@test("T-A2-13")
def t_a2_13(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b, Host(ctx.net) as dest:
        netlog_start(ctx)
        begin(a, reg_dest=dest.port)
        dest.recv(0.05)
        b.keepalive()
        got_dest = dest.recv(0.5)
        got_a = a.recv(0.05)
        check_copies(ctx, f"General bytes 31-32 = {dest.port}: copies at that port", got_dest, None, dest.port)
        ctx.eq("no copy at A's source port", len(notices(got_a)), 0)
        # writes from A every 30 ms for 0.6 s: NOT_OWNER at its source port, notices at dest
        n_tick = [0]

        def tick():
            a.send(PORT_REG, b"\0\0\0\0" + struct.pack(">HI", TX_ARM_TOKEN, 0))
            n_tick[0] += 1
            if n_tick[0] % 7 == 0:
                b.keepalive()
        # 1.5 s of host time: at least 0.45 s of radio time even on an emulator at 0.3 of real
        # time, room for three answers and three notices at one per 100 ms
        got = recv_multi({"a": a, "dest": dest, "b": b}, 1.5, tick, 0.03)
        at_src, at_dest = got["a"], got["dest"]
        no_owner = [d for p, d, _ in at_src
                    if p == PORT_REG and any(r == "NOT_OWNER" for _, _, r in write_status(parse_regs(d)))]
        nd = notices(at_dest)
        log = netlog_read(ctx) if ctx.__dict__.get("_netlog") else None
        if log:
            # in radio time: the span of A's writes as the radio received them, and how many
            # answers and notices that span allows at one per 100 ms each (both at their limit:
            # neither consumes the other)
            mine = lambda e, port: str(e.get("peer", "")).endswith(f":{port}")
            w = [e["t_us"] for e in log if e.get("dir") == "to_radio" and e.get("radio_port") == PORT_REG and mine(e, a.port)]
            span = (max(w) - min(w)) / 1e6 if len(w) > 1 else 0.0
            want = max(1, int(span / 0.1))  # one per 100 ms, less one for the edges of the span
            ans = [e for e in log if e.get("dir") == "from_radio" and e.get("radio_port") == PORT_REG and mine(e, a.port)]
            nts = [e for e in log if e.get("dir") == "from_radio" and e.get("radio_port") == PORT_REG and mine(e, dest.port)]
            ctx.check("register writes from A: NOT_OWNER answers at A's source port, one per 100 ms of radio time (B8.5; net.log)",
                      len(ans) >= want, f"{len(ans)} answers over {span:.3f} s of writes, want >= {want}")
            ctx.check("and notices at the register destination, one per 100 ms of radio time (net.log)", len(nts) >= want,
                      f"{len(nts)} notices over {span:.3f} s of writes, want >= {want}")
        else:
            ctx.check("register writes from A: NOT_OWNER answers at A's source port (B8.5)", len(no_owner) >= 3,
                      f"{len(no_owner)} answers at the source port in 1.5 s (host clock)")
            ctx.check("and notices at the register destination", len(nd) >= 3, f"{len(nd)} notices in 1.5 s (host clock)")
        gaps = [nd[i + 1][0] - nd[i][0] for i in range(len(nd) - 1)]
        ctx.check("at most one notice per 100 ms", all(g >= NOTICE_MIN_S - 0.01 for g in gaps),
                  f"smallest gap {min(gaps) * 1000:.0f} ms" if gaps else "fewer than 2")
        ctx.eq("no notice at A's source port", len(notices(at_src)), 0)
        b.stop()
    # bytes 31-32 = 0: the copies at A's source port
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        begin(a)
        b.keepalive()
        ctx.eq("General bytes 31-32 = 0: the copies at A's source port", len(notices(a.recv(0.5))), 3)
        b.stop()


@test("T-A2-15")
def t_a2_15(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        begin(a)
        # the takeover packet is B's General: its effect, the unasked capability record (B4.4),
        # must not wait for the second and third copy to A
        b.send(PORT_DISC, general())
        t_sent = time.monotonic()
        got = recv_multi({"a": a, "b": b}, 0.2)
        got_a, got_b = got["a"], got["b"]
        n = notices(got_a)
        rec = [t for p, d, t in got_b if p == PORT_REG and any(x[0] < 0x0100 for x in parse_regs(d))]
        if ctx.check("B receives the unasked capability record at its first General", bool(rec), f"{len(got_b)} packets to B"):
            if len(n) >= 2:
                ctx.check("the record to B comes before the second copy to A (steps 2-5 do not wait)",
                          rec[0] < n[1][0], f"record +{(rec[0] - t_sent) * 1000:.1f} ms, copy 2 +{(n[1][0] - t_sent) * 1000:.1f} ms")
            else:
                ctx.check("A receives the copies", False, f"{len(n)} copies")
        b.keepalive()
        b.stop()


def token_now(ctx, h: Host) -> int:
    return need_token(ctx, h, [])


@test("T-A2-16")
def t_a2_16(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        tok_a = need_token(ctx, a, sum((parse_regs(d) for p, d, _ in begin(a) if p == PORT_REG), []))
        b.keepalive()
        t_take = time.monotonic()
        t_take_radio = emu_now(ctx)  # the radio's clock at the takeover (just after it)
        n = check_copies(ctx, "A not armed", a.recv(0.5), None, a.port)
        b.keepalive()
        tok_b = token_now(ctx, b)
        ctx.check("the token in force after the takeover differs from the one before (A3.2)", tok_b != tok_a,
                  f"before {tok_a:#010x}, after {tok_b:#010x}")
        # repeated notice: DUC I&Q from A at 1.5 s and 4.5 s after the takeover answered, at 5.6 s not
        res = {}
        clock = "emulated (hw.observe t_us)" if t_take_radio is not None else "wall"
        for at in (1.5, 4.5, 5.6):
            wait_radio_time(ctx, b, t_take_radio, t_take, at)
            a.recv(0.01)
            a.send(PORT_DUCIQ, duc_iq())
            res[at] = len(notices(a.recv(0.2)))
        log = netlog_read(ctx) if ctx.__dict__.get("_netlog") else None
        if log:
            # emulated arrival of B's takeover packet, of A's DUC I&Q, and the notices sent to A
            take = [e["t_us"] for e in log if e.get("dir") == "to_radio" and str(e.get("peer", "")).endswith(f":{b.port}")]
            iq = [e["t_us"] for e in log if e.get("dir") == "to_radio" and e.get("radio_port") == PORT_DUCIQ
                  and str(e.get("peer", "")).endswith(f":{a.port}")]
            sent = [e["t_us"] for e in log if e.get("dir") == "from_radio" and e.get("radio_port") == PORT_REG
                    and str(e.get("peer", "")).endswith(f":{a.port}")]
            if take and iq:
                t0 = take[0]
                rows = []
                for t in iq:
                    answered = any(t <= s_ < t + 200_000 for s_ in sent)
                    rows.append((round((t - t0) / 1e6, 2), answered, (t - t0) < NOTICE_S * 1e6))
                ctx.check("repeated notice: every DUC I&Q from A within 5 s of the takeover answered, none after "
                          "(net.log, emulated clock)", all(a_ == w for _, a_, w in rows), str(rows))
                return b.stop()
        ctx.eq(f"repeated notice for DUC I&Q at 1.5 s and 4.5 s, none at 5.6 s after the takeover ({clock} clock)",
               (res[1.5] >= 1, res[4.5] >= 1, res[5.6]), (True, True, 0))
        b.stop()


@test("T-A2-17")
def t_a2_17(ctx):
    with Host(ctx.net) as a, Host(ctx.net) as b:
        netlog_start(ctx)
        # A: no memory-mapped packet, bytes 31-32 = 0, unarmed
        got = begin(a)
        tok = value(sum((parse_regs(d) for p, d, _ in got if p == PORT_REG), []), TX_ARM_TOKEN)
        b.keepalive()
        check_copies(ctx, "A without register use", a.recv(0.5), None, a.port)
        t0, got, last_b = time.monotonic(), [], 0.0
        while time.monotonic() - t0 < 0.8:
            a.send(PORT_DUCIQ, duc_iq())
            if time.monotonic() - last_b > 0.2:
                b.keepalive()
                last_b = time.monotonic()
            got += a.recv(0.03)
        n = notices(got)
        gaps = [n[i + 1][0] - n[i][0] for i in range(len(n) - 1)]
        ctx.check("its DUC I&Q within 5 s: repeated notices, at most one per 100 ms",
                  len(n) >= 4 and all(g >= NOTICE_MIN_S - 0.01 for g in gaps), f"{len(n)} notices, gaps {[round(g * 1000) for g in gaps]} ms")
        b.keepalive()
        st = value(b.write([(TX_ARM_STATE, 0)]), TX_ARM_STATE)
        ctx.eq("TX_ARM_STATE read by B: drop reason 4 TAKEOVER", None if st is None else (st >> 16) & 0xFF, DROP_TAKEOVER)
        tok_b = token_now(ctx, b)
        ctx.check("and a new token", tok is not None and tok_b != tok, f"before {tok}, after {tok_b}")
        b.stop()
    # a session carried over at a restart by the run bit (B5.4 item 1), bytes 31-32 = dest
    with Host(ctx.net) as a, Host(ctx.net) as b, Host(ctx.net) as dest:
        begin(a, reg_dest=dest.port)
        a.stop()
        a.keepalive()  # restart with the run bit only: the General (bytes 31-32) carried over
        a.recv(0.1)
        dest.recv(0.2)
        b.keepalive()
        got = dest.recv(0.5)
        ctx.eq(f"carried session with bytes 31-32 = {dest.port}: the copies at that port", len(notices(got)), 3)
        b.stop()


def regs_from(h: Host, timeout: float = 0.2):
    return [(seq_of(d), parse_regs(d)) for p, d, _ in h.recv(timeout) if p == PORT_REG]


@test("T-B8-21")
def t_b8_21(ctx):
    """B2.7, 0051: answers to a non-owner on port 7250 are numbered by their own counter, which
    only a reset of the radio resets; the owner's register stream shows no gap."""
    from .p2 import CAP_REQUEST, wait_reachable
    fresh = "power.cycle" in ctx.endpoint.get("fixtures", [])
    if fresh:
        ctx.fixture("power.cycle")
        wait_reachable(ctx)
    with Host(ctx.net) as a, Host(ctx.net) as c, Host(ctx.net) as b:
        got = begin(a)
        a_seqs = [seq_of(d) for p, d, _ in got if p == PORT_REG]
        c_seqs = []
        read = b"\0\0\0\0" + struct.pack(">HI", TX_ARM_TOKEN, 0)
        for i in range(4):
            c.send(PORT_REG, read)  # NOT_OWNER answer
            if i == 1:
                c.send(PORT_REG, b"\0\0\0\0" + struct.pack(">HI", CAP_REQUEST, 1))
            c_seqs += [q for q, _ in regs_from(c, 0.15)]
            a.keepalive()
            a.send(PORT_REG, read)  # the owner's own read
            a_seqs += [q for q, _ in regs_from(a, 0.1)]
        if fresh:
            ctx.check("after a reset the non-owner counter starts at 0", bool(c_seqs) and c_seqs[0] == 0, str(c_seqs))
        ctx.check("answers to C: consecutive numbers of the non-owner counter",
                  len(c_seqs) >= 4 and all(c_seqs[i + 1] == c_seqs[i] + 1 for i in range(len(c_seqs) - 1)), str(c_seqs))
        ctx.check("A's register stream: no gap from the answers to C",
                  len(a_seqs) >= 4 and all(a_seqs[i + 1] == a_seqs[i] + 1 for i in range(len(a_seqs) - 1)), str(a_seqs))
        # not reset by a session end ...
        a.stop()
        time.sleep(0.15)
        c.send(PORT_REG, read)
        after_end = [q for q, _ in regs_from(c, 0.2)]
        ctx.check("A's session end does not reset it", bool(after_end) and bool(c_seqs) and after_end[0] == c_seqs[-1] + 1,
                  f"last {c_seqs[-1] if c_seqs else None}, then {after_end}")
        # ... nor by a takeover (B takes over from A)
        begin(a)
        b.keepalive()
        a.recv(0.1)
        time.sleep(0.15)
        c.send(PORT_REG, read)
        after_take = [q for q, _ in regs_from(c, 0.2)]
        last = (after_end or c_seqs or [None])[-1]
        ctx.check("a takeover does not reset it", bool(after_take) and last is not None and after_take[0] == last + 1,
                  f"last {last}, then {after_take}")
        b.stop()
