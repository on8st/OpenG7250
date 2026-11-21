# 0068 — The log is kept over a reset without power loss

- Status: proposed (2026-11-22); answers a testbench question on T-A1-34, with the firmware's
  input (relayed by the openg7250 session, 2026-11-22)
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
4. **The start entry.** At every start the application MUST log, as the first entry of the
   run, `start after <cause>: log kept` or `start after <cause>: log lost`, with `<cause>` one
   of `power-on`, `software`, `watchdog`, `brown-out`, `pin`, `unknown`. "lost" means the
   ring started empty (power loss, or the check of point 2 failed as a whole). Until P-SYS-2
   is set the cause is not read and is `unknown` (`IDENTIFY` tag 17 = 0, Annex P placeholder).
5. **Boot stage.** The boot stage's outcomes (a trial started, a rollback because the trial
   reset before confirming itself, a slot refused, 0067) MUST appear in the log right after
   the start entry, either written by the boot stage into the kept ring or derived by the
   application from the boot record (the firmware's choice). Lines: `boot: trial of slot <s>
   started`, `boot: rollback to slot <s> (trial not confirmed)`, `boot: slot <s> refused:
   <check>` (0067).
6. **Pending hardware.** Whether this RAM survives the stage that runs before the boot stage
   at a reset is the question P-SYS-9 already asks for the counters, and is measured by the
   same radio check, now over the log ring's range too. P-SYS-2 keeps only the reset cause.
   Until P-SYS-9 is set the log is allowed to start empty after a reset (`log lost`); nothing
   else depends on it. P-SYS-9 stays part of the freeze rule because of the counters, not
   because of the log.
7. **What T-A1-34 checks** (reference model or emulator, where RAM survives a reset without
   power loss): after the restart, `READ_LOG` returns, in sequence with no gap, the entry
   `restart: update-abandoned` with an uptime of the previous run within 5 s of the
   `UPDATE_BEGIN` plus 10 min, then `start after software: log kept` (`start after unknown:
   log kept` before P-SYS-2), then the entries of the new run; `UPDATE_STATUS` reports idle,
   `IDENTIFY` tag 10 reports 0, and RX works. On the radio it is run once P-SYS-9 is set;
   before that it checks only that the start entry is present, with `log kept` or `log lost`.
   Test T-A1-91 (new) covers the other reasons, the boot-stage lines, a watchdog reset with
   the entries before it kept, a power cycle (`start after power-on: log lost`, sequence at 0)
   and a kept ring with one entry corrupted.

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

- Spec A1.7 ("Kept over a reset"); Annex P rows P-SYS-2 (reset cause only) and P-SYS-9 (the
  log ring added); tests T-A1-34 (rewritten) and T-A1-91 (new).

## Sources

- Decisions 0008, 0022, 0035, 0044, 0050, 0055, 0063, 0064, 0067; testbench question on
  T-A1-34 and the firmware's input (relayed by the openg7250 session, 2026-11-22).
