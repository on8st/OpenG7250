# 0045 — The safe state holds from 20 µs after the boot stage starts

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers a question
  from the first emulator run with correct clocks (testbench T-B10-L11, 2026-11-21)
- Date: 2026-11-21
- Builds on: 0028 (boot stage at 0x08020000); [HIS] HIS 85, HIS 90 (`5292ba8`).

## Context

At power-on the RF control pins are inputs, floating, until firmware sets them. On the radio the
vendor bootloader in sector 0 runs first. Our firmware sets every HIS 85 pin about 400
instructions (under 10 µs at the 64 MHz reset clock) after its reset vector. No firmware can hold
the levels before that; only the board's pull resistors (not known) or the vendor bootloader can.

## Decision

1. A3.6: the RF control pins of HIS 85 MUST be at their safe-state levels no later than 20 µs
   after the boot stage's first instruction at 0x08020000, and before any other initialisation.
2. Before that point the levels are set by the hardware and the vendor bootloader and are outside
   this spec. Whether the board pulls hold safe levels then is pending hardware (P-SYS-1, reworded).
3. T-A3-06 starts its power-on check at that point.

The 20 µs leave twice the firmware's measured margin; the bound is on the order of the pins'
reset-to-init window, not a value of the hardware.

## Sources

- Emulator run via openg7250, 2026-11-21; [HIS] HIS 85, HIS 90; decision 0028.
