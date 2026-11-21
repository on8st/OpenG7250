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

from .core import Blocked, Ctx, PartSkip, Skip, test
from .mgmt import Mgmt
from .p2 import (PORT_REG, STREAM_STATUS, TX_MODE_CLASS, Host, parse_regs, value, wait_reachable, write_status)
from .update import HW_ID, SECTOR, Updater, header, image, layout, version_of

SMALL = 2048 + 100  # five blocks, the last one short
# Keep it a multiple of neither 512 (a short last block) nor 32 (a partial last flash word): the
# partial word found a firmware bug on fw-6ca08d1 (the last 4 bytes never programmed, VERIFY 55).
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
        u = Updater(c, s)
        try:
            yield u
        finally:
            if restart_at_end:
                restart(ctx, u)
            else:
                u.req("END_WRITE_SESSION")  # leave the write session free for the next test


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


def trial_package(ctx, slot: int, ver, seed: int):
    """(image, header, version) for an update whose trial must run. A real package of this
    firmware for that slot where the runner passes one (params.package_slot_a, package_slot_b:
    paths to Table M-4 packages built on the firmware side, read as opaque bytes); else the
    synthetic SMALL image on the reference model, whose trial is a model; else Skip, since a
    synthetic image cannot run on an emulator or a radio."""
    from pathlib import Path
    from .update import parse_package
    path = ctx.params.get(f"package_slot_{'ab'[slot - 1]}")
    if path:
        h, img, f = parse_package(Path(path).read_bytes())
        if f["slot"] not in (0, slot):
            raise PartSkip(f"params.package_slot_{'ab'[slot - 1]} is linked for slot {f['slot']}")
        return img, h, f["version"]
    if "model" in ctx.endpoint:
        img = image(SMALL, seed=seed)
        return img, pkg(img, newer(ver), slot), newer(ver)
    raise PartSkip(f"needs a real package (params.package_slot_{'ab'[slot - 1]}) for a running trial")


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


@test("T-A1-80")
def t_a1_80(ctx: Ctx):
    """A1.8 "No transfer in progress" (0063): after a restart, after UPDATE_ABORT and after an
    activation."""
    s = secret(ctx)
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
        no_transfer_checks(ctx, u, "after an activation")
        deadline = time.monotonic() + 20  # let the trial confirm itself, not left half-way
        while time.monotonic() < deadline and u.identify().get(10) == b"\x01":
            time.sleep(0.5)


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
    """time.advance (the reference model's clock; the emulator's later): s seconds pass."""
    # the bench sends nothing meanwhile; the answer comes when the advance is done (the emulator
    # runs about 2.7 x real time, so allow the radio time itself plus a margin on the wall clock)
    ctx.fixture("time.advance", _timeout=s + 60, us=int(s * 1e6))
    time.sleep(0.05)  # a tick of the model at the new time


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
    with Host(ctx.net) as h:
        h.start()
        got = [p for p, _, _ in h.recv(0.3) if p == 1025]
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


@test("T-A1-34")
def t_a1_34(ctx: Ctx):
    """A1.8: after UPDATE_BEGIN, 10 min without an update command: aborted, logged, restart into
    the running image; RX works afterwards."""
    need_clock(ctx)
    s = secret(ctx)
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        n0 = last_log_seq(u)
        up0 = uptime_s(u) or 0
        r = u.begin(pkg(image(SMALL, seed=34), newer(ver), other))
        if not ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        expect_restart_at(ctx, u, "after UPDATE_BEGIN only", 600, 0)
        ctx.eq("after the restart: the running image unchanged", (u.identify().get(23, b"")[:1]), bytes([run]))
        texts = logs_since(u, n0)
        ctx.check("the abandonment is logged (a new log entry; the text is not specified)", bool(texts), f"{texts[-3:]}")
    ctx.check("RX works afterwards (a session gets status packets)", rx_works(ctx), "no status packet")


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
                ok = u.full(img, pkg(img, newer(ver), other)) == (0, "verified")
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
    version, new version) or None."""
    ctx.__dict__["_upd_fail"] = ""
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        if net_down:
            # the trial keeps its network down: trial.sim on the reference model; on an emulator a
            # CH394 that stays silent for the next boot only (UP-05: "no CH394 answer")
            if "trial.sim" in ctx.endpoint.get("fixtures", []):
                ctx.fixture("trial.sim", network_down=True)
            else:
                ctx.need("ch394.silent")
                ctx.fixture("ch394.silent", boots=1)
        img, hdr, newver = trial_package(ctx, other, ver, seed)
        st, step = u.full(img, hdr)
        if (st, step) != (0, "verified"):
            ctx.__dict__["_upd_fail"] = f"{step} answered {st}"
            return None
        r = u.req("UPDATE_ACTIVATE", b"\x00")
        if not r or r["status"] != 0:
            ctx.__dict__["_upd_fail"] = f"UPDATE_ACTIVATE answered {r['status'] if r else None}"
            return None
    time.sleep(0.3)
    wait_reachable(ctx, "UPDATE_ACTIVATE")
    return run, other, ver, newver


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
            advance(ctx, 8)
            ctx.eq("8 s after start: still a trial (tag 10 = 1)", u.identify().get(10), b"\x01")
            advance(ctx, 3)
            t = u.identify()
            ctx.eq("after 10 s: confirmed (tag 10 = 0), the new slot runs", (t.get(10), t.get(23, b"")[:1]), (b"\x00", bytes([a[1]])))
    # keeps its network down: rolls back after 60 s
    a = activate_new(ctx, s, 682, net_down=True)
    if ctx.check("update activated (network-down case)", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        with updating(ctx, s, restart_at_end=False) as u:
            n0 = last_log_seq(u)
            advance(ctx, 55)
            ctx.eq("55 s after start: still the trial in the new slot", (u.identify().get(10), u.identify().get(23, b"")[:1]),
                   (b"\x01", bytes([a[1]])))
            advance(ctx, 10)
            wait_reachable(ctx, "trial rollback")
            t = u.identify()
            ctx.eq("after 60 s: rolled back (tag 10 = 2), the previous slot and version run",
                   (t.get(10), t.get(23, b"")[:1], version_of(t)), (b"\x02", bytes([a[0]]), a[2]))
            ctx.check("the rollback is logged", bool(logs_since(u, n0)), "no new log entry")
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
            ctx.check("the rollback is logged", bool(logs_since(u, n0)), "no new log entry")



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
        ctx.eq("the failed image's slot checks bad now (tag 9 = 2)", t.get(9, b"..")[1:2], b"\x02")
        r = u.req("ROLLBACK")
        ctx.eq("with the other slot bad: 55 and nothing changes", (r["status"] if r else None, u.identify().get(23, b"")[:1]),
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
