"""Reading an hw.trace event list (testlist-format v1.2 section 4) against the HIS 85 safe
state. Only the HIS 85 safe-state table and pin map are used; unknown event kinds are ignored.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

import json
from pathlib import Path

# [HIS] HIS 85, "Safe state at reset": RX levels of the T/R lines
TR_RX = {"PF4": 1, "PF6": 0, "PC7": 0, "PH11": 0}
CHAIN_A_LATCH = "PA11"  # SPI1 chain A (TX low-pass bank + T/R), latch
CHAIN_A_OE = "PA10"  # active low
ATU_LATCHES = ("PD13", "PG2", "PG5")  # the CS line is the latch
DAC_RESET = "PB14"  # active low
CODEC_RESET = "PB10"


def load(path: str | Path) -> list[dict]:
    out = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            out.append(json.loads(line))
    return sorted(out, key=lambda e: e.get("seq", 0))


def check_safe(events: list[dict]) -> list[tuple[str, bool, str]]:
    """(check name, ok, detail) for the trace of a test in which TX stays frozen."""
    res = []
    tr_bad = [e for e in events if e.get("kind") == "gpio" and e.get("pin") in TR_RX
              and e.get("level") != TR_RX[e["pin"]]]
    res.append(("T/R lines never leave their RX level", not tr_bad, _first(tr_bad)))
    # chain A (8 bit): the last 1-byte SPI1 transfer before an edge of its latch line is taken as
    # the latched frame; bit 7 must be clear. Any edge counts: which edge latches is not in the
    # HIS (the chip type is HYPOTHESIS there).
    last_spi1, bad_frames = None, []
    for e in events:
        k = e.get("kind")
        if k == "spi" and e.get("bus") == "SPI1":
            last_spi1 = e
        elif k == "gpio" and e.get("pin") == CHAIN_A_LATCH and last_spi1 is not None:
            data = bytes.fromhex(last_spi1.get("data", ""))
            if len(data) == 1 and data[0] & 0x80:
                bad_frames.append(last_spi1)
            last_spi1 = None
    res.append(("chain A bit 7 clear in every latched frame", not bad_frames, _first(bad_frames)))
    atu = [e for e in events if e.get("kind") == "gpio" and e.get("pin") in ATU_LATCHES]
    res.append(("ATU latches never toggled (0032)", not atu, _first(atu)))
    codec = [e for e in events if e.get("kind") == "gpio" and e.get("pin") == CODEC_RESET and e.get("level") == 1]
    res.append(("codec held in reset (0031)", not codec, _first(codec)))
    # PA DAC: the word layout per channel is the BU2506 datasheet's, not in the HIS; the bias
    # check therefore uses the hw.observe snapshot (`pa_dac.ch`) until the HIS gives it.
    return res


def _first(lst):
    return "" if not lst else json.dumps(lst[0])
