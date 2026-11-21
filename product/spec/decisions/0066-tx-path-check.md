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
   5 ms, from the end of the boot stage's safe-state set-up.
2. **Both are checked.**
   1. The MCU's configuration: output mode, output register at the safe level, and DAC value 0
      for a bias channel. This catches a firmware fault.
   2. The level at the pin, from its input register. This catches a line held from outside,
      a board fault. Bias channels have no pin read and are checked by item 1 only.
3. A mismatch seen by every check over at least 10 ms is drop event 13, `TX_PATH_NOT_BLOCKED`,
   class 2. So a mismatch shorter than 10 ms never counts, and one of 20 ms or more always does,
   within 20 ms of its start. The rule is in time rather than in a number of checks, so that a
   firmware that checks more often than every 5 ms does not latch on a shorter glitch.
   - The log names the pin and which comparison failed.
   - The firmware sets the safe state again and stays in the fault; RX goes on.
4. No check while armed or keyed, because the path is then meant to be open.
5. Tests:
   - T-A3-61 (new) covers both stimuli: the pin level forced from outside with the output
     register unchanged, and the register or mode changed; it also checks the 20 ms bound,
     that a 5 ms pulse gives no fault, and the behaviour after a reset;
   - T-A1-68 uses the first stimulus.

## Addendum: the pins (testbench finding SF-30, 2026-11-21)

6. The check covers the HIS 85 TX-side lines with a known safe level, as Table A3-6b in A3.6b:
   - PF4 high; PF6, PC7, PH11 low (T/R lines);
   - PA10 high (chain A output enable);
   - PB14 low (PA DAC reset);
   - DAC channels 2, 7, 8 at 0, by the value last written only, since the DAC cannot be read
     back.
   This is the bench's list.
7. "TX path disabled" in A3.6 means the T/R lines at RX levels and chain A disabled. The ATU
   and antenna strobes, chain B, and the pins with unknown safe level (PA4, PA12, PD12) are
   not checked.
8. SEND and the TUNER trigger, while P-SYS-8 is pending: no pin, not driven, not offered, not
   checked. When P-SYS-8 is set, their pins join Table A3-6b with their released level.

## Addendum: start of the check and the DAC rows (firmware questions, 2026-11-21)

9. **Start.** A bounded gap is accepted rather than a check loop in the boot stage. The first
   check runs no later than 100 ms after the boot stage's first instruction. Between the boot
   stage's safe-state set-up (0045) and that check, no code writes the Table A3-6b pins or
   their configuration. So the pins stay as set, and only a board fault could change them
   unseen for at most that time. Measured on the emulator as for the 20 µs of 0045. In T-A3-61,
   a pin forced from power-on gives the fault within 120 ms of the first instruction.
10. **DAC rows.** Confirmed. While the firmware never writes the DAC and holds PB14 low, the
    rows hold by construction, and PB14 itself is checked on its pin level. The DAC part of
    T-A3-61 is judged by host tests of the check until a DAC driver exists, then also on the
    emulator. No test hook goes into the product image.

## Addendum: the check during flash work (firmware question, 2026-11-21)

11. Option (a), as the firmware advised. During a sector erase or program the 5 ms period does
    not apply: the operation cannot be split and runs with interrupts masked.
    - The firmware SHOULD sample the Table A3-6b pins in its wait loop, where it already
      services the watchdog. A register read costs nothing there, and a sample counts as a
      check.
    - It MUST run a check at once when the operation ends.
    - A mismatch present at the end counts from the start of flash work, or from its first
      sample, so it gives the fault no later than 20 ms after the end.
    This is safe because flash work never overlaps an arm (0055), and no code writes the RF
    pins during it (0065 item 1). Test T-B5-22 (flash work on the emulator).

## Alternatives considered

- Configuration only: misses a board fault that holds a line, which is the case that matters
  most for a transmitter.
- Pin level only: misses a pin turned into an input that floats near the safe level.
- A single check, or a count of checks: a glitch would latch a class 2 fault, or the bound
  would depend on how often the firmware checks.

## Consequences

- Spec A3.6b, Table A3-6b; tests T-A1-68 (wording), T-A3-61 (pins of Table A3-6b).

## Sources

- [HIS] HIS 85; decisions 0007, 0022, 0045, 0064; testbench finding SF-29.
