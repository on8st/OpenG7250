"""Reading an hw.trace event list and an hw.observe snapshot (testlist-format v1.4 section 4)
against the HIS 85 safe state ([HIS] at d07627e, gate review 7) and HIS 70 (idle link).

Pin levels (v1.4): 0 or 1 when something drives the pin (an output, the pull of an input, or
hw.input); "z" when nothing drives it; None when the emulator cannot tell (an alternate-function
pin it does not model). A v1.3 event has no "mode" and an integer level.

What each pin must do comes from the wording of the HIS, pin by pin:
- "held" or "keep" at a level: anything else fails, "z" (not held) and None (unknown) included;
- "never" a level: only that level fails; "z" passes, None is noted but cannot fail.

Only the HIS 85 safe-state table and pin map, and HIS 70, are used; unknown event kinds are
ignored. GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import json
from pathlib import Path

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
LEGACY_TR = {"PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0}


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
    if v in (0, 1, "z"):
        return v
    return None


def _describe(v) -> str:
    return {0: "driven low (0)", 1: "driven high (1)", "z": "not driven (z)", None: "unknown (null)"}[v]


def check_safe(events: list[dict]) -> list[tuple[str, bool, str]]:
    """(check name, ok, detail) for the trace of a test in which TX stays frozen."""
    viol: dict[str, list] = {}
    notes: dict[str, list] = {}
    oe_free = {"PA10": False, "PH10": False}
    last_spi1 = None
    bad_frames = []
    for e in events:
        k = e.get("kind")
        if k == "spi" and e.get("bus") == "SPI1":
            last_spi1 = e
            continue
        if k != "gpio":
            continue
        pin, v = e.get("pin"), _lvl(e)
        if pin in (CHAIN_A_LATCH, CHAIN_B_LATCH) and last_spi1 is not None:
            # the last SPI1 transfer before an edge of a latch line is the latched frame (any
            # edge: which one latches is not in the HIS, the chip type is HYPOTHESIS there)
            data = bytes.fromhex(last_spi1.get("data", ""))
            if pin == CHAIN_A_LATCH and len(data) == 1:
                if data[0] & 0x80:
                    bad_frames.append(last_spi1)
                else:
                    oe_free["PA10"] = True
            elif pin == CHAIN_B_LATCH and len(data) == 3:
                oe_free["PH10"] = True
            last_spi1 = None
        if pin in HOLD and v != HOLD[pin][0]:
            viol.setdefault(pin, []).append((e, v))
        if pin in OE and not oe_free[pin] and v != 1:
            viol.setdefault(pin, []).append((e, v))
        if pin in NEVER:
            if v == NEVER[pin][0]:
                viol.setdefault(pin, []).append((e, v))
            elif v is None:
                notes.setdefault(pin, []).append(e)
    res = [("chain A bit 7 clear in every latched frame (HIS 85)", not bad_frames,
            "" if not bad_frames else json.dumps(bad_frames[0]))]
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
    e, v = bad[0]
    more = f" (+{len(bad) - 1} more)" if len(bad) > 1 else ""
    return (f"{pin}: {why}", False, f"{_describe(v)} at seq {e.get('seq')}, t_us {e.get('t_us')}, "
                                    f"mode {e.get('mode', 'n/a')}{more}")


def check_pins(pins: dict) -> list[tuple[str, bool, str]]:
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
