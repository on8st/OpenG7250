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

from .core import Blocked, Ctx, PartSkip, RadioClock, Skip, test
from .mgmt import Mgmt
from .p2 import (PORT_REG, STREAM_STATUS, TX_MODE_CLASS, Host, parse_regs, value, wait_reachable, write_status)
from .update import HW_ID, SECTOR, Updater, header, image, layout, version_of

SMALL = 2048 + 100  # five blocks, the last one short
# Kept a multiple of neither 512 (a short last block) nor 32 (a partial last flash word). A
# synthetic SMALL image has no image record of this firmware: it gets 55 at UPDATE_VERIFY (0067)
# and tag 9 = 0; only good_package() verifies.
assert SMALL % 512 and SMALL % 32, "SMALL must leave a short last block and a partial last flash word"


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
        c.clock = RadioClock(ctx)
        u = Updater(c, s)
        try:
            yield u
        finally:
            if restart_at_end:
                restart(ctx, u)
            else:
                u.req("END_WRITE_SESSION")  # leave the write session free for the next test
            if getattr(c, "stale", 0):
                ctx.notes.append(f"{c.stale} late management replies (to an earlier request id) set aside")


def facts(ctx, u: Updater):
    """(running slot, other slot, version, layout) from IDENTIFY, or Skip."""
    t = u.identify()
    if not t:
        from .core import Missing
        raise Missing(f"IDENTIFY: {u.c.why or 'no reply to the request and 5 retransmissions 500 ms apart (A1.4)'}"
                      if getattr(u, "no_identify", False) else "IDENTIFY: a status other than 0")
    lay = layout(t)
    run = t.get(23, b"")[:1]
    if not lay or run not in (b"\x01", b"\x02"):
        raise Skip(f"IDENTIFY has no flash layout (tag 25: {t.get(25)!r}) or running slot (tag 23: {t.get(23)!r})")
    slot = run[0]
    return slot, 3 - slot, version_of(t) or (0, 0, 0), lay


def newer(v):
    return (v[0], v[1], (v[2] + 1) & 0xFFFF)


def slot_bytes(lay, slot: int) -> int:
    return lay[slot - 1][1] * SECTOR


def pkg(img: bytes, version, slot: int, **kw) -> bytes:
    return header(len(img), hashlib.sha256(img).digest(), version=version, slot=slot, **kw)


def real_package(ctx, slot: int, name: str = "package_slot"):
    """(header, image, fields) of the runner's package for that slot (params.package_slot_a,
    package_slot_b: package files of Annex M built on the firmware side, read as opaque bytes), or
    None."""
    from pathlib import Path
    from .update import parse_package
    path = ctx.params.get(f"{name}_{'ab'[slot - 1]}")
    return parse_package(Path(path).read_bytes()) if path else None


def good_package(ctx, slot: int, ver, seed: int):
    """(image, header, version) of a package that passes UPDATE_VERIFY and whose trial can run
    (0067): the runner's real package for that slot; on the reference model a synthetic image
    with the model's image record; else PartSkip (a synthetic image fails the image check)."""
    from .update import model_image
    real = real_package(ctx, slot)
    if real:
        h, img, f = real
        if f["slot"] not in (0, slot):
            raise PartSkip(f"params.package_slot_{'ab'[slot - 1]} is linked for slot {f['slot']}")
        return img, h, f["version"]
    if "model" in ctx.endpoint:
        img = model_image(SMALL, slot, newer(ver), seed=seed)
        return img, pkg(img, newer(ver), slot), newer(ver)
    raise PartSkip(f"needs a real package (params.package_slot_{'ab'[slot - 1]}) to pass UPDATE_VERIFY")


trial_package = good_package


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
    """A1.8, A1.8a (0025 R3): for an image of this firmware and for a foreign image alike, a bad CRC gets
    53, a SHA-256 that differs gets 55 at VERIFY, other hardware gets 51 before anything is written."""
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
    if (ctx.endpoint.get("model") or {}).get("f1_gate") == "open":
        from .t_installer import foreign_59
        foreign_59(ctx)
    else:
        ctx.part("the foreign-image half: only with the F1 gate open (the reference model started with --f1 open; P-SYS-5)")


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
        img, hdr, newver = trial_package(ctx, other, ver, 67)
        st, step = u.full(img, hdr)
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
               (bytes([other]), newver))
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
        pr = h.cap_request(retry=True)
        ctx.check("CAP_REQUEST is still answered (A1.8; 0022 section 10)", bool(pr) and any(a == 0 for a, _ in pr[1:]),
                  f"{len(pr or [])} pairs")
        from .p2 import ARM_MAGIC, TX_ARM, TX_ARM_STATE
        ans = h.write([(TX_ARM, 0x12345678 ^ ARM_MAGIC)])
        st = value(ans, TX_ARM_STATE)
        ws = write_status(ans)
        ctx.check("an arm after UPDATE_BEGIN is not accepted: BUSY as every register write (B8.4), the arm state "
                  "not armed (MAINTENANCE cannot show over P2 behind BUSY; finding SF-28)",
                  any(r_ == "BUSY" for _, _, r_ in ws) and (st is None or st & 0xF == 0), f"{ws}, TX_ARM_STATE {st}")
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


def no_transfer_checks(ctx, u: Updater, when: str):
    img = image(600, seed=80)
    from .update import block_payload
    r = u.req("UPDATE_BLOCK", block_payload(0, img[:512]))
    ctx.eq(f"{when}: UPDATE_BLOCK gets 54", r["status"] if r else None, 54)
    r = u.req("UPDATE_BLOCK", block_payload(0, img[:512], crc=0x12345678))
    ctx.eq(f"{when}: UPDATE_BLOCK with a bad CRC also 54", r["status"] if r else None, 54)
    r = u.req("UPDATE_VERIFY")
    ctx.eq(f"{when}: UPDATE_VERIFY gets 55", r["status"] if r else None, 55)
    r = u.req("UPDATE_ACTIVATE", b"\x00")
    ctx.eq(f"{when}: UPDATE_ACTIVATE gets 55", r["status"] if r else None, 55)
    r = u.req("UPDATE_BLOCK", b"\x00\x00")
    ctx.eq(f"{when}: a malformed UPDATE_BLOCK payload gets 17", r["status"] if r else None, 17)
    st = u.status()
    ctx.eq(f"{when}: nothing written, UPDATE_STATUS idle", (st or {}).get("state"), 0)


def abort_no_state(ctx, u: Updater, when: str):
    """UPDATE_ABORT with no update state (0074): 0 with "restart needed" 0, and nothing changes."""
    with Host(ctx.net) as h:
        h.start()
        h.recv(0.3)
        n0 = last_log_seq(u)
        r = u.req("UPDATE_ABORT")
        ctx.eq(f"{when}: UPDATE_ABORT with no update state: 0, restart needed 0", (r["status"], r["payload"]) if r else None, (0, b"\x00"))
        ctx.check(f"{when}: it is logged", bool(logs_since(u, n0)), "no new log entry")
        ctx.eq(f"{when}: UPDATE_STATUS idle", (u.status() or {}).get("state"), 0)
        got = []
        for _ in range(10):  # the same session: status packets go on without a new start
            h.keepalive()
            got = [p for p, _, _ in h.recv(0.3) if p == 1025]
            if got:
                break
        ctx.check(f"{when}: the P2 session goes on (status packets)", bool(got), "none in 3 s")
        ss = value(h.write([(STREAM_STATUS, 0)]), STREAM_STATUS)
        ctx.check(f"{when}: no stream stopped for an update (STREAM_STATUS DDC0 reason not 10)", ss is not None and ss & 0x7F != 10, f"{ss}")
        h.stop()


@test("T-A1-80", whole=True)
def t_a1_80(ctx: Ctx):
    """A1.8 "No transfer in progress" (0063, 0074): after a restart, after UPDATE_ABORT and after an
    activation; UPDATE_ABORT with no update state."""
    s = secret(ctx)
    with updating(ctx, s):
        pass  # a RESTART: no UPDATE_BEGIN since the start
    with updating(ctx, s, restart_at_end=False) as u:
        abort_no_state(ctx, u, "after a restart")
        if "time.advance" in ctx.endpoint.get("fixtures", []):
            up0 = uptime_s(u)
            advance(ctx, 605)
            up = uptime_s(u)
            ctx.check("no restart 10 min after that UPDATE_ABORT (uptime goes on)", None not in (up0, up) and up >= up0 + 600,
                      f"uptime {up0} -> {up} s")
        else:
            ctx.part("'no restart after 10 min' not run: no time.advance")
    with updating(ctx, s) as u:
        no_transfer_checks(ctx, u, "after a restart")
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=81)
        r = u.begin(pkg(img, newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            u.req("UPDATE_ABORT")
            no_transfer_checks(ctx, u, "after UPDATE_ABORT")
    activated = []
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = trial_package(ctx, other, ver, 82)
        st, step = u.full(img, hdr)
        if ctx.eq("a complete transfer for the activation part", (st, step), (0, "verified")):
            r = u.req("UPDATE_ACTIVATE", b"\x00")
            if r and r["status"] == 0:
                activated.append(1)
    if not activated:
        with updating(ctx, s):
            return
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    with updating(ctx, s, restart_at_end=False) as u:
        trial = u.identify().get(10) == b"\x01"
        if trial:
            abort_no_state(ctx, u, "in a trial image before confirmation")
        else:
            ctx.part("UPDATE_ABORT in a trial image: it had confirmed itself before it was reached")
        no_transfer_checks(ctx, u, "after an activation")
        # let the trial confirm itself, not left half-way
        w0, t0 = time.monotonic(), u.c.clock.now()
        while (time.monotonic() - w0 < 20 or (u.c.clock.emu and time.monotonic() - w0 < 240 and u.c.clock.now() - t0 < 20)) \
                and u.identify().get(10) == b"\x01":
            time.sleep(0.5)
        if trial:
            ctx.eq("the trial confirms as usual (tag 10 = 0)", u.identify().get(10), b"\x00")


@test("T-A1-81")
def t_a1_81(ctx: Ctx):
    """A1.8 "Unreadable or unsupported header" (0063)."""
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=83)
        sha = hashlib.sha256(img).digest()
        r = u.begin(header(len(img), sha, version=newer(ver), slot=other, magic=0x12345678))
        ctx.eq("wrong magic: 17", r["status"] if r else None, 17)
        r = u.begin(header(len(img), sha, version=newer(ver), slot=other, hver=2))
        ctx.eq("header version 2: 17", r["status"] if r else None, 17)
        ctx.eq("and nothing changes: UPDATE_STATUS idle", (u.status() or {}).get("state"), 0)
        r = u.begin(header(len(img), sha, version=newer(ver), slot=other, component=2))
        ctx.eq("readable header, component 2: 56", r["status"] if r else None, 56)
        r = u.begin(header(len(img), sha, version=newer(ver), slot=other, component=2, hw=HW_ID + 1))
        ctx.eq("component 2 and the wrong hardware id: 51 (51 comes before 56)", r["status"] if r else None, 51)
        ctx.eq("nothing changed: UPDATE_STATUS idle", (u.status() or {}).get("state"), 0)


# ---------------- phase 2: the 10-minute rule and the trial rollback ----------------
def advance(ctx, s: float):
    """time.advance: s seconds of radio time pass; the bench sends nothing meanwhile and the
    answer comes when they have passed. Wall time allowed: the radio time itself (no speed-up is
    assumed: about 1x while the image is busy) plus 60 s, at least 120 s (a restart inside a short
    step). Where hw.observe is offered, the emulated time before and after is read, and an answer
    that came before s seconds of radio time passed is noted."""
    obs = "hw.observe" in ctx.endpoint.get("fixtures", []) and "model" not in ctx.endpoint
    t0 = (ctx.fixture("hw.observe") or {}).get("t_us") if obs else None
    w0 = time.monotonic()
    ctx.fixture("time.advance", _timeout=max(s + 60, 120), us=int(s * 1e6))
    time.sleep(0.05)  # a tick of the model at the new time
    if t0 is not None:
        t1 = (ctx.fixture("hw.observe") or {}).get("t_us")
        if t1 is not None and t1 >= t0 and t1 - t0 < s * 1e6 - 1000:
            ctx.notes.append(f"time.advance {s:g} s answered after {(t1 - t0) / 1e6:.3f} s of radio time "
                             f"({time.monotonic() - w0:.1f} s wall)")


def uptime_s(u: Updater) -> float | None:
    v = u.identify().get(16, b"")
    return struct.unpack(">I", v)[0] / 1000 if len(v) == 4 else None


def logs_since(u: Updater, n0: int) -> list[str]:
    from .mgmt import read_log
    return [e[2] for e in read_log(u.c) if e[0] > n0]


def last_log_seq(u: Updater) -> int:
    from .mgmt import read_log
    return max([e[0] for e in read_log(u.c)], default=-1)


def rx_works(ctx) -> bool:
    """A session gets status packets (every 50 ms of radio time; an emulator may run slower)."""
    with Host(ctx.net) as h:
        h.start()
        got = []
        for _ in range(10):
            got = [p for p, _, _ in h.recv(0.3) if p == 1025]
            if got:
                break
            h.keepalive()
        h.stop()
        return bool(got)


def expect_restart_at(ctx, u: Updater, label: str, total_s: float, already_s: float):
    """Advance to 5 s before total_s (still in the update state), then past it (restarted)."""
    advance(ctx, total_s - 5 - already_s)
    up = uptime_s(u)
    ctx.check(f"{label}: no restart before {total_s:.0f} s - 5 s (uptime goes on)", up is not None and up >= total_s - 6,
              f"uptime {up} s")
    advance(ctx, 10)
    wait_reachable(ctx, label)
    up = uptime_s(u)
    ctx.check(f"{label}: restarted into the running image by {total_s:.0f} s + 5 s (uptime restarted)", up is not None and up < 30,
              f"uptime {up} s")


def need_clock(ctx):
    ctx.need("time.advance")


@test("T-A1-46")
def t_a1_46(ctx: Ctx):
    """A1.8: after UPDATE_ABORT, and after UPDATE_VERIFY without UPDATE_ACTIVATE, with only
    UPDATE_STATUS for 10 min: restart into the running image, logged, RX works afterwards."""
    need_clock(ctx)
    s = secret(ctx)
    for case in ("UPDATE_ABORT", "UPDATE_VERIFY without UPDATE_ACTIVATE"):
        with updating(ctx, s, restart_at_end=False) as u:
            run, other, ver, lay = facts(ctx, u)
            img = image(SMALL, seed=46)
            if case == "UPDATE_ABORT":
                r = u.begin(pkg(img, newer(ver), other))
                ok = r is not None and r["status"] == 0 and u.req("UPDATE_ABORT")["status"] == 0
            else:
                gimg, ghdr, _ = good_package(ctx, other, ver, 46)
                ok = u.full(gimg, ghdr) == (0, "verified")
            if not ctx.check(f"{case}: set up", ok, "the update could not be started"):
                continue
            n0 = last_log_seq(u)
            for _ in range(3):
                u.status()  # only UPDATE_STATUS, which does not restart the timer
            expect_restart_at(ctx, u, f"after {case}", 600, 0)
            ctx.check(f"after {case}: logged", bool(logs_since(u, n0)), "no new log entry")
        ctx.check(f"after {case}: RX works afterwards", rx_works(ctx), "no status packet")


@test("T-A1-82")
def t_a1_82(ctx: Ctx):
    """A1.8 "What restarts the timer", "After UPDATE_ABORT" (0063 addendum)."""
    need_clock(ctx)
    s = secret(ctx)
    from .update import block_payload
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=82)
        r = u.begin(pkg(img, newer(ver), other))
        if not ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        r = u.req("UPDATE_ABORT")
        ctx.eq("UPDATE_ABORT: 0", r["status"] if r else None, 0)
        n0 = last_log_seq(u)
        strays = [("UPDATE_BLOCK", block_payload(0, img[:512]), 54), ("UPDATE_VERIFY", b"", 55),
                  ("UPDATE_ACTIVATE", b"\x00", 55), ("UPDATE_BEGIN", None, 58), ("ROLLBACK", b"", 58),
                  ("UPDATE_ABORT", b"", 0)]
        got = []
        for i in range(9):  # every 60 s, up to 9 min after the reply to UPDATE_ABORT
            advance(ctx, 60)
            cmd, p, want = strays[i % len(strays)]
            r = u.begin(pkg(img, newer(ver), other)) if cmd == "UPDATE_BEGIN" else u.req(cmd, p)
            got.append((cmd, r["status"] if r else None, want))
        ctx.all_of("after UPDATE_ABORT the stray commands get 54, 55, 55, 58, 58, 0", got, lambda x: x[1] == x[2])
        ctx.eq("UPDATE_STATUS stays idle", (u.status() or {}).get("state"), 0)
        expect_restart_at(ctx, u, "strays every 60 s after UPDATE_ABORT", 600, 540)
        ctx.check("logged", bool(logs_since(u, n0)), "no new log entry")
    ctx.check("RX works afterwards", rx_works(ctx), "no status packet")
    # a RESTART after 2 min restarts at once
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        r = u.begin(pkg(img, newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted (RESTART case)", r["status"] if r else None, 0):
            u.req("UPDATE_ABORT")
            advance(ctx, 120)
            r = u.req("RESTART")
            ctx.eq("RESTART 2 min after UPDATE_ABORT: 0", r["status"] if r else None, 0)
            time.sleep(0.2)
            wait_reachable(ctx, "RESTART")
            up = uptime_s(u)
            ctx.check("restarted at once (uptime restarted)", up is not None and up < 30, f"uptime {up} s")
    # during a transfer, a block refused with 53 every 60 s does not delay the abandonment
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        r = u.begin(pkg(img, newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted (refused-block case)", r["status"] if r else None, 0):
            bad = []
            for _ in range(9):
                advance(ctx, 60)
                r = u.req("UPDATE_BLOCK", block_payload(0, img[:512], crc=0xDEADBEEF))
                bad.append(r["status"] if r else None)
            ctx.all_of("the bad blocks get 53", bad, lambda x: x == 53)
            expect_restart_at(ctx, u, "a block refused with 53 every 60 s", 600, 540)


def activate_new(ctx, s, seed: int, net_down: bool = False):
    """A complete update into the other slot and its activation; (old slot, new slot, old
    version, new version) or None. With net_down on an emulator (ch394.silent) the trial never
    answers, so this does not wait for it: the caller uses trial_rolls_back()."""
    ctx.__dict__["_upd_fail"] = ""
    silent = False
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        if net_down:
            # the trial keeps its network down: trial.sim on the reference model; on an emulator a
            # CH394 that stays silent for the next boot only (UP-05: "no CH394 answer")
            if "trial.sim" in ctx.endpoint.get("fixtures", []):
                ctx.fixture("trial.sim", network_down=True)
            else:
                ctx.need("ch394.silent")
                silent = True
        img, hdr, newver = trial_package(ctx, other, ver, seed)
        st, step = u.full(img, hdr)
        if (st, step) != (0, "verified"):
            ctx.__dict__["_upd_fail"] = u.said(step, st)
            return None
        r = None
        if silent:
            ctx.fixture("ch394.silent", boots=1)  # the next start only: the trial's
        try:
            r = u.req("UPDATE_ACTIVATE", b"\x00")
        finally:
            if silent and not (r and r["status"] == 0):  # no trial started: not left for the next reset
                ctx.fixture("ch394.silent", boots=0)
        if not r or r["status"] != 0:
            ctx.__dict__["_upd_fail"] = u.said("UPDATE_ACTIVATE", r["status"] if r else None)
            return None
        ctx.__dict__["_activated_at"] = u.c.clock
        ctx.__dict__["_activated_t"] = u.c.clock.now()
    if silent:
        return run, other, ver, newver
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    return run, other, ver, newver


def trial_rolls_back(ctx, what: str = "trial rollback"):
    """The trial cannot confirm itself: its 60 s limit passes, it resets, and the previous image
    starts and reaches the network (a boot and the power-on address exchange, so more than 60 s).
    On an emulator: the start of the previous image, in radio time since the activation reply
    (the time now less its uptime), is not before the 60 s limit."""
    advance(ctx, 65)
    wait_reachable(ctx, what, limit_s=40)
    clock = ctx.__dict__.get("_activated_at")
    if clock is not None and clock.emu:
        with Mgmt(ctx.net) as c:
            c.clock = clock
            up = uptime_s(Updater(c, b""))
            t = clock.now() - ctx.__dict__["_activated_t"]
        if up is not None:
            # uptime counts from the image's own start, about 0.2 s after the reset
            ctx.check(f"{what}: not before 60 s after the trial's start (radio time)", t - up >= 59.0,
                      f"the previous image started {t - up:.1f} s after the activation reply")


def rollback_logged(ctx, u: Updater, slot: int):
    """The boot stage's rollback line in the running run's log (0068), after its start entry."""
    from .mgmt import read_log
    texts = [e[2] for e in read_log(u.c)]
    k = [i for i, x in enumerate(texts) if x.startswith("start after ")]
    want = f"boot: rollback to slot {'AB'[slot - 1]} (trial not confirmed)"
    ctx.check(f"the rollback is logged: '{want}' after the start entry", bool(k) and want in texts[k[-1]:], f"{texts[-5:]}")


@test("T-A1-68", "T-A1-09")
def t_a1_68(ctx: Ctx):
    """0044: a trial image that meets the criterion confirms itself after 10 s without a host; one
    that keeps its network down rolls back after 60 s through a commanded reset; one that resets
    before confirming starts the previous image at the next start; each rollback is logged."""
    need_clock(ctx)
    s = secret(ctx)
    # confirms itself after 10 s, without a host
    a = activate_new(ctx, s, 681)
    if ctx.check("update activated (confirm case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        with updating(ctx, s, restart_at_end=False) as u:
            # 10 s with the network up (0044): the network came up at the latest when the radio
            # was reached, so it is confirmed 11 s from here; under 10 s of uptime it cannot be
            up = uptime_s(u)
            if up is not None and up < 7.5:
                advance(ctx, 8 - up)
                t, up = u.identify(), uptime_s(u)
                if up is not None and up < 9.5:
                    ctx.eq(f"{up:.1f} s after start: still a trial (tag 10 = 1)", t.get(10), b"\x01")
            else:
                ctx.notes.append(f"reached {up} s after the trial's start: 'still a trial before 10 s' not judged")
            advance(ctx, 11)
            t = u.identify()
            ctx.eq("after 10 s with the network up: confirmed (tag 10 = 0), the new slot runs", (t.get(10), t.get(23, b"")[:1]),
                   (b"\x00", bytes([a[1]])))
    # keeps its network down: rolls back after 60 s
    a = activate_new(ctx, s, 682, net_down=True)
    if ctx.check("update activated (network-down case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        modelled = "trial.sim" in ctx.endpoint.get("fixtures", [])  # the model's trial still answers
        n0 = None
        if modelled:
            with updating(ctx, s, restart_at_end=False) as u:
                n0 = last_log_seq(u)
                advance(ctx, 55)
                ctx.eq("55 s after start: still the trial in the new slot", (u.identify().get(10), u.identify().get(23, b"")[:1]),
                       (b"\x01", bytes([a[1]])))
                advance(ctx, 10)
                wait_reachable(ctx, "trial rollback")
        else:  # the trial has no network at all: the previous image answers after 60 s and a boot
            trial_rolls_back(ctx)
        with updating(ctx, s, restart_at_end=False) as u:
            t = u.identify()
            ctx.eq("after 60 s: rolled back (tag 10 = 2), the previous slot and version run",
                   (t.get(10), t.get(23, b"")[:1], version_of(t)), (b"\x02", bytes([a[0]]), a[2]))
            rollback_logged(ctx, u, a[0])
    # a class 2 fault (a safe-state pin forced, TX_PATH_NOT_BLOCKED): not confirmed, rolls back
    if "trial.sim" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("trial.sim", class2_fault=True)
        a = activate_new(ctx, s, 684)
        if ctx.check("update activated (class 2 case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
            with updating(ctx, s, restart_at_end=False) as u:
                advance(ctx, 30)
                ctx.eq("30 s after start with a class 2 fault: still not confirmed", u.identify().get(10), b"\x01")
                advance(ctx, 35)
                wait_reachable(ctx, "trial rollback (class 2)")
                t = u.identify()
                ctx.eq("after 60 s: rolled back to the previous slot and version", (t.get(10), t.get(23, b"")[:1], version_of(t)),
                       (b"\x02", bytes([a[0]]), a[2]))
    elif "gpio.force" in ctx.endpoint.get("fixtures", []):
        # a safe-state pin forced away from its HIS 85 level during the trial (PF4 held at 1)
        a = activate_new(ctx, s, 684)
        if ctx.check("update activated (class 2 case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
            with updating(ctx, s, restart_at_end=False) as u:
                ctx.fixture("gpio.force", pin="PF4", level=0, target="idr")  # the pin level (A3.6b item 2; 0066)
                try:
                    advance(ctx, 30)
                    ctx.eq("30 s after start with PF4 forced low: still not confirmed", u.identify().get(10), b"\x01")
                    advance(ctx, 35)
                    wait_reachable(ctx, "trial rollback (class 2)")
                    t = u.identify()
                    ctx.eq("after 60 s: rolled back to the previous slot and version", (t.get(10), t.get(23, b"")[:1], version_of(t)),
                           (b"\x02", bytes([a[0]]), a[2]))
                finally:
                    ctx.fixture("gpio.force", pin="PF4", level=None, target="idr")
        ctx.notes.append("the class 2 fault code (FAULT_CODE 16) is not judged here: A3.5 tests; what 'a safe-state pin forced' "
                         "means (pin level or output register) is asked of the spec session (SF-29)")
    else:
        ctx.notes.append("class 2 case not run: neither trial.sim nor gpio.force offered")
    # resets before confirming: the previous image at the next start
    a = activate_new(ctx, s, 683)
    if ctx.check("update activated (reset case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        ctx.need("power.cycle")
        with updating(ctx, s, restart_at_end=False) as u:
            n0 = last_log_seq(u)
            ctx.fixture("power.cycle")
            wait_reachable(ctx, "power cycle during the trial")
            t = u.identify()
            ctx.eq("a power loss before confirmation: the previous slot and version at the next start (tag 10 = 2)",
                   (t.get(10), t.get(23, b"")[:1], version_of(t)), (b"\x02", bytes([a[0]]), a[2]))
            rollback_logged(ctx, u, a[0])


@test("T-A1-84")
def t_a1_84(ctx: Ctx):
    """A1.8 "ROLLBACK" (0064)."""
    need_clock(ctx)
    s = secret(ctx)
    a = activate_new(ctx, s, 841)  # so that the other slot holds a good image
    if not ctx.check("an update installed (the other slot holds a good image)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    with updating(ctx, s, restart_at_end=False) as u:
        r = u.req("ROLLBACK")
        ctx.eq("during the trial boot: 58", r["status"] if r else None, 58)
        advance(ctx, 11)
        ctx.eq("confirmed", u.identify().get(10), b"\x00")
        ctx.eq("tag 9: the other slot checks good", u.identify().get(9, b"..")[1:2], b"\x01")
        n0 = last_log_seq(u)
        r = u.req("ROLLBACK")
        ctx.eq("ROLLBACK replies 0", r["status"] if r else None, 0)
    time.sleep(0.3)
    wait_reachable(ctx, "ROLLBACK")
    with updating(ctx, s, restart_at_end=False) as u:
        t = u.identify()
        ctx.eq("it starts the other image as a trial (tag 23, tag 10 = 1, its version)",
               (t.get(23, b"")[:1], t.get(10), version_of(t)), (bytes([a[0]]), b"\x01", a[2]))
        texts = logs_since(u, n0)
        frm, to = "AB"[a[1] - 1], "AB"[a[0] - 1]
        ctx.check(f"logged: 'rollback commanded: slot {frm} <version> to slot {to} <version>' (version format not fixed)",
                  any(f"rollback commanded: slot {frm}" in x and f"to slot {to}" in x for x in texts), f"{texts[-3:]}")
        advance(ctx, 11)
        ctx.eq("the started image confirms itself", u.identify().get(10), b"\x00")
    if "trial.sim" not in ctx.endpoint.get("fixtures", []):
        ctx.notes.append("failed-trial part not run: no trial.sim")
        return
    # the started image fails to confirm: back to the image that sent ROLLBACK
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        ctx.fixture("trial.sim", network_down=True)
        r = u.req("ROLLBACK")
        ctx.eq("a second ROLLBACK replies 0", r["status"] if r else None, 0)
    time.sleep(0.3)
    wait_reachable(ctx, "ROLLBACK")
    with updating(ctx, s, restart_at_end=False) as u:
        advance(ctx, 65)
        wait_reachable(ctx, "trial rollback after ROLLBACK")
        t = u.identify()
        ctx.eq("the started image did not confirm: back to the image that sent ROLLBACK (tag 23)", t.get(23, b"")[:1], bytes([run]))
        ctx.eq("the failed trial: tag 10 = 2 (rolled back), and its slot keeps tag 9 = 1 (integrity; 0067)",
               (t.get(10), t.get(9, b"..")[1:2]), (b"\x02", b"\x01"))
    # the other slot at 0 (an aborted staging erased it): ROLLBACK 55 and nothing changes
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        r = u.begin(pkg(image(SMALL, seed=842), newer(ver), other))
        if r and r["status"] == 0:
            u.req("UPDATE_ABORT")
    with updating(ctx, s, restart_at_end=False) as u:
        t = u.identify()
        ctx.eq("after an aborted staging the other slot reports tag 9 = 0", t.get(9, b"..")[1:2], b"\x00")
        r = u.req("ROLLBACK")
        ctx.eq("with the other slot at 0: 55 and nothing changes", (r["status"] if r else None, u.identify().get(23, b"")[:1]),
               (55, bytes([run])))
    ctx.notes.append("the other slot empty: only on a fresh radio; armed (50): T-A1-05")


@test("T-EMU-IDLE")
def t_emu_idle(ctx: Ctx):
    """Information for the emulator's idle analysis, not judged: GET_STATS before and after 60 s
    of radio time with no session and no traffic (the bench sends nothing during time.advance)."""
    need_clock(ctx)
    from .mgmt import stats

    def emu_t():
        return (ctx.fixture("hw.observe") or {}).get("t_us") if "hw.observe" in ctx.endpoint.get("fixtures", []) else None

    def dump(st):
        return ", ".join(f"{k}={st[k]}" for k in sorted(st)) if st else "no GET_STATS answer"

    with Mgmt(ctx.net) as c:
        before = stats(c)
    t0, w0 = emu_t(), time.monotonic()
    ctx.fixture("time.advance", _timeout=120, us=60_000_000)
    w1, t1 = time.monotonic(), emu_t()
    with Mgmt(ctx.net) as c:
        after = stats(c)
    ctx.check("information (not judged): GET_STATS before the 60 s", True, dump(before))
    ctx.check("information (not judged): GET_STATS after the 60 s", True, dump(after))
    ctx.check("information (not judged): emulated t_us before and after, wall time of the advance", True,
              f"t_us {t0} -> {t1} ({(t1 - t0) / 1e6:.3f} s emulated), wall {w1 - w0:.3f} s"
              if t0 is not None and t1 is not None else f"no hw.observe t_us; wall {w1 - w0:.3f} s")


# ---------------- 0067: the image check, VERIFY, tag 9 and the boot-stage refusals ----------------
def reheader(h: bytes, img: bytes, version=None, slot=None) -> bytes:
    """A real package header with its length and SHA-256 set for img, and optionally another
    version (bytes 10-13) or slot (byte 52); every other byte kept as the firmware side built it."""
    b = bytearray(h)
    b[14:18] = struct.pack(">I", len(img))
    b[18:50] = hashlib.sha256(img).digest()
    if version is not None:
        b[10:14] = bytes([version[0], version[1]]) + struct.pack(">H", version[2])
    if slot is not None:
        b[52] = slot
    return bytes(b)


def flip(img: bytes, at: int | None = None) -> bytes:
    """One byte of the image changed (the middle one by default)."""
    b = bytearray(img)
    b[len(b) // 2 if at is None else at] ^= 0x5A
    return bytes(b)


def tag9(t: dict, which: int = 1) -> bytes:
    """IDENTIFY tag 9: byte 0 the running image, byte 1 the other slot."""
    return t.get(9, b"..")[which:which + 1]


def stage(u: Updater, img: bytes, hdr: bytes):
    """BEGIN and every block; the status of the first refusal, else 0."""
    r = u.begin(hdr)
    if not r or r["status"] != 0:
        return r["status"] if r else None
    st, _ = u.blocks(img, struct.unpack(">H", r["payload"][:2])[0])
    return st


@test("T-A1-88")
def t_a1_88(ctx: Ctx):
    """A1.8 "Image check", "UPDATE_VERIFY" (0067): packages whose SHA-256 matches the header but
    whose image is not runnable get 55 at VERIFY and at ACTIVATE; nothing changes; tag 9 = 0;
    sending the right blocks again and verifying again succeeds."""
    from .update import model_image
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
    nv = newer(ver)
    # (case, image, header or None, the check words the log may name: A1.8 "Log names")
    cases = [("no image record", image(SMALL, seed=881), None, {"no-record"})]
    if "model" in ctx.endpoint:
        # the model's own record (0067 leaves the layout to the firmware)
        mi = lambda **kw: model_image(SMALL, other, nv, seed=882, **kw)
        base_run = 0x08000000 + {1: 2, 2: 4}[run] * 128 * 1024
        cases += [("a record for other hardware", mi(hw=HW_ID + 1), None, {"hardware"}),
                  ("a record of the boot stage", mi(kind=2), None, {"kind"}),
                  ("linked for the running slot", mi(rslot=run), pkg(mi(rslot=run), nv, 0), {"slot"}),
                  ("a failing integrity check", flip(mi()), None, {"integrity"}),
                  ("an entry point outside the slot", mi(entry=base_run + 0x201), None, {"vectors"}),
                  ("the record's version differs from the header", mi(), pkg(mi(), newer(nv), other), {"header"}),
                  ("the record's length differs from the header", mi(rlen=SMALL - 4), None, {"header"})]
    else:
        real, run_real = real_package(ctx, other), real_package(ctx, run)
        if real:
            h, img, f = real
            # 32 bytes appended: hash and image check pass (judged over the record's length), the
            # record against the header fails: "header" (A1.8 "Image check"; 0067 point 1)
            cases += [("a failing integrity check", flip(img), reheader(h, flip(img), slot=other), {"integrity"}),
                      ("the record's version differs from the header", img, reheader(h, img, version=newer(f["version"]), slot=other),
                       {"header"}),
                      ("the record's length differs from the header", img + b"\xff" * 32, reheader(h, img + b"\xff" * 32, slot=other),
                       {"header"})]
        if run_real:
            h, img, f = run_real
            cases.append(("linked for the running slot", img, reheader(h, img, slot=0), {"slot"}))
        # the record's layout is the firmware's, so these are built on the firmware side (opaque
        # bytes, one per slot; hash and header right, so only the image check refuses them)
        for key, name, word in (("hw", "a record for other hardware", "hardware"), ("kind", "a record of the boot stage", "kind"),
                                ("entry", "an entry point outside the slot", "vectors")):
            bad = real_package(ctx, other, f"package_bad_{key}")
            if bad:
                cases.append((name, bad[1], bad[0], {word}))
            else:
                ctx.notes.append(f"{name}: not run, needs params.package_bad_{key}_{'ab'[other - 1]}")
        if not (real and run_real):
            ctx.notes.append("the cases built from real packages need params.package_slot_a and _b")
    import re
    for name, img, hdr, words in cases:
        with updating(ctx, s) as u:
            t0 = u.identify()
            st = stage(u, img, hdr or pkg(img, nv, other))
            if not ctx.eq(f"{name}: UPDATE_BEGIN and every block accepted", st, 0):
                continue
            n0 = last_log_seq(u)
            r = u.req("UPDATE_VERIFY")
            ctx.eq(f"{name}: UPDATE_VERIFY 55", r["status"] if r else None, 55)
            texts = logs_since(u, n0)
            ctx.check(f"{name}: the failed check named in the log ({' or '.join(sorted(words))})",
                      any(re.search(rf"\b{re.escape(w)}\b", x) for x in texts for w in words), f"{texts[-3:]}")
            r = u.req("UPDATE_ACTIVATE", b"\x00")
            ctx.eq(f"{name}: UPDATE_ACTIVATE 55", r["status"] if r else None, 55)
            t = u.identify()
            ctx.eq(f"{name}: running image and boot record unchanged (tags 23, 10)", (t.get(23), t.get(10)), (t0.get(23), t0.get(10)))
            ctx.eq(f"{name}: tag 9 for the staging slot 0", tag9(t), b"\x00")
    # the transfer stays open after a failed verify: the right blocks again, then a good verify
    with updating(ctx, s) as u:
        gimg, ghdr, _ = good_package(ctx, other, ver, 889)
        st = stage(u, flip(gimg), ghdr)
        if ctx.eq("wrong blocks for a good header: accepted", st, 0):
            r = u.req("UPDATE_VERIFY")
            ctx.eq("UPDATE_VERIFY 55 (the hash)", r["status"] if r else None, 55)
            r = u.req("UPDATE_ACTIVATE", b"\x00")
            ctx.eq("UPDATE_ACTIVATE 55 until a verify succeeds", r["status"] if r else None, 55)
            bs = 512
            st, _ = u.blocks(gimg, bs)
            ctx.eq("the right blocks sent again: accepted", st, 0)
            r = u.req("UPDATE_VERIFY")
            ctx.eq("UPDATE_VERIFY again: 0", r["status"] if r else None, 0)
            ctx.eq("tag 9 for the staging slot 1 after the good verify", tag9(u.identify()), b"\x01")
            st, _ = u.blocks(gimg[:bs], bs)
            ctx.eq("block 0 sent again alone (a new request id): accepted", st, 0)
            r = u.req("UPDATE_ACTIVATE", b"\x00")
            ctx.eq("a block after the good verify cancels it: UPDATE_ACTIVATE 55", r["status"] if r else None, 55)
            stt = u.status() or {}
            rec, cnt = stt.get("received"), stt.get("count")
            ctx.check("UPDATE_STATUS: blocks received at most the count", rec is not None and cnt and rec <= cnt, f"{stt}")
            r = u.req("UPDATE_VERIFY")
            if rec is not None and cnt and rec < cnt:
                ctx.notes.append(f"the radio counts {cnt - rec} blocks as not received again after one block was sent again")
                ctx.eq("with blocks counted as not received again: UPDATE_VERIFY 55", r["status"] if r else None, 55)
                st, _ = u.blocks(gimg, bs)
                r = u.req("UPDATE_VERIFY")
            ctx.eq("the last data per block verifies: UPDATE_VERIFY 0", r["status"] if r else None, 0)
    missing_block(ctx, s, 884)
    store_full(ctx, s, 885)


def named(texts: list[str], word: str) -> bool:
    import re
    return any(re.search(rf"(?<![\w-]){re.escape(word)}(?![\w-])", x) for x in texts)


def missing_block(ctx, s: bytes, seed: int):
    """T-A1-88 (0067 addendum): the last block not sent: 55 with missing-blocks in the log, not hash;
    sending it lets the verify succeed."""
    from .update import block_payload
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = good_package(ctx, other, ver, seed)
        r = u.begin(hdr)
        if not ctx.eq("missing block: UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        n = (len(img) + 511) // 512
        st, _ = u.blocks(img[:512 * (n - 1)])
        if not ctx.eq("missing block: every block but the last accepted", st, 0):
            return
        n0 = last_log_seq(u)
        r = u.req("UPDATE_VERIFY")
        ctx.eq("missing block: UPDATE_VERIFY 55", r["status"] if r else None, 55)
        texts = logs_since(u, n0)
        ctx.check("missing block: the log names missing-blocks, not hash", named(texts, "missing-blocks") and not named(texts, "hash"),
                  f"{texts[-3:]}")
        ctx.eq("missing block: the transfer stays open (UPDATE_STATUS receiving, blocks received)",
               [(u.status() or {}).get(k) for k in ("state", "received")], [1, n - 1])
        r = u.req("UPDATE_BLOCK", block_payload(n - 1, img[512 * (n - 1):]))
        ctx.eq("missing block: the last block sent: accepted", r["status"] if r else None, 0)
        r = u.req("UPDATE_VERIFY")
        ctx.eq("missing block: UPDATE_VERIFY 0", r["status"] if r else None, 0)


def judge_64(ctx, u: Updater, k: int, payload: bytes, n0: int, before: dict | None):
    """What follows a 64 on block k: the log line, the transfer and its count unchanged, and a new
    request with the same block taken."""
    ctx.check("store full: the log shows 'store: room made (update)'", any("store: room made (update)" in x for x in logs_since(u, n0)),
              f"{logs_since(u, n0)[-3:]}")
    st = u.status() or {}
    ctx.eq("store full: the transfer still receiving, the same blocks received", (st.get("state"), st.get("received")),
           ((before or {}).get("state"), (before or {}).get("received")))
    r = u.req("UPDATE_BLOCK", payload)
    ctx.eq(f"store full: block {k} sent again with a new request id: taken", r["status"] if r else None, 0)
    ctx.eq("store full: one block more received", (u.status() or {}).get("received"), (before or {}).get("received", -9) + 1)


def store_full(ctx, s: bytes, seed: int):
    """T-A1-88 (0067 addendum, full store): an UPDATE_BLOCK gets 64; a verify with the store full
    succeeds."""
    from .update import block_payload
    if "store.full" not in ctx.endpoint.get("fixtures", []):
        ctx.notes.append("store full not run: no store.full (the next update request finds no room in the store; only the firmware "
                         "knows its store's fill level); a 64 in any other transfer is followed as the host rule says")
        return
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img, hdr, _ = good_package(ctx, other, ver, seed)
        r = u.begin(hdr)
        if not ctx.eq("store full: UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        n = (len(img) + 511) // 512
        st, _ = u.blocks(img[:1024])
        before, n0 = u.status(), last_log_seq(u)
        ctx.fixture("store.full")
        payload = block_payload(2, img[1024:1536])
        data = u.post("UPDATE_BLOCK", payload)
        r = u.collect(data, "UPDATE_BLOCK 2") if data else None
        if not ctx.eq("store full: UPDATE_BLOCK 64", r["status"] if r else None, 64):
            return
        r = u.collect(data, "UPDATE_BLOCK 2 repeated")
        ctx.eq("store full: a repeat with the same id gets 64 again", r["status"] if r else None, 64)
        judge_64(ctx, u, 2, payload, n0, before)
        st, _ = u.blocks(img, start=3)
        ctx.eq("store full: the rest of the blocks", st, 0)
        ctx.fixture("store.full")
        n0 = last_log_seq(u)
        r = u.req("UPDATE_VERIFY")
        ctx.eq("store full: UPDATE_VERIFY succeeds", r["status"] if r else None, 0)
        ctx.check("store full: the verify made room ('store: room made (update)' logged)",
                  any("store: room made (update)" in x for x in logs_since(u, n0)), f"{logs_since(u, n0)[-3:]}")


def corrupt(ctx, slot: int, release: bool = False, length: int | None = None):
    """slot.corrupt (v1.8): one bit of the byte at the middle of the image in that slot changed
    (offset from the slot start; ECC kept consistent, so it reads as changed data); release writes
    it back. The image length: given, else from the runner's package for that slot (the image the
    slot holds), else on the reference model any (it ignores the offset)."""
    if "slot.corrupt" not in ctx.endpoint.get("fixtures", []):
        raise PartSkip("needs slot.corrupt (one byte of a slot changed)")
    done = ctx.__dict__.setdefault("_corrupted", set())
    if release:
        if slot in done:
            ctx.fixture("slot.corrupt", slot=slot, restore=True)
            done.discard(slot)
        return
    if length is None:
        real = real_package(ctx, slot)
        if real:
            length = real[2]["length"]
        elif "model" in ctx.endpoint:
            length = 2
        else:
            raise PartSkip(f"the image length in slot {'AB'[slot - 1]} is unknown: needs params.package_slot_{'ab'[slot - 1]}")
    r = ctx.fixture("slot.corrupt", slot=slot, offset=length // 2) or {}
    done.add(slot)
    if "old" in r and "new" in r:
        ctx.check(f"slot.corrupt: the byte at {r.get('address')} changed", r["old"] != r["new"], f"old {r['old']}, new {r['new']}")


@test("T-A1-89", whole=("slot.corrupt", "ch394.silent", "package_slot_a", "package_slot_b"))
def t_a1_89(ctx: Ctx):
    """A1.8 "Integrity in IDENTIFY" (0067): tag 9 for the other slot."""
    need_clock(ctx)
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        r = u.begin(pkg(image(SMALL, seed=891), newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            ctx.eq("from the first erase of UPDATE_BEGIN: 0", tag9(u.identify()), b"\x00")
            u.req("UPDATE_ABORT")
            ctx.eq("after UPDATE_ABORT: 0", tag9(u.identify()), b"\x00")
    with updating(ctx, s) as u:
        ctx.eq("after a restart: still 0", tag9(u.identify()), b"\x00")
        r = u.req("ROLLBACK")
        ctx.eq("ROLLBACK with the other slot at 0: 55, nothing changes", (r["status"] if r else None, u.identify().get(23, b"")[:1]),
               (55, bytes([run])))
        img = image(SMALL, seed=892)
        if stage(u, img, pkg(img, newer(ver), other)) == 0:
            r = u.req("UPDATE_VERIFY")
            ctx.eq("a synthetic image: VERIFY 55", r["status"] if r else None, 55)
            ctx.eq("after a failed verify: 0", tag9(u.identify()), b"\x00")
    with updating(ctx, s) as u:
        ctx.eq("after a failed verify and a restart: 0", tag9(u.identify()), b"\x00")
        gimg, ghdr, _ = good_package(ctx, other, ver, 893)
        ctx.eq("a good package verifies", u.full(gimg, ghdr), (0, "verified"))
        ctx.eq("after a good verify: 1", tag9(u.identify()), b"\x01")
    # 2: a confirmed other image with one byte changed, judged at the next start
    a = activate_new(ctx, s, 894)
    if not ctx.check("an update installed", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    with updating(ctx, s, restart_at_end=False) as u:
        advance(ctx, 11)
        ctx.eq("confirmed; the other slot (the previous image) 1", (u.identify().get(10), tag9(u.identify())), (b"\x00", b"\x01"))
    corrupt(ctx, a[0])
    try:
        with updating(ctx, s) as u:
            pass
        with updating(ctx, s) as u:
            t = u.identify()
            ctx.eq("one byte of the confirmed other image changed, after a restart: 2", tag9(t), b"\x02")
            r = u.req("ROLLBACK")
            ctx.eq("ROLLBACK with the other slot at 2: 55, nothing changes", (r["status"] if r else None, u.identify().get(23, b"")[:1]),
                   (55, bytes([a[1]])))
    finally:
        corrupt(ctx, a[0], release=True)
    # a trial that failed to confirm itself: its slot keeps 1, and ROLLBACK to it starts it as a
    # trial again (0067; 0064 item 3.1)
    if not ({"trial.sim", "ch394.silent"} & set(ctx.endpoint.get("fixtures", []))):
        raise PartSkip("failed-trial part: needs trial.sim or ch394.silent (a trial that cannot confirm itself)")
    b = activate_new(ctx, s, 895, net_down=True)
    if not ctx.check("an update installed for a trial that fails", b is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    trial_rolls_back(ctx)
    with updating(ctx, s, restart_at_end=False) as u:
        t = u.identify()
        ctx.eq("the failed trial rolled back (tag 23 the previous slot, tag 10 = 2)", (t.get(23, b"")[:1], t.get(10)),
               (bytes([b[0]]), b"\x02"))
        ctx.eq("the failed trial's slot keeps tag 9 = 1", tag9(t), b"\x01")
        r = u.req("ROLLBACK")
        ctx.eq("ROLLBACK to it: accepted", r["status"] if r else None, 0)
    time.sleep(0.3)
    wait_reachable(ctx, "ROLLBACK to the failed trial's slot")
    with updating(ctx, s, restart_at_end=False) as u:
        t = u.identify()
        ctx.eq("it starts as a trial (tag 23 its slot, tag 10 = 1)", (t.get(23, b"")[:1], t.get(10)), (bytes([b[1]]), b"\x01"))
        advance(ctx, 11)
        ctx.eq("with the network up it confirms itself", u.identify().get(10), b"\x00")


@test("T-A1-90", whole=("slot.corrupt", "package_slot_a", "package_slot_b"))
def t_a1_90(ctx: Ctx):
    """A1.8 "Boot-stage refusals" (0067)."""
    need_clock(ctx)
    ctx.need("slot.corrupt")  # every case changes a slot: without it nothing of the claim is shown
    s = secret(ctx)
    # 1: the staged image changed after a good verify and UPDATE_ACTIVATE
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        gimg, ghdr, _ = good_package(ctx, other, ver, 901)
        if not ctx.eq("a good package verifies", u.full(gimg, ghdr), (0, "verified")):
            return
        n0 = last_log_seq(u)
        corrupt(ctx, other, length=len(gimg))
        r = u.req("UPDATE_ACTIVATE", b"\x00")
        ctx.eq("UPDATE_ACTIVATE 0 (it adds no check)", r["status"] if r else None, 0)
    try:
        time.sleep(0.3)
        wait_reachable(ctx, "UPDATE_ACTIVATE")
        with updating(ctx, s) as u:
            t = u.identify()
            ctx.eq("the trial slot refused: the previous image starts, tag 10 = 2", (t.get(23, b"")[:1], t.get(10)), (bytes([run]), b"\x02"))
            texts = logs_since(u, n0)
            want = f"boot: slot {'AB'[other - 1]} refused: integrity"
            ctx.check(f"logged: '{want}'", any(want in x for x in texts), f"{texts[-3:]}")
    finally:
        corrupt(ctx, other, release=True)
    # 2: the confirmed active image changed, the other slot good: the other starts without a trial
    a = activate_new(ctx, s, 902)
    if not ctx.check("an update installed", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    with updating(ctx, s, restart_at_end=False) as u:
        advance(ctx, 11)
        ctx.eq("confirmed", u.identify().get(10), b"\x00")
        n0 = last_log_seq(u)
    corrupt(ctx, a[1])
    try:
        with updating(ctx, s) as u:
            pass
        with updating(ctx, s, restart_at_end=False) as u:
            t = u.identify()
            ctx.eq("the active slot refused: the other slot starts without a trial (tag 23, tag 10 = 2)",
                   (t.get(23, b"")[:1], t.get(10)), (bytes([a[0]]), b"\x02"))
            texts = logs_since(u, n0)
            want = [f"boot: slot {'AB'[a[1] - 1]} refused: integrity",
                    f"boot: fallback to slot {'AB'[a[0] - 1]} (active slot refused, no trial)"]
            ctx.check(f"logged one after the other: {want}", any(texts[i:i + 2] == want for i in range(len(texts))), f"{texts[-5:]}")
    finally:
        corrupt(ctx, a[1], release=True)
    # 3: both slots changed: safe state, no answer (an emulator; the model does not model it)
    if "model" in ctx.endpoint:
        ctx.part("both slots changed: not run on the reference model (the safe state without network is not modelled)")
        return
    ctx.need("power.cycle")
    try:
        corrupt(ctx, 1)
        corrupt(ctx, 2)
        ctx.fixture("power.cycle")
        time.sleep(3)
        with Host(ctx.net) as h:
            ctx.check("both slots changed: the radio does not answer (discovery, 5 s)", not h.discover(5.0), "a discovery answer")
    finally:
        corrupt(ctx, 1, release=True)
        corrupt(ctx, 2, release=True)
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "after both slots were restored")
