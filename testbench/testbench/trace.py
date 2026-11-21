"""Reading an hw.trace event list and an hw.observe snapshot (testlist-format v1.4 section 4)
against the safe state of A3.6 ([HIS] HIS 85 and HIS 70 at 5292ba8) and the FPGA bus rules of
[HIS] HIS 80.

Pin levels (v1.4): 0 or 1 when something drives the pin (an output, the pull of an input, or
hw.input); "z" when nothing drives it; None when the emulator cannot tell (an alternate-function
pin it does not model). A v1.3 event has no "mode" and an integer level.

From when (spec A3.6, decision 0045): the RF control pins must be at their safe levels from
20 us after the boot stage's first instruction on. A trace starts with the replayed boot event
and boot-phase events. Two ways to judge:
- since="boot": from boot.t_us + 20 us on, at every boot in the trace (T-A3-06, T-B10-L11);
- since="start": from the trace start on (the state during the test), so a boot-phase event
  is never judged twice; a boot during the trace is judged from its boot.t_us + 20 us.
Events before the point are outside the spec (hardware and vendor bootloader) and fail nothing.

What each pin must do comes from the wording of the HIS, pin by pin:
- "held" or "keep" at a level: anything else fails, "z" (not held) and None (unknown) included;
- "never" a level: only that level fails; "z" passes, None is noted but cannot fail.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import json
from pathlib import Path

BOOT_MARGIN_US = 20  # A3.6, 0045
REPLAY_WINDOW_US = 10_000  # boot-phase events replayed at a trace start lie within 10 ms of the boot

# Pins that the HIS wants held at a level: pin -> (level, why)
HOLD = {
    "PF4": (1, "T/R line at its RX level (HIS 85)"),
    "PF6": (0, "T/R line at its RX level (HIS 85)"),
    "PC7": (0, "T/R line at its RX level (HIS 85)"),
    "PH11": (0, "T/R line at its RX level (HIS 85)"),
    "PD13": (1, "ATU strobe driven to its inactive level, never pulsed (HIS 85; 0032)"),
    "PG2": (1, "ATU strobe driven to its inactive level, never pulsed (HIS 85; 0032)"),
    "PG5": (1, "ATU strobe driven to its inactive level, never pulsed (HIS 85; 0032)"),
    "PD12": (0, "PD12 kept low (HIS 85)"),
    # codec RESET is active low (TI AIC3204 datasheet, public): "held in reset" = low
    "PB10": (0, "codec held in reset (HIS 85, HIS 50; 0031)"),
}
# Output enables: inactive (high) until their chain has latched a frame (HIS 85)
OE = {"PA10": "chain A output enable inactive (high) until a frame with bit 7 clear is latched (HIS 85)",
      "PH10": "chain B output enable inactive (high) until a frame is latched (HIS 85)"}
# Pins that must never be at a level: pin -> (forbidden level, why)
NEVER = {"PD5": (0, "USART2 TX never low: idle high or high-impedance (HIS 70 item 2; 0023)")}

CHAIN_A_LATCH, CHAIN_B_LATCH = "PA11", "PH12"

# FPGA bus ([HIS] HIS 80)
FPGA_D = [f"PE{i}" for i in range(8)]
FPGA_CMD, FPGA_STB, FPGA_DRDY = "PE8", "PE9", "PE10"
FPGA_PINS = FPGA_D + [FPGA_CMD, FPGA_STB, FPGA_DRDY]
FORBIDDEN_CMDS = {0x05, 0x06, 0x09, 0x0A}  # "MUST NOT be sent"


def load(path: str | Path) -> list[dict]:
    out = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            out.append(json.loads(line))
    return sorted(out, key=lambda e: e.get("seq", 0))


def _lvl(e: dict):
    """The level of a gpio event or snapshot entry: 0, 1, "z" or None."""
    v = e.get("level")
    return v if v in (0, 1, "z") else None


def _describe(v) -> str:
    return {0: "driven low (0)", 1: "driven high (1)", "z": "not driven (z)", None: "unknown (null)"}[v]


def _t(e: dict) -> int:
    return e.get("t_us", e.get("seq", 0))


def _judged_from(events: list[dict], since: str, start_t_us: int | None) -> list[tuple]:
    """For each event: (time from which its boot period is judged, judge the carried state at
    that time). None: no boot seen, judge every event. At boot + 20 us the state is judged, a pin
    need not change; from the start snapshot on, the snapshot gives every pin itself."""
    out, cur, first_boot = [], (None, False), True
    for e in events:
        if e.get("kind") == "boot":
            if since == "boot" or not first_boot:
                cur = (_t(e) + BOOT_MARGIN_US, True)
            else:  # the replayed boot at a trace start: judge from the start snapshot on
                cur = (start_t_us if start_t_us is not None else _t(e) + REPLAY_WINDOW_US, False)
            first_boot = False
        out.append(cur)
    return out


def check_safe(events: list[dict], since: str = "start", start_t_us: int | None = None):
    """(check name, ok, detail) for the trace of a test in which TX stays frozen."""
    viol: dict[str, list] = {}
    notes: dict[str, list] = {}
    state: dict[str, dict] = {}
    oe_free = {"PA10": False, "PH10": False}
    last_spi1, bad_frames = None, []
    judged = _judged_from(events, since, start_t_us)
    crossed = False

    def judge(pin, e, v, at=""):
        if pin in HOLD and v != HOLD[pin][0]:
            viol.setdefault(pin, []).append((e, v, at))
        if pin in OE and not oe_free[pin] and v != 1:
            viol.setdefault(pin, []).append((e, v, at))
        if pin in NEVER:
            if v == NEVER[pin][0]:
                viol.setdefault(pin, []).append((e, v, at))
            elif v is None:
                notes.setdefault(pin, []).append(e)

    for e, (frm, judge_state) in zip(events, judged):
        k = e.get("kind")
        if k in ("boot", "reset"):
            state.clear()
            oe_free = {"PA10": False, "PH10": False}
            last_spi1, crossed = None, False
            continue
        active = frm is None or _t(e) >= frm
        if active and not crossed and frm is not None:
            if judge_state:  # boot + 20 us: every pin seen so far must already be right
                for pin, pe in state.items():
                    judge(pin, pe, _lvl(pe), at=f" (state at boot + 20 us, t_us {frm})")
            crossed = True
        if k == "spi" and e.get("bus") == "SPI1":
            last_spi1 = e
            continue
        if k != "gpio":
            continue
        pin, v = e.get("pin"), _lvl(e)
        state[pin] = e
        if pin in (CHAIN_A_LATCH, CHAIN_B_LATCH) and last_spi1 is not None:
            # the last SPI1 transfer before an edge of a latch line is the latched frame (any
            # edge: which one latches is not in the HIS, the chip type is HYPOTHESIS there)
            data = bytes.fromhex(last_spi1.get("data", ""))
            if pin == CHAIN_A_LATCH and len(data) == 1:
                if data[0] & 0x80:
                    if active:
                        bad_frames.append(last_spi1)
                else:
                    oe_free["PA10"] = True
            elif pin == CHAIN_B_LATCH and len(data) == 3:
                oe_free["PH10"] = True
            last_spi1 = None
        if active:
            judge(pin, e, v)
    res = [("chain A bit 7 clear in every latched frame (HIS 85)", not bad_frames,
            "none latched with bit 7" if not bad_frames else json.dumps(bad_frames[0]))]
    for pin, (want, why) in HOLD.items():
        res.append(_result(pin, why, viol.get(pin), f"held at {want}"))
    for pin, why in OE.items():
        res.append(_result(pin, why, viol.get(pin), "held at 1 until its chain latched a frame"))
    for pin, (_, why) in NEVER.items():
        r = _result(pin, why, viol.get(pin), "never at the forbidden level")
        if notes.get(pin) and r[1]:
            r = (r[0], True, f"level unknown in {len(notes[pin])} event(s), e.g. {json.dumps(notes[pin][0])}")
        res.append(r)
    return res


def _result(pin, why, bad, ok_text):
    if not bad:
        return (f"{pin}: {why}", True, ok_text)
    e, v, at = bad[0]
    more = f" (+{len(bad) - 1} more)" if len(bad) > 1 else ""
    return (f"{pin}: {why}", False, f"{_describe(v)} at seq {e.get('seq')}, t_us {e.get('t_us')}, "
                                    f"mode {e.get('mode', 'n/a')}{at}{more}")


def check_pins(pins: dict):
    """The same rules on an hw.observe v1.4 `pins` map {pin: {mode, pull, level}} at one instant.
    Output enables are not judged here: whether their chain has latched a frame is in the trace."""
    res = []
    for pin, (want, why) in HOLD.items():
        if pin in pins:
            v = _lvl(pins[pin])
            res.append((f"{pin}: {why}", v == want, f"{_describe(v)}, mode {pins[pin].get('mode', 'n/a')}"))
    for pin, (bad, why) in NEVER.items():
        if pin in pins:
            v = _lvl(pins[pin])
            res.append((f"{pin}: {why}", v != bad, f"{_describe(v)}, mode {pins[pin].get('mode', 'n/a')}"))
    return res


def last_boot_t_us(events: list[dict]) -> int | None:
    t = None
    for e in events:
        if e.get("kind") == "boot":
            t = _t(e)
    return t


def check_fpga(events: list[dict]):
    """[HIS] HIS 80: no forbidden command; a 0x01 frame only with the RF section in the safe
    state; D0-D7 switched back to outputs only after STB is low. A command byte is the level of
    D0-D7 when STB rises while CMD is high (both are active-high pulses, MEDIUM)."""
    lv: dict[str, object] = {}
    mode: dict[str, str] = {}
    forbidden, unsafe_01, early_out, unknown_cmd = [], [], [], []
    cmds = 0
    for e in events:
        if e.get("kind") in ("boot", "reset"):
            lv.clear()
            mode.clear()
            continue
        if e.get("kind") != "gpio":
            continue
        pin, v, m = e.get("pin"), _lvl(e), e.get("mode")
        prev_mode = mode.get(pin)
        if m:
            mode[pin] = m
        if pin in FPGA_D and m == "output" and prev_mode == "input" and lv.get(FPGA_STB) == 1:
            early_out.append(e)
        rising = pin == FPGA_STB and v == 1 and lv.get(FPGA_STB) == 0
        lv[pin] = v
        if rising and lv.get(FPGA_CMD) == 1:
            bits = [lv.get(d) for d in FPGA_D]
            if any(b not in (0, 1) for b in bits):
                unknown_cmd.append(e)
                continue
            cmd = sum(b << i for i, b in enumerate(bits))
            cmds += 1
            if cmd in FORBIDDEN_CMDS:
                forbidden.append((e, cmd))
            if cmd == 0x01:
                bad = [p for p, (want, _) in HOLD.items() if p in lv and lv[p] != want]
                if bad:
                    unsafe_01.append((e, bad))
    return [
        ("FPGA: no command 0x05, 0x06, 0x09 or 0x0A (HIS 80)", not forbidden,
         f"{cmds} command bytes seen" if not forbidden else f"command {forbidden[0][1]:#04x} at seq {forbidden[0][0].get('seq')}"),
        ("FPGA: a 0x01 frame only with the RF section in the safe state (HIS 80; B6.1)", not unsafe_01,
         "ok" if not unsafe_01 else f"0x01 at seq {unsafe_01[0][0].get('seq')} with {unsafe_01[0][1]} not at their safe levels"),
        ("FPGA: D0-D7 back to outputs only after STB is low (HIS 80)", not early_out,
         "ok" if not early_out else f"{early_out[0].get('pin')} to output with STB high at seq {early_out[0].get('seq')}"),
        ("FPGA: command bytes readable (D0-D7 driven at the strobe)", not unknown_cmd,
         "ok" if not unknown_cmd else f"{len(unknown_cmd)} command strobe(s) with D0-D7 not driven, first seq {unknown_cmd[0].get('seq')}"),
    ]
