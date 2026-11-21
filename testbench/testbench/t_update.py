"""Firmware update of this firmware (A1.8; Annex M Tables M-1, M-2, M-4), phase 1: the own-image
path. T-A1-05, -23, -33, -59 (own image), -67 (slot-linked images and the trial boot), -78, -79,
T-B8-10. The foreign image and the streaming installer (A1.8a) come later.

Every test reads the running slot, the version and the flash layout from IDENTIFY (tags 4, 23,
25), so that it works whatever an earlier test installed, and ends with a RESTART so that no
update state is left behind.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import struct
import time
from contextlib import contextmanager

from .core import Blocked, Ctx, Skip, test
from .mgmt import Mgmt
from .p2 import (PORT_REG, STREAM_STATUS, TX_MODE_CLASS, Host, parse_regs, value, wait_reachable, write_status)
from .update import HW_ID, SECTOR, Updater, header, image, layout, version_of

SMALL = 2048 + 100  # five blocks, the last one short


def secret(ctx) -> bytes:
    from .t_mgmt import secret_of
    return secret_of(ctx)


def restart(ctx, u: Updater):
    """End the test with a commanded reset (the update state lasts until a restart, A1.8)."""
    r = u.req("RESTART")
    ctx.check("RESTART at the end of the test: 0", r is not None and r["status"] == 0, f"status {r['status'] if r else None}")
    time.sleep(0.2)
    wait_reachable(ctx, "RESTART")


@contextmanager
def updating(ctx, s: bytes, restart_at_end: bool = True):
    """A management socket holding the write session; at the end, also after an error, a
    RESTART so that no update state is left for the next test."""
    with Mgmt(ctx.net) as c:
        u = Updater(c, s)
        try:
            yield u
        finally:
            if restart_at_end:
                restart(ctx, u)


def facts(ctx, u: Updater):
    """(running slot, other slot, version, layout) from IDENTIFY, or Skip."""
    t = u.identify()
    lay = layout(t)
    run = t.get(23, b"")[:1]
    if not lay or run not in (b"\x01", b"\x02"):
        raise Skip("IDENTIFY has no flash layout (tag 25) or running slot (tag 23)")
    slot = run[0]
    return slot, 3 - slot, version_of(t) or (0, 0, 0), lay


def newer(v):
    return (v[0], v[1], (v[2] + 1) & 0xFFFF)


def slot_bytes(lay, slot: int) -> int:
    return lay[slot - 1][1] * SECTOR


def pkg(img: bytes, version, slot: int, **kw) -> bytes:
    return header(len(img), hashlib.sha256(img).digest(), version=version, slot=slot, **kw)


@test("T-A1-79")
def t_a1_79(ctx: Ctx):
    s = secret(ctx)
    with updating(ctx, s) as u:
        t = u.identify()
        v = t.get(25)
        if not ctx.check("IDENTIFY tag 25 present with length 6 (0062)", v is not None and len(v) == 6, repr(v)):
            return
        lay = layout(t)
        ranges = [set(range(a, a + n)) for a, n in lay]
        ctx.check("slots A, B and the store do not overlap", not (ranges[0] & ranges[1] or ranges[0] & ranges[2] or ranges[1] & ranges[2]),
                  str(lay))
        ctx.check("all lie within sectors 2 to 7 (never 0 or 1)", all(r and min(r) >= 2 and max(r) <= 7 for r in ranges), str(lay))
        run = t.get(23, b"")[:1]
        ctx.check("tag 23 names a slot (1 or 2) and the running image version is set",
                  run in (b"\x01", b"\x02") and (version_of(t) or (0, 0, 0)) != (0, 0, 0), f"tag 23 {run!r}, tag 4 {t.get(4)!r}")
        restart(ctx, u)
        u = Updater(u.c, s)
        t2 = u.identify()
        ctx.eq("tag 25 unchanged across a restart", t2.get(25), v)
        ctx.notes.append("tag 25 across an update: judged in T-A1-67")


@test("T-A1-78")
def t_a1_78(ctx: Ctx):
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        size = slot_bytes(lay, other)
        sha = b"\0" * 32
        r = u.begin(header(size + 1, sha, version=newer(ver), slot=other))
        ctx.eq(f"slot size + 1 ({size + 1} bytes) for slot {other}: 61 (A1.8; 0061, 0062)", r["status"] if r else None, 61)
        st = u.status()
        ctx.eq("and nothing written: UPDATE_STATUS idle", (st or {}).get("state"), 0)
        r = u.begin(header(size + 1, sha, version=newer(ver), slot=other, hw=HW_ID + 1))
        ctx.eq("wrong hardware id, oversized: 51 (51 comes before 61)", r["status"] if r else None, 51)
        r = u.begin(header(size + 1, sha, version=newer(ver), slot=run))
        ctx.eq("for the running slot, oversized: 63 (63 comes before 61)", r["status"] if r else None, 63)
        r = u.begin(header(16, sha, version=newer(ver), slot=run))
        ctx.eq("for the running slot, small: 63", r["status"] if r else None, 63)
        older = (0, 0, 1) if ver > (0, 0, 1) else None
        if older:
            r = u.begin(header(size + 1, sha, version=older, slot=other))
            ctx.eq("too large and older than the running image, no downgrade flag: 61 (61 comes before 52)",
                   r["status"] if r else None, 61)
            r = u.begin(header(16, sha, version=older, slot=other))
            ctx.eq("older, small, no downgrade flag: 52", r["status"] if r else None, 52)
        else:
            ctx.notes.append("older-image part not run: the running version is the lowest")
        r = u.begin(header(size, sha, version=newer(ver), slot=other))
        ctx.eq(f"exactly the slot size ({size} bytes): accepted", r["status"] if r else None, 0)
        if r and r["status"] == 0:
            ctx.eq("block count for the slot size", struct.unpack(">I", r["payload"][2:6])[0], (size + 511) // 512)
            u.req("UPDATE_ABORT")


@test("T-A1-59")
def t_a1_59(ctx: Ctx):
    """The own-image part: a bad CRC gets 53, a SHA-256 that differs gets 55 at VERIFY, other
    hardware gets 51 before anything is written. The foreign-image part comes with A1.8a."""
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=59)
        r = u.begin(pkg(img, newer(ver), other, hw=0x1234))
        ctx.eq("image for other hardware: 51", r["status"] if r else None, 51)
        ctx.eq("before anything is written: UPDATE_STATUS idle", (u.status() or {}).get("state"), 0)
        r = u.begin(pkg(img, newer(ver), other))
        if not ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        st, at = u.blocks(img, bad_crc=1)
        ctx.eq("a block with a wrong CRC-32 gets 53", (st, at), (53, 1))
        ctx.eq("the bad block is not written: blocks received 1", (u.status() or {}).get("received"), 1)
        u.req("UPDATE_ABORT")
    with updating(ctx, s) as u:
        bad = header(len(img), hashlib.sha256(img + b"x").digest(), version=newer(ver), slot=other)
        r = u.begin(bad)
        if ctx.eq("UPDATE_BEGIN with a wrong SHA-256 in the header: accepted", r["status"] if r else None, 0):
            st, _ = u.blocks(img)
            ctx.eq("all blocks accepted", st, 0)
            r = u.req("UPDATE_VERIFY")
            ctx.eq("UPDATE_VERIFY with a hash that differs from the header: 55", r["status"] if r else None, 55)
            u.req("UPDATE_ABORT")
    ctx.notes.append("foreign-image part not run yet (A1.8a, phase 2)")


@test("T-A1-67")
def t_a1_67(ctx: Ctx):
    s = secret(ctx)
    activated = []
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=67)
        r = u.begin(pkg(img, newer(ver), run))
        ctx.eq(f"running in slot {run}: a package for slot {run} gets 63", r["status"] if r else None, 63)
        ctx.eq("and nothing is written", (u.status() or {}).get("state"), 0)
        st, step = u.full(img, pkg(img, newer(ver), other))
        if not ctx.eq(f"a package for slot {other}: accepted, transferred and verified", (st, step), (0, "verified")):
            return
        r = u.req("UPDATE_ACTIVATE", b"\x00")
        if ctx.eq("UPDATE_ACTIVATE: 0, not one-way", (r["status"], r["payload"][:1]) if r else None, (0, b"\x00")):
            activated.append(1)
    if not activated:
        with updating(ctx, s):
            return  # clean up the unfinished update with a RESTART
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    with updating(ctx, s) as u:
        t = u.identify()
        ctx.eq("the trial boot runs the new slot (tag 23) with the new version (tag 4)", (t.get(23, b"")[:1], version_of(t)),
               (bytes([other]), newer(ver)))
        ctx.eq("IDENTIFY update state: trial boot (tag 10 = 1)", t.get(10), b"\x01")
        r = u.begin(pkg(image(SMALL, seed=68), newer(newer(ver)), run))
        ctx.eq("during the unconfirmed trial boot UPDATE_BEGIN gets 58", r["status"] if r else None, 58)
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline and u.identify().get(10) == b"\x01":
            time.sleep(0.5)
        t = u.identify()
        ctx.eq("the trial image confirms itself (tag 10 = 0) and keeps running", (t.get(10), t.get(23, b"")[:1]), (b"\x00", bytes([other])))
        ctx.eq("the other slot holds the previous version (tag 5)", version_of(t, 5), ver)
        ctx.eq("tag 25 unchanged across the update (T-A1-79)", layout(t), lay)
    ctx.notes.append("write-failure part (status 55, log, running image and boot record unchanged) not run: needs flash.fault (v1.8)")


@test("T-A1-23")
def t_a1_23(ctx: Ctx):
    s = secret(ctx)
    with updating(ctx, s) as u, Host(ctx.net) as h:
        run, other, ver, lay = facts(ctx, u)
        h.start()
        h.recv(0.3)
        img = image(SMALL, seed=23)
        r = u.begin(pkg(img, newer(ver), other))
        if not ctx.eq("UPDATE_BEGIN accepted during a session", r["status"] if r else None, 0):
            return
        h.recv(0.1)
        streams = [p for p, d, _ in h.recv(0.3) if p in (1025, 1035)]
        ctx.eq("the session ended: no status or DDC0 packet after UPDATE_BEGIN", len(streams), 0)
        reason10 = []
        end = time.monotonic() + 0.6
        while time.monotonic() < end:
            h.keepalive()
            for p, d, t in h.recv(0.02, PORT_REG):
                v = value(parse_regs(d), STREAM_STATUS)
                if v is not None and v & 0x7F == 10:
                    reason10.append(t)
        gaps = [b - a for a, b in zip(reason10, reason10[1:])]
        ctx.check("a run request is answered with STREAM_STATUS reason 10", bool(reason10), f"{len(reason10)} answers in 0.6 s")
        ctx.check("at most once per 100 ms", all(g >= 0.09 for g in gaps), f"gaps {[round(g * 1000) for g in gaps]} ms")
        ws = write_status(h.write([(TX_MODE_CLASS, 1)]))
        ctx.check("register writes get BUSY", any(r_ == "BUSY" for _, _, r_ in ws), str(ws))
        r = u.req("UPDATE_ABORT")
        ctx.eq("UPDATE_ABORT: 0, restart needed", (r["status"], r["payload"][:1]) if r else None, (0, b"\x01"))
        ws = write_status(h.write([(TX_MODE_CLASS, 1)]))
        ctx.check("after UPDATE_ABORT the state lasts: still BUSY", any(r_ == "BUSY" for _, _, r_ in ws), str(ws))
        h.running = False
    with Host(ctx.net) as h:
        h.start()
        h.recv(0.2)
        ws = write_status(h.write([(TX_MODE_CLASS, 1)]))
        ctx.check("after RESTART register writes work again (no BUSY)", not any(r_ == "BUSY" for _, _, r_ in ws), str(ws))
        h.stop()


@test("T-B8-10")
def t_b8_10(ctx: Ctx):
    s = secret(ctx)
    with updating(ctx, s) as u, Host(ctx.net) as a, Host(ctx.net) as b:
        run, other, ver, lay = facts(ctx, u)
        a.start()
        a.recv(0.2)
        r = u.begin(pkg(image(SMALL, seed=10), newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            ws = write_status(b.write([(TX_MODE_CLASS, 1)]))
            ctx.check("during the update a non-owner's write gets BUSY, not NOT_OWNER (B8.4)",
                      [x for _, _, x in ws] == ["BUSY"], str(ws))
            u.req("UPDATE_ABORT")
        a.running = False


def armed_session(ctx, h: Host):
    from .t_mgmt import arm_once, tx_bit4
    if tx_bit4(h) != 1:
        from .t_cw import pending_reason
        raise Blocked(pending_reason(ctx))
    st = arm_once(ctx, h)
    if not ctx.eq("armed", None if st is None else st & 0xF, 1):
        raise Skip("arm not accepted")


@test("T-A1-05", "T-A1-33")
def t_a1_05(ctx: Ctx):
    """While armed: UPDATE_STATUS is answered with 0 and changes nothing; every other update
    command gets 50. The keyed case needs a key-down, not reachable with this test's set-up."""
    s = secret(ctx)
    with updating(ctx, s) as u, Host(ctx.net) as h:
        run, other, ver, lay = facts(ctx, u)
        armed_session(ctx, h)
        img = image(SMALL, seed=5)
        h.keepalive()
        r = u.begin(pkg(img, newer(ver), other))
        ctx.eq("armed: UPDATE_BEGIN gets 50", r["status"] if r else None, 50)
        for cmd, p in (("UPDATE_BLOCK", struct.pack(">II", 0, 0) + img[:512]), ("UPDATE_VERIFY", b""),
                       ("UPDATE_ACTIVATE", b"\x00"), ("UPDATE_ABORT", b""), ("ROLLBACK", b"")):
            h.keepalive()
            r = u.req(cmd, p)
            ctx.eq(f"armed: {cmd} gets 50", r["status"] if r else None, 50)
        h.keepalive()
        r = u.c.req("UPDATE_STATUS")
        ctx.eq("armed: UPDATE_STATUS answered with 0", r["status"] if r else None, 0)
        from .p2 import TX_ARM_STATE
        st = value(h.write([(TX_ARM_STATE, 0)]), TX_ARM_STATE)
        ctx.eq("and the arm is unchanged", None if st is None else st & 0xF, 1)
        h.stop()
    ctx.notes.append("keyed part not run: needs a key-down (TX tests)")
