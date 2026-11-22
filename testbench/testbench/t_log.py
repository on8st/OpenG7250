"""The log kept over a reset (A1.7; 0068): T-A1-34 and T-A1-91. Reference model or emulator, where
RAM survives a reset without power loss; on the radio the kept part is judged once P-SYS-9 is set.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import time

from .core import Ctx, PartSkip, test
from .mgmt import Mgmt, read_log, req_retransmit, tlv
from .p2 import wait_reachable
from .t_update import (SMALL, Updater, activate_new, advance, facts, image, need_clock, newer, pkg, restart, rx_works, secret,
                       trial_rolls_back, updating, uptime_s)

START = "start after "


def letter(slot: int) -> str:
    return "AB"[slot - 1]


def dotted(v) -> str:
    return f"{v[0]}.{v[1]}.{v[2]}"


def entries(ctx) -> list[tuple[int, int, str]]:
    from .core import RadioClock
    with Mgmt(ctx.net) as c:
        c.clock = RadioClock(ctx)
        return read_log(c)


def ram_kept(ctx) -> bool:
    """On the radio the log may start empty until P-SYS-9 is set (0068); elsewhere it is kept."""
    return "model" in ctx.endpoint or ctx.endpoint.get("kind") != "radio" or bool(ctx.params.get("p_sys_9"))


def last_start(log: list) -> int | None:
    """The index of the start entry of the running run: the last one."""
    i = [k for k, e in enumerate(log) if e[2].startswith(START)]
    return i[-1] if i else None


def judge_run(ctx, label: str, log: list, n0: int, reason: str | None, cause: str = "software", boot: tuple = ()) -> int | None:
    """The end of the previous run and the start of this one: `reason` is the last entry before the
    start entry (None: not commanded, only kept entries), the start entry names `cause` and "log
    kept", the boot lines follow it in order, and the numbers go on without a gap from n0 (the last
    entry read before). The index of the start entry in `log`."""
    k = last_start(log)
    tail = [e[2] for e in log[-6:]]
    if not ctx.check(f"{label}: a start entry", k is not None, f"{tail}"):
        return None
    if not ram_kept(ctx):
        ctx.check(f"{label}: the start entry with log kept or log lost (P-SYS-9 not set)",
                  log[k][2] in (f"{START}{cause}: log kept", f"{START}{cause}: log lost"), log[k][2])
        return k
    ctx.eq(f"{label}: the start entry", log[k][2], f"{START}{cause}: log kept")
    if reason is not None:
        ctx.eq(f"{label}: the last entry before it", log[k - 1][2] if k else None, reason)
    if boot:
        ctx.eq(f"{label}: the boot stage's lines right after it", [e[2] for e in log[k + 1:k + 1 + len(boot)]], list(boot))
    seqs = [e[0] for e in log if e[0] > n0]
    ctx.check(f"{label}: entries in sequence with no gap across the reset", seqs == list(range(n0 + 1, n0 + 1 + len(seqs))) and
              any(e[0] <= n0 for e in log), f"last before {n0}, then {seqs[:12]}")
    return k


@test("T-A1-34")
def t_a1_34(ctx: Ctx):
    """A1.8, A1.7 (0068): after UPDATE_BEGIN and 10 min without an update command the update is
    aborted and the radio restarts into the running image; READ_LOG then shows why."""
    from .t_update import expect_restart_at
    need_clock(ctx)
    s = secret(ctx)
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        n0 = max([e[0] for e in read_log(u.c)], default=-1)
        state0 = u.identify().get(10)
        before = uptime_s(u) or 0
        r = u.begin(pkg(image(SMALL, seed=34), newer(ver), other))
        after = uptime_s(u) or 0
        if not ctx.eq("UPDATE_BEGIN accepted", r["status"] if r else None, 0):
            return
        expect_restart_at(ctx, u, "after UPDATE_BEGIN only", 600, 0)
        t = u.identify()
        ctx.eq("after the restart: the running image unchanged", t.get(23, b"")[:1], bytes([run]))
        if state0 == b"\x00":
            ctx.eq("IDENTIFY tag 10 reports 0", t.get(10), b"\x00")
        else:
            ctx.notes.append(f"tag 10 = 0 not judged: an earlier test left {state0!r} before this one (a rollback), now {t.get(10)!r}")
        ctx.eq("UPDATE_STATUS reports idle", (u.status() or {}).get("state"), 0)
        log = read_log(u.c)
        k = judge_run(ctx, "READ_LOG", log, n0, "restart: update-abandoned")
        if k and log[k - 1][2] == "restart: update-abandoned":
            up = log[k - 1][1] / 1000
            ctx.check("its uptime (of the previous run) within 5 s of UPDATE_BEGIN plus 10 min",
                      before + 595 <= up <= after + 605, f"{up:.1f} s, UPDATE_BEGIN at {before:.1f} to {after:.1f} s")
    ctx.check("RX works afterwards (a session gets status packets)", rx_works(ctx), "no status packet")


def lost(ctx, label: str, cause: str):
    """The log of a start that counts as a power loss, or whose ring failed as a whole."""
    from .p2 import Host, value
    log = entries(ctx)
    ctx.eq(f"{label}: the first entry", log[0][::2] if log else None, (0, f"{START}{cause}: log lost"))
    ctx.eq(f"{label}: numbers in sequence from 0", [e[0] for e in log], list(range(len(log))))
    if cause == "unknown" or label.startswith("power-on"):
        with Host(ctx.net) as h:
            ctx.eq(f"{label}: TRIP_COUNT from 0", value(h.write([(0x0214, 0)], timeout=0.3), 0x0214), 0)


def newest(ctx) -> int:
    return max([e[0] for e in entries(ctx)], default=-1)


@test("T-A1-91")
def t_a1_91(ctx: Ctx):
    """A1.7 "Kept over a reset", "The reason before a commanded reset", "The start entry" (0068)."""
    need_clock(ctx)
    s = secret(ctx)
    fx = ctx.endpoint.get("fixtures", [])
    # RESTART
    n0 = newest(ctx)
    with updating(ctx, s):
        pass
    judge_run(ctx, "RESTART", entries(ctx), n0, "restart: restart")
    # the abandonment after UPDATE_ABORT
    with updating(ctx, s, restart_at_end=False) as u:
        run, other, ver, lay = facts(ctx, u)
        r = u.begin(pkg(image(SMALL, seed=91), newer(ver), other))
        if ctx.eq("UPDATE_BEGIN accepted (abandonment)", r["status"] if r else None, 0):
            u.req("UPDATE_ABORT")
            n0 = max([e[0] for e in read_log(u.c)], default=-1)
            advance(ctx, 610)
            wait_reachable(ctx, "the abandonment")
            judge_run(ctx, "abandonment after UPDATE_ABORT", read_log(u.c), n0, "restart: update-abandoned")
    # a watchdog reset keeps the entries before it
    if "reset.wdt" in fx:
        n0 = newest(ctx)
        ctx.fixture("reset.wdt")
        time.sleep(0.3)
        wait_reachable(ctx, "a watchdog reset")
        judge_run(ctx, "watchdog reset", entries(ctx), n0, None, cause="watchdog")
        with Mgmt(ctx.net) as c:
            r = req_retransmit(c, "IDENTIFY")
            ctx.eq("watchdog reset: IDENTIFY tag 17 = 3", tlv(r["payload"]).get(17) if r else None, b"\x03")
    else:
        ctx.notes.append("watchdog part not run: no reset.wdt")
    # one entry of the kept ring corrupted: it and those after it are dropped
    if "log.corrupt" in fx:
        log = entries(ctx)
        bad = log[-2][0]
        ctx.fixture("log.corrupt", seq=bad)
        with updating(ctx, s):
            pass
        got = entries(ctx)
        ctx.eq("a corrupted entry: the entries before it kept", [e for e in got if e[0] < bad], [e for e in log if e[0] < bad])
        ctx.check("it and those after it dropped, the numbers going on from the last one kept",
                  bool(got) and any(e[0] == bad and e[2] == f"{START}software: log kept" for e in got),
                  f"{[e for e in got if e[0] >= bad][:3]}")
    else:
        ctx.notes.append("corrupted entries not run: no log.corrupt (one entry of the ring changed in RAM; the layout is the firmware's)")
        # the oldest entry corrupted: nothing is kept, which is a ring that fails as a whole
        ctx.fixture("log.corrupt", seq=entries(ctx)[0][0])
        with updating(ctx, s):
            pass
        lost(ctx, "the oldest entry corrupted", "software")
    # a start with no reset flag, or with power-on beside software: a power loss, whatever is named
    if "reset.flags" in fx:
        for label, flags, cause in (("no reset flag", [], "unknown"), ("power-on beside software", ["software", "power-on"], "software")):
            ctx.fixture("reset.flags", flags=flags)
            with updating(ctx, s):
                pass
            lost(ctx, label, cause)
    else:
        ctx.notes.append("a start with no reset flag, and power-on beside software, not run: no reset.flags")
    # FACTORY_RESET is judged in T-A1-39; a power cycle empties the log
    if "power.cycle" in fx:
        ctx.fixture("power.cycle")
        wait_reachable(ctx, "a power cycle")
        log = entries(ctx)
        ctx.eq("power cycle: sequence 0 is the start entry, log lost", log[0][::2] if log else None, (0, f"{START}power-on: log lost"))
        ctx.eq("power cycle: in sequence from 0", [e[0] for e in log], list(range(len(log))))
    # updates: these need a package that verifies
    n0 = newest(ctx)
    a = activate_new(ctx, s, 911)
    if not ctx.check("an update activated", a is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    judge_run(ctx, "UPDATE_ACTIVATE", entries(ctx), n0, f"restart: update-activated slot {letter(a[1])} {dotted(a[3])}",
              boot=(f"boot: trial of slot {letter(a[1])} started",))
    advance(ctx, 11)
    with updating(ctx, s, restart_at_end=False) as u:
        if not ctx.eq("the trial confirmed", u.identify().get(10), b"\x00"):
            return
        n0 = max([e[0] for e in read_log(u.c)], default=-1)
        r = u.req("ROLLBACK")
        ctx.eq("ROLLBACK: 0", r["status"] if r else None, 0)
    time.sleep(0.3)
    wait_reachable(ctx, "ROLLBACK")
    judge_run(ctx, "ROLLBACK", entries(ctx), n0,
              f"rollback commanded: slot {letter(a[1])} {dotted(a[3])} to slot {letter(a[0])} {dotted(a[2])}",
              boot=(f"boot: trial of slot {letter(a[0])} started",))
    # that trial resets before confirming
    back = f"boot: rollback to slot {letter(a[1])} (trial not confirmed)"
    if "reset.wdt" in fx:
        with updating(ctx, s, restart_at_end=False) as u:
            trial = u.identify().get(10) == b"\x01"
            n0 = max([e[0] for e in read_log(u.c)], default=-1)
        if trial:
            ctx.fixture("reset.wdt")
            time.sleep(0.3)
            wait_reachable(ctx, "a watchdog reset in the trial")
            judge_run(ctx, "trial reset before confirming", entries(ctx), n0, None, cause="watchdog", boot=(back,))
        else:
            ctx.notes.append("trial reset before confirming: the trial had confirmed itself before the reset; not judged")
    else:
        advance(ctx, 11)
    # the trial's 60 s limit
    if not ({"trial.sim", "ch394.silent"} & set(fx)):
        raise PartSkip("the trial's 60 s limit: needs trial.sim or ch394.silent")
    n0 = newest(ctx)
    b = activate_new(ctx, s, 912, net_down=True)
    if not ctx.check("an update activated (60 s limit)", b is not None, ctx.__dict__.get("_upd_fail") or "the update did not complete"):
        return
    trial_rolls_back(ctx)
    judge_run(ctx, "the trial's 60 s limit", entries(ctx), n0, "restart: trial-unconfirmed",
              boot=(f"boot: rollback to slot {letter(b[0])} (trial not confirmed)",))
