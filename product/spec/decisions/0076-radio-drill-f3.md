# 0076 — Radio drill F3: torn store words, the unreadable word, and kept RAM

- Status: accepted as text (ON8ST, 2026-11-22, relayed by the openg7250 session); the drill
  itself runs only with its own go from ON8ST. Proposed 2026-11-22; answers an emulator
  finding relayed by the openg7250 session, 2026-11-22. One question is open (Decision 3).
- Date: 2026-11-22
- Builds on: 0065 (ECC errors are torn records; T-A1-87), 0068 (log kept over a reset,
  P-SYS-9), 0071 (released builds, drills F1 and F2), 0075 (unexpected stop).

## Context

For 0065 and T-A1-87 the emulator shows a double flash ECC error only in its flags; it cannot
raise the bus fault the real chip gives on such a read (the emulator sets the fault status
but takes no exception). So "no fault and no reset" in T-A1-87 is not proven by an emulator
pass, and no drill covers it on the radio.

## Decision

1. **P-SYS-10** (Annex P): whether the store reads a word with a double ECC error without a
   fault and without a reset on the radio. Pending hardware: on the radio, with a supply
   switch the bench controls; ON8ST's, no date. Until set, not proven: the store's read path
   is shown by review and by the emulator's flags only.
2. **Drill F3** (Annex P, after Table P-1), on the radio with RF in the safe state, a dummy
   load on both antenna ports and a supply switch the bench controls: (0) kept RAM at every
   power-on start; (1) 200 power cuts inside store writes, half in compactions, half in
   appended writes; (2) every start within the normal time, no reset loop, network and RX
   up; (3) every item old or new, never another; (4) starts that log an unreadable word
   counted; (5) one bus fault of 0075 on a test build, with one RF control pin on a scope.
   F3 passes if (0), (2) and (3) hold for every start; if (4) counts none, P-SYS-10 stays
   unset and ON8ST decides. Test builds are accepted for a radio drill, not for a conformance
   test.
3. **Open.** Whether programming a flash word a second time without an erase leaves a word
   whose data and ECC disagree, and so reads with a double ECC error. To be settled from the
   STM32H72x/H73x reference manual (RM0468, flash chapter) before drill F3. It could not be
   read for this decision (the publisher's site did not serve the PDF, and the spec's sources
   hold no copy), so no section is cited and nothing is claimed; it enters `sources.md` only
   once read. If it holds, F3 gains a fixed case (one such word in the non-live sector and one
   in a record of the live generation); the random cuts stay as the survival test.

## Alternatives considered

- Accepting the emulator's flags as proof: they do not show the bus fault, which is the
  point at issue.

## Consequences

- Annex P: row P-SYS-10, the drill F3 text and the open fixed case. T-A1-87 gains "on the
  radio: drill F3 (P-SYS-10)". T-A3-62 names step (5) of F3 for the bus fault.
- No accepted text is reworded.
- Cost: a supply switch on the bench (a relay on the 13.8 V line, timed from the bench);
  about 200 starts; power cuts at the main unit only, which the product must survive anyway;
  no RF. A test build for (5) and, if RM0468 allows, for the fixed case.

## Sources

- Decisions 0065, 0068, 0071, 0075; [STM32H72x] (flash ECC, through 0065).
