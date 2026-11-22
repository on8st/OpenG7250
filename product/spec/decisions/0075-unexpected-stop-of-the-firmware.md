# 0075 — Unexpected stop of the firmware

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session); proposed
  2026-11-22; answers a firmware question relayed by the openg7250 session, 2026-11-22. MUST
  be in force before the first build in which TX can be armed (0036).
- Date: 2026-11-22
- Builds on: 0036 (armable once calibrated), 0045 (safe state from the first 20 µs), 0055
  (no flash work while armed), 0065 (flash work), 0066 (TX path check), 0068 (log kept over
  a reset, reason lines).

## Context

A processor fault the firmware does not expect leaves every pin as it was until the
processor watchdog resets the MCU (about 2 s on the HIS reading); while keyed, that is up to
one watchdog time of unsupervised transmission. The accepted text has no rule for it: A3.6
(0045) puts the safe state in force at start, A3.5 and Table A3-1 event 5 treat a processor
watchdog reset seen at start-up as a fault, A3.10 makes safety timing independent of load;
none covers the time between the fault and the reset.

## Decision

1. **The rule** (A3.6 "Unexpected stop of the firmware"). For any processor fault the
   firmware does not handle as part of its normal work, and for its own stops (failed
   assertion, panic), in the boot stage and the image: the RF control pins at their
   safe-state levels within 20 µs, before anything else and without trusting stack, RAM or
   peripheral state; then a log line where it can; then a reset within 10 ms, without waiting
   for the watchdog.
2. **Log lines.** `restart: processor fault <kind>` (`hard`, `bus`, `usage`, `memmanage`,
   `other`) or `restart: stop <code>`, a short fixed code (lower-case letters, digits and
   hyphens, at most 16 characters) or a decimal number; never a file name, path, source line
   or panic text.
3. **During flash work** the 20 µs count from when the flash answers again; the pins hold
   their A3.6 levels meanwhile, and flash work never overlaps an arm.
4. **Evidence at the next start.** A software reset is a commanded reset only if the last
   kept log entry is a reason line of A1.7; in every other case (a fault or stop line, log
   lost, no reason line) it is an unexpected stop, entered as `WDT_RESET_AT_START` (class 1;
   Table A3-1 event 5, drop reason `RESET`) and logged `start: unexpected stop`. The boot
   stage logs a reason line before any software reset of its own. Power loss is never an
   unexpected stop; a watchdog flag is judged as today.
5. **Residuals.** A lockup is left to the processor watchdog (the pins keep the levels set,
   if the first step ran); a fault inside the first 20 µs is a residual. Open question for the
   HIS, written down and not looked up (AGENTS.md rule 7): HIS-21.
6. **Firmware build check** (not a conformance test): every fault vector of the image points
   to the handler of this rule.

## Alternatives considered

- Leaving it to the processor watchdog: up to about 2 s of unsupervised TX.
- A mark of its own in kept RAM for a commanded reset: the reason lines of 0068 already are
  that mark, checked as every kept entry is.
- A test build that raises the fault for the conformance test: the test runs on the released
  image with an emulator fixture instead; a bus fault on the radio is left to a radio drill.

## Consequences

- A3.6: new paragraphs "Unexpected stop of the firmware", "During flash work", "Evidence at
  the next start". A3.5 "Entry" gains "an unexpected stop". A1.7 gains the two `restart:`
  lines and `start: unexpected stop`; its "Pending hardware" bullet says that a software
  reset with the log lost counts as an unexpected stop until P-SYS-9 is set.
- Test T-A3-62 new: the unarmed part pending the emulator's fault fixture, the keyed part
  pending with the other TX tests.
- `open-questions-his.md`: HIS-21.
- Firmware: one handler on every fault vector and on its own stop path, written to run
  without trusting the stack; the log line where possible; the boot stage logs a reason
  before any software reset of its own.
- Operator and host: until P-SYS-9 is set the log may not survive a reset on the radio, so a
  `RESTART` may start with `WDT_RESET_AT_START` (class 1, clearable by `FAULT_CLEAR` without
  another reset; A3.5, T-A3-15). A software reset made by something other than this firmware
  (the vendor bootloader) counts once the same way. Neither stops a trial from confirming,
  and both act on nothing while TX is `NOT_CALIBRATED`.

## Sources

- Decisions 0036, 0045, 0055, 0065, 0066, 0068; [HIS] HIS 85 (RF control pins, through A3.6).
