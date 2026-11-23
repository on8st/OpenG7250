"""T-A3-47: drop events while not armed (A3.2 "Drop events while not armed", A3.6; 0022 §73 item 3,
§79; 0072 point D). GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import threading
import time

from .core import Ctx, test
from .p2 import PORT_REG, TX_ARM, TX_ARM_STATE, TX_ARM_TOKEN, Host, parse_regs, value

DROP_TAKEOVER = 4  # TX_ARM_STATE [23:16] (Table B8-3, Table A3-1 event 4)
WATCHDOG_S = 1.0  # B5.6


def pushed(got, addr: int) -> list[int]:
    """The values of one register in the register packets received."""
    return [v for p, d, _ in got if p == PORT_REG for a, v in parse_regs(d) if a == addr]


def begin(h: Host) -> list:
    """A session of host h; its packets of the first 0.3 s (the session-start push among them)."""
    h.start()
    got = []
    for _ in range(3):
        h.keepalive()
        got += h.recv(0.1)
    return got


def state(h: Host) -> int | None:
    """TX_ARM_STATE, read by a write to it (RO: answered with its value, B8.4a)."""
    return value(h.write([(TX_ARM_STATE, 0)]), TX_ARM_STATE)


def token(h: Host) -> int | None:
    return value(h.write([(TX_ARM_TOKEN, 0)]), TX_ARM_TOKEN)


def fields(v: int | None) -> str:
    return "none" if v is None else f"state {v & 0xF}, reject {(v >> 8) & 0xFF}, drop {(v >> 16) & 0xFF}, generation {v >> 24}"


def kept(ctx, label: str, before: int | None, after: int | None):
    ctx.check(f"{label}: no field of TX_ARM_STATE changed", before is not None and after == before,
              f"before {fields(before)}; after {fields(after)}")


def session_start_token(ctx, label: str, got: list, h: Host) -> int | None:
    """The token pushed at a session start (B10.2 item 1), from the packets of its first 0.3 s."""
    t = pushed(got, TX_ARM_TOKEN)
    ctx.check(f"{label}: TX_ARM_TOKEN pushed at the next session start", bool(t), f"{len(got)} packets")
    return t[-1] if t else token(h)


def arm_zero(ctx, a: Host, st0: int, tok0: int):
    """TX_ARM = 0 while not armed: the session goes on, nothing changes, nothing is pushed."""
    label = "TX_ARM = 0 while not armed"
    got = a.write([(TX_ARM, 0)], timeout=0.3)
    ctx.eq(f"{label}: its answer is TX_ARM_STATE, unchanged", value(got, TX_ARM_STATE), st0)
    later = a.recv_alive(0.5)
    ctx.eq(f"{label}: no TX_ARM_TOKEN push", pushed(later, TX_ARM_TOKEN), [])
    kept(ctx, label, st0, state(a))
    ctx.eq(f"{label}: no new token", token(a), tok0)


def c3_change(ctx, a: Host, st0: int, tok0: int):
    """A C-3 change while not armed (a drop event, Table A3-1 event 11): nothing changes; if the session
    ends by its watchdog over the flash work (B2.3), that end issues its token as usual (0072 point D)."""
    from .t_flash import set_tx_time, tx_time
    from .t_mgmt import secret_of
    from .t_update import updating
    label = "a C-3 change while not armed"
    stop, got = threading.Event(), []

    def feed():  # C&C every 100 ms during the request: only the radio's flash work can stop the session
        while not stop.is_set():
            a.keepalive()
            time.sleep(0.1)
    with updating(ctx, secret_of(ctx), restart_at_end=False) as u:
        ms = tx_time(u)
        if ms is None:
            ctx.part(f"{label}: not run, GET_CONFIG gives no TX time")
            return
        th = threading.Thread(target=feed, daemon=True)
        clock = u.c.clock
        t0 = clock.now()
        th.start()
        try:
            r = set_tx_time(u, ms)  # the same value: a C-3 write all the same (always a compaction, 0072)
        finally:
            dt = clock.now() - t0
            stop.set()
            th.join()
    if not ctx.eq(f"{label}: SET_CONFIG of the TX time 0", r["status"] if r else None, 0):
        return
    got = a.recv_alive(0.4)
    starts = pushed(got, TX_ARM_TOKEN)
    if starts:  # the session ended and the keepalive began a new one
        ctx.check(f"{label}: the session ended only by its watchdog over the flash work (reply after {dt:.2f} s, B2.3)",
                  dt >= WATCHDOG_S, f"reply after {dt:.2f} s of {'radio' if clock.emu else 'host'} time")
        ctx.check(f"{label}: that end issued a new token, pushed at the next session start", starts[-1] != tok0,
                  f"before {tok0:#010x}, pushed {starts[-1]:#010x}")
        kept(ctx, label, st0, state(a))
        ctx.notes.append(f"{label}: the session ended by its watchdog over the flash work ({dt:.2f} s), which 0072 allows")
    else:
        kept(ctx, label, st0, state(a))
        ctx.eq(f"{label}: no new token", token(a), tok0)


def session_end(ctx, how: str):
    """A session end while not armed (the watchdog stop, RUN_CLEARED): TX_ARM_STATE unchanged, a new
    token issued at the end and pushed at the next session start."""
    label = f"{how} while not armed"
    with Host(ctx.net) as a:
        got = begin(a)
        tok0 = pushed(got, TX_ARM_TOKEN)
        tok0 = tok0[-1] if tok0 else token(a)
        st0 = state(a)
        if how == "the watchdog stop":
            a.recv(WATCHDOG_S + 0.7)  # no C&C: the watchdog ends the session
        else:
            a.stop()
        tok1 = session_start_token(ctx, label, begin(a), a)
        ctx.check(f"{label}: a new token at the session end", tok0 is not None and tok1 != tok0,
                  f"before {tok0}, after {tok1}")
        kept(ctx, label, st0, state(a))
        a.stop()


@test("T-A3-47")
def t_a3_47(ctx: Ctx):
    """A3.2 "Drop events while not armed", A3.6, A2.4 (0022 §73 item 3, §79; 0072 point D)."""
    with Host(ctx.net) as a:
        got = begin(a)
        tok0 = pushed(got, TX_ARM_TOKEN)
        tok0 = tok0[-1] if tok0 else token(a)
        st0 = state(a)
        if not ctx.check("a session, not armed (TX_ARM_STATE [3:0] = 0)", st0 is not None and st0 & 0xF == 0, fields(st0)):
            a.stop()
            return
        arm_zero(ctx, a, st0, tok0)
        c3_change(ctx, a, st0, token(a))
        a.stop()
    for how in ("the watchdog stop", "RUN_CLEARED"):
        session_end(ctx, how)
    # a takeover while not armed: drop reason 4 (exception 2 of A3.2)
    with Host(ctx.net) as a, Host(ctx.net) as b:
        begin(a)
        st0 = state(a)
        b.start()
        b.recv_alive(0.3)
        st = state(b)
        ctx.eq("a takeover while not armed: drop reason 4 TAKEOVER ([23:16])", None if st is None else (st >> 16) & 0xFF, DROP_TAKEOVER)
        ctx.eq("and the state still disarmed, the generation unchanged",
               None if st is None else (st & 0xF, st >> 24), None if st0 is None else (0, st0 >> 24))
        b.stop()
