"""Tests of the management channel (A1) and level selection (A2). GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import os
import struct

from .core import Ctx, Skip, test
from .mgmt import CMD, Mgmt, build, tlv


def m(ctx: Ctx) -> Mgmt:
    return Mgmt(ctx.radio_ip, ctx.host_ip)


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


@test("T-A1-01")
def t_a1_01(ctx):
    with m(ctx) as c:
        r = c.req("IDENTIFY")
        if not ctx.check("identification answered", r is not None and r["status"] == 0):
            return
        t = tlv(r["payload"])
        ctx.eq("model", t.get(1), b"G7250")
        ctx.check("MAC present (6 bytes)", len(t.get(2, b"")) == 6)


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
        t = tlv(c.req("IDENTIFY")["payload"])
        ctx.eq("flags: no stock image, no vendor recovery", t.get(8), b"\x00")
        c.authed("END_WRITE_SESSION", s)


@test("T-A1-04", "T-A1-03", "T-A1-20", "T-A1-22")
def t_a1_secret(ctx):
    """Claims the first-secret window, so it runs only where the window is open (fresh model)."""
    with m(ctx) as c, m(ctx) as c2:
        t = tlv(c.req("IDENTIFY")["payload"])
        if t.get(11) != b"\x00" or not t.get(12, b"\0")[0]:
            raise Skip("first-secret window not open (secret set or window used)")
        s = os.urandom(32)
        r = c.req("SET_FIRST_SECRET", s)
        ctx.eq("first secret set", r["status"] if r else None, 0)
        r = c2.req("SET_FIRST_SECRET", os.urandom(32))
        ctx.eq("second claim: 36", r["status"] if r else None, 36)
        t = tlv(c.req("IDENTIFY")["payload"])
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
        ch1, ch2 = c.challenge(), c2.challenge()
        r = c.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("other host's challenge: 33", r["status"] if r else None, 33)
        r = c2.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("own challenge: 0", r["status"] if r else None, 0)
        r = c2.authed("END_WRITE_SESSION", s, challenge=ch2)
        ctx.eq("challenge reused: 33", r["status"] if r else None, 33)
        # T-A1-22: CHANGE_SECRET masked
        import hashlib, hmac
        new = os.urandom(32)
        ch = c.challenge()
        mask = hmac.new(s, b"G7NS" + ch, hashlib.sha256).digest()
        masked = bytes(a ^ b for a, b in zip(new, mask))
        r = c.authed("CHANGE_SECRET", s, masked, challenge=ch)
        ctx.eq("CHANGE_SECRET: 0", r["status"] if r else None, 0)
        ctx.check("new secret not in the datagram", new not in masked)
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
                  r1 and r2 and r1[0]["payload"] == r2[0]["payload"])
        r3 = c.raw(build(CMD["IDENTIFY"], b"", rid))
        ctx.eq("same id, other command: executed", r3[0]["cmd"] if r3 else None, CMD["IDENTIFY"])


@test("T-A1-26")
def t_a1_26(ctx):
    with m(ctx) as c:
        for i in range(40):
            c.s.sendto(build(CMD["GET_STATS"], b"", 1000 + i), (ctx.radio_ip, 7251))
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
    with m(ctx) as c, Host(ctx.radio_ip, ctx.host_ip) as h:
        h.start()
        h.drain(0.1)
        t = tlv(c.req("IDENTIFY")["payload"])
        ctx.eq("active level 1", t.get(13), b"\x01")
        owner = t.get(14, b"")
        ctx.eq("owner port", struct.unpack(">H", owner[4:6])[0] if len(owner) == 6 else None, h.port)
        rec = {a: v for a, v in (h.cap_request() or [])[1:]}
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
