# 0064 — Torn boot-record writes and the ROLLBACK command

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers testbench
  run-sheet gaps 1–4 (tb 1b93429), 2026-11-21
- Date: 2026-11-21
- Builds on: 0008 (U-5, U-6, U-8), 0022 §10, 0043, 0044, 0061, 0065 (store writes).

## Context

The testbench's run-sheet mapping found four cases in A1.8 with no test, two of them with no
rule: a power loss during the boot stage's trial-attempt write or during the
self-confirmation write, and what `ROLLBACK` does (Table M-1 and A1.8 gave only its status
rules). The reference model starts the other slot if its image is good, and gives 55
otherwise.

## Decision

1. **Gaps 1 and 2: tests only.** `CAP_REQUEST` during an update and the arm reject (0022 §10)
   are added to T-A1-23. Corrected after testbench finding SF-28: over P2 every register write
   during an update gets `BUSY`, which takes precedence (B8.4), so a `TX_ARM` write gets `BUSY`
   in `WRITE_STATUS` and the radio stays disarmed; `MAINTENANCE` appears only in the arm-update
   race (T-A1-40) and on level 2. A trial image with a class 2 fault is not confirmed
   and rolls back (0044); this is added to T-A1-68.
2. **Torn boot-record writes** (gap 3). A boot-record write cut by a power loss counts as not
   written; the store keeps the older record (0065). A torn attempt record means the trial
   image has not run, so the boot stage may start it once more; a torn confirmation means the
   trial is unconfirmed, so the previous image starts (rollback, logged). Test T-A1-83;
   T-A1-08 stays the general test.
3. **`ROLLBACK`** (gap 4), as the reference model does, with these details:
   1. it is accepted only if the running image is confirmed (58 during a trial boot or an update) and
      the other slot holds an image of this firmware that checks good (`IDENTIFY` tag 9);
      otherwise it gets 55 and nothing changes;
   2. no downgrade flag is needed: going back is the purpose of the command;
   3. when accepted: write the boot record, log
      `rollback commanded: slot <from> <version> to slot <to> <version>`, reply 0, then a
      commanded reset (A1.9; 59 if RF stays);
   4. the image started is a trial (U-6), so a failed rollback target returns to the image that
      sent `ROLLBACK`;
   5. configuration the older image does not understand is kept but not used (U-8).
   Test T-A1-84.

## Alternatives considered

- Start the rollback target as confirmed: it ran before, but it may not run with today's
  configuration or store; a trial costs nothing and keeps a way back.
- A new status for "no good image to roll back to": 55 already means "the image does not check
  good".

## Consequences

- Spec A1.8 ("Torn boot-record writes", "`ROLLBACK`"), Table A1-2 status 55; tests T-A1-23,
  T-A1-68, T-A1-83, T-A1-84.

## Sources

- Decisions 0008, 0022, 0043, 0044, 0061, 0065; testbench run-sheet mapping (tb `1b93429`).
