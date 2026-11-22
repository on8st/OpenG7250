# 0068 — The log is kept over a reset without power loss

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session, as at `f3c15c1`);
  proposed 2026-11-22; answers a testbench question on T-A1-34, with the firmware's
  input and two firmware questions on the start entry folded in (relayed by the openg7250
  session, 2026-11-22); addendum on readings for the firmware build accepted
  (ON8ST, 2026-11-22, as at `1728f5d`)
- Date: 2026-11-22
- Builds on: 0008 (L-1 to L-3), 0022 §27 (counters), §40 (commanded reset), 0035 (P-SYS-2),
  0050 (P-SYS-9), 0055 (flash work), 0063 (abandonment), 0064 (`ROLLBACK`), 0067 (log names,
  boot-stage refusals).

## Context

A1.7 requires the log to record resets and their cause and every update step (L-2). It says
the log is kept in RAM and that whether it survives a warm reset is pending hardware
(P-SYS-2). Several events end in a reset: the abandonment of an update after 10 minutes,
`RESTART`, `FACTORY_RESET`, `UPDATE_ACTIVATE`, `ROLLBACK`, a trial that does not confirm
itself within 60 s, and a watchdog reset. An entry written just before the reset is lost if
the log does not survive it, so T-A1-34 ("aborted and logged") had nothing to check, and a host
could never learn why the radio restarted.

The firmware's input: its log is today cleared at every start, warm resets included. It
named three routes: (a) keep the log ring in RAM over a reset without power loss, as the
counters of A3.5 are; (b) keep only the last reason before a reset; (c) keep the log in the
store in flash. It advises (a), and rejects (c) for flash work during operation (0055) and
wear.

## Decision

1. **Route (a).** The log ring of A1.7 MUST be kept over every reset without power loss, as
   the counters of A3.5 are (0022 §27): commanded resets, the trial's own reset, and watchdog
   and other resets. It starts empty only after a power loss, or when what is kept fails its
   check (point 2).
2. **Integrity.** The log ring is kept in RAM that is not initialised at start (its range the
   firmware's choice). Each entry carries a check word; an entry that fails its check, and
   every entry after it in the ring, is dropped at start. Sequence numbers continue from the
   last entry kept, so `READ_LOG` pages stay consistent across a reset (L-3); after a power
   loss or a log that fails as a whole they start again at 0. Entries kept from the previous
   run keep their uptime of that run; the start entry of point 4 separates the runs.
3. **The reason before a commanded reset.** Before every reset it commands, the firmware MUST
   log the reason as its last entry, after the unkey and RF gone (A1.9 step 4) and before the
   reset:
   - `restart: restart` (`RESTART`);
   - `restart: factory-reset` (`FACTORY_RESET`; the log itself is kept, Table A1-4);
   - `restart: update-activated slot <s> <version>` (`UPDATE_ACTIVATE`);
   - `restart: update-abandoned` (the 10-minute rule of A1.8, also after `UPDATE_ABORT`);
   - `restart: installer-abandoned` (the streaming installer restarting into this firmware
     before its first erase, A1.8a item 3);
   - `restart: trial-unconfirmed` (no self-confirmation within 60 s, 0044);
   - for `ROLLBACK`, 0064's line `rollback commanded: slot <from> <version> to slot <to>
     <version>`.
   Slots and versions are written as 0067 says (`A` or `B`; `major.minor.build`). For a reset
   it does not command (watchdog, a crash), the entries written before it are what survives.
4. **The start entry, written by the boot stage** (firmware question 1, 2026-11-22). At
   every start the boot stage MUST write, as the first entry of the run and before anything
   else it logs, `start after <cause>: log kept` or `start after <cause>: log lost`. "lost"
   means the ring started empty (power loss, or the check of point 2 failed as a whole). It
   is the only writer of the start entry: the application writes none and continues the ring.
   If the boot stage starts no image (0067 point 3.3), the start entry and its lines are still
   in the ring and are read after the next start that reaches the network.
   **The cause** (firmware question 2, 2026-11-22) is named from the MCU's reset flags, read
   once per start by the boot stage and then cleared, so that each start reports its own
   reset. A reset that sets several flags is named by the first of this order: `watchdog`
   (either watchdog), `software`, `power-on`, `brown-out`, `pin`; `unknown` only when no flag
   is set. The application gets the cause from the boot stage (the firmware's way), and the
   start entry, `IDENTIFY` tag 17, the reset counters of A3.5 and `WDT_RESET_AT_START` all use
   that one reading. P-SYS-2 is only whether the flags are trustworthy on this radio, the
   vendor bootloader having run first; its radio check confirms, for each kind of reset, the
   cause named.
5. **Boot stage lines.** The boot stage's outcomes (a trial started, a rollback because the
   trial reset before confirming itself, a slot refused, 0067) MUST follow the start entry,
   written by the boot stage into the kept ring. Lines: `boot: trial of slot <s>
   started`, `boot: rollback to slot <s> (trial not confirmed)`, `boot: slot <s> refused:
   <check>` (0067).
6. **Pending hardware.** Whether this RAM survives the stage that runs before the boot stage
   at a reset is the question P-SYS-9 already asks for the counters, and is measured by the
   same radio check, now over the log ring's range too. P-SYS-2 is now only whether the reset flags are trustworthy (point 4).
   Until P-SYS-9 is set the log is allowed to start empty after a reset (`log lost`); nothing
   else depends on it. P-SYS-9 stays part of the freeze rule because of the counters, not
   because of the log.
7. **What T-A1-34 checks** (reference model or emulator, where RAM survives a reset without
   power loss): after the restart, `READ_LOG` returns, in sequence with no gap, the entry
   `restart: update-abandoned` with an uptime of the previous run within 5 s of the
   `UPDATE_BEGIN` plus 10 min, then `start after software: log kept`, then the entries of the new run; `UPDATE_STATUS` reports idle,
   `IDENTIFY` tag 10 reports 0, and RX works. On the radio it is run once P-SYS-9 is set;
   before that it checks only that the start entry is present, with `log kept` or `log lost`.
   Test T-A1-91 (new) covers the other reasons, the boot-stage lines, a watchdog reset with
   the entries before it kept, a power cycle (`start after power-on: log lost`, sequence at 0)
   and a kept ring with one entry corrupted.

## Addendum: readings for the firmware build (accepted by ON8ST, 2026-11-22)

Firmware questions on its 0068 build, relayed by the openg7250 session, 2026-11-22.

1. **What counts as a power loss**, for the log and for the counters of A3.5 alike: a start
   whose reset flags include power-on or brown-out, whatever cause the start entry names, or a
   start with no reset flag at all. So `start after software: log lost` can occur, and a start
   with no flag gives `start after unknown: log lost` and counters from 0. Reason: with no flag
   the firmware cannot tell that RAM was kept, and RAM not written since power-up must not be
   believed. This is safe only if every reset without power loss sets at least one flag, so the
   P-SYS-2 radio check MUST also show that for each kind of reset, and P-SYS-9 is not set
   before P-SYS-2 has shown it (until then TX stays locked anyway).
2. **Oldest entry failing its check**: by point 2 nothing is kept, and that is "fails as a
   whole": `log lost`, sequence numbers from 0.
3. **When the `ROLLBACK` line is logged**: as the last entry before the reset, after the unkey
   and RF gone, like every other reason of point 3; not at the moment the command is accepted.
   0064 item 3.3 is read so.
4. **Fallback from a refused active slot** (0067 point 3.2): the boot stage logs
   `boot: slot <s> refused: <check>` for the refused slot, then
   `boot: fallback to slot <s> (active slot refused, no trial)` for the slot it starts.
   "rollback" stays reserved for a return from a trial.

Tests: T-A1-91 (no flag; power-on flag with software named; oldest entry corrupted; the
`ROLLBACK` line as the last entry before the reset), T-A1-90 (the fallback lines).

## Alternatives considered

- (b) Keep only the last reason before a reset, in the persist block: it covers commanded
  resets, but a watchdog reset would keep nothing of what led to it, and it depends on the
  same P-SYS-9 measurement as (a).
- (c) Keep the log, or a restart record, in the store in flash: survives everything, but adds
  flash work and wear for each reset, and gives nothing for a watchdog reset.
- Let the log start empty after every reset until P-SYS-2 is set: a host could not learn why
  the radio restarted, and the "every update step" rule of L-2 would fail for every step that
  ends in a reset.

## Consequences

- Spec A1.7 ("Kept over a reset", "The start entry"); Table M-2 tag 17 (values); Annex P rows
  P-SYS-2 (whether the reset flags are trustworthy) and P-SYS-9 (the
  log ring added); tests T-A1-34 (rewritten) and T-A1-91 (new).

## Sources

- Decisions 0008, 0022, 0035, 0044, 0050, 0055, 0063, 0064, 0067; testbench question on
  T-A1-34 and the firmware's input (relayed by the openg7250 session, 2026-11-22).
