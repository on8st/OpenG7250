"""The factory reset (A1.6a, A1.10 "The old secret in flash", B2.3 "Flash work"; 0069): T-A1-39, and
the host rule of 0069 on the reference model. T-A1-92 is in t_stale.py.

What is in flash is judged on a dump of the store sectors (flash.dump {address, len} -> {hex},
proposed): the bench only looks for the 32 bytes of a secret it knows. Where the radio does not
keep the secret as those 32 bytes in a row the search proves nothing, so the bench first asks that
it finds the secret in force, and says so when it does not.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import hmac
import os
import struct
import time

from .core import Ctx, RadioClock, test
from .mgmt import CMD, FLASH_WINDOW_S, Mgmt, build, config_tlv, read_log, req_retransmit, send_retransmit, tlv
from .p2 import Host, value, wait_reachable
from .t_flash import SECTOR, flog, flog_start, in_range, is_erase, store_range, stretch
from .t_update import secret, updating
from .update import Updater

TRIP_COUNT = 0x0214


def keep_secret(ctx, s: bytes):
    """The secret in force from now on, for the tests that follow."""
    ctx.endpoint.setdefault("_learned", {})["secret"] = s.hex()
    if ctx.params.get("secret"):
        ctx.params["secret"] = s.hex()


def change_secret(ctx, u: Updater, new: bytes, window: float = FLASH_WINDOW_S):
    """CHANGE_SECRET, masked (A1.10), retransmitted for 16 s (0069); the reply or None."""
    c = u.c
    r = req_retransmit(c, "GET_CHALLENGE")
    if not r or r["status"] != 0:
        return None
    mask = hmac.new(u.s, b"G7NS" + r["payload"], hashlib.sha256).digest()
    msg = build(CMD["CHANGE_SECRET"], bytes(a ^ b for a, b in zip(new, mask)), c.next_id(), flags=1)
    t0 = c.clock.now()
    got = send_retransmit(c, msg + hmac.new(u.s, r["payload"] + msg, hashlib.sha256).digest(), window, "CHANGE_SECRET")
    c.reply_s = c.clock.now() - t0
    if got and got["status"] == 0:
        u.s = new
        keep_secret(ctx, new)
    return got


def first_secret(ctx, new: bytes):
    """SET_FIRST_SECRET in the window a factory reset opened."""
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        r = req_retransmit(c, "SET_FIRST_SECRET", new)
    if r and r["status"] == 0:
        keep_secret(ctx, new)
    return r["status"] if r else None


def uptime(c: Mgmt) -> float | None:
    r = req_retransmit(c, "IDENTIFY")
    up = tlv(r["payload"]).get(16, b"") if r and r["status"] == 0 else b""
    return struct.unpack(">I", up)[0] / 1000 if len(up) == 4 else None


def state_before(c: Mgmt):
    """What reset_done() compares with, read before the request: (clock, whole log, uptime)."""
    return c.clock, read_log(c, complete=True), uptime(c)


def new_start(log0: list, up0: float | None, log: list, up: float | None) -> str:
    """Why this answer comes from an image that started after `log0` and `up0` were read, or "". No
    sum of uptime and elapsed time: whether uptime counts flash work is open, so it may run late."""
    old = {e[0]: e for e in log0}
    for e in log:  # a start entry that was not there (also where the numbers began again: log lost)
        if e[2].startswith("start after ") and old.get(e[0]) != e:
            return f"a new start entry in READ_LOG ({e[0]} '{e[2]}')"
    if up is not None and up0 is not None and up < up0:
        return f"uptime dropped from {up0} to {up} s"
    return ""


def reset_done(ctx, what: str, before) -> str:
    """Wait for the reset after a FACTORY_RESET reply: it comes up to 17 s after the reply (0069),
    and until then the radio may still answer, so an answer counts only with evidence of a start
    since `before` (state_before() before the request). The evidence, or "" (the test fails)."""
    clock, log0, up0 = before
    t0 = clock.now()
    while True:
        wait_reachable(ctx, what, limit_s=20 + 17)  # a host allows 17 s more than after a RESTART
        with Mgmt(ctx.net) as c:
            c.clock, c.pace = clock, 18  # read over and over: under the cap of A1.10 S4
            why = new_start(log0, up0, read_log(c), uptime(c))
        if why:
            return why
        if clock.now() - t0 > 20 + 17:
            ctx.check(f"{what}: the radio restarted within 17 s of the reply and 20 s to be reachable", False,
                      "it answers, with no new start entry in READ_LOG and no drop of uptime")
            return ""
        time.sleep(0.2)


def factory_reset(ctx, u: Updater):
    """FACTORY_RESET; its status, the radio seconds from the reply to the reset or to the start of
    the image after it (None where not known), and what that time rests on."""
    from .t_a3 import boot_t_us
    before = state_before(u.c)
    clock = u.c.clock
    clock.now()
    us0 = clock.last if clock.emu else None
    r = u.req("FACTORY_RESET")
    st = r["status"] if r else None
    t0 = clock.now()
    reply_us = clock.last if clock.emu else None
    if not reset_done(ctx, "FACTORY_RESET", before):
        return st, None, "no restart seen"
    boot = boot_t_us(ctx)
    if us0 is not None and reply_us is not None and boot > us0:
        return st, max(0.0, (boot - reply_us) / 1e6), "the boot event of hw.trace"
    with Mgmt(ctx.net) as c:
        c.clock = clock
        up = uptime(c)
    return st, None if up is None else clock.now() - t0 - up, UPTIME_BASIS


UPTIME_BASIS = "the uptime of IDENTIFY, which may run late by flash work (open with the spec session); start-up included"


def sectors(u: Updater) -> list[int]:
    lo, hi = store_range(u)
    return list(range(lo, hi, SECTOR))


def holds(ctx, sector: int, secret_: bytes) -> bool:
    out = b""
    while len(out) < SECTOR:
        out += bytes.fromhex(ctx.fixture("flash.dump", address=sector + len(out), len=min(32768, SECTOR - len(out)))["hex"])
    return secret_ in out


def where(ctx, secs: list[int], s: bytes) -> list[int]:
    return [x for x in secs if holds(ctx, x, s)]


def has(ctx, fx: str) -> bool:
    return fx in ctx.endpoint.get("fixtures", [])


def address(e: dict) -> int:
    a = e.get("address")
    return int(a, 0) if isinstance(a, str) else a


# ---------------- T-A1-39 ----------------
@test("T-A1-39")
def t_a1_39(ctx: Ctx):
    """A1.6a (Table A1-4): what FACTORY_RESET restores and keeps; and 0069 on the emulator."""
    s = secret(ctx)
    ip = bytes(int(x) for x in ctx.endpoint["radio"]["ip"].split("."))
    mask = bytes(int(x) for x in ctx.endpoint["radio"].get("netmask", "255.255.0.0").split("."))
    with updating(ctx, s, restart_at_end=False) as u:
        conf0 = tlv((req_retransmit(u.c, "GET_CONFIG") or {"payload": b""})["payload"])
        t0 = u.identify()
        old_ms = struct.unpack(">I", conf0[7])[0]
        # C-1 static with the address in use (so the radio stays reachable), C-3 tightened, C-7 the
        # radio's own subnet; C-2 and C-6 take only their factory values in this version
        items = [(1, b"\x01"), (2, ip), (3, mask), (4, ip), (7, struct.pack(">I", old_ms - 1000)), (11, ip[:2] + b"\0\0" + b"\x10")]
        r = u.req("SET_CONFIG", config_tlv(*items))
        if not ctx.eq("SET_CONFIG of C-1, C-3 and C-7", r["status"] if r else None, 0):
            return
        conf1 = tlv((req_retransmit(u.c, "GET_CONFIG") or {"payload": b""})["payload"])
        ctx.check("changed beforehand: C-1, C-3, C-7", conf1.get(1) == b"\x01" and conf1.get(2) == ip and conf1.get(11) not in (None, b"")
                  and conf1.get(7) == struct.pack(">I", old_ms - 1000), f"{conf1}")
        with Host(ctx.net) as h:
            trips = value(h.write([(TRIP_COUNT, 0)], timeout=0.3), TRIP_COUNT)
        n0 = max([e[0] for e in read_log(u.c)], default=-1)
        secs, flash = [], has(ctx, "flash.log")
        if flash:
            secs = sectors(u)
            flog_start(ctx)
        st, _, _ = factory_reset(ctx, u)
        ctx.eq("FACTORY_RESET: 0", st, 0)
    new = os.urandom(32)
    with updating(ctx, b"", restart_at_end=False) as u:
        u.s = new  # END_WRITE_SESSION at the end is signed with the secret set below
        t = u.identify()
        ctx.eq("the secret is cleared (tag 11) and the first-secret window open (tag 12)", (t.get(11), t.get(12, b"")[:1]), (b"\x00", b"\x01"))
        conf = tlv((req_retransmit(u.c, "GET_CONFIG") or {"payload": b""})["payload"])
        ctx.eq("C-1 address mode: DHCP", conf.get(1), b"\x00")
        ctx.eq("C-2 default level 1, C-6 region 0, C-7 allow-list empty", (conf.get(5), conf.get(6), conf.get(11)), (b"\x01", b"\x00", b""))
        ctx.eq("the C-3 limits are kept", conf.get(7), struct.pack(">I", old_ms - 1000))
        ctx.eq("the firmware images are unchanged (tags 4, 9, 23)", [t.get(k) for k in (4, 9, 23)], [t0.get(k) for k in (4, 9, 23)])
        with Host(ctx.net) as h:
            ctx.eq("TRIP_COUNT is kept", value(h.write([(TRIP_COUNT, 0)], timeout=0.3), TRIP_COUNT), trips)
        from .t_log import judge_run
        judge_run(ctx, "the log is kept", read_log(u.c), n0, "restart: factory-reset")
        if flash:
            ers = [e for e in flog(ctx) if is_erase(e) and any(in_range(e, x, x + SECTOR) for x in secs)]
            ctx.check("flash work after the reply: at most two store erases (0069)", len(ers) <= 3, f"{len(ers)} store erases, one of "
                      "them possibly at the next start")
        st = first_secret(ctx, new)
        ctx.eq("SET_FIRST_SECRET in the new window: 0", st, 0)
        r = u.req("SET_CONFIG", config_tlv((7, struct.pack(">I", old_ms))))
        ctx.eq("the C-3 limit put back toward its built-in value", r["status"] if r else None, 0)
    ctx.notes.append("not judged here: the TCI port (no management item reads it), the fault and reset counters (T-A3-60); "
                     "C-2 and C-6 have only their factory value in this version")
    if not flash:
        ctx.notes.append("0069 part (flash) not run: no flash.log")
        return
    erased_reset(ctx, new)


def stretched(ctx):
    """Every erase at t_erase_max where the emulator offers it; how to put it back."""
    t_max = (ctx.endpoint.get("flash_timing") or {}).get("t_erase_max_us")
    if has(ctx, "flash.timing") and t_max:
        return stretch(ctx, t_max)
    ctx.notes.append("erases not stretched: " + " and ".join(
        ([] if has(ctx, "flash.timing") else ["the fixture flash.timing is not offered"]) +
        ([] if t_max else ["the endpoint has no flash_timing.t_erase_max_us" if "flash_timing" in ctx.endpoint else
                           "the endpoint has no flash_timing"])))
    return lambda: None


def erased_reset(ctx, s: bytes):
    """T-A1-39 on the emulator: no store sector holds the old secret after the reset; the reset comes
    within 17 s of the reply with every erase stretched; an erase that fails after the reply is
    logged and finished at the next start before the management socket opens."""
    back = stretched(ctx)
    try:
        with updating(ctx, s, restart_at_end=False) as u:
            secs = sectors(u)
            known = has(ctx, "flash.dump") and bool(where(ctx, secs, s))
            if not known:
                ctx.notes.append("what is in flash not judged: " + ("the secret in force is not found as 32 bytes in a row"
                                                                    if has(ctx, "flash.dump") else "no flash.dump"))
            st, took, basis = factory_reset(ctx, u)
            ctx.eq("FACTORY_RESET with stretched erases: 0", st, 0)
            # from the uptime `took` counts the boot stage too (1 s allowed); the bound is on the reset
            ctx.check("the reset within 17 s of the reply (radio time)", took is not None and took <= (18.0 if basis is UPTIME_BASIS else 17.0),
                      f"{took if took is None else round(took, 2)} s, from {basis}")
            if basis is UPTIME_BASIS:
                ctx.notes.append(f"the 17 s check rests on {basis}")
            if known:
                ctx.eq("neither store sector holds the old secret", where(ctx, secs, s), [])
        second = os.urandom(32)
        ctx.eq("SET_FIRST_SECRET", first_secret(ctx, second), 0)
    finally:
        back()
    if not has(ctx, "flash.fault"):
        ctx.notes.append("a failed erase after the reply not run: no flash.fault")
        return
    with updating(ctx, second, restart_at_end=False) as u:
        ctx.fixture("flash.fault", op="erase", after=1, flag="PGSERR")  # the second erase of the request
        st, _, _ = factory_reset(ctx, u)
        ctx.eq("FACTORY_RESET whose second erase fails: 0 (it fails after the reply)", st, 0)
        texts = [e[2] for e in read_log(u.c)]
        i = [k for k, x in enumerate(texts) if x.startswith("restart: factory-reset")]
        # the failed erase may have left the old secret's record whole or in part (0069 addendum)
        ctx.check("finished at the next start: one of the start check's lines in this run's log", bool(i) and
                  any("store: stale secret erased" in x or "store: non-live sector erased (unrecognised data)" in x
                      for x in texts[i[-1]:]), f"{texts[-5:]}")
        if known:
            ctx.eq("neither store sector holds the old secret (failed erase finished)", where(ctx, secs, second), [])
    ctx.eq("SET_FIRST_SECRET", first_secret(ctx, os.urandom(32)), 0)


# ---------------- the host rule of 0069 on the reference model (T-B5-22) ----------------
def host_rule_on_model(ctx):
    """T-B5-22 on the reference model: replies that come after 6 s of flash work, with the
    retransmissions of 0069 arriving meanwhile: each request executed once and answered."""
    from .t_update import SMALL, facts, image, newer, pkg
    s = secret(ctx)
    with updating(ctx, s) as u:
        run, other, ver, lay = facts(ctx, u)
        img = image(SMALL, seed=520)
        ctx.fixture("flash.delay", us=6_000_000, count=1)
        w0 = time.monotonic()
        r = u.begin(pkg(img, newer(ver), other))
        ctx.check("UPDATE_BEGIN with 6 s of flash work: answered 0 to a host that goes on for 16 s", r is not None and r["status"] == 0
                  and time.monotonic() - w0 >= 5.9, f"{u.said('UPDATE_BEGIN', r['status'] if r else None)} after {time.monotonic() - w0:.1f} s")
        ctx.fixture("flash.delay", us=6_000_000, count=1)
        st, n = u.blocks(img)
        ctx.eq("the blocks, the first with 6 s of flash work", (st, n), (0, 5))
        ctx.eq("each executed once (blocks received)", (u.status() or {}).get("received"), 5)
        u.req("UPDATE_ABORT")
