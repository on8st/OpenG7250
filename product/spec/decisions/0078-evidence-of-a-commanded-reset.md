# 0078 — Evidence of a commanded reset, test timings, counter details

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session); proposed
  2026-11-22; answers four testbench questions after building 0073 to 0077, relayed and
  sharpened by the openg7250 session, 2026-11-22. Point 1 MUST be in force in the boot stage
  before the first build in which TX can be armed (0036).
- Date: 2026-11-22
- Builds on: 0045 (safe state), 0068 (log kept over a reset, reason lines), 0073 (time over
  flash work), 0074 (Table M-6), 0075 (unexpected stop), 0077 (streaming installer).

## Context

Point 1 closes a hole in 0075 on the unsafe side. If a run's start entry fails its check, or
the stop comes before that entry reaches the kept RAM, the last kept entry is the reason line
of the run before, and the start reads as a commanded reset; a fault in that window is missed.
The testbench shows it on the reference model (`RESTART`, start entry made invalid, then a
software reset: no unexpected stop). Points 2 and 3 are editorial: test figures that could not
show what they were meant to, and counter details the tests needed.

## Decision

1. **The commanded-reset word.** One word in the RAM kept over a reset (the range of P-SYS-9).
   - Set as the last write before every commanded reset, after the reason line, and by the boot
     stage's own software resets. It names the reason line's sequence number; a partial write,
     the cleared state and power-on content cannot read as valid (encoding the firmware's
     choice). "Written" means reached the kept RAM, not held in a cache or write buffer.
   - The fault handler clears it by one write right after the pins: pins, clear, log, reset.
   - The boot stage reads the reset flags first. At a power loss it neither reads the word nor
     judges evidence. It clears the word at every start, by a write of whole ECC words that
     needs no read, which reaches the kept RAM before the ring check or any other work after
     the safe state; it judges from the value it read.
   - A software reset is a commanded reset only if the word was valid, names the last entry
     kept, and that entry is a listed reason line. Everything else is an unexpected stop.
   - The log is not touched, and `READ_LOG` still shows the reason line.
   - A read of kept RAM at start never stops the start: an ECC error or a fault makes that
     entry, or the word, fail its check (A1.7).
   - Residual: a fault inside the handler before its clear is a lockup and ends by the
     processor watchdog (a watchdog flag, A3.5).
   - Until P-SYS-9 is set the word may be lost like the log; the start is then an unexpected
     stop, as under 0075.
2. **Test timings** (editorial; the intent stays: the limit falls due during the flash work). A
   compaction takes one erase of at most `t_erase_max` = 4 s and programs one sector, at most
   4096 × 580 µs, about 2.4 s ([STM32H72x] Table 48, as B2.3 uses it): about 6.4 s if it
   succeeds. Only a failing operation takes the reply up to `t_flash_reply`. T-A1-94: received
   2 s before the 60 s. T-A1-15: 57 s after the last request, and 55 s with an erase that hangs.
   T-A1-60: 57 s in both places.
3. **Counters** (editorial). Every counter of Table M-6 other than `fault_<code>`,
   `reset_<cause>` and `trip_count` starts at 0 at every start; `reset_unknown` is always 0
   while a start with no reset flag counts as power loss (0068 addendum); a packet discarded
   whole that has a counter of its own counts in `drop_<port>` too, so `cap_rate_drop` (its row
   reworded: it counts requests, not answers) and `notowner_drop` also count in `drop_7250`; a
   gap is defined with the serial-number arithmetic of B11.6a and counts once.

## Alternatives considered

- Counting dropped entries as evidence, and writing the start entry before judging: both only
  narrow the hole, because a stop before the first word of the start entry reaches the kept RAM
  leaves the ring exactly as the commanded reset left it.
- A rule over the ring ("no later check may find the reason line last without a dropped one"):
  right, but it depends on the ring's layout and on the order in which an entry's words reach
  the RAM, so it is hard to build and to test.
- 0075 rejected a mark of its own because the reason lines were that mark; the case above shows
  that they are not enough.
- A handler that resets through the processor watchdog, so that the reset flag is the evidence:
  rests on the reference manual RM0468, which has not been read; not proposed.

## Cost and transition

- Firmware: one word in the kept range, set last on every commanded-reset path and by the boot
  stage's own resets, cleared by the boot stage and by the fault handler; the cache or
  write-buffer handling so that each reaches the kept RAM.
- Host and operator: none; an unexpected stop is still class 1 and clearable.
- 0078 acts from the boot stage that carries it. A radio whose boot stage was installed before
  0078 keeps the evidence rule of 0075 until a re-install through the vendor path (U-7; A1.8a
  item 9); 0078, like 0075, MUST be in force in the boot stage before the first build in which
  TX can be armed (0036). With such a boot stage, a commanded reset made by an image built
  before 0078 sets no word and is an unexpected stop at the next start: once for each such
  reset, for example an `UPDATE_ACTIVATE` or `ROLLBACK` made by the older image, in either
  direction between images of different age, and every `RESTART` it makes. It is class 1 and
  clearable (A3.5), and does not stop a trial from confirming (A1.8 asks for no class 2 fault).
- No test of this spec needs an image built before 0078. A test or bench run that activates or
  rolls back across one MUST allow one `WDT_RESET_AT_START` at the start after each commanded
  reset made by that image, and clear it with `FAULT_CLEAR`.

## Consequences

Accepted text that changes:
- A1.7 "The reason before a commanded reset" (0068): the word, and the meaning of "written".
- A1.7 "Kept over a reset" (0068): the read rule.
- A3.6 "Unexpected stop of the firmware" (0075): the clear after the pins; the residual sentence.
- A3.6 "Evidence at the next start" (0075): rule rewritten on the word; "with no mark of its
  own" removed; the boot stage's own resets set the word.
- Annex P, P-SYS-9 (0050, 0068): the word added.
- T-A3-62 (0075): cases added.
- T-A1-94 (0073), T-A1-15 and T-A1-60 (0077): timings; the Consequences line of the 0077
  decision file (a record).
- Table M-6 (0074): intro, rows `reset_unknown` and `cap_rate_drop`.
- B11.1 (0020 §19, §28; 0074) and B11.6 (0020 §19; 0074).
- T-A1-95 (0074) and T-B11-04.

Not in 0078: a boot-stage line "log entries dropped", and the variant that resets through the
watchdog.

## Sources

- Decisions 0045, 0050, 0068, 0073, 0074, 0075, 0077; [HIS] HIS 90 (through P-SYS-9);
  [STM32H72x] Table 48 (through B2.3); RFC 1982 (through B11.6a).
