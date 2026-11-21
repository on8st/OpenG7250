"""TX frozen until the calibration set is complete (r1.0; Annex P freeze rule, A1.12, B10
intro; decisions 0035, 0036, 0039): every arm is rejected with NOT_CALIBRATED, no TX request
keys, the HIS 85 safe state holds. Spec tests T-B10-03, T-B10-41, T-B10-42 and the bench tests
T-B10-L01, L07, L09, L11. GPL-3.0-or-later, (c) 2026 ON8ST."""
from __future__ import annotations

import time

from .core import Ctx, Skip, test
from .p2 import *  # noqa: F401,F403

NOT_CALIBRATED_ARM = 12  # Table B8-3 arm reject reason
STREAM_TX_NOT_CALIBRATED = 14  # Table B8-3 stream reason


def host(ctx):
    return Host(ctx.net)


def session(h: Host, pa=True, wd=True):
    h.start(pa=pa, wd=wd)
    got = h.recv(0.3, PORT_REG)
    return sum((parse_regs(d) for _, d, _ in got), [])


def token_of(ctx, h: Host, pairs):
    return need_token(ctx, h, pairs)


def current_token(h: Host):
    return value(h.write([(TX_ARM_TOKEN, 0)]), TX_ARM_TOKEN)


def status_detail(status: list) -> str:
    """What the High Priority status packets showed (B9.2 byte 4)."""
    if not status:
        return "no High Priority status packet from port 1025 received"
    bits = sorted({d[4] & 0b111 for d in status})
    return f"{len(status)} status packets, byte 4 bits 0-2 seen: {bits}"


def arm_reason(st):
    return None if st is None else ARM_REJECT.get((st >> 8) & 0xFF, (st >> 8) & 0xFF)


def has_obs(ctx):
    return "hw.observe" in ctx.endpoint.get("fixtures", [])


def safe(ctx: Ctx, label: str):
    """Check the HIS 85 safe-state items through hw.observe (and the trace, where the runner
    gives a trace file). Pin levels count from 20 us after the boot stage's first instruction
    (A3.6; 0045): a snapshot taken before that point is not judged."""
    from .trace import check_pins, check_safe, last_boot_t_us, load, BOOT_MARGIN_US
    events = load(ctx.trace_file) if ctx.trace_file else None
    boot_t = last_boot_t_us(events) if events else None
    o = ctx.fixture("hw.observe")
    deadline = time.monotonic() + 2.0
    while boot_t is not None and o.get("t_us", 0) < boot_t + BOOT_MARGIN_US and time.monotonic() < deadline:
        time.sleep(0.05)
        o = ctx.fixture("hw.observe")
    if boot_t is not None and o.get("t_us", 0) < boot_t + BOOT_MARGIN_US:
        ctx.notes.append(f"{label}: snapshot at t_us {o.get('t_us')} is before boot + 20 us, not judged")
    elif "pins" in o:  # v1.4: mode, pull and level ("z", null) per watched pin
        for name, ok, detail in check_pins(o["pins"]):
            ctx.check(f"{label}: {name}", ok, detail)
    else:
        tr = o["tr_lines"]
        ctx.eq(f"{label}: T/R lines at RX levels (HIS 85)", (tr["PF4"], tr["PF6"], tr["PC7"], tr["PH11"]), (1, 0, 0, 0))
    a = o["spi1_a"]
    ctx.check(f"{label}: chain A bit 7 clear", not a.get("bit7_ever", False) and
              (a.get("last_frame") is None or a["last_frame"] & 0x80 == 0), str(a))
    dac = o["pa_dac"]
    ch = dac.get("ch")
    bias_ok = not dac.get("bias_nonzero_ever", False) and (ch is None or all(ch[i] in (None, 0) for i in (1, 6, 7)))
    ctx.check(f"{label}: PA bias channels 2, 7, 8 at 0 or never written", bias_ok, str(dac))
    ctx.check(f"{label}: ATU latches never written (0032)", all(v == 0 for v in o["atu_latch_writes"].values()),
              str(o["atu_latch_writes"]))
    if "hw.trace" in ctx.endpoint.get("fixtures", []):
        # v1.3: hw.trace start returns the live trace file (flushed per event); read it at
        # this checkpoint, all events since the test start. Without a file (the reference
        # model) the snapshot checks above are all there is.
        if ctx.trace_file:
            events = load(ctx.trace_file)  # again: the events up to this checkpoint
            for name, ok, detail in check_safe(events, since=ctx.trace_since, start_t_us=ctx.trace_start_t_us):
                ctx.check(f"{label} (trace since {ctx.trace_since}): {name}", ok, detail)


def record(ctx, h: Host) -> dict:
    return need_record(ctx, h)


@test("T-B10-L01")
def t_l01(ctx):
    with host(ctx) as h:
        rec = record(ctx, h)
        tx = rec[0x20]
        ctx.eq("tx bit 0 TX hardware present", tx & 1, 1)
        ctx.eq("tx bit 1 TX implemented (0036)", (tx >> 1) & 1, 1)
        ctx.eq("tx bits 2, 3 (arm and mode class required)", (tx >> 2) & 3, 3)
        ctx.eq("tx bit 4 = 0 until calibrated (Annex P)", (tx >> 4) & 1, 0)
        ctx.eq("streams bit 1 (TX IQ) follows tx bit 1", (rec[0x0C] >> 1) & 1, 1)
        ctx.eq("regmap_groups bits 0-5 set, 6-7 clear (0035)", rec[0x53] & 0xFF, 0x3F)
        ctx.eq("cal_info bits 0-4 = 0 until calibrated (0039)", rec[0x57] & 0x1F, 0)


@test("T-B10-03")
def t_b10_03(ctx):
    """The reasons reachable with TX frozen, and their order (B10.2 item 3)."""
    with host(ctx) as h:
        rec = record(ctx, h)
        if (rec.get(0x20, 0) >> 4) & 1:
            ctx.notes.append("TX calibrated here: NOT_CALIBRATED rows not checked")
        # 1. watchdog bit clear, no mode class, PTT clear: WATCHDOG_DISABLED before NO_MODE_CLASS
        pairs = session(h, wd=False)
        tok = token_of(ctx, h, pairs)
        st = value(h.write([(TX_ARM, tok ^ 0x1234)]), TX_ARM_STATE)
        ctx.eq("wrong value: BAD_TOKEN first", arm_reason(st), "BAD_TOKEN")
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        ctx.eq("watchdog bit clear: WATCHDOG_DISABLED before PTT, mode class, calibration", arm_reason(st), "WATCHDOG_DISABLED")
        h.stop()
    with host(ctx) as h:
        pairs = session(h)
        tok = token_of(ctx, h, pairs)
        h.keepalive(ptt=True)
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        ctx.eq("PTT at 1: PTT_ACTIVE before NO_MODE_CLASS", arm_reason(st), "PTT_ACTIVE")
        h.keepalive(ptt=False)
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        ctx.eq("no mode class: NO_MODE_CLASS before NOT_CALIBRATED", arm_reason(st), "NO_MODE_CLASS")
        h.write([(TX_MODE_CLASS, 1)])
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        if not (rec.get(0x20, 0) >> 4) & 1:
            ctx.eq("only the calibration missing: NOT_CALIBRATED", (st >> 8) & 0xFF if st is not None else None, NOT_CALIBRATED_ARM)
        h.stop()
    ctx.notes.append("NOT_RUNNING (not set over P2), IN_FAULT, INHIBITED, LEVEL_NOT_ACTIVE, "
                     "MAINTENANCE: not reachable with TX frozen; host tests")


@test("T-B10-41")
def t_b10_41(ctx):
    with host(ctx) as h:
        rec = record(ctx, h)
        if not ctx.eq("record tx bit 1 = 1, bit 4 = 0", rec.get(0x20, 0) & 0b1_0010, 0b10):
            return
        pairs = session(h)
        tok = token_of(ctx, h, pairs)
        h.write([(TX_MODE_CLASS, 1)])
        before = value(h.write([(TX_ARM_STATE, 0)]), TX_ARM_STATE)
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        if ctx.check("TX_ARM answered with TX_ARM_STATE", st is not None, "no TX_ARM_STATE in the answer within 0.3 s" if st is None else hex(st)):
            ctx.eq("state stays 0", st & 0xF, 0)
            ctx.eq("generation unchanged", st >> 24, (before or 0) >> 24)
            ctx.eq("reject reason NOT_CALIBRATED (12)", (st >> 8) & 0xFF, NOT_CALIBRATED_ARM)
        for label in ("unarmed", "after the refused arm"):
            ws = write_status(h.write([(ATU_TUNE, 1)]))
            ctx.check(f"ATU_TUNE = 1 {label}: NOT_CALIBRATED", any(a == ATU_TUNE and r == "NOT_CALIBRATED" for a, _, r in ws), str(ws))
        ctx.eq("ATU_STATE 5 (unknown, Annex P)", value(h.write([(ATU_STATE, 0)]), ATU_STATE), 5)
        if has_obs(ctx):
            safe(ctx, "after the arm and ATU_TUNE")
        h.stop()
    ctx.notes.append("'with bit 4 = 1 the arm is accepted': T-A1-63 (calibration set, host)")


@test("T-B10-42")
def t_b10_42(ctx):
    with host(ctx) as h:
        pairs = session(h)
        tok = token_of(ctx, h, pairs)
        h.write([(TX_MODE_CLASS, 1)])
        h.write([(TX_ARM, tok ^ ARM_MAGIC)])
        for _ in range(20):
            h.send(PORT_DUCIQ, bytes(1444))
        ss = value(h.write([(STREAM_STATUS, 0)]), STREAM_STATUS)
        ctx.check("STREAM_STATUS DUC0 not running", ss is not None and not (ss >> 8) & 0x80, hex(ss or 0))
        ctx.eq("DUC0 reason 14 (TX not calibrated)", None if ss is None else (ss >> 8) & 0x7F, STREAM_TX_NOT_CALIBRATED)
        # PTT toggled with every other key condition met; then CWX with break-in
        keyed, status = False, []
        ptt = False
        for _ in range(16):
            ptt = not ptt
            h.keepalive(ptt=ptt, duc0_hz=14.2e6, drive=255)
            for p, d, _ in h.recv(0.05):
                if p == PORT_REG:
                    v = value(parse_regs(d), TX_STATE)
                    keyed |= v is not None and bool(v & 1)
                elif p == PORT_DDC:
                    status.append(d)
        h.send(PORT_DUC, bytes(5) + bytes([0x80]) + bytes(3) + bytes([20]) + bytes(50))
        for i in range(12):
            h.keepalive(cwx=0b011 if i % 2 else 0b101)
            for p, d, _ in h.recv(0.03, PORT_REG):
                v = value(parse_regs(d), TX_STATE)
                keyed |= v is not None and bool(v & 1)
        ctx.check("PTT and CWX key nothing (no TX_STATE push with bit 0)", not keyed, "a TX_STATE push showed bit 0 = 1" if keyed else "no key seen")
        ts = value(h.write([(TX_STATE, 0)]), TX_STATE)
        ctx.eq("TX_STATE bit 0 after the requests (not latched)", None if ts is None else ts & 1, 0)
        ctx.check("status byte 4 bits 0-2 are 0 (0030)", status and all(d[4] & 0b111 == 0 for d in status),
                  status_detail(status))
        if has_obs(ctx):
            safe(ctx, "after DUC I&Q, PTT and CWX")
        h.stop()
    ctx.notes.append("'discarded and counted': the GET_STATS counter name is not fixed by the spec")


@test("T-B10-L07")
def t_l07(ctx):
    with host(ctx) as h, host(ctx) as other:
        session(h)
        # A3.9b item 1: 0 from the owner is always accepted, a no-op with no cycle running;
        # its answer is the pair (address, value in force) of B8.4. ATU_STATE comes with the
        # answer only to a start that is a no-op, so it is read here as B8.4a says: a write
        # to the read-only register answers its value with READ_ONLY.
        ans = h.write([(ATU_TUNE, 0)])
        ctx.eq("ATU_TUNE = 0 accepted (no rejection)", write_status(ans), [])
        ctx.eq("ATU_TUNE = 0 answered with the value in force 0 (B8.4)", value(ans, ATU_TUNE), 0)
        ans = h.write([(ATU_STATE, 0)])
        ctx.eq("ATU_STATE read by a write: READ_ONLY (B8.4a)", [r for _, _, r in write_status(ans)], ["READ_ONLY"])
        st = value(ans, ATU_STATE)
        ctx.eq("ATU_STATE 5 (unknown) while P-ATU-1 is not set (A3.9b; 0032)", st, 5)
        if st == 5:
            ans = h.write([(ATU_TUNE, 1)])
            ctx.eq("ATU_TUNE = 1 rejected NOT_CALIBRATED before NOT_ARMED (A3.9b item 1)",
                   [r for _, _, r in write_status(ans)], ["NOT_CALIBRATED"])
            ctx.eq("the rejected start is not latched: ATU_TUNE in force 0", value(ans, ATU_TUNE), 0)
        ans = other.write([(ATU_TUNE, 0)])
        ctx.check("from a non-owner NOT_OWNER", any(r == "NOT_OWNER" for _, _, r in write_status(ans)), f"WRITE_STATUS seen: {write_status(ans)}")
        h.stop()


@test("T-B10-L13")
def t_l13(ctx):
    """TX frozen: TX_STATE bit 4 (PA disabled by the host) follows General byte 58 bit [0]: 1
    while it is 0, 0 once the owner sets it; cleared again at a takeover (A3.6a, Table A3-4).
    Bit 0 and the source [11:8] stay 0, also under a PTT edge (B10; Table B8-2)."""
    def tx_state(h, label):
        ts = value(h.write([(TX_STATE, 0)]), TX_STATE)
        if not ctx.check(f"{label}: TX_STATE read (READ_ONLY answer)", ts is not None,
                         "no TX_STATE in the answer" if ts is None else f"{ts:#010x}"):
            return None
        ctx.eq(f"{label}: bit 0 keyed", ts & 1, 0)
        ctx.eq(f"{label}: source [11:8]", (ts >> 8) & 0xF, 0)
        return ts

    with host(ctx) as a, host(ctx) as b:
        a.start(pa=False)
        a.drain(0.1)
        ts = tx_state(a, "byte 58 bit 0 = 0")
        if ts is not None:
            ctx.eq("byte 58 bit 0 = 0: bit 4 PA disabled = 1", (ts >> 4) & 1, 1)
        a.send(PORT_DISC, general(pa=True))
        a.keepalive()
        a.drain(0.05)
        ts = tx_state(a, "byte 58 bit 0 = 1")
        if ts is not None:
            ctx.eq("byte 58 bit 0 = 1: bit 4 = 0", (ts >> 4) & 1, 0)
        a.keepalive(ptt=True)
        a.drain(0.1)
        tx_state(a, "PTT held")
        a.keepalive(ptt=False)
        # same-host restart by a High Priority packet only: the General is carried over
        # without byte 38 bit 0 and byte 58 bit 0 (B5.4 restart item 1), so PA disabled again
        a.stop()
        a.keepalive()
        a.drain(0.1)
        ts = tx_state(a, "same-host restart")
        if ts is not None:
            ctx.eq("same-host restart: bit 4 = 1 (byte 58 bit 0 not carried over, B5.4)", (ts >> 4) & 1, 1)
        try:
            ctx.need("net.hosts")
        except Skip:
            ctx.notes.append("takeover part not run: fixture net.hosts not offered")
            a.stop()
            return
        b.keepalive()  # takeover with a High Priority packet only: no General from b
        time.sleep(0.05)
        b.drain(0.05)
        ts = tx_state(b, "after takeover")
        if ts is not None:
            ctx.eq("after takeover: bit 4 = 1 (PA enable cleared, A3.6a)", (ts >> 4) & 1, 1)
        b.stop()


@test("T-B10-L09", "T-B10-08", "T-B10-19")
def t_l09(ctx):
    with host(ctx) as h:
        session(h)
        rej = lambda ans: [r for _, _, r in write_status(ans)]
        for v, ok in ((0, False), (1, True), (4, True), (5, False)):
            ctx.eq(f"TX_MODE_CLASS {v}", rej(h.write([(TX_MODE_CLASS, v)])), [] if ok else ["BAD_VALUE"])
        for v, ok in ((99, False), (100, True), (500, True), (501, False)):
            ctx.eq(f"KEYED_TIMEOUT {v}", rej(h.write([(KEYED_TIMEOUT, v)])), [] if ok else ["BAD_VALUE"])
        ans = h.write([(LIMIT_FWD_POWER, 10_000)])
        ctx.eq("LIMIT_FWD_POWER 10 W accepted, in force", value(ans, LIMIT_FWD_POWER), 10_000)
        ans = h.write([(LIMIT_FWD_POWER, 0)])
        ctx.eq("writing 0 returns the persistent value (100 W)", value(ans, LIMIT_FWD_POWER), 100_000)
        ctx.eq("looser than persistent", rej(h.write([(LIMIT_FWD_POWER, 200_000)])), ["LOOSER_LIMIT"])
        ctx.eq("LIMIT_TX_TIME looser than 300 000 ms", rej(h.write([(LIMIT_TX_TIME, 300_001)])), ["LOOSER_LIMIT"])
        if has_obs(ctx):
            safe(ctx, "after TX register writes")
        h.stop()
        h.start()
        h.drain(0.1)
        ctx.eq("TX_MODE_CLASS back to 0 at session end", value(h.write([(TX_MODE_CLASS, 9)]), TX_MODE_CLASS), 0)
        ctx.eq("KEYED_TIMEOUT back to 500", value(h.write([(KEYED_TIMEOUT, 1)]), KEYED_TIMEOUT), 500)
        h.stop()


@test("T-B10-L11", trace_since="boot")
def t_l11(ctx):
    ctx.need("hw.observe")
    with host(ctx) as a, host(ctx) as b:
        safe(ctx, "idle")
        pairs = session(a)
        safe(ctx, "session")
        tok = token_of(ctx, a, pairs)
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
            wait_reachable(ctx)


@test("T-A3-06", trace_since="boot")
def t_a3_06(ctx):
    """After power-on (from 20 us after the boot stage's first instruction, 0045) and after every
    session end (stop, watchdog standby, takeover) the state of A3.6 holds: the HIS 85 RF control
    pins at their safe levels; TX_STATE unkeyed; TX_MODE_CLASS 0 and KEYED_TIMEOUT 500 at the next
    session (A3.6, A3.6a)."""
    ctx.need("hw.observe")
    if "power.cycle" in ctx.endpoint.get("fixtures", []):
        ctx.fixture("power.cycle")
        time.sleep(1.0)
        safe(ctx, "after power-on")
        wait_reachable(ctx)
    else:
        ctx.notes.append("power-on part not run: fixture power.cycle not offered")
    with host(ctx) as a, host(ctx) as b:
        # TX_MODE_CLASS, KEYED_TIMEOUT and TX_STATE are group 2 registers, present "from M3"
        # and announced by regmap_groups bit 2 (Table B8-1, B8.3). Without that bit they are
        # absent by the spec and their part of A3.6 cannot be read: only the pins are judged.
        # With bit 2 set they are judged whatever record tx bit 1 says: A3.6 sets them at every
        # session end, with no exception for a build without TX.
        groups = record(ctx, a)[0x53]
        group2 = bool(groups & 0x4)
        if not group2:
            ctx.notes.append(f"TX register part not run: regmap_groups {groups:#x} without bit 2 "
                             "(group 2 absent before M3, Table B8-1); RF control pins judged only")
        for end in ("stop", "watchdog standby", "takeover"):
            session(a)
            if group2:
                a.write([(TX_MODE_CLASS, 2), (KEYED_TIMEOUT, 200)])
            if end == "stop":
                a.stop()
            elif end == "watchdog standby":
                time.sleep(1.3)
            else:
                b.keepalive()
                time.sleep(0.05)
            safe(ctx, f"after {end}")
            owner = b if end == "takeover" else a
            if not group2:
                owner.stop()
                continue
            owner.start()
            owner.drain(0.1)
            ctx.eq(f"after {end}: TX_MODE_CLASS back to 0", value(owner.write([(TX_MODE_CLASS, 9)]), TX_MODE_CLASS), 0)
            ctx.eq(f"after {end}: KEYED_TIMEOUT back to 500", value(owner.write([(KEYED_TIMEOUT, 1)]), KEYED_TIMEOUT), 500)
            ts = value(owner.write([(TX_STATE, 0)]), TX_STATE)
            ctx.eq(f"after {end}: TX_STATE unkeyed", None if ts is None else ts & 1, 0)
            owner.stop()


@test("T-B6-L01", trace_pins=["PE%d" % i for i in range(11)])
def t_b6_l01(ctx):
    """[HIS] HIS 80 on the FPGA bus during a session: no command 0x05, 0x06, 0x09 or 0x0A; a 0x01
    frame only with the RF section in the safe state (B6.1; 0042); D0-D7 back to outputs only
    after STB is low. Needs the hw.trace file with PE0-PE10 (emulator)."""
    from .trace import check_fpga, load
    if not ctx.trace_file:
        from .core import Skip
        raise Skip("needs an hw.trace file with PE0-PE10 (emulator)")
    with host(ctx) as h:
        h.start(192)
        h.recv_alive(1.0)
        h.keepalive(ddc0_hz=14.2e6)
        h.recv_alive(0.5)
        h.stop()
    events = load(ctx.trace_file)
    seen = {e.get("pin") for e in events if e.get("kind") == "gpio"}
    ctx.check("the trace watches the FPGA bus pins", {"PE8", "PE9"} <= seen, f"pins seen: {sorted(p for p in seen if p and p.startswith('PE'))}")
    for name, ok, detail in check_fpga(events):
        ctx.check(name, ok, detail)


@test("T-A3-58")
def t_a3_58(ctx):
    """v1.0 (0030): toggling the local PTT (PH6) and KEY contacts (PH3, PH4) keys nothing; High
    Priority status byte 4 bits [0]-[2] stay 0. Levels are electrical; both levels are driven,
    so the active level ([HIS] HIS 85 does not give it) does not matter."""
    ctx.need("hw.input")

    def toggle(h: Host, label: str):
        keyed, status = False, []
        for pin in ("PH6", "PH3", "PH4"):
            for level in (0, 1, 0, 1):
                ctx.fixture("hw.input", pin=pin, level=level)
                h.keepalive()
                for p, d, _ in h.recv(0.03):
                    if p == PORT_REG:
                        v = value(parse_regs(d), TX_STATE)
                        keyed |= v is not None and bool(v & 1)
                    elif p == PORT_DDC:
                        status.append(d)
        ctx.check(f"{label}: nothing keys", not keyed, "a TX_STATE push showed bit 0 = 1" if keyed else "no key seen")
        ctx.check(f"{label}: status byte 4 bits 0-2 stay 0", status and all(d[4] & 0b111 == 0 for d in status),
                  status_detail(status))

    with host(ctx) as h:
        pairs = session(h)
        toggle(h, "unarmed")
        h.send(PORT_DUC, bytes(5) + bytes([0x82]) + bytes(54))  # hardware CW selected, break-in
        toggle(h, "unarmed, hardware CW selected")
        rec = record(ctx, h)
        if (rec.get(0x20, 0) >> 4) & 1:
            tok = token_of(ctx, h, pairs)
            h.write([(TX_MODE_CLASS, 2)])
            st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
            if ctx.eq("armed", None if st is None else st & 0xF, 1):
                toggle(h, "armed, hardware CW selected")
        else:
            ctx.notes.append("armed part not run: TX frozen (record tx bit 4 = 0)")
        if has_obs(ctx):
            safe(ctx, "after the inputs")
        h.stop()
        for pin in ("PH6", "PH3", "PH4"):
            ctx.fixture("hw.input", pin=pin, level=0)


@test("T-B4-18")
def t_b4_18(ctx):
    """r1.0 without a complete calibration set: record tx bits 0-3 = 1, bit 4 = 0; an arm that meets
    every other condition gets NOT_CALIBRATED (12), not TX_NOT_IMPLEMENTED (11) (B4.8, B10.2; 0036)."""
    with host(ctx) as h:
        tx = record(ctx, h)[0x20]
        ctx.eq("record tx bits 0-3", tx & 0xF, 0xF)
        if (tx >> 4) & 1:
            ctx.notes.append("TX calibrated here (tx bit 4 = 1): the arm part needs a radio without a complete set")
            return
        ctx.eq("record tx bit 4 (no complete calibration set)", (tx >> 4) & 1, 0)
        pairs = session(h)
        tok = token_of(ctx, h, pairs)
        h.write([(TX_MODE_CLASS, 1)])
        st = value(h.write([(TX_ARM, tok ^ ARM_MAGIC)]), TX_ARM_STATE)
        ctx.eq("arm reject reason 12 NOT_CALIBRATED, not 11", None if st is None else (st >> 8) & 0xFF, NOT_CALIBRATED_ARM)
        h.stop()
