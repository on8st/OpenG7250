"""Tests of the management channel (A1) and level selection (A2). GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import os
import struct
import time

from .core import Ctx, RadioClock, Skip, test
from .mgmt import CMD, Mgmt, build, identify, req_retransmit, tlv


def m(ctx: Ctx) -> Mgmt:
    c = Mgmt(ctx.net)
    c.clock = RadioClock(ctx)  # the waits of 0069 in radio time
    return c


@test("T-A1-13", "T-A1-19")
def t_a1_13(ctx):
    with m(ctx) as c:
        ctx.eq("wrong magic: dropped", c.raw(b"XXXX" + build(1)[4:]), [])
        r = c.raw(build(1)[:15])
        ctx.eq("15 bytes: status 17", r[0].get("status") if r else None, 17)
        ctx.eq("11 bytes: no reply", c.raw(build(1)[:11]), [])
        r = c.raw(build(1, b"ab", declared_n=5))
        ctx.eq("length mismatch: 17", r[0]["status"] if r else None, 17)
        r = c.raw(build(1, major=2))
        ctx.eq("major 2: 18", r[0]["status"] if r else None, 18)
        ctx.eq("own version in bytes 4-5", (r[0]["major"], r[0]["minor"]) if r else None, (1, 0))
        r = c.raw(build(0x7777))
        ctx.eq("unknown command: 16", r[0]["status"] if r else None, 16)


@test("T-A1-47")
def t_a1_47(ctx):
    with m(ctx) as c:
        r = c.raw(build(1, b"", declared_n=990))
        ctx.eq("declared 18+990 > 1000: 19", r[0]["status"] if r else None, 19)
        r = c.raw(build(1, bytes(1100), declared_n=10))
        ctx.eq("long datagram, small N: 17", r[0]["status"] if r else None, 17)


@test("T-A1-43")
def t_a1_43(ctx):
    with m(ctx) as c:
        ctx.eq("type 2 (reply): no answer", c.raw(build(1, typ=2)), [])
        ctx.eq("type 2, 14 bytes: no answer", c.raw(build(1, typ=2)[:14]), [])
        r = c.req("IDENTIFY")  # liveness: the same port answers a request
        ctx.check("a type 1 request is answered", r is not None, "no reply on port 7251 within 0.5 s" if r is None else f"status {r['status']}")


@test("T-A1-01")
def t_a1_01(ctx):
    with m(ctx) as c:
        r = c.req("IDENTIFY")
        if not ctx.check("identification answered", r is not None and r["status"] == 0,
                         "no reply on port 7251 within 0.5 s" if r is None else f"status {r['status']}"):
            return
        t = tlv(r["payload"])
        ctx.eq("model", t.get(1), b"G7250")
        ctx.check("MAC present (6 bytes)", len(t.get(2, b"")) == 6, f"tag 2: {t.get(2)!r}")


@test("T-A1-53")
def t_a1_53(ctx):
    s = secret_of(ctx)
    with m(ctx) as c:
        for cmd in ("RETURN_TO_STOCK", "ENTER_VENDOR_RECOVERY"):
            r = c.authed(cmd, s)
            ctx.eq(f"{cmd}: 56", r["status"] if r else None, 56)
        t = identify(ctx, c)
        ctx.eq("flags: no stock image, no vendor recovery", t.get(8), b"\x00")
        c.authed("END_WRITE_SESSION", s)


@test("T-A1-04", "T-A1-03", "T-A1-20", "T-A1-22")
def t_a1_secret(ctx):
    """Claims the first-secret window, so it runs only where the window is open (fresh model)."""
    with m(ctx) as c, m(ctx) as c2:
        t = identify(ctx, c)
        if t.get(11) != b"\x00" or not t.get(12, b"\0")[0]:
            raise Skip("first-secret window not open (secret set or window used)")
        s = os.urandom(32)
        r = c.req("SET_FIRST_SECRET", s)
        ctx.eq("first secret set", r["status"] if r else None, 0)
        r = c2.req("SET_FIRST_SECRET", os.urandom(32))
        ctx.eq("second claim: 36", r["status"] if r else None, 36)
        t = identify(ctx, c)
        ctx.eq("identify: secret set", t.get(11), b"\x01")
        ctx.eq("identify: window closed", t.get(12, b"\1")[0], 0)
        # T-A1-03: without valid HMAC refused
        r = c.raw(build(CMD["END_WRITE_SESSION"]))
        ctx.eq("write without auth flag: 32", r[0]["status"] if r else None, 32)
        r = c.authed("END_WRITE_SESSION", s, mangle=True)
        ctx.eq("wrong HMAC: 33", r["status"] if r else None, 33)
        r = c.authed("END_WRITE_SESSION", s)
        ctx.eq("valid HMAC: 0", r["status"] if r else None, 0)
        # T-A1-20: challenge per host, used once
        ch1 = ctx.got(c.challenge(), "GET_CHALLENGE (host 1)", "no challenge on port 7251 within 0.5 s")
        ch2 = ctx.got(c2.challenge(), "GET_CHALLENGE (host 2)", "no challenge on port 7251 within 0.5 s")
        r = c.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("other host's challenge: 33", r["status"] if r else None, 33)
        r = c2.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("own challenge: 0", r["status"] if r else None, 0)
        r = c2.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("challenge reused: 33", r["status"] if r else None, 33)
        # T-A1-22: CHANGE_SECRET masked
        import hashlib, hmac
        new = os.urandom(32)
        ch = ctx.got(c.challenge(), "GET_CHALLENGE", "no challenge on port 7251 within 0.5 s")
        mask = hmac.new(s, b"G7NS" + ch, hashlib.sha256).digest()
        masked = bytes(a ^ b for a, b in zip(new, mask))
        r = c.authed("CHANGE_SECRET", s, masked, challenge=ch)
        ctx.eq("CHANGE_SECRET: 0", r["status"] if r else None, 0)
        ctx.check("new secret not in the datagram", new not in masked, "masked payload contains the secret" if new in masked else "absent")
        r = c.authed("END_WRITE_SESSION", new)
        ctx.eq("new secret works", r["status"] if r else None, 0)
        ctx.endpoint.setdefault("_learned", {})["secret"] = new.hex()


@test("T-A1-14")
def t_a1_14(ctx):
    with m(ctx) as c:
        rid = c.next_id()
        r1 = c.raw(build(CMD["GET_CHALLENGE"], b"", rid))
        r2 = c.raw(build(CMD["GET_CHALLENGE"], b"", rid))
        ctx.check("repeat answered from the stored reply (same challenge)",
                  r1 and r2 and r1[0]["payload"] == r2[0]["payload"],
                  f"replies: first {'none' if not r1 else 'one'}, repeat {'none' if not r2 else 'one'}"
                  + ("" if not (r1 and r2) else ", challenges equal" if r1[0]["payload"] == r2[0]["payload"] else ", challenges differ"))
        r3 = c.raw(build(CMD["IDENTIFY"], b"", rid))
        ctx.eq("same id, other command: executed", r3[0]["cmd"] if r3 else None, CMD["IDENTIFY"])


@test("T-A1-26")
def t_a1_26(ctx):
    with m(ctx) as c:
        for i in range(40):
            c.s.sendto(build(CMD["GET_STATS"], b"", 1000 + i), ctx.net.dest(7251))
        n = 0
        import select, time
        end = time.monotonic() + 0.8
        while (left := end - time.monotonic()) > 0:
            r, _, _ = select.select([c.s], [], [], left)
            if not r:
                break
            c.s.recvfrom(4096)
            n += 1
        ctx.check("at most 20 replies per second per source", 1 <= n <= 20, f"{n} replies")


@test("T-A2-01")
def t_a2_01(ctx):
    from .p2 import Host
    with m(ctx) as c, Host(ctx.net) as h:
        h.start()
        h.drain(0.1)
        t = identify(ctx, c)
        ctx.eq("active level 1", t.get(13), b"\x01")
        owner = t.get(14, b"")
        ctx.eq("owner port", struct.unpack(">H", owner[4:6])[0] if len(owner) == 6 else None, h.port)
        rec = need_record(ctx, h)
        ctx.eq("record level: active level 1", (rec.get(0x0B, 0) >> 4) & 0xF, 1)
        h.stop()


@test("T-A2-05")
def t_a2_05(ctx):
    s = secret_of(ctx)
    with m(ctx) as c:
        r = c.authed("SWITCH_LEVEL", s, b"\x01")
        ctx.eq("to the active level: 0", r["status"] if r else None, 0)
        r = c.authed("SWITCH_LEVEL", s, b"\x02")
        ctx.eq("to level 2 before G6: 56", r["status"] if r else None, 56)
        c.authed("END_WRITE_SESSION", s)


# ---------------- calibration (A1.12, Annex M, Annex P) ----------------
from .mgmt import FREEZE_SET, SavedCalibration, calibration_payload, config_tlv, parse_calibration  # noqa: E402
from .p2 import (ARM_MAGIC, LIMIT_SWR, LIMIT_PA_TEMP, PORT_REG, TX_ARM, TX_ARM_STATE,  # noqa: E402
                 TX_ARM_TOKEN, TX_MODE_CLASS, TX_STATE, Host, need_record, need_token, parse_regs, value,
                 write_status)


def secret_of(ctx) -> bytes:
    """The management secret: params.secret, else one the bench set earlier in this process,
    else, with the first-secret window open (factory state, A1.10; IDENTIFY tag 12), a random
    secret the bench sets now with SET_FIRST_SECRET and keeps for the following tests."""
    s = ctx.params.get("secret") or ctx.endpoint.get("_learned", {}).get("secret")
    if s:
        return bytes.fromhex(s)
    with Mgmt(ctx.net) as c:
        r = req_retransmit(c, "IDENTIFY")
        win = tlv(r["payload"]).get(12, b"") if r and r.get("status") == 0 else b""
        if win[:1] == b"\x01":
            new = os.urandom(32)
            c.clock = RadioClock(ctx)
            r = req_retransmit(c, "SET_FIRST_SECRET", new)  # flash work: 16 s (0069)
            if r and r.get("status") == 0:
                ctx.endpoint.setdefault("_learned", {})["secret"] = new.hex()
                ctx.notes.append("secret set by the bench in the open first-secret window (A1.10)")
                return new
    raise Skip("needs params.secret (the radio's management secret), and the first-secret window is not open")


def tx_bit4(h: Host) -> int | None:
    pr = h.cap_request()
    rec = {a: v for a, v in (pr or [])[1:]}
    return None if 0x20 not in rec else (rec[0x20] >> 4) & 1


def simulated_set() -> bytes:
    # Annex P: "Every test ... runs against the reference model, with simulated values"; the
    # value encoding is fixed in Table P-1 only when a parameter is set from measurements.
    return calibration_payload({p: b"\x01" for p in FREEZE_SET}, date=1_763_683_200)


def arm_once(ctx, h: Host) -> int | None:
    h.start(pa=True)
    pairs = sum((parse_regs(d) for _, d, _ in h.recv(0.3, PORT_REG)), [])
    tok = need_token(ctx, h, pairs)
    h.write([(TX_MODE_CLASS, 1)])
    return value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)


@test("T-A1-63")
def t_a1_63(ctx):
    s = secret_of(ctx)
    with m(ctx) as c, Host(ctx.net) as h:
        saved = SavedCalibration(ctx, c)
        try:
            s = _t_a1_63(ctx, c, h, s, saved)
        finally:
            saved.restore(bytes.fromhex(ctx.endpoint.get("_learned", {}).get("secret", s.hex())))
    ctx.notes.append("'while armed 50' and 'during an update 58' not reachable here: an arm needs "
                     "a session (57 comes first, Table A1-3), the update transfer is not modelled")


def _t_a1_63(ctx, c, h, s, saved):
    if True:
        h.start()
        h.drain(0.1)
        r = c.authed("SET_CALIBRATION", s, simulated_set())
        ctx.eq("while a P2 session runs: 57", r["status"] if r else None, 57)
        h.stop()
        r = c.authed("SET_CALIBRATION", s, calibration_payload({"P-XX-9": b"\x01"}))
        ctx.eq("unknown parameter id: 48", r["status"] if r else None, 48)
        g = c.req("GET_CALIBRATION")
        ctx.eq("after 48 nothing changed (set id as before)", g["payload"][:32] if g else None, saved.set_id)
        r = c.authed("SET_CALIBRATION", s, simulated_set())
        ctx.eq("complete set: 0", r["status"] if r else None, 0)
        ctx.eq("reply: TX calibrated", r["payload"][32:33] if r else None, b"\x01")
        set_id = r["payload"][:32] if r else b""
        ctx.eq("record tx bit 4 = 1 from the next session", tx_bit4(h), 1)
        t = identify(ctx, c)
        ctx.eq("IDENTIFY tag 20 (I-7)", t.get(20), b"\x01")
        ctx.eq("IDENTIFY tag 21 set id", t.get(21, b"")[:32], set_id)
        st = arm_once(ctx, h)
        ctx.eq("with bit 4 = 1 the arm is accepted (T-B10-41)", None if st is None else st & 0xF, 1)
        h.stop()
        from .t_store import reset_done, state_before
        before = state_before(c)
        r = c.authed("FACTORY_RESET", s)
        ctx.eq("FACTORY_RESET: 0", r["status"] if r else None, 0)
        reset_done(ctx, "FACTORY_RESET", before)
        new = os.urandom(32)
        r = c.req("SET_FIRST_SECRET", new)
        if ctx.eq("factory reset opened the first-secret window", r["status"] if r else None, 0):
            ctx.endpoint.setdefault("_learned", {})["secret"] = new.hex()
            s = new
        g = c.req("GET_CALIBRATION")
        ctx.eq("FACTORY_RESET keeps the set", g["payload"][:32] if g else None, set_id)
        ctx.eq("tx bit 4 still 1", tx_bit4(h), 1)
        r = c.authed("CLEAR_CALIBRATION", s)
        ctx.eq("CLEAR_CALIBRATION: 0", r["status"] if r else None, 0)
        ctx.eq("tx bit 4 = 0 from the next session", tx_bit4(h), 0)
        g = c.req("GET_CALIBRATION")
        ctx.eq("after CLEAR_CALIBRATION nothing stored (set id zeros)", g["payload"][:32] if g else None, bytes(32))
    return s


@test("T-A1-64")
def t_a1_64(ctx):
    with m(ctx) as c:
        g = ctx.got(c.req("GET_CALIBRATION"), "GET_CALIBRATION", "no reply on port 7251 within 0.5 s")
        if "P-PWR-2" in parse_calibration(g["payload"])[3]:
            raise Skip("the calibration set already holds P-PWR-2: the test needs a radio before that")
    with Host(ctx.net) as h:
        h.start()
        h.drain(0.1)
        for reg_, name in ((LIMIT_SWR, "LIMIT_SWR"), (LIMIT_PA_TEMP, "LIMIT_PA_TEMP")):
            ws = write_status(h.write([(reg_, 1500)]))
            ctx.check(f"{name} before calibration: NOT_CALIBRATED", any(a == reg_ and r == "NOT_CALIBRATED" for a, _, r in ws), str(ws))
        h.stop()
    try:
        s = secret_of(ctx)
    except Skip:
        ctx.notes.append("SET_CONFIG part not run: no management secret known")
        return
    with m(ctx) as c:
        for tag in (9, 10):
            r = c.authed("SET_CONFIG", s, config_tlv((tag, struct.pack(">H", 200))))
            ctx.eq(f"SET_CONFIG tag {tag} before calibration: 62", r["status"] if r else None, 62)
        c.authed("END_WRITE_SESSION", s)


@test("T-A1-65")
def t_a1_65(ctx):
    s = secret_of(ctx)
    with m(ctx) as c0:
        saved = SavedCalibration(ctx, c0)
        try:
            _t_a1_65(ctx, s, c0)
        finally:
            saved.restore(s)


def _t_a1_65(ctx, s, c):
    ranges = [(1.8, 2.0), (3.5, 3.9), (7.0, 7.2), (10.1, 10.15), (14.0, 14.35), (18.068, 18.168),
              (21.0, 21.45), (24.89, 24.99), (28.0, 29.7), (50.0, 54.0)]  # Table A3-0
    with Host(ctx.net) as h:
        r = c.authed("SET_CONFIG", s, config_tlv((6, b"\x01")))
        ctx.eq("region 1: 48", r["status"] if r else None, 48)
        r = c.authed("SET_CONFIG", s, config_tlv((6, b"\x00")))
        ctx.eq("region 0: 0", r["status"] if r else None, 0)
        rec = need_record(ctx, h)
        bands = [(rec[0x22 + 3 * i], rec[0x23 + 3 * i]) for i in range(rec.get(0x21, 0))]
        ctx.eq("tx_band entries = Table A3-0", bands, [(round(a * 1e6), round(b * 1e6)) for a, b in ranges])
        r = c.authed("SET_CALIBRATION", s, simulated_set())
        if not ctx.eq("simulated calibration set accepted", r["status"] if r else None, 0):
            return
        st = arm_once(ctx, h)
        if not ctx.eq("armed", None if st is None else st & 0xF, 1):
            return
        for lo, hi in ranges:
            for f, inside in ((lo * 1e6, True), (hi * 1e6, True), (lo * 1e6 - 1, False), (hi * 1e6 + 1, False)):
                h.keepalive(ptt=True, duc0_hz=f)
                ts = None
                for _, d, _ in h.recv(0.04, PORT_REG):
                    ts = value(parse_regs(d), TX_STATE) or ts
                h.keepalive(ptt=False, duc0_hz=f)
                h.recv(0.03)
                keyed = ts is not None and ts & 1 == 1
                ctx.eq(f"{f:.0f} Hz {'inside' if inside else 'outside'}", keyed, inside)
        h.stop()


# ---------------- DHCP (A1.11, A1.11a; 0041) ----------------
from .p2 import reread_endpoint, wait_reachable  # noqa: E402


def dhcp_messages(ctx) -> list[dict]:
    """dhcp.log: the runner answers {"messages": [...]}; a bare list is read too."""
    r = ctx.fixture("dhcp.log")
    if isinstance(r, dict):
        r = r.get("messages", [])
    return [m for m in (r or []) if isinstance(m, dict)]


def sock_log_read(ctx, judge: bool = True) -> list[dict]:
    """sock.log (v1.7), paged to its end; dropped entries fail the test (where judged)."""
    out: list = []
    for _ in range(1000):
        r = ctx.fixture("sock.log", action="read", **{"from": len(out)}) or {}
        out += r.get("entries", [])
        if len(out) >= r.get("count", 0):
            if judge:
                ctx.check("sock.log: no entries dropped", not r.get("dropped", 0), f"dropped {r.get('dropped', 0)}")
            break
    return out


def mgmt_borrows(entries: list[dict]):
    """(t_us of the last start-up OPEN on port 7251, [(borrow start, borrow end)]): a borrow runs
    from the CLOSE of the socket that has port 7251, through an OPEN of that socket on port 68,
    to its next OPEN on port 7251 (0041). A CLOSE without the OPEN on 68 (a reset) is no borrow.
    Entries carry "boot" (1, 2, ...) and t_us restarts at every power-on: only the last boot's
    entries are compared (the test's power cycle starts it)."""
    if any("boot" in e for e in entries):
        last = max(e.get("boot", 0) for e in entries)
        entries = [e for e in entries if e.get("boot", 0) == last]
    entries = sorted(entries, key=lambda e: e.get("t_us", 0))
    mgmt_sock, opened, start, on68, borrows = None, None, None, False, []
    for e in entries:
        act, sk, port = str(e.get("action", "")).upper(), e.get("socket"), e.get("port")
        if act == "OPEN" and port == 7251:
            if start is not None and on68 and sk == mgmt_sock:
                borrows.append((start, e["t_us"]))
            elif not borrows:
                opened = e["t_us"]  # a start-up OPEN (the last one before the first borrow)
            start, on68, mgmt_sock = None, False, sk
        elif act == "OPEN" and port == 68 and sk == mgmt_sock and start is not None:
            on68 = True
        elif act == "CLOSE" and sk == mgmt_sock and mgmt_sock is not None and start is None:
            start = e["t_us"]
    return opened, borrows


def retry_exchanges(log, linklocal: str) -> list[int]:
    """Start times (t_us) of the DHCP retry exchanges while link-local: the client's DISCOVERs
    from the link-local address (0052), those less than 2 s apart counted as one exchange (one
    borrow of at most 2 s, 0041)."""
    t = sorted(m["t_us"] for m in log if str(m.get("type", "")).upper().replace("DHCP", "") == "DISCOVER"
               and str(m.get("src", "")).split(":")[0] == linklocal and not m.get("t_us_approx"))
    # one exchange: every DISCOVER within 2.5 s of the exchange's first (a borrow lasts at most
    # 2 s, and may hold DISCOVERs 2 s apart); exchanges are at least 3 s apart (gap 4 s -1 s)
    starts: list[int] = []
    for x in t:
        if not starts or x - starts[-1] > 2_500_000:
            starts.append(x)
    return starts


def discovers(log):
    log = [m for m in log if not m.get("t_us_approx")]  # approximate stamps are not timed
    """t_us of the client's DHCPDISCOVER and DHCPREQUEST messages in a dhcp.log (only a client
    sends these two types, RFC 2131 Table 2)."""
    return [m["t_us"] for m in log if str(m.get("type", "")).upper().replace("DHCP", "") in ("DISCOVER", "REQUEST")]


@test("T-A1-66")
def t_a1_66(ctx):
    """No DHCP server: link-local address [RFC3927], retries from 4 s doubling to at most 64 s,
    each borrow of the management socket at most 2 s, so that a request repeated every 500 ms
    around a retry is answered (A1.2, A1.11; 0041)."""
    ctx.need("dhcp.server")
    ctx.need("power.cycle")
    ctx.fixture("dhcp.stop")
    socklog = "sock.log" in ctx.endpoint.get("fixtures", [])
    if socklog:
        ctx.fixture("sock.log", action="start")  # from before the power cycle: the start-up OPENs
    ctx.fixture("power.cycle")
    wait_reachable(ctx, "power-on with no DHCP server")
    # radio time when the bench first saw the P2 and management sockets open (they opened at or
    # before it; the discovery poll lags by up to 0.5 s)
    t_open = (ctx.fixture("hw.observe") or {}).get("t_us") if "hw.observe" in ctx.endpoint.get("fixtures", []) else None
    reread_endpoint(ctx)
    ip = ctx.endpoint["radio"]["ip"]
    ctx.check("link-local address while no lease is held (169.254.0.0/16)", ip.startswith("169.254."), ip)
    with m(ctx) as c:
        t = identify(ctx, c)
        ctx.check("management answers on the link-local address", t.get(1) == b"G7250", repr(t.get(1)))
    # The retries while no lease is held (A1.11; 0041). The power-on exchange is not a retry: its
    # DISCOVERs come from 0.0.0.0, the retries from the link-local address (0052). Each retry is
    # one exchange inside a borrow of at most 2 s; the 4, 8, 16, 32, 64 s are the free gaps
    # between borrows. The first link-local exchange is borrow 1, 4 s after the sockets open;
    # from it on, exchange start to exchange start is gap 8, 16, 32, 64, 64 s + the borrow before
    # (<= 2 s), each gap randomised by -1 to +1 s (RFC 2131 4.1). The sixth exchange comes about
    # 198 s after the sockets open: 240 s leaves a margin.
    # the window is 240 s of radio time from the sockets opening (the sixth exchange comes about
    # 198 s after it); on an emulator slower than real time that is longer on the wall clock, so
    # the bench counts radio time (hw.observe t_us) and caps the wall time at 900 s
    wall_cap = time.monotonic() + 900
    starts = []
    while time.monotonic() < wall_cap:
        starts = retry_exchanges(dhcp_messages(ctx), ip)
        borrows_ok = True
        if socklog:
            borrows_ok = len(mgmt_borrows(sock_log_read(ctx, judge=False))[1]) >= 6
        if len(starts) >= 6 and borrows_ok:
            break
        now_emu = (ctx.fixture("hw.observe") or {}).get("t_us") if t_open is not None else None
        if now_emu is not None and now_emu - t_open > 240e6:
            break
        if now_emu is None and time.monotonic() > wall_cap - 660:  # no radio clock: 240 s wall
            break
        time.sleep(2.0)
    if socklog:
        # v1.7: the borrows themselves, in emulated time: the management socket closed and opened
        # on port 68 until it is opened on 7251 again
        entries = sock_log_read(ctx)
        opens, borrows = mgmt_borrows(entries)
        ctx.check("sock.log: at least six borrows of the management socket within 240 s", len(borrows) >= 6,
                  f"{len(borrows)} borrows")
        lens = [round((e - b) / 1e6, 3) for b, e in borrows]
        ctx.all_of("each borrow at most 2 s (0041; sock.log, emulated)", lens, lambda x: x <= 2.0)
        free = []
        if opens is not None and borrows:
            free.append((borrows[0][0] - opens) / 1e6)
        free += [(borrows[i + 1][0] - borrows[i][1]) / 1e6 for i in range(len(borrows) - 1)]
        want = [min(4 * 2 ** n, 64) for n in range(len(free))]
        if opens is None:  # no start-up OPEN seen: the first gap judged is between borrows 1 and 2
            want = [min(8 * 2 ** n, 64) for n in range(len(free))]
            ctx.notes.append("sock.log: the start-up OPEN of port 7251 not seen; the free gap before borrow 1 not judged")
        ctx.all_of("free gaps 4 s (from the sockets opening) doubling to 64 s, each +-1 s (A1.11, RFC 2131 4.1; "
                   "sock.log, emulated)", list(zip([round(f, 3) for f in free], want)), lambda x: abs(x[0] - x[1]) <= 1.0)
    times = starts
    gaps = [(b - a) / 1e6 for a, b in zip(starts, starts[1:])]
    want = [min(8 * 2 ** n, 64) for n in range(len(gaps))]
    if socklog:
        ctx.notes.append("DISCOVER timing (dhcp.log) judged as well; sock.log gives the borrows themselves")
    ctx.check("at least six DHCP retry exchanges while link-local within 240 s", len(starts) >= 6, f"{len(starts)} exchanges")
    if starts and t_open is not None:
        first = (starts[0] - t_open) / 1e6
        ctx.check("the first link-local exchange (borrow 1) 4 s +-1 s after the sockets open (radio clock; the bench "
                  "saw them open up to 0.5 s late)", 4.0 - 1.0 - 0.5 <= first <= 4.0 + 1.0, f"{first:.2f} s after the bench saw them open")
    elif starts:
        ctx.notes.append("first exchange against the socket opening not judged: no radio clock (hw.observe t_us)")
    ctx.all_of("retry spacing from the first link-local exchange on: free gap 8 s doubling to 64 s plus the borrow "
               "before it (at most 2 s), each gap +-1 s, exchange start to exchange start",
               list(zip([round(g, 2) for g in gaps], want)), lambda x: x[1] - 1.0 <= x[0] <= x[1] + 2.0 + 1.0)
    if len(times) < 2:
        return
    # the borrow: send IDENTIFY every 500 ms from 1.5 s before the next expected retry for 6 s
    # (the retry starts 64 s +-1 s after the end of the last borrow, which lasted up to 2 s)
    nxt = times[-1] / 1e6 + 64.0
    now_emu = (ctx.fixture("hw.observe") or {}).get("t_us", 0) / 1e6 if "hw.observe" in ctx.endpoint.get("fixtures", []) else None
    if now_emu is None:
        ctx.notes.append("borrow part not run: no emulated clock (hw.observe t_us)")
        return
    wait = nxt - 1.5 - now_emu
    if wait > 70:
        ctx.notes.append(f"borrow part not run: next retry {wait:.0f} s away")
        return
    time.sleep(max(0.0, wait))
    answered = []
    with m(ctx) as c:
        rid = c.next_id()
        for i in range(12):
            r = c.raw(build(CMD["IDENTIFY"], b"", rid), timeout=0.45)  # one request id: retransmissions
            answered.append(bool(r))
            time.sleep(0.05)
    ctx.check("a request repeated every 500 ms for 6 s around a retry is answered", any(answered), str(answered))
    longest, run = 0, 0
    for a in answered:
        run = 0 if a else run + 1
        longest = max(longest, run)
    ctx.check("no gap longer than 2 s (4 unanswered tries) during the borrow", longest <= 4, f"longest run unanswered: {longest} x 500 ms")


# ---------------- start-up bound and DHCP while link-local (0048, 0052) ----------------
START_LEASE_S, START_LINKLOCAL_S = 5.0, 17.0  # A1.11 "Bound on start-up" (0048, 0057: 17 s without DHCP)


def link_up_t_us(ctx) -> int | None:
    """The radio's link-up time from a trace "link" event where the runner gives one (proposed
    for v1.6), else None."""
    if not ctx.trace_file:
        return None
    from .trace import load
    ups = [e for e in load(ctx.trace_file) if e.get("kind") == "link" and e.get("up", 1)]
    return ups[-1]["t_us"] if ups else None


def reachable_both(ctx, limit_s: float) -> float | None:
    """Seconds until both IDENTIFY and P2 discovery are answered (polled every 0.25 s), or None."""
    from .p2 import Host, reread_endpoint
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit_s + 5:
        if "net.endpoint" in ctx.endpoint.get("fixtures", []):
            reread_endpoint(ctx)
        try:
            with Host(ctx.net) as h, Mgmt(ctx.net) as c:
                if h.discover(0.25) and c.req("IDENTIFY", timeout=0.25):
                    return time.monotonic() - t0
        except Skip:
            time.sleep(0.25)
    return None


def startup(ctx, label: str, limit_s: float):
    ctx.fixture("power.cycle")
    t_cycle = time.monotonic()
    emu0 = None
    took = reachable_both(ctx, limit_s)
    if took is None:
        ctx.check(f"{label}: IDENTIFY and discovery answered", False, f"nothing within {limit_s + 5:.0f} s of the power cycle")
        return
    up = link_up_t_us(ctx)
    if up is not None and "hw.observe" in ctx.endpoint.get("fixtures", []):
        emu0 = (ctx.fixture("hw.observe") or {}).get("t_us")
    if up is not None and emu0 is not None:
        # the answer came by emu0 (emulated); from link up that is at most:
        since = (emu0 - up) / 1e6
        ctx.check(f"{label}: answered within {limit_s:.0f} s of link up (trace link event, emulated clock)",
                  since <= limit_s, f"answered by {since:.1f} s after link up (upper bound)")
    else:
        ctx.check(f"{label}: answered within {limit_s:.0f} s, counted from the power cycle (includes boot and "
                  "link negotiation, so stricter than the spec; no link-up event offered)", took <= limit_s, f"{took:.1f} s")


@test("T-A1-69")
def t_a1_69(ctx):
    ctx.need("dhcp.server")
    ctx.need("power.cycle")
    ctx.fixture("dhcp.start")
    try:
        startup(ctx, "DHCP server answering at once", START_LEASE_S)
    finally:
        ctx.fixture("dhcp.stop")
    startup(ctx, "no DHCP server (link-local)", START_LINKLOCAL_S)
    ctx.notes.append("'after both announcements' not checked yet: needs the ARP frames of eth.log (v1.6)")
    ip = ctx.endpoint["radio"]["ip"]
    ctx.check("with no DHCP server the answers come from the link-local address", ip.startswith("169.254."), ip)


def bflag(m: dict):
    """The BOOTP broadcast flag of a dhcp.log message: "broadcast" (bool) or "flags" bit 15."""
    if "broadcast" in m:
        return bool(m["broadcast"])
    if "flags" in m:
        return bool(int(m["flags"]) & 0x8000)
    return None


def mtype(m: dict) -> str:
    return str(m.get("type", "")).upper().replace("DHCP", "")


@test("T-A1-74")
def t_a1_74(ctx):
    """A1.11, 0052: DHCP source address at power-on and while link-local."""
    ctx.need("dhcp.server")
    ctx.need("power.cycle")
    from .p2 import wait_reachable
    ctx.fixture("dhcp.stop")
    ctx.fixture("power.cycle")
    wait_reachable(ctx, "power-on with no DHCP server", START_LINKLOCAL_S + 5)
    ll = ctx.endpoint["radio"]["ip"]
    if not ctx.check("link-local address after the power-on exchange", ll.startswith("169.254."), ll):
        return
    first = [m for m in (dhcp_messages(ctx)) if mtype(m) == "DISCOVER"]
    ctx.check("power-on DHCPDISCOVER (before any address): IP source 0.0.0.0", bool(first) and str(first[0].get("src", "")).split(":")[0] == "0.0.0.0",
              str(first[0]) if first else "no DISCOVER in dhcp.log")
    # wait for a retry while link-local (from 4 s, doubling), then let a server answer
    deadline, retry = time.monotonic() + 20, None
    while time.monotonic() < deadline and retry is None:
        time.sleep(0.5)
        retry = next((m for m in (dhcp_messages(ctx)) if mtype(m) == "DISCOVER"
                      and str(m.get("src", "")).split(":")[0] == ll), None)
    if not ctx.check("a DHCPDISCOVER retry while link-local", retry is not None, "none within 20 s"):
        return
    ctx.eq("while link-local: DISCOVER source is the link-local address, ciaddr 0.0.0.0, BROADCAST flag set",
           (str(retry.get("src", "")).split(":")[0], retry.get("ciaddr"), bflag(retry)), (ll, "0.0.0.0", True))
    ctx.fixture("dhcp.start")
    try:
        deadline, req = time.monotonic() + 40, None
        while time.monotonic() < deadline and req is None:
            time.sleep(0.5)
            req = next((m for m in (dhcp_messages(ctx)) if mtype(m) == "REQUEST"), None)
        if ctx.check("a DHCPREQUEST after the server's OFFER", req is not None, "none within 40 s"):
            ctx.eq("while link-local: REQUEST source is the link-local address, ciaddr 0.0.0.0, BROADCAST flag set",
                   (str(req.get("src", "")).split(":")[0], req.get("ciaddr"), bflag(req)), (ll, "0.0.0.0", True))
            took = wait_reachable(ctx, "the lease", 10)
            ip = ctx.endpoint["radio"]["ip"]
            ctx.check("the broadcast OFFER and ACK give the radio its lease (reachable at the leased address)",
                      not ip.startswith("169.254."), f"{ip} after {took:.1f} s")
    finally:
        ctx.fixture("dhcp.stop")


# ---------------- flash work and transmit (B2.3 "Flash work"; 0055) ----------------
@test("T-B5-21")
def t_b5_21(ctx):
    """B2.3 "Flash work" (0055) in the order of Table A1-3 (36, 35, 58, 57, 50): while armed,
    CHANGE_SECRET gets 50, SET_CALIBRATION 57 (a P2 session runs), and neither changes anything;
    FACTORY_RESET disarms before its reply; armed with the first-secret window open (after the
    factory reset), SET_FIRST_SECRET gets 50; unarmed, a SET_CONFIG reply comes after its flash
    write."""
    import hashlib
    import hmac
    import threading
    from .core import Blocked
    from .p2 import wait_reachable
    from .t_cw import pending_reason
    s = secret_of(ctx)
    with m(ctx) as c, Host(ctx.net) as h:
        if tx_bit4(h) != 1:
            raise Blocked(pending_reason(ctx))
        st = arm_once(ctx, h)
        if not ctx.eq("armed", None if st is None else st & 0xF, 1):
            return
        h.keepalive()
        cfg0, cal0 = c.req("GET_CONFIG"), c.req("GET_CALIBRATION")
        ch = ctx.got(c.challenge(), "GET_CHALLENGE", "no challenge")
        mask = hmac.new(s, b"G7NS" + ch, hashlib.sha256).digest()
        r = c.authed("CHANGE_SECRET", s, bytes(a ^ b for a, b in zip(os.urandom(32), mask)), challenge=ch)
        ctx.eq("armed: CHANGE_SECRET refused with 50", r["status"] if r else None, 50)
        h.keepalive()
        r = c.authed("SET_CALIBRATION", s, simulated_set())
        ctx.eq("armed: SET_CALIBRATION refused with 57 (a P2 session runs; 57 comes before 50, Table A1-3)",
               r["status"] if r else None, 57)
        h.keepalive()
        r = c.req("SET_FIRST_SECRET", os.urandom(32))
        ctx.eq("SET_FIRST_SECRET with a secret set: 36 (before 50, Table A1-3)", r["status"] if r else None, 36)
        h.keepalive()
        r = c.authed("END_WRITE_SESSION", s)
        ctx.eq("the old secret still works (CHANGE_SECRET changed nothing)", r["status"] if r else None, 0)
        cfg1, cal1 = c.req("GET_CONFIG"), c.req("GET_CALIBRATION")
        ctx.check("no flash work while armed: configuration and calibration set unchanged",
                  bool(cfg1 and cal1) and cfg1.get("payload") == (cfg0 or {}).get("payload")
                  and cal1.get("payload") == (cal0 or {}).get("payload"),
                  "unchanged" if cfg1 and cal1 else "GET_CONFIG or GET_CALIBRATION unanswered")
        # FACTORY_RESET while armed: a commanded reset (A1.9): unkey and disarm, then the reply
        h.keepalive()
        pushes: list = []
        t = threading.Thread(target=lambda: pushes.extend(h.recv(1.0, PORT_REG)))
        from .t_store import reset_done, state_before
        before = state_before(c)
        h.keepalive()  # the log was read meanwhile
        t.start()
        r = c.authed("FACTORY_RESET", s)
        t_reply = time.monotonic()
        t.join()
        ctx.eq("armed: FACTORY_RESET answered 0", r["status"] if r else None, 0)
        disarm = [tt for _, d, tt in pushes if (value(parse_regs(d), TX_ARM_STATE) if value(parse_regs(d), TX_ARM_STATE) is not None else 1) & 0xF == 0]
        ctx.check("FACTORY_RESET disarmed before its reply (A1.9; no flash work while armed)",
                  # two sockets read by two threads: the bench cannot order arrivals closer than this
                  bool(disarm) and disarm[0] <= t_reply + 0.005,
                  f"disarm push {format((disarm[0] - t_reply) * 1000, '+.2f') + ' ms' if disarm else 'not seen'} relative to the reply "
                  "(5 ms: the bench's resolution between two sockets)")
    # after the factory reset: the network back to its default, the secret cleared, the window open
    reset_done(ctx, "FACTORY_RESET", before)
    with m(ctx) as c, Host(ctx.net) as h:
        st = arm_once(ctx, h)
        if ctx.eq("armed again after the factory reset", None if st is None else st & 0xF, 1):
            h.keepalive()
            r = c.req("SET_FIRST_SECRET", os.urandom(32))
            ctx.eq("armed, window open: SET_FIRST_SECRET refused with 50", r["status"] if r else None, 50)
        h.stop()
        time.sleep(0.1)
        new = os.urandom(32)
        r = c.req("SET_FIRST_SECRET", new)
        if ctx.eq("unarmed, window open: SET_FIRST_SECRET 0", r["status"] if r else None, 0):
            ctx.endpoint.setdefault("_learned", {})["secret"] = new.hex()
            s = new
        # unarmed SET_CONFIG: the reply comes after the flash write, so the radio is not stalled
        # once it has answered
        r = c.authed("SET_CONFIG", s, config_tlv((6, b"\x00")))
        ctx.eq("unarmed SET_CONFIG: 0", r["status"] if r else None, 0)
        from .p2 import PORT_DISC, discovery
        t0 = time.monotonic()
        h.send(PORT_DISC, discovery())
        d = h.recv(0.05, PORT_DISC, until=lambda g: bool(g))
        ctx.check("right after the SET_CONFIG reply the radio answers discovery within 50 ms (its flash write "
                  "ended before the reply)", bool(d), f"{(d[0][2] - t0) * 1000:.1f} ms" if d else "no reply within 50 ms")
        c.authed("END_WRITE_SESSION", s)  # leave the write session free for the next test


# ---------------- malformed payload (A1.4; 0056) ----------------
@test("T-A1-75")
def t_a1_75(ctx):
    """A payload without the structure of Annex M is 17 before every state check; a well-formed
    one with a disallowed value keeps its command-specific code after them."""
    s = secret_of(ctx)
    past_end = struct.pack(">IB", 0, 2) + b"tb" + struct.pack(">B", 6) + b"P-RF-1" + struct.pack(">H", 10) + b"\x01"
    unknown = calibration_payload({"P-XX-9": b"\x01"})
    with m(ctx) as c:
        _t_a1_75(ctx, c, s, past_end, unknown)


def _t_a1_75(ctx, c, s, past_end, unknown):
    with Host(ctx.net) as h:
        armed = False
        if tx_bit4(h) == 1:
            st = arm_once(ctx, h)
            armed = ctx.eq("armed during the session", None if st is None else st & 0xF, 1)
        else:
            h.start()
            ctx.notes.append("armed part not run: TX locked (record tx bit 4 = 0); the session part ran")
        where = "armed and in a P2 session" if armed else "in a P2 session"
        h.keepalive()
        r = c.req("SET_FIRST_SECRET", os.urandom(31))
        ctx.eq(f"{where}: SET_FIRST_SECRET with 31 bytes gets 17 (not 36, 57 or 50)", r["status"] if r else None, 17)
        h.keepalive()
        r = c.authed("SET_CALIBRATION", s, past_end)
        ctx.eq(f"{where}: SET_CALIBRATION with a TLV past the end gets 17", r["status"] if r else None, 17)
        h.keepalive()
        r = c.authed("SET_CALIBRATION", s, unknown)
        ctx.eq(f"{where}: well-formed SET_CALIBRATION with an unknown id gets 57", r["status"] if r else None, 57)
        h.stop()
    ctx.notes.append("'outside a session while armed, 50' not reachable over P2: an arm ends with its session "
                     "(Table A3-1); it needs another level")
    time.sleep(0.1)
    # the same management socket: it holds the write session (A1.10)
    r = c.authed("SET_CALIBRATION", s, unknown)
    ctx.eq("unarmed, no session: the unknown id gets 48 (command-specific, after the state checks)",
           r["status"] if r else None, 48)
    c.authed("END_WRITE_SESSION", s)



# ---------------- link-local address used unprobed (A1.11; 0057) ----------------
FAULT_FLAGS = 0x0212


@test("T-A1-76")
def t_a1_76(ctx):
    """MACRAW on socket 0 fails to open: the chosen link-local address is used unprobed, reported
    by IDENTIFY tag 24, the log, GET_STATS ll_unprobed; IP_CONFLI still gives it up; no A3.5
    fault."""
    from .mgmt import read_log, stats
    from .p2 import Host, reread_endpoint, wait_reachable
    ctx.need("dhcp.server")
    ctx.need("power.cycle")
    ctx.fixture("dhcp.stop")
    if "macraw.fail" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("macraw.fail", on=True)
    else:
        ctx.notes.append("no macraw.fail fixture: relies on the runner having no MACRAW (judged by IDENTIFY tag 24)")
    try:
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "power-on with MACRAW failing", START_LINKLOCAL_S + 5)
        ll = ctx.endpoint["radio"]["ip"]
        ctx.check("link-local address in use", ll.startswith("169.254."), ll)
        with m(ctx) as c:
            t = identify(ctx, c)
            if t.get(24) == b"\x00":
                raise Skip("MACRAW opened on this runner (IDENTIFY tag 24 = 0): not the unprobed path")
            ctx.eq("IDENTIFY tag 24 = 1 (address in use without a complete probe)", t.get(24), b"\x01")
            texts = [e[2] for e in read_log(c)]
            want = f"link-local {ll} used without a complete RFC 3927 probe"
            ctx.check(f"log entry containing '{want}'", any(want in x for x in texts),
                      f"{len(texts)} entries" + (f", last {texts[-1]!r}" if texts else ""))
            st = stats(c) or {}
            ctx.check("GET_STATS ll_unprobed raised", st.get("ll_unprobed", 0) >= 1, f"ll_unprobed {st.get('ll_unprobed')}")
        with Host(ctx.net) as h:
            ff = value(h.write([(FAULT_FLAGS, 0)]), FAULT_FLAGS)
        ctx.eq("no A3.5 fault (FAULT_FLAGS 0)", ff, 0)
        ctx.notes.append("console line 'G7250 FAULT LINKLOCAL <address> UNPROBED' not checked: no console fixture")
        if "ip.conflict" in ctx.endpoint.get("fixtures", []):
            ctx.fixture("ip.conflict", action="raise")
            time.sleep(1.0)
            took = wait_reachable(ctx, "IP_CONFLI", START_LINKLOCAL_S + 5)
            new = ctx.endpoint["radio"]["ip"]
            ctx.check("IP_CONFLI still gives the unprobed address up: a new link-local address", new != ll and new.startswith("169.254."),
                      f"{ll} -> {new} after {took:.1f} s")
        else:
            ctx.notes.append("IP_CONFLI part not run: no ip.conflict fixture (format v1.6)")
    finally:
        if "macraw.fail" in ctx.endpoint.get("fixtures", []):
            ctx.fixture("macraw.fail", on=False)
