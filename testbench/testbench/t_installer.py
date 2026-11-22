"""The streaming installer (A1.8a item 3; 0028, 0074, 0077): T-A1-60, T-A1-62. Only where the F1 gate
(P-SYS-5) is open: the reference model started with --f1 open. The image is synthetic and opaque.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import os
import time

from .core import Ctx, RadioClock, Skip, test
from .mgmt import Mgmt, build, CMD, req_retransmit, send_retransmit, signed, tlv
from .p2 import Host, wait_reachable
from .t_details import NO_EVIDENCE_60
from .t_store import get_config, has
from .t_update import advance, last_log_seq, logs_since, secret, updating
from .update import FLAG_FOREIGN, SECTOR, Updater, begin_payload, block_payload, header, image

BLOCKS = SECTOR // 512  # of one sector
IMG = image(SECTOR + 1500, seed=600)


def gate_open(ctx):
    if (ctx.endpoint.get("model") or {}).get("f1_gate") != "open":
        raise Skip("the F1 gate is closed (P-SYS-5): only the reference model started with --f1 open")
    for fx in ("installer.state", "vendor.install", "time.advance"):
        ctx.need(fx)


def package() -> bytes:
    return header(len(IMG), hashlib.sha256(IMG).digest(), version=(0, 0, 0), slot=0)


def begin(ctx, u: Updater, label: str) -> bool:
    r = u.begin(package(), FLAG_FOREIGN)
    return ctx.eq(f"{label}: foreign UPDATE_BEGIN: 0, 512-byte blocks, their count, one-way", (r["status"], r["payload"]) if r else None,
                  (0, (512).to_bytes(2, "big") + ((len(IMG) + 511) // 512).to_bytes(4, "big") + b"\x01"))


def block(u: Updater, n: int):
    r = u.req("UPDATE_BLOCK", block_payload(n, IMG[512 * n:512 * n + 512]))
    return r["status"] if r else None


def blocks(u: Updater, a: int, z: int):
    """Blocks a to z - 1: the first status that is not 0, else 0."""
    return next((st for st in (block(u, n) for n in range(a, z)) if st != 0), 0)


def second(ctx, s: bytes) -> Updater:
    """Host B: another source port."""
    c = Mgmt(ctx.net)
    c.clock = RadioClock(ctx)
    return Updater(c, s)


def st_of(r):
    return r["status"] if r else None


def fresh(ctx) -> bytes:
    """The vendor path: this firmware again, with a secret set by the bench."""
    ctx.fixture("vendor.install")
    ctx.endpoint.get("_learned", {}).pop("secret", None)
    return secret(ctx)


def installer_answers(ctx, u: Updater, erased: bool):
    """What the installer says and refuses (item 3)."""
    when = "after the first erase" if erased else "before the first erase"
    t = u.identify()
    ctx.eq(f"{when}: IDENTIFY tag 10 = 3 (installer), tag 22 the sectors written", (t.get(10), t.get(22)), (b"\x03", bytes([int(erased)])))
    ctx.eq(f"{when}: UPDATE_STATUS state and sectors", [(u.status() or {}).get(k) for k in ("state", "sectors")], [4, 1] if erased else [3, 0])
    ctx.eq(f"{when}: IDENTIFY tag 11 (a secret set)", t.get(11), b"\x00" if erased else b"\x01")
    ctx.eq(f"{when}: tag 12: no window", t.get(12, b"\xff")[:1], b"\x00")
    ctx.eq(f"{when}: SET_FIRST_SECRET 36", st_of(req_retransmit(u.c, "SET_FIRST_SECRET", os.urandom(32))), 36)
    for cmd, p in (("GET_CONFIG", b""), ("GET_STATS", b""), ("READ_LOG", b"\0\0\0\0"), ("GET_CALIBRATION", b"")):
        ctx.eq(f"{when}: {cmd}: 58", st_of(req_retransmit(u.c, cmd, p)), 58)
    for cmd, p in (("RESTART", b""), ("CLEAR_LOG", b""), ("SET_CONFIG", b""), ("ROLLBACK", b"")):
        ctx.eq(f"{when}: {cmd}: 58", st_of(u.req(cmd, p)), 58)
    with Host(ctx.net) as h:
        ctx.check(f"{when}: discovery gets no answer", not h.discover(0.5), "a discovery reply")
        h.start()
        ctx.eq(f"{when}: P2 gets no answer", len(h.recv(0.5)), 0)
        h.running = False
    if has(ctx, "hw.input") and has(ctx, "hw.observe"):
        from .t_tx_locked import safe
        for pin in ("PH6", "PH3", "PH4"):
            for level in (0, 1):
                ctx.fixture("hw.input", pin=pin, level=level)
        safe(ctx, f"{when}, the local PTT and KEY inputs toggled")


def has_state(ctx, label: str, state: str):
    ctx.eq(f"{label}: the application area (installer.state)", (ctx.fixture("installer.state") or {}).get("state"), state)


def back_in_this_firmware(ctx, s: bytes, label: str, conf0: dict, n0: int):
    wait_reachable(ctx, label)
    has_state(ctx, label, "own")
    with updating(ctx, s, restart_at_end=False) as u:
        t = u.identify()
        ctx.eq(f"{label}: this firmware runs, a secret set (tags 10, 11)", (t.get(10), t.get(11)), (b"\x00", b"\x01"))
        ctx.eq(f"{label}: the secret is unchanged (an authenticated request: 0)", st_of(u.req("END_WRITE_SESSION")), 0)
        ctx.eq(f"{label}: the configuration is unchanged", get_config(u.c), conf0)
        ctx.check(f"{label}: logged", "restart: installer-abandoned" in logs_since(u, n0), f"{logs_since(u, n0)[-4:]}")


@test("T-A1-60", own_boot=True)
def t_a1_60(ctx: Ctx):
    """A1.8a item 3, A1.10 (0028, 0077)."""
    gate_open(ctx)
    s = secret(ctx)
    try:
        # before the first erase: UPDATE_ABORT restarts into this firmware
        with updating(ctx, s, restart_at_end=False) as u:
            conf0, n0 = get_config(u.c), last_log_seq(u)
            if not begin(ctx, u, "first run"):
                return
            has_state(ctx, "first run", "before-erase")
            installer_answers(ctx, u, False)
            ctx.eq("a block before the first erase: 0", block(u, 0), 0)
            r = u.req("UPDATE_ABORT")
            ctx.eq("UPDATE_ABORT before the first erase: 0, restart needed 0", (r["status"], r["payload"]) if r else None, (0, b"\x00"))
            back_in_this_firmware(ctx, s, "UPDATE_ABORT before the first erase", conf0, n0)
        # the transfer, host B, and the pause of the idle time
        with updating(ctx, s, restart_at_end=False) as u:
            b = second(ctx, s)
            try:
                if not begin(ctx, u, "second run"):
                    return
                ctx.eq("blocks 0 to 254", blocks(u, 0, BLOCKS - 1), 0)
                has_state(ctx, "255 blocks of the first sector", "before-erase")  # no sector before all its blocks
                r = u.req("UPDATE_BLOCK", block_payload(BLOCKS - 1, IMG[512 * (BLOCKS - 1):SECTOR], crc=1))
                ctx.eq("its last block with a bad CRC: 53", st_of(r), 53)
                has_state(ctx, "after the bad CRC", "before-erase")
                ctx.eq("host B's UPDATE_BLOCK: 35", block(b, BLOCKS - 1), 35)
                # the sector's block 50 s after the previous reply, its flash work 8 s, the next block 50 s after that reply
                ctx.eq("host A's block again: 0", block(u, 0), 0)
                advance(ctx, 50)
                ctx.fixture("flash.delay", us=8_000_000, count=1)
                ctx.eq("the block that completes the sector, 50 s later, with 8 s of flash work: 0", block(u, BLOCKS - 1), 0)
                ctx.eq("host B straight after its reply: 35", block(b, BLOCKS), 35)
                has_state(ctx, "the first sector complete", "writing")
                advance(ctx, 49)
                ctx.eq("host B 49 s after that reply: 35", block(b, BLOCKS), 35)
                ctx.eq("the next block, 50 s after that reply, with the old secret: 0", block(u, BLOCKS), 0)
                installer_answers(ctx, u, True)
                ctx.eq("UPDATE_ABORT after the first erase: 60", st_of(u.req("UPDATE_ABORT")), 60)
                ctx.eq("host B's END_WRITE_SESSION: 35", st_of(b.req("END_WRITE_SESSION")), 35)
                ctx.eq("host B's UPDATE_BLOCK: 35", block(b, BLOCKS + 1), 35)
                ctx.eq("host A's END_WRITE_SESSION: 0", st_of(u.req("END_WRITE_SESSION")), 0)
                ctx.eq("then host B's block: 0", block(b, BLOCKS + 1), 0)
                advance(ctx, 700)
                ctx.eq("11 min without a block: the installer waits (UPDATE_STATUS 4)", (u.status() or {}).get("state"), 4)
                ctx.eq("and host B completes the transfer", blocks(b, BLOCKS + 2, (len(IMG) + 511) // 512), 0)
                r = b.req("UPDATE_VERIFY")
                ctx.eq("UPDATE_VERIFY: 0 and the hash of the image", (r["status"], r["payload"]) if r else None, (0, hashlib.sha256(IMG).digest()))
            finally:
                b.c.close()
        # a second run: A falls silent, B goes on after 60 s
        s = fresh(ctx)
        a, b = second(ctx, s), second(ctx, s)
        try:
            if not begin(ctx, a, "third run"):
                return
            ctx.eq("blocks of the first sector", blocks(a, 0, BLOCKS), 0)
            ctx.eq("host B: 35", block(b, BLOCKS), 35)
            advance(ctx, 60.5)
            ctx.eq("host B after 60 s with no request from A: 0, and it completes the transfer", blocks(b, BLOCKS, (len(IMG) + 511) // 512), 0)
            ctx.eq("UPDATE_VERIFY: 0", st_of(b.req("UPDATE_VERIFY")), 0)
        finally:
            a.c.close()
            b.c.close()
        ctx.notes.append(NO_EVIDENCE_60)
        ctx.notes.append("not judged: 'sector 0 and the option bytes are never written' (the model has neither), TCI; the stretched erase "
                         "is flash.delay of the model")
    finally:
        ctx.fixture("vendor.install")
        ctx.endpoint.get("_learned", {}).pop("secret", None)


@test("T-A1-62", own_boot=True)
def t_a1_62(ctx: Ctx):
    """A1.8a item 3 before the first erase (0028, 0074): 10 min, a repeated UPDATE_BEGIN, UPDATE_ABORT."""
    gate_open(ctx)
    s = secret(ctx)
    for case in ("10 min", "UPDATE_ABORT"):
        with updating(ctx, s, restart_at_end=False) as u:
            conf0, n0 = get_config(u.c), last_log_seq(u)
            data = signed(u.c, "UPDATE_BEGIN", s, begin_payload(FLAG_FOREIGN, package()))
            r = send_retransmit(u.c, data, 16.0, "UPDATE_BEGIN")
            if not ctx.eq(f"{case}: foreign UPDATE_BEGIN: 0", st_of(r), 0):
                return
            again = u.c.raw(data, 1.0)
            ctx.eq(f"{case}: a repeat of it is answered from the stored reply", again[0]["raw"] if again else None, r["raw"])
            t = u.identify()
            ctx.eq(f"{case}: IDENTIFY reports the installer, no sector written (tags 10, 22)", (t.get(10), t.get(22)), (b"\x03", b"\x00"))
            ctx.eq(f"{case}: block 0: 0", block(u, 0), 0)
            if case == "10 min":
                for _ in range(2):
                    advance(ctx, 297)
                    ctx.eq("UPDATE_STATUS meanwhile: 3, before the first erase", (u.status() or {}).get("state"), 3)
                advance(ctx, 11)
            else:
                r = u.req("UPDATE_ABORT")
                ctx.eq("UPDATE_ABORT: 0, restart needed 0", (r["status"], r["payload"]) if r else None, (0, b"\x00"))
            back_in_this_firmware(ctx, s, f"{case}, no RESTART sent", conf0, n0)
