"""The factory reset (A1.6a, A1.10 "The old secret in flash", B2.3 "Flash work"; 0069): T-A1-39, and
the host rule of 0069 on the reference model. T-A1-92 is in t_stale.py.

What is in flash is judged on a dump of the store sectors (flash.dump {address, len} -> {hex}):
the bench only looks for the 32 bytes of a secret it knows. Where the radio does not
keep the secret as those 32 bytes in a row the search proves nothing, so the bench first asks that
it finds the secret in force, and says so when it does not.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import hashlib
import hmac
import os
import re
import struct
import time

from .core import Ctx, RadioClock, Skip, test
from .mgmt import CMD, FLASH_WINDOW_S, Mgmt, build, config_tlv, read_log, req_retransmit, send_retransmit, tlv
from .p2 import Host, need_record, value, wait_reachable
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
    """Why this answer comes from an image that started after `log0` and `up0` were read, or ""."""
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


UPTIME_BASIS = "the uptime of IDENTIFY (it counts flash work, 0073); start-up included"


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
    if has(ctx, "c3.lose"):
        lost_at_reset(ctx, new)
    else:
        ctx.part("the C-3 record unreadable at the FACTORY_RESET: not run, its place is the firmware's choice (0072)")
    if not flash:
        ctx.notes.append("0069 part (flash) not run: no flash.log")
        return
    erased_reset(ctx, new)


LOST = "config: C-3 lost, TX closed until rewritten"


def lost_at_reset(ctx, s: bytes):
    """0072 on the reference model (c3.lose): the C-3 record unreadable at the FACTORY_RESET. The next
    start has TX closed (0 W on every band, the line of A1.6 "Lost") and the other items of Table A1-4
    restored. The model is then put back by the vendor path (vendor.install), which returns C-3 to the
    built-in values (0072 point 6)."""
    ip = bytes(int(x) for x in ctx.endpoint["radio"]["ip"].split("."))
    with updating(ctx, s, restart_at_end=False) as u:
        r = u.req("SET_CONFIG", config_tlv((11, ip[:2] + b"\0\0" + b"\x10")))
        if not ctx.eq("C-3 lost at the FACTORY_RESET: C-7 changed beforehand", r["status"] if r else None, 0):
            return
        n0 = max([e[0] for e in read_log(u.c)], default=-1)
        ctx.fixture("c3.lose")
        st, _, _ = factory_reset(ctx, u)
        if not ctx.eq("C-3 lost at the FACTORY_RESET: FACTORY_RESET 0", st, 0):
            return
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        conf = get_config(c)
        texts = [e[2] for e in read_log(c, n0 + 1)]
        t = tlv((req_retransmit(c, "IDENTIFY") or {"payload": b""})["payload"])
    ctx.check("C-3 lost at the FACTORY_RESET: the next start has TX closed (0 W on every band in GET_CONFIG)",
              bool(conf[8]) and all(v == 0 for v in conf[8].values()), f"{conf[8]}")
    ctx.eq(f"C-3 lost at the FACTORY_RESET: '{LOST}' logged at that start", texts.count(LOST), 1)
    ctx.eq("C-3 lost at the FACTORY_RESET: the other items restored (C-1 DHCP, C-7 empty, no secret, the window open)",
           (conf.get(1), conf.get(11), t.get(11), t.get(12, b"")[:1]), (b"\x00", b"", b"\x00", b"\x01"))
    ctx.fixture("vendor.install")
    ctx.endpoint.get("_learned", {}).pop("secret", None)
    secret(ctx)
    ctx.notes.append("after the C-3 case the model is put back by vendor.install: no secret, a new one set by the bench")


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
    STALE, UNREC = "store: stale secret erased", "store: non-live sector erased (unrecognised data)"
    with updating(ctx, second, restart_at_end=False) as u:
        holders = where(ctx, secs, second) if known else []
        flash = has(ctx, "flash.log")
        if flash:
            flog_start(ctx)
        ctx.fixture("flash.fault", op="erase", after=1, flag="PGSERR")  # the second erase of the request
        st, _, _ = factory_reset(ctx, u)
        ctx.eq("FACTORY_RESET whose second erase fails: 0 (it fails after the reply)", st, 0)
        texts = [e[2] for e in read_log(u.c)]
        i = [k for k, x in enumerate(texts) if x.startswith("restart: factory-reset")]
        run = texts[i[-1]:] if i else []
        # the line that is due: the failed erase's flash.log entry says which sector it hit and how much it
        # erased; on the sector that held the old secret and with nothing erased, the record is whole
        failed = [e for e in (flog(ctx) if flash else []) if is_erase(e) and str(e.get("result", "")).startswith("error")]
        hit = [x for x in holders if failed and in_range(failed[0], x, x + SECTOR)]
        whole = bool(hit) and not failed[0].get("fraction_done")
        if whole:
            ctx.check(f"finished at the next start: '{STALE}' (the failed erase left the old secret's record whole: flash.dump "
                      "before, flash.log of the erase)", any(STALE in x for x in run), f"{texts[-5:]}")
        else:
            # whole or in part (0069 addendum): either line
            ctx.check("finished at the next start: one of the start check's lines in this run's log",
                      any(STALE in x or UNREC in x for x in run), f"{texts[-5:]}")
            ctx.notes.append("the failed erase: which line is due not decided (" + (
                "no flash.log" if not flash else "no failed erase in flash.log" if not failed else
                "the secret's sector not known" if not holders else "not the secret's sector" if not hit else
                f"{failed[0].get('fraction_done')} of the sector erased") + "): either start-check line taken")
        if known:
            ctx.eq("neither store sector holds the old secret (failed erase finished)", where(ctx, secs, second), [])
    ctx.eq("SET_FIRST_SECRET", first_secret(ctx, os.urandom(32)), 0)


# ---------------- T-A1-16, T-A1-04 ----------------
WINDOW_S = 300


@test("T-A1-16", "T-A1-04", own_boot=True)
def t_a1_16(ctx: Ctx):
    """A1.10 "First secret": the window opens only in factory state, lasts 300 s from network up; a
    restart inside it opens a fresh one; once it ran out, or a secret is set, no restart reopens it.
    It ends without a secret and without a window: the model is put back with vendor.install, an
    emulator boots afresh for the next test, a radio would stay locked out, so never on one."""
    from .t_update import advance
    if ctx.kind not in ("refmodel", "emulator"):
        raise Skip("leaves the radio without a secret and without a window: only on the reference model or an emulator")
    for fx in ("power.cycle", "time.advance"):
        ctx.need(fx)
    if ctx.kind == "refmodel":
        ctx.need("vendor.install")
    restarts = ["power.cycle"] + (["reset.wdt"] if has(ctx, "reset.wdt") else [])
    clock = RadioClock(ctx)

    def window():
        """IDENTIFY tags 11, 12: (secret set, window open, seconds left)."""
        with Mgmt(ctx.net) as c:
            c.clock = clock
            r = req_retransmit(c, "IDENTIFY")
            t = tlv(r["payload"]) if r and r["status"] == 0 else {}
        w = t.get(12, b"")
        ctx.got(t.get(11) and len(w) == 3, "IDENTIFY tags 11 and 12")
        return t[11] == b"\x01", w[0] == 1, int.from_bytes(w[1:], "big")

    def restarted(how: str):
        ctx.fixture(how)
        wait_reachable(ctx, how)

    def closed(when: str, secret_set: bool):
        st = first_secret(ctx, os.urandom(32))
        ctx.eq(f"{when}: SET_FIRST_SECRET 36", st, 36)
        ctx.eq(f"{when}: no window (tag 12), secret set (tag 11)", window()[:2][::-1], (False, secret_set))
        return st == 36

    def opened(when: str):
        was, is_open, left = window()
        return ctx.check(f"{when}: no secret, the window open with at most {WINDOW_S} s left", not was and is_open and 0 < left <= WINDOW_S,
                         f"secret set {was}, open {is_open}, {left} s left")

    try:
        was, is_open, _ = window()
        if not is_open:
            s = secret(ctx)  # or Skip: no secret known and no window
            with updating(ctx, s, restart_at_end=False) as u:
                ctx.eq("FACTORY_RESET: 0", factory_reset(ctx, u)[0], 0)
                u.s = b""
            ctx.endpoint.get("_learned", {}).pop("secret", None)
        if not opened("in factory state"):
            return
        for how in restarts:
            advance(ctx, 100)
            before = window()[2]
            t0 = clock.now()
            restarted(how)
            if opened(f"{how} inside the window"):
                left, took = window()[2], clock.now() - t0
                ctx.check(f"{how} inside the window: a fresh {WINDOW_S} s", left >= WINDOW_S - took - 2, f"{before} s left before, {left} s "
                          f"{took:.1f} s after the restart began")
        s = os.urandom(32)
        if not ctx.eq("SET_FIRST_SECRET in the window", first_secret(ctx, s), 0):
            return
        closed("after a secret is set", True)
        for how in restarts:
            restarted(how)
            closed(f"a secret set, after {how}", True)
        with updating(ctx, s, restart_at_end=False) as u:
            if not ctx.eq("FACTORY_RESET: 0", factory_reset(ctx, u)[0], 0):
                return
            u.s = b""
        ctx.endpoint.get("_learned", {}).pop("secret", None)
        if not opened("after FACTORY_RESET"):
            return
        # the end of the window, against the seconds left the radio itself reports
        t0, left = clock.now(), window()[2]
        if left > 12:
            advance(ctx, left - 10)
            elapsed = clock.now() - t0 + (0 if clock.emu else left - 10)
            is_open = window()[1]
            if elapsed < left - 2:
                ctx.check("the window open until its end", is_open, f"{elapsed:.1f} s into the {left} s left")
            else:
                ctx.notes.append("'open until its end' not judged: the radio's time ran past it")
        if ctx.kind == "emulator":  # the claim of 0073 is the emulator's: a part not shown leaves no pass there
            n_parts = len(ctx.parts)
            going_on = hung_at_the_end(ctx, clock, window)
            ctx.no_pass = ctx.no_pass or len(ctx.parts) > n_parts
            if not going_on:
                return
        advance(ctx, 15)
        if not closed(f"{WINDOW_S} s after network up", False):
            return
        for how in restarts:
            restarted(how)
            closed(f"the window run out, after {how}: not reopened", False)
        ctx.notes.append("restarts: " + ", ".join(restarts) + " (RESTART needs a secret)")
    finally:
        if has(ctx, "vendor.install"):
            ctx.fixture("vendor.install")
            ctx.endpoint.get("_learned", {}).pop("secret", None)


def hung_at_the_end(ctx, clock, window) -> bool:
    """0073: host A's SET_FIRST_SECRET 2 s before the window ends, its erase ending without a flag 100 ms after the give-up (0081);
    B's request received during that work and A's retry get 36. False: the secret got set."""
    from .t_flash import flog, running
    from .t_update import advance
    from .mgmt import send_retransmit
    if not (has(ctx, "flash.fault") and has(ctx, "flash.log")):
        ctx.part("a SET_FIRST_SECRET whose erase ends late over the end of the window not run: no flash.fault or flash.log")
        return True
    back = stretched(ctx)
    try:
        with Mgmt(ctx.net) as a, Mgmt(ctx.net) as b:
            a.clock = b.clock = clock
            left = window()[2]
            if left > 3:
                advance(ctx, left - 2.5)
            _, is_open, left = window()
            if not is_open or left > 5:
                ctx.part(f"a SET_FIRST_SECRET whose erase ends late over the end of the window: not placed ({left} s left, open {is_open})")
                return True
            flog_start(ctx)
            from .t_flash import late_erase
            if not late_erase(ctx):  # 0081: the erase ends without a flag 100 ms after the give-up
                return True
            msg_a = build(CMD["SET_FIRST_SECRET"], os.urandom(32), a.next_id())
            msg_b = build(CMD["SET_FIRST_SECRET"], os.urandom(32), b.next_id())
            a.post(msg_a)
            w0, seen = time.monotonic(), False
            while not seen and time.monotonic() - w0 < 60 and not (got := a.listen(msg_a, 0.02)):
                seen = any(running(e) for e in flog(ctx))
            if seen:
                b.post(msg_b)
                got = []
                t0 = clock.now()
                while not got and clock.now() - t0 < FLASH_WINDOW_S and time.monotonic() - w0 < 300:
                    got = a.listen(msg_a, 0.5)
            st = got[0]["status"] if got else None
            if st == 0:
                keep_secret(ctx, msg_a[18:])
                ctx.part("a SET_FIRST_SECRET whose erase ends late over the end of the window: answered 0, the late erase was not its own")
                return False
            if not seen:
                ctx.part(f"a SET_FIRST_SECRET whose erase ends late over the end of the window: no erase running in flash.log (answered {st})")
                return True
            ctx.eq("SET_FIRST_SECRET whose erase ends late over the end of the window: 55", st, 55)
            rb = b.listen(msg_b, 1.0)
            rb = rb[0] if rb else send_retransmit(b, msg_b, 4.0, "SET_FIRST_SECRET of B")
            ctx.eq("B's SET_FIRST_SECRET received during that work: 36", rb["status"] if rb else None, 36)
            ra = req_retransmit(a, "SET_FIRST_SECRET", os.urandom(32))
            ctx.eq("A's retry after the 55: 36", ra["status"] if ra else None, 36)
    finally:
        back()
    return True


# ---------------- T-A1-28 ----------------
BUILT_IN_TX_MS, BUILT_IN_BAND = 300_000, 1000  # Table B8-2; A3.4 (100 W PEP; tag 8 in 0.1 W)


def get_config(c: Mgmt) -> dict:
    """GET_CONFIG by tag; tag 8 (one TLV per band) as {band: limit}."""
    r = req_retransmit(c, "GET_CONFIG")
    p = r["payload"] if r and r["status"] == 0 else b""
    out, i = {8: {}}, 0
    while i + 2 <= len(p):
        v = p[i + 2:i + 2 + p[i + 1]]
        if p[i] == 8 and len(v) == 3:
            out[8][v[0]] = struct.unpack(">H", v[1:])[0]
        elif p[i] != 8:
            out[p[i]] = v
        i += 2 + p[i + 1]
    return out


def names_value(text: str, value: int, scales: tuple) -> bool:
    """The log text holds the value as a number of its own, in any of the units (the spec does not
    fix the text): value / scale, for example 800 (0.1 W), 80 or 80.0 (W), 80000 (mW)."""
    nums = [float(x) for x in re.findall(r"(?<![\d.])\d+(?:\.\d+)?(?![\d]|\.\d)", text)]
    return any(abs(n - value / s) < 1e-6 for n in nums for s in scales)


def logged_change(entries: list, old: int, new: int, ip: str, scales: tuple) -> bool:
    """One entry with the old value, the new value and the source address (A1.6, A1.7)."""
    for _, _, t in entries:
        rest = re.sub(r"(?<![\d.])" + re.escape(ip) + r"(?::\d+)?(?!\d)", " ", t)  # its numbers are no values
        if rest != t and names_value(rest, old, scales) and names_value(rest, new, scales):
            return True
    return False


@test("T-A1-28")
def t_a1_28(ctx: Ctx):
    """A1.6 C-3, A1.7, Table A1-4: a loosened persistent limit is accepted and logged with old value,
    new value and source; looser than built-in 49; built-in values not writable; FACTORY_RESET
    keeps the limits. While P-PWR-2 is pending, 50 MHz is 0 W in the record and in tag 8, 50 W there
    gets 49 and 0 gets 0, and every other band takes 100 W (B4.8; 0079)."""
    s = secret(ctx)
    ip = ctx.host_ip
    u32, u16 = (lambda v: struct.pack(">I", v)), (lambda v: struct.pack(">H", v))
    with updating(ctx, s, restart_at_end=False) as u:
        conf0 = get_config(u.c)
        if not ctx.check("GET_CONFIG lists the TX time (tag 7) and the power limit per band (tag 8)", len(conf0.get(7, b"")) == 4
                         and bool(conf0[8]), f"{conf0}"):
            return
        band = min(conf0[8])
        tx0, band0 = struct.unpack(">I", conf0[7])[0], conf0[8][band]
        tx = lambda v: (7, u32(v))
        pw = lambda v: (8, bytes([band]) + u16(v))

        def write(*items):
            r = u.req("SET_CONFIG", config_tlv(*items))
            return r["status"] if r else None

        def now(tag):
            c = get_config(u.c)
            v = c[8].get(band) if tag == 8 else c.get(tag)
            return int.from_bytes(v, "big") if isinstance(v, bytes) else v

        # (name, tag, the TLV, tight, loosened, value at the start, the units a log text may use)
        cases = [("TX time", 7, tx, 200_000, 250_000, tx0, (1, 1000)),
                 (f"power of band {band}", 8, pw, 400, 600, band0, (1, 10, 0.01))]
        for name, tag, scales in (("SWR limit S2", 9, (1, 100, 0.1)), ("PA temperature T3", 10, (1, 10, 0.01))):
            v0 = int.from_bytes(conf0.get(tag, b""), "big")
            if not v0 or write((tag, u16(v0))) != 0:
                ctx.part(f"{name} (tag {tag}) not run: not set by the calibration set (A1.12)")
                continue
            cases.append((name, tag, lambda v, t=tag: (t, u16(v)), v0 - v0 // 10, v0 - v0 // 20, v0, scales))
        try:
            for name, tag, item, tight, loose, v0, scales in cases:
                n0 = max([e[0] for e in read_log(u.c, complete=True)], default=-1)
                if not ctx.eq(f"{name}: tightened to {tight}: 0", write(item(tight)), 0):
                    continue
                new = read_log(u.c, n0 + 1, complete=True)
                if v0 != tight:
                    ctx.check(f"{name}: the change logged with the source address {ip} (A1.7)", any(ip in e[2] for e in new), f"{[e[2] for e in new]}")
                n0 = max([e[0] for e in new], default=n0)
                ctx.eq(f"{name}: loosened to {loose}: 0", write(item(loose)), 0)
                ctx.eq(f"{name}: in GET_CONFIG", now(tag), loose)
                new = read_log(u.c, n0 + 1, complete=True)
                ctx.check(f"{name}: the loosening logged with old value {tight}, new value {loose} and source {ip} in one entry (A1.6)",
                          logged_change(new, tight, loose, ip, scales), f"{[e[2] for e in new]}")
            for name, tag, item, built in (("TX time", 7, tx, BUILT_IN_TX_MS), (f"power of band {band}", 8, pw, BUILT_IN_BAND)):
                was = now(tag)
                ctx.eq(f"{name}: {built + 1}, looser than any built-in value: 49", write(item(built + 1)), 49)
                ctx.eq(f"{name}: unchanged after it", now(tag), was)
            fifty_mhz(ctx, u, write, conf0[8])
            kept = get_config(u.c)
            if conf0.get(1) == b"\x00":
                st, _, _ = factory_reset(ctx, u)
                if ctx.eq("FACTORY_RESET: 0", st, 0):
                    got = get_config(u.c)
                    ctx.eq("FACTORY_RESET leaves the persistent limits unchanged (tags 7 to 10)", [got.get(k) for k in (7, 8, 9, 10)],
                           [kept.get(k) for k in (7, 8, 9, 10)])
                    new_s = os.urandom(32)
                    u.s = new_s
                    if not ctx.eq("SET_FIRST_SECRET after it: 0", first_secret(ctx, new_s), 0):
                        return
                    ctx.eq(f"TX time after FACTORY_RESET: {BUILT_IN_TX_MS + 1} still gets 49", write(tx(BUILT_IN_TX_MS + 1)), 49)
            else:
                ctx.part("'FACTORY_RESET leaves the persistent limits unchanged' not run: static address, which it would clear")
        finally:
            for name, _, item, _, _, v0, _ in cases:
                ctx.eq(f"{name}: put back to {v0}", write(item(v0)), 0)
            ctx.eq("every band limit put back", write(*[(8, bytes([i]) + u16(v)) for i, v in sorted(conf0[8].items())]), 0)


def fifty_mhz(ctx: Ctx, u, write, bands0: dict):
    """0079: the built-in limit of a band is the maximum power of its tx_band entry (B4.8); at 50 MHz
    0 W while P-PWR-2 is pending (Annex P), so the record and tag 8 carry 0 there."""
    with Host(ctx.net) as h:
        rec = need_record(ctx, h)
    entries = [(rec[0x22 + 3 * i], rec[0x24 + 3 * i]) for i in range(rec.get(0x21, 0))]
    six = [i for i, (lo, _) in enumerate(entries) if lo >= 50_000_000]
    if not ctx.check("the record has one tx_band entry from 50 MHz (Table A3-0)", len(six) == 1, f"{entries}"):
        return
    i6, band = six[0], (lambda i, v: (8, bytes([i]) + struct.pack(">H", v)))
    ctx.eq(f"tx_band[{i6}] (50 MHz): both power fields 0 while P-PWR-2 is pending (B4.8; 0079)", entries[i6][1], 0)
    ctx.eq(f"GET_CONFIG tag 8, band {i6} (50 MHz): 0", get_config(u.c)[8].get(i6), 0)
    ctx.eq("a 50 MHz limit of 50 W: 49", write(band(i6, 500)), 49)
    ctx.eq("a 50 MHz limit of 0 W: 0", write(band(i6, 0)), 0)
    for i in sorted(bands0):
        if i != i6:
            ctx.eq(f"band {i} ({entries[i][0] / 1e6:g} MHz): a limit of 100 W: 0", write(band(i, 1000)), 0)


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
