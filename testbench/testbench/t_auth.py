"""T-A1-99: status 32, authentication required (A1.4, A1.9, A1.10, Table M-6; 0080).
GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import hashlib
import os

from .core import Ctx, RadioClock, test
from .mgmt import FLASH_WINDOW_S, Mgmt, parse_stats, req_retransmit, send_retransmit, signed, tlv
from .p2 import wait_reachable
from .t_store import first_secret, get_config, has
from .t_update import secret, updating
from .update import Updater, begin_payload, block_payload, header

# Table A1-2: the commands marked "yes" in the Auth column (UPDATE_STATUS is "no"), each with a payload
# of the size and structure of Table M-1, since a malformed one gets 17 before authentication (0056)
def auth_yes() -> list[tuple[str, bytes]]:
    from .t_mgmt import simulated_set
    data = bytes(512)
    hdr = header(len(data), hashlib.sha256(data).digest())
    return [("CHANGE_SECRET", os.urandom(32)), ("END_WRITE_SESSION", b""), ("SET_CONFIG", bytes([5, 1, 1])),
            ("FACTORY_RESET", b""), ("SWITCH_LEVEL", b"\x01"), ("RESTART", b""), ("CLEAR_LOG", b""),
            ("UPDATE_BEGIN", begin_payload(0, hdr)), ("UPDATE_BLOCK", block_payload(0, data)), ("UPDATE_VERIFY", b""),
            ("UPDATE_ACTIVATE", b"\x00"), ("UPDATE_ABORT", b""), ("ROLLBACK", b""), ("RETURN_TO_STOCK", b""),
            ("ENTER_VENDOR_RECOVERY", b""), ("SET_CALIBRATION", simulated_set()), ("CLEAR_CALIBRATION", b"")]


def st_of(r):
    return r["status"] if r else None


def counted(c: Mgmt) -> int | None:
    r = req_retransmit(c, "GET_STATS")
    return parse_stats(r["payload"]).get("mg_auth_required") if r and r["status"] == 0 else None


def identify(c: Mgmt) -> dict:
    r = req_retransmit(c, "IDENTIFY")
    return tlv(r["payload"]) if r and r["status"] == 0 else {}


def no_secret(ctx, label: str):
    """With no secret set: every "yes" command gets 32, with and without an HMAC trailer, and is not
    executed; ten and more within 10 s never give 34; each 32 is counted; the window still takes a
    first secret."""
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        c.pace = 15  # requests per second, under the 20 replies per second to one source (A1.10)
        t = identify(c)
        if not ctx.eq(f"{label}: IDENTIFY tag 11, no secret set", t.get(11), b"\x00"):
            return False
        n0, up0, conf0 = counted(c), int.from_bytes(t.get(16, b"\0\0\0\0"), "big"), get_config(c)
        got = []
        for cmd, p in auth_yes():  # one datagram each, a wait of 1 s: each 32 the bench sees is one the radio sent
            got.append((cmd, "with a trailer", st_of(c.authed(cmd, os.urandom(32), p, timeout=1.0))))
            got.append((cmd, "without one", st_of(c.req(cmd, p, timeout=1.0))))
        ctx.all_of(f"{label}: every command marked yes gets 32, with and without an HMAC trailer ({len(got)} requests, never 34)",
                   got, lambda x: x[2] == 32)
        t = identify(c)
        up = int.from_bytes(t.get(16, b"\0\0\0\0"), "big")
        ctx.check(f"{label}: none executed (no reset: uptime {up0} -> {up} s; no secret; the configuration unchanged)",
                  up >= up0 and t.get(11) == b"\x00" and get_config(c) == conf0, f"tag 11 {t.get(11)!r}")
        n = counted(c)
        ctx.eq(f"{label}: mg_auth_required counts each 32", None if n is None or n0 is None else n - n0,
               sum(x[2] == 32 for x in got))
        ctx.eq(f"{label}: GET_CHALLENGE still answered (no 34)", st_of(c.req("GET_CHALLENGE")), 0)
    new = os.urandom(32)
    return ctx.eq(f"{label}: SET_FIRST_SECRET in the window after them: 0", first_secret(ctx, new), 0)


def after_factory_reset(ctx, s: bytes):
    """FACTORY_RESET answered 0, then its retransmission after the reset: 32."""
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        if get_config(c).get(1) != b"\x00":
            ctx.part("after a FACTORY_RESET: not run, static address, which it would clear")
            return False
        data = signed(c, "FACTORY_RESET", s)
        r = send_retransmit(c, data, FLASH_WINDOW_S, "FACTORY_RESET") if data else None
        if not ctx.eq("FACTORY_RESET: 0", st_of(r), 0):
            return False
    wait_reachable(ctx, "FACTORY_RESET")
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        n0 = counted(c)
        ctx.eq("the retransmission of the FACTORY_RESET after its reset: 32", st_of(c.once("FACTORY_RESET", data, 2.0)), 32)
        ctx.eq("counted in mg_auth_required", None if n0 is None else (counted(c) or 0) - n0, 1)
    return no_secret(ctx, "after a FACTORY_RESET")


def with_secret(ctx, s: bytes):
    """A write without an HMAC trailer gets 32, one with a wrong HMAC 33; a 32 leaves the challenge usable."""
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        n0 = counted(c)
        ch = c.challenge()
        ctx.eq("with a secret: a write without an HMAC trailer: 32", st_of(c.req("END_WRITE_SESSION", timeout=1.0)), 32)
        ctx.eq("then the challenge taken before it still authenticates: 0",
               st_of(c.authed("END_WRITE_SESSION", s, challenge=ch)), 0)
        ctx.eq("a write with a wrong HMAC: 33", st_of(c.authed("END_WRITE_SESSION", s, mangle=True)), 33)
        ctx.eq("mg_auth_required counts the 32 only", None if n0 is None else (counted(c) or 0) - n0, 1)


def installer(ctx, s: bytes) -> bool:
    """In the installer after the first sector erase (tag 11 = 0): a wrong HMAC 33, the secret kept in RAM 0."""
    from .t_installer import BLOCKS, begin, block, blocks
    with updating(ctx, s, restart_at_end=False) as u:
        if not begin(ctx, u, "installer"):
            return False
        if not ctx.eq("installer: the first sector's blocks", blocks(u, 0, BLOCKS), 0):
            return False
        ctx.eq("installer: after the first erase IDENTIFY tag 11 reports no secret", u.identify().get(11), b"\x00")
        ctx.eq("installer: an UPDATE_BLOCK with a wrong HMAC: 33", block(Updater(u.c, os.urandom(32)), BLOCKS), 33)
        ctx.eq("installer: the same block with the secret kept in RAM: 0", block(u, BLOCKS), 0)
    return True


@test("T-A1-99", own_boot=True)
def t_a1_99(ctx: Ctx):
    """A1.4, A1.9, A1.10, Table M-6 (0080): status 32 with no secret set, and without an HMAC trailer."""
    s = secret(ctx)
    with_secret(ctx, s)
    if not after_factory_reset(ctx, s):
        return
    s = secret(ctx)  # the one SET_FIRST_SECRET set
    with_secret(ctx, s)
    model = (ctx.endpoint.get("model") or {}).get("f1_gate")
    if model == "open" and has(ctx, "vendor.install"):
        installer(ctx, s)
    else:
        ctx.part("in the installer: not run, only on the reference model started with --f1 open")
    if has(ctx, "vendor.install"):  # the factory state, on the model: the vendor path
        ctx.fixture("vendor.install")
        ctx.endpoint.get("_learned", {}).pop("secret", None)
        no_secret(ctx, "in the factory state (vendor.install)")
    else:
        ctx.part("in the factory state: shown after the FACTORY_RESET only (no vendor.install)")
