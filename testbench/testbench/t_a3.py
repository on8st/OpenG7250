"""A3.6b TX path check (0066, with c245f5e: a check at least every 5 ms; the fault when every
check over at least 10 ms sees the mismatch, always within 20 ms for one of 20 ms or more):
T-A3-61. Emulator, with gpio.force (format v1.8).

The pins are those of Table A3-6 (A3.6b; 0066 addendum, SF-30), with their safe level at reset
from HIS 85: PF4 high; PF6, PC7, PH11 low (T/R lines); PA10 high (chain A output enable); PB14
low (PA DAC reset); each checked by configuration and pin level. The DAC bias channels 2, 7, 8
are checked by the value last written only; their stimulus (a simulated firmware fault writing a
non-zero value) needs a fixture the format does not have yet, so that part is not run.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import time

from .core import Ctx, test
from .p2 import Host, value, wait_reachable

TX_PINS = {"PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0, "PA10": 1, "PB14": 0}
FAULT_CODE, FAULT_FLAGS = 0x0211, 0x0212
TX_PATH_NOT_BLOCKED = 16


def faults(ctx):
    with Host(ctx.net) as h:
        ans = h.write([(FAULT_CODE, 0), (FAULT_FLAGS, 0)], timeout=0.2)
    return value(ans, FAULT_CODE), value(ans, FAULT_FLAGS)


def wait_radio(ctx, ms: float):
    if "time.wait" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("time.wait", us=int(ms * 1000))
    else:
        time.sleep(ms / 1000)


def fault16(code, flags) -> bool:
    return code == TX_PATH_NOT_BLOCKED and flags is not None and flags >> TX_PATH_NOT_BLOCKED & 1 == 1


@test("T-A3-61")
def t_a3_61(ctx: Ctx):
    ctx.need("gpio.force")
    from .mgmt import read_log
    from .t_update import Updater, rx_works, updating
    from .t_mgmt import secret_of
    s = secret_of(ctx)

    def clean(label: str):
        """The fault stays until a reset (class 2): a RESTART with every force released."""
        with updating(ctx, s) as u:
            pass
        code, flags = faults(ctx)
        ctx.check(f"{label}: after a RESTART with the force removed the fault is gone", not fault16(code, flags),
                  f"FAULT_CODE {code}, FAULT_FLAGS {flags}")

    with updating(ctx, s, restart_at_end=False) as u:
        n0 = max([e[0] for e in read_log(u.c)], default=-1)
    for pin, safe in TX_PINS.items():
        other = 1 - safe
        # item 2: the level at the pin, held from outside (output register unchanged)
        ctx.fixture("gpio.force", pin=pin, level=other, target="idr")
        wait_radio(ctx, 20)
        code, flags = faults(ctx)
        ctx.check(f"{pin} held at {other} from outside: FAULT_CODE 16 TX_PATH_NOT_BLOCKED within 20 ms (item 2)",
                  fault16(code, flags), f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
        ctx.fixture("gpio.force", pin=pin, level=None, target="idr")
        if pin == "PF4":
            ctx.check(f"{pin}: RX goes on with the fault", rx_works(ctx), "no status packet")
            with updating(ctx, s, restart_at_end=False) as u:
                texts = [e[2] for e in read_log(u.c) if e[0] > n0]
            ctx.check(f"{pin}: the log names the pin and the comparison", any(pin in x for x in texts), f"{texts[-3:]}")
        clean(pin)
    # item 1: the configuration (output register, an input), separately
    for target, kw in (("odr", dict(level=0)), ("moder", dict(mode="input"))):
        ctx.fixture("gpio.force", pin="PF4", target=target, **kw)
        wait_radio(ctx, 20)
        code, flags = faults(ctx)
        ctx.check(f"PF4 {target} changed (emulator memory write): FAULT_CODE 16 within 20 ms (item 1)", fault16(code, flags),
                  f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
        ctx.fixture("gpio.force", pin="PF4", target=target, level=None)
        clean(f"PF4 {target}")
    # a 5 ms pulse (a mismatch shorter than 10 ms never gives the fault; c245f5e): no fault
    ctx.fixture("gpio.force", pin="PF4", level=0, target="idr", from_us=1000)
    ctx.fixture("gpio.force", pin="PF4", level=None, target="idr", from_us=6000)
    wait_radio(ctx, 40)
    code, flags = faults(ctx)
    ctx.check("a 5 ms pulse gives no fault (a mismatch shorter than 10 ms never does)", not fault16(code, flags),
              f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
    # with the force kept, the fault comes back at start-up
    ctx.fixture("gpio.force", pin="PF4", level=0, target="idr")
    try:
        with updating(ctx, s) as u:
            pass
        wait_radio(ctx, 40)
        code, flags = faults(ctx)
        ctx.check("after a RESTART with the force kept the fault comes back at start-up", fault16(code, flags),
                  f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
    finally:
        ctx.fixture("gpio.force", pin="PF4", level=None, target="idr")
    clean("final")
    ctx.notes.append("'armed with the TX path open, no fault' not run: needs TX unlocked (an arm)")
    # the bias channels 2, 7, 8: a non-zero value written by a simulated firmware fault
    # (dac.force, v1.8), checked by the value last written (A3.6b; Table A3-6)
    if "dac.force" in ctx.endpoint.get("fixtures", []):
        for ch in (2, 7, 8):
            ctx.fixture("dac.force", channel=ch, value=1)
            wait_radio(ctx, 20)
            code, flags = faults(ctx)
            ctx.check(f"DAC channel {ch} set non-zero: FAULT_CODE 16 within 20 ms", fault16(code, flags),
                      f"FAULT_CODE {code}, FAULT_FLAGS {flags}")
            ctx.fixture("dac.force", channel=ch, value=0)
            clean(f"DAC channel {ch}")
    else:
        ctx.notes.append("DAC part (a non-zero value on channel 2, 7 or 8) not run: no dac.force")
