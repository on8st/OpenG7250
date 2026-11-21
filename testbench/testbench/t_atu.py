"""ATU tune cycle and its unkey reasons: A3.9b item 6 (0047), T-B10-47.

The test needs TX unlocked (record tx bit 4 = 1) and a way to make a cycle end tuned, end
failed or run into the ATU tune time. On the reference model that is the model-only fixture
atu.sim; the abort by ATU_TUNE = 0 runs without it. The "stop request" of item 1 is the TCI
command of part C C5.7: not on P2.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import time

from .core import Blocked, Ctx, Skip, test
from .p2 import (ARM_MAGIC, PORT_REG, TX_ARM, TX_ARM_STATE, TX_MODE_CLASS, TX_STATE, Host, need_record,
                 need_token, parse_regs, value, write_status)
from .t_cw import pending_reason

ATU_TUNE, ATU_STATE = 0x0400, 0x0401
F_TX = 14.05e6
NOOP_S = 2.0  # a start within 2 s of the end of the previous cycle is a no-op (item 1)
LIMIT_ATU_TIME = 8


def read(h: Host, a: int):
    """A read-only register read by a write (B8.4a); the answer comes within 5 ms (B8.4)."""
    return value(h.write([(a, 0)], timeout=0.05), a)


def tx_state(h: Host):
    return read(h, TX_STATE)


def atu_state(h: Host):
    return read(h, ATU_STATE)


def setup(ctx, h: Host):
    rec = need_record(ctx, h)
    if not (rec[0x20] >> 4) & 1:
        raise Blocked(pending_reason(ctx))
    h.start(pa=True)
    h.keepalive(duc0_hz=F_TX)
    pairs = sum((parse_regs(d) for _, d, _ in h.recv(0.3, PORT_REG)), [])
    tok = need_token(ctx, h, pairs)
    h.write([(TX_MODE_CLASS, 2)])
    st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
    if not ctx.check("armed (TX_ARM_STATE [3:0] = 1)", st is not None and st & 0xF == 1, f"TX_ARM_STATE {st}"):
        raise Skip("arm not accepted")


def wait(h: Host, s: float):
    end = time.monotonic() + s
    while time.monotonic() < end:
        h.keepalive(duc0_hz=F_TX)
        h.recv(min(0.1, max(0.0, end - time.monotonic())))


def start(ctx, h: Host, label: str) -> bool:
    ans = h.write([(ATU_TUNE, 1)], timeout=0.05)
    ok = ctx.eq(f"{label}: ATU_TUNE = 1 starts a cycle (no rejection)", write_status(ans), [])
    ts = tx_state(h)
    ctx.eq(f"{label}: keyed, TX source 4 ATU, ATU_STATE 2", (None if ts is None else (ts & 1, (ts >> 8) & 0xF), atu_state(h)),
           ((1, 4), 2))
    return ok


def ended(ctx, h: Host, label: str, reason: int, atu: int, limit: int | None = None):
    ts = tx_state(h)
    want = (0, reason) if limit is None else (0, reason, limit)
    got = None if ts is None else ((ts & 1, (ts >> 16) & 0xFF) if limit is None else (ts & 1, (ts >> 16) & 0xFF, ts >> 24))
    ctx.eq(f"{label}: unkeyed, TX_STATE unkey reason {reason}" + (f", limit id {limit}" if limit else "") + " (0047)", got, want)
    ctx.eq(f"{label}: ATU_STATE {atu}", atu_state(h), atu)


@test("T-B10-47")
def t_b10_47(ctx: Ctx):
    with Host(ctx.net) as h:
        setup(ctx, h)
        sim = "atu.sim" in ctx.endpoint.get("fixtures", [])
        if sim:
            for result, atu in (("tuned", 3), ("failed", 4)):
                ctx.fixture("atu.sim", result=result, after_ms=800)
                if start(ctx, h, f"ends {result}"):
                    wait(h, 1.0)
                    ended(ctx, h, f"ends {result} by itself", 11, atu)
                wait(h, NOOP_S + 0.2)
            ctx.fixture("atu.sim", result="none")
        else:
            ctx.notes.append("tuned, failed and tune-time parts not run: no way to choose the outcome (atu.sim is a reference-model fixture)")
        # abort by ATU_TUNE = 0
        if start(ctx, h, "abort"):
            wait(h, 0.2)
            ans = h.write([(ATU_TUNE, 0)])
            ctx.eq("abort: ATU_TUNE = 0 accepted", write_status(ans), [])
            ended(ctx, h, "aborted by ATU_TUNE = 0", 1, 4)
        ctx.notes.append("abort by a stop request: the TCI command of part C C5.7, not on P2")
        if sim:
            wait(h, NOOP_S + 0.2)
            ctx.fixture("atu.sim", result="none", tune_time_ms=2000)
            if start(ctx, h, "tune time"):
                wait(h, 1.4)
                ts = tx_state(h)
                ctx.eq("tune time: still keyed before the limit", None if ts is None else ts & 1, 1)
                wait(h, 0.8)
                ended(ctx, h, "stopped by the ATU tune time", 4, 4, LIMIT_ATU_TIME)
            ctx.fixture("atu.sim", result="tuned", tune_time_ms=11000)
        h.stop()
