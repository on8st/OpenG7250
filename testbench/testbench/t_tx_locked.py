"""TX-locked state (r1.0 until calibrated; ON8ST 2026-11-20): every TX request is refused and
the HIS 85 safe state holds. Test ids T-B10-Lnn are added by the bench (catalogue), plus the
spec tests that run while locked. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import time

from .core import Ctx, test
from .p2 import *  # noqa: F401,F403

# Expected locked arm reject reason: the spec as written (B10.2 item 3) unless the run passes
# params {"lock_reason": n} (finding SF-01 proposes a NOT_CALIBRATED arm reason).
SPEC_LOCK_REASON = 11  # TX_NOT_IMPLEMENTED


def host(ctx):
    return Host(ctx.radio_ip, ctx.host_ip)


def session(h: Host, pa=True):
    h.start(pa=pa)
    got = h.recv(0.3, PORT_REG)
    pairs = sum((parse_regs(d) for _, d, _ in got), [])
    return pairs


def token_of(h: Host, pairs):
    t = value(pairs, TX_ARM_TOKEN)
    if t is None:
        t = value(h.write([(TX_ARM_TOKEN, 0)]), TX_ARM_TOKEN)
    return t


def safe(ctx: Ctx, label: str):
    """Check the HIS 85 safe-state items through hw.observe."""
    o = ctx.fixture("hw.observe")
    tr = o["tr_lines"]
    ctx.eq(f"{label}: T/R lines at RX levels (HIS 85)", (tr["PF4"], tr["PF6"], tr["PC7"], tr["PH11"]), (1, 0, 0, 0))
    a = o["spi1_a"]
    ctx.check(f"{label}: chain A bit 7 clear", not a.get("bit7_ever", False) and
              (a.get("last_frame") is None or a["last_frame"] & 0x80 == 0), str(a))
    ctx.check(f"{label}: PA bias never non-zero", not o["pa_dac"].get("bias_nonzero_ever", False))
    ctx.check(f"{label}: ATU latches never written (0032)", all(v == 0 for v in o["atu_latch_writes"].values()))


def lock_reason(ctx):
    return ctx.params.get("lock_reason", SPEC_LOCK_REASON)


@test("T-B10-L01", "T-B10-01")
def t_l01(ctx):
    with host(ctx) as h:
        rec = record_of(h.cap_request()[1:])
        tx = rec[0x20]
        ctx.eq("tx bit 0 TX hardware present", tx & 1, 1)
        ctx.eq("tx bit 1 TX implemented = 0 while locked", (tx >> 1) & 1, 0)
        ctx.eq("tx bits 2, 3 (arm and mode class required)", (tx >> 2) & 3, 3)
        ctx.eq("regmap_groups bit 2 (TX group present)", (rec[0x53] >> 2) & 1, 1)
        ctx.eq("streams bit 1 (TX IQ) follows tx bit 1", (rec[0x0C] >> 1) & 1, 0)


@test("T-B10-L02")
def t_l02(ctx):
    with host(ctx) as h:
        pairs = session(h)
        tok = token_of(h, pairs)
        h.write([(TX_MODE_CLASS, 1)])
        before = value(h.write([(TX_ARM_STATE, 0)]), TX_ARM_STATE)
        ans = h.write([(TX_ARM, tok ^ ARM_MAGIC)])
        st = value(ans, TX_ARM_STATE)
        if ctx.check("TX_ARM answered with TX_ARM_STATE", st is not None):
            ctx.eq("state stays 0", st & 0xF, 0)
            ctx.eq("generation unchanged", st >> 24, (before or 0) >> 24)
            ctx.eq("reject reason", ARM_REJECT.get((st >> 8) & 0xFF, (st >> 8) & 0xFF),
                   ARM_REJECT.get(lock_reason(ctx), lock_reason(ctx)))
        h.stop()


@test("T-B10-L04", "T-B10-L12")
def t_l04(ctx):
    with host(ctx) as h:
        pairs = session(h)
        tok = token_of(h, pairs)
        h.write([(TX_MODE_CLASS, 1)])
        h.write([(TX_ARM, tok ^ ARM_MAGIC)])
        keyed = False
        t_end = time.monotonic() + 1.0
        ptt = False
        status = []
        while time.monotonic() < t_end:
            ptt = not ptt
            h.keepalive(ptt=ptt, duc0_hz=14.2e6, drive=255)
            for p, d, _ in h.recv(0.05):
                if p == PORT_REG:
                    v = value(parse_regs(d), TX_STATE)
                    keyed |= v is not None and bool(v & 1)
                elif p == PORT_DDC:
                    status.append(d)
        ctx.check("no TX_STATE push shows a key", not keyed)
        ts = value(h.write([(TX_STATE, 0)]), TX_STATE)
        ctx.eq("TX_STATE bit 0", None if ts is None else ts & 1, 0)
        ctx.check("status byte 4 bits 0-2 are 0 (0030)", status and all(d[4] & 0b111 == 0 for d in status))
        if "hw.observe" in ctx.endpoint.get("fixtures", []):
            safe(ctx, "after PTT toggling")
        h.stop()


@test("T-B10-L05")
def t_l05(ctx):
    with host(ctx) as h:
        session(h)
        h.send(PORT_DUC, bytes(5) + bytes([0x80]) + bytes(3) + bytes([20]) + bytes(50))
        keyed = False
        for i in range(20):
            h.keepalive(cwx=0b011 if i % 2 else 0b101)
            for p, d, _ in h.recv(0.03, PORT_REG):
                v = value(parse_regs(d), TX_STATE)
                keyed |= v is not None and bool(v & 1)
        ctx.check("CWX keys nothing", not keyed)
        if "hw.observe" in ctx.endpoint.get("fixtures", []):
            safe(ctx, "after CWX")
        h.stop()


@test("T-B10-L06")
def t_l06(ctx):
    with host(ctx) as h:
        pairs = session(h)
        for label in ("unarmed", "after an arm attempt"):
            ans = h.write([(ATU_TUNE, 1)])
            ws = write_status(ans)
            ctx.check(f"{label}: NOT_CALIBRATED", any(a == ATU_TUNE and r == "NOT_CALIBRATED" for a, _, r in ws), str(ws))
            st = value(h.write([(ATU_STATE, 0)]), ATU_STATE)
            ctx.eq(f"{label}: ATU_STATE 0", st, 0)
            tok = token_of(h, pairs)
            h.write([(TX_MODE_CLASS, 1)])
            h.write([(TX_ARM, tok ^ ARM_MAGIC)])
        if "hw.observe" in ctx.endpoint.get("fixtures", []):
            safe(ctx, "after ATU_TUNE")
        h.stop()


@test("T-B10-L07")
def t_l07(ctx):
    with host(ctx) as h, host(ctx) as other:
        session(h)
        ans = h.write([(ATU_TUNE, 0)])
        ctx.eq("ATU_TUNE = 0 accepted (no rejection)", write_status(ans), [])
        ans = other.write([(ATU_TUNE, 0)])
        ctx.check("from a non-owner NOT_OWNER", any(r == "NOT_OWNER" for _, _, r in write_status(ans)))
        h.stop()


@test("T-B10-L08")
def t_l08(ctx):
    with host(ctx) as h:
        session(h)
        for _ in range(20):
            h.send(PORT_DUCIQ, bytes(1444))
        ss = value(h.write([(STREAM_STATUS, 0)]), STREAM_STATUS)
        ctx.check("STREAM_STATUS DUC0 not running", ss is not None and not (ss >> 8) & 0x80, hex(ss or 0))
        ctx.eq("DUC0 reason (proposal SF-04: 2 not offered)", None if ss is None else (ss >> 8) & 0x7F, 2)
        if "hw.observe" in ctx.endpoint.get("fixtures", []):
            safe(ctx, "after DUC I&Q")
        h.stop()


@test("T-B10-L09", "T-B10-08", "T-B10-19")
def t_l09(ctx):
    with host(ctx) as h:
        session(h)
        rej = lambda ans: [r for _, _, r in write_status(ans)]
        for v, ok in ((0, False), (1, True), (4, True), (5, False)):
            ans = h.write([(TX_MODE_CLASS, v)])
            ctx.eq(f"TX_MODE_CLASS {v}", rej(ans), [] if ok else ["BAD_VALUE"])
        for v, ok in ((99, False), (100, True), (500, True), (501, False)):
            ans = h.write([(KEYED_TIMEOUT, v)])
            ctx.eq(f"KEYED_TIMEOUT {v}", rej(ans), [] if ok else ["BAD_VALUE"])
        ans = h.write([(LIMIT_FWD_POWER, 10_000)])
        ctx.eq("LIMIT_FWD_POWER 10 W accepted, in force", value(ans, LIMIT_FWD_POWER), 10_000)
        ans = h.write([(LIMIT_FWD_POWER, 0)])
        ctx.eq("writing 0 returns the persistent value (100 W)", value(ans, LIMIT_FWD_POWER), 100_000)
        ans = h.write([(LIMIT_FWD_POWER, 200_000)])
        ctx.eq("looser than persistent", rej(ans), ["LOOSER_LIMIT"])
        if "hw.observe" in ctx.endpoint.get("fixtures", []):
            safe(ctx, "after TX register writes")
        h.stop()
        time.sleep(0.05)
        h.start()
        h.drain(0.1)
        ctx.eq("TX_MODE_CLASS back to 0 at session end", value(h.write([(TX_STATE, 0), (TX_MODE_CLASS, 9)]), TX_MODE_CLASS), 0)
        ctx.eq("KEYED_TIMEOUT back to 500", value(h.write([(KEYED_TIMEOUT, 1)]), KEYED_TIMEOUT), 500)
        h.stop()


@test("T-B10-L11")
def t_l11(ctx):
    ctx.need("hw.observe")
    with host(ctx) as a, host(ctx) as b:
        safe(ctx, "idle")
        pairs = session(a)
        safe(ctx, "session")
        tok = token_of(a, pairs)
        a.write([(TX_MODE_CLASS, 1), (TX_ARM, tok ^ ARM_MAGIC)])
        t_end = time.monotonic() + 2.0
        while time.monotonic() < t_end:
            a.keepalive(ptt=True, drive=255)
            time.sleep(0.05)
        safe(ctx, "PTT held 2 s")
        b.keepalive(ptt=True)
        time.sleep(0.05)
        safe(ctx, "takeover")
        time.sleep(1.3)
        safe(ctx, "watchdog standby")
        if "power.cycle" in ctx.endpoint.get("fixtures", []):
            ctx.fixture("power.cycle")
            safe(ctx, "power-on")
