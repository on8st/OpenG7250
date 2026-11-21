# 0066 — What `TX_PATH_NOT_BLOCKED` detects, and how fast

- Status: proposed (to the repository owner, ON8ST); answers testbench finding SF-29,
  2026-11-21
- Date: 2026-11-21
- Builds on: 0007 Decision 5 (safe state, fault classes), 0022 §5, 0045 (safe state from the
  boot stage), 0064 (T-A1-68); [HIS] HIS 85.

## Context

Drop event 13 and fault code 16 `TX_PATH_NOT_BLOCKED` existed (Table A3-1, Table B8-3), but the
spec did not say what is checked: the MCU's own configuration of the pin, or the level at the
pin as the hardware sees it, nor how fast. The test bench and the firmware need one stimulus.

## Decision (proposed)

1. New A3.6b. While TX is not armed and not keyed, the firmware checks the TX-side outputs of
   A3.6 (TX path disable, SEND, TUNER trigger, PA bias channels; [HIS] HIS 85) at least every
   10 ms, from the end of the boot stage's safe-state set-up.
2. **Both are checked.**
   1. The MCU's configuration: output mode, output register at the safe level, and DAC value 0
      for a bias channel. This catches a firmware fault.
   2. The level at the pin, from its input register. This catches a line held from outside,
      a board fault. Bias channels have no pin read and are checked by item 1 only.
3. A mismatch on two checks in a row, so within 20 ms, is drop event 13, `TX_PATH_NOT_BLOCKED`,
   class 2.
   - The log names the pin and which comparison failed.
   - The firmware sets the safe state again and stays in the fault; RX goes on.
   - One check alone is not enough, so that a single glitch causes no fault.
4. No check while armed or keyed, because the path is then meant to be open.
5. Tests:
   - T-A3-61 (new) covers both stimuli: the pin level forced from outside with the output
     register unchanged, and the register or mode changed; it also checks the 20 ms bound,
     that a pulse shorter than 10 ms gives no fault, and the behaviour after a reset;
   - T-A1-68 uses the first stimulus.

## Alternatives considered

- Configuration only: misses a board fault that holds a line, which is the case that matters
  most for a transmitter.
- Pin level only: misses a pin turned into an input that floats near the safe level.
- A single check: a glitch on a long line would latch a class 2 fault.

## Consequences

- Spec A3.6b; tests T-A1-68 (wording), T-A3-61.

## Sources

- [HIS] HIS 85; decisions 0007, 0022, 0045, 0064; testbench finding SF-29.
