"""Tests of the management channel (A1) and level selection (A2). GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import os
import struct
import time

from .core import Ctx, Skip, test
from .mgmt import CMD, Mgmt, build, identify, tlv


def m(ctx: Ctx) -> Mgmt:
    return Mgmt(ctx.net)


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
    secret = ctx.params.get("secret") or ctx.endpoint.get("_learned", {}).get("secret")
    with m(ctx) as c:
        if secret is None:
            raise Skip("needs params.secret (the radio's management secret)")
        s = bytes.fromhex(secret)
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
    secret = ctx.params.get("secret") or ctx.endpoint.get("_learned", {}).get("secret")
    if not secret:
        raise Skip("needs params.secret")
    s = bytes.fromhex(secret)
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
    s = ctx.params.get("secret") or ctx.endpoint.get("_learned", {}).get("secret")
    if not s:
        raise Skip("needs params.secret (the radio's management secret)")
    return bytes.fromhex(s)


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
        r = c.authed("FACTORY_RESET", s)
        ctx.eq("FACTORY_RESET: 0", r["status"] if r else None, 0)
        time.sleep(0.2)
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


def discovers(log):
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
    ctx.fixture("power.cycle")
    wait_reachable(ctx, "power-on with no DHCP server")
    reread_endpoint(ctx)
    ip = ctx.endpoint["radio"]["ip"]
    ctx.check("link-local address while no lease is held (169.254.0.0/16)", ip.startswith("169.254."), ip)
    with m(ctx) as c:
        t = identify(ctx, c)
        ctx.check("management answers on the link-local address", t.get(1) == b"G7250", repr(t.get(1)))
    # wait for six attempts: 4 + 8 + 16 + 32 + 64 s after the first, about 2 min
    deadline = time.monotonic() + 200
    times = []
    while time.monotonic() < deadline:
        times = discovers(ctx.fixture("dhcp.log") or [])
        if len(times) >= 6:
            break
        time.sleep(2.0)
    gaps = [(b - a) / 1e6 for a, b in zip(times, times[1:])]
    want = [4, 8, 16, 32, 64][:len(gaps)]
    # RFC 2131 4.1: each delay randomised by -1 to +1 s
    ctx.check("at least six DHCP attempts within 200 s", len(times) >= 6, f"{len(times)} attempts")
    ctx.all_of("retry spacing 4 s doubling to 64 s (each +-1 s)", list(zip([round(g, 2) for g in gaps], want)),
               lambda x: abs(x[0] - x[1]) <= 1.0)
    if len(times) < 2:
        return
    # the borrow: send IDENTIFY every 500 ms from 0.5 s before the next expected retry for 2.5 s
    nxt = times[-1] / 1e6 + 64.0
    now_emu = (ctx.fixture("hw.observe") or {}).get("t_us", 0) / 1e6 if "hw.observe" in ctx.endpoint.get("fixtures", []) else None
    if now_emu is None:
        ctx.notes.append("borrow part not run: no emulated clock (hw.observe t_us)")
        return
    wait = nxt - 0.5 - now_emu
    if wait > 70:
        ctx.notes.append(f"borrow part not run: next retry {wait:.0f} s away")
        return
    time.sleep(max(0.0, wait))
    answered = []
    with m(ctx) as c:
        rid = c.next_id()
        for i in range(6):
            r = c.raw(build(CMD["IDENTIFY"], b"", rid), timeout=0.45)  # one request id: retransmissions
            answered.append(bool(r))
            time.sleep(0.05)
    ctx.check("a request repeated every 500 ms for 2.5 s around a retry is answered", any(answered), str(answered))
    longest, run = 0, 0
    for a in answered:
        run = 0 if a else run + 1
        longest = max(longest, run)
    ctx.check("no gap longer than 2 s (4 unanswered tries) during the borrow", longest <= 4, f"longest run unanswered: {longest} x 500 ms")
