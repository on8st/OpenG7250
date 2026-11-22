# 0067 — What an image of this firmware must be: the image check, VERIFY and IDENTIFY tag 9

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session, as at `f3c15c1`);
  addendum on two log words proposed 2026-11-22; proposed 2026-11-21; answers testbench finding SF-31
- Date: 2026-11-21
- Builds on: 0008 (U-4, U-5, U-6), 0028 (boot stage), 0043 (slot-linked images), 0044
  (self-confirmation), 0061 (slot size), 0064 (`ROLLBACK`), 0065 (ECC errors).

## Context

A1.8 says the staged image is hashed and compared with the header before activation, and that
no signature is checked (U-4). It says nothing else about what an image of this firmware must
be. `IDENTIFY` tag 9 reports "integrity: good / bad" per slot without saying what is judged or
when, and `ROLLBACK` depends on it since 0064. A test therefore cannot say what happens to a
package whose SHA-256 matches its header but which is not a runnable image: refused at
`UPDATE_VERIFY` or `UPDATE_ACTIVATE`, refused by the boot stage, or started and rolled back.

This decision states the host-visible contract only: what the radio accepts, refuses and
reports, and when. How an image is laid out inside, and how the firmware carries out each
check, stay the firmware's design; the test bench and the reference model work from this
contract alone. Folded in before acceptance (testbench points on 0067, 2026-11-21): the
Context reworded to the contract alone (a), blocks sent again (b), slot names in log lines
(c), and `ROLLBACK` to a slot whose trial failed (d).

## Decision

1. **The image check** (SF-31 (i)). An image of this firmware passes the image check for a
   slot when all of these hold:
   1. every flash word of the image can be read (no double ECC error, 0065);
   2. it carries an image record of this firmware, in a place and layout the firmware defines
      and documents, naming the G7250 and the kind "application";
   3. the length it records fits the slot (0061);
   4. it is linked for that slot (0043);
   5. its own integrity check over the whole image holds (the firmware chooses it, for example
      a CRC-32);
   6. its initial stack pointer lies in RAM and its entry point in that slot.
   **The image is what its record describes** (bench question on T-A1-88, 2026-11-21): every
   item above is judged over the length the image record gives, never over the length
   received, because the boot stage has no package header and must reach the same result as
   `UPDATE_VERIFY`. Bytes received beyond the recorded length are covered by the hash (check
   1 of point 2) and by the record–header agreement (check 3). So a package with bytes
   appended to an intact image, header length and hash adjusted, passes the image check and
   MUST be refused with `header`, not `integrity`.
   The boot stage is not an image of this kind; it is not updated through this channel (U-7).
   A foreign image (A1.8a item 3) gets no image check: it stays opaque, and only its SHA-256 is
   checked.
2. **`UPDATE_VERIFY`** (SF-31 (iii)). For an image of this firmware, `UPDATE_VERIFY` checks, in
   this order, with status 55 at the first that fails and the failed check named in the log:
   1. the SHA-256 of the received image equals Table M-4 bytes 18–49;
   2. the image check of item 1 holds for the staging slot;
   3. the image record agrees with the package header: length (bytes 14–17), version (bytes
      10–13) and, when byte 52 is not 0, slot.
   Point 3 is needed because the checks of `UPDATE_BEGIN` (51, 63, 61, 52) read the header, and
   the version the image reports later (`IDENTIFY`, B4.8 `fw_version`) comes from the record.
   After a failed `UPDATE_VERIFY` the transfer stays in progress (the host may send blocks
   again and verify again, as for a hash mismatch today); `UPDATE_ACTIVATE` gets 55 until an
   `UPDATE_VERIFY` succeeds. `UPDATE_ACTIVATE` adds no check of its own; an accepted
   `UPDATE_BLOCK` after a good `UPDATE_VERIFY` cancels that verify.
   - **Blocks sent again** (point b). During a transfer, an `UPDATE_BLOCK` for a block number
     already received, sent as a new request (a new request id, so not a repeat of A1.4), with
     a good CRC, MUST be accepted and written: its data replaces what was received for that
     block, and the image hashed and checked at the next verify is the last data received for
     each block. If replacing a block needs an erase, the radio MAY count the other blocks of
     the same erase sector as not received again; "blocks received" (`UPDATE_BLOCK` reply,
     `UPDATE_STATUS`) then drops by that number, and a verify with any block not received
     gets 55. A host that sends again after a failed verify SHOULD therefore send every block
     again, in order. A repeat with the same request id is still answered from the stored
     reply and not written (A1.4).
3. **The boot stage** applies the image check to a slot every time before it starts it.
   1. A trial slot that fails it is not started: the boot stage starts the previous image,
      logs `boot: slot <s> refused: <check>` and the rollback, and the update state becomes
      "rolled back" (tag 10 = 2).
   2. A confirmed active slot that fails it: the boot stage starts the other slot if that slot
      counts as good (tag 9 = 1 by item 4: it passes the image check and its staging, if any,
      ended with a good `UPDATE_VERIFY`), as the active image and without a trial (there is nothing left
      to return to), records this in the boot record, logs it, and the update state becomes
      "rolled back".
   3. Neither slot passes: the boot stage holds the A3.6 safe state, services the watchdog and
      does not start the network; the radio does not answer. Recovery is the vendor path
      (A1.8a item 9).
   4. **Slot names in log lines** (point c). In every update and boot log line, this one and
      0064's `rollback commanded: slot <from> <version> to slot <to> <version>` included, a
      slot is named by its letter, `A` or `B` (Table M-2 tag 23 reports them as 1 and 2), and
      `<check>` is one of `unreadable`, `no-record`, `hardware`, `kind`, `length`, `slot`,
      `integrity`, `vectors` (the items of point 1, in order); a failed `UPDATE_VERIFY` names
      the same words, or `hash` or `header` for its checks 1 and 3. The version is written
      `major.minor.build`.
4. **`IDENTIFY` tag 9** (SF-31 (ii)), one value per slot:
   - 1 good: the slot passed the image check when last judged, and its staging (if any) ended
     with a good `UPDATE_VERIFY`;
   - 0 none: the slot holds no image to start: it is erased, has no image record, or its
     staging has not ended with a good `UPDATE_VERIFY` (from the first erase of
     `UPDATE_BEGIN`; also after `UPDATE_ABORT`, a failed verify, a write failure or a timeout,
     until a later good `UPDATE_VERIFY` of that slot);
   - 2 bad: the slot has an image record and no unfinished staging, but fails the image check
     (for example an unreadable word, or a failed integrity check).
   The firmware keeps the mark "staging not ended with a good verify" across a reset (in the
   store or in the slot; its choice), so that 0 survives a restart.
   **When it is judged:** the running image is the one the boot stage judged at this start
   (always 1 while it runs). The other slot is judged once at each start (by the boot stage or
   the application, before the management socket opens), at a good `UPDATE_VERIFY`, and again
   at each `ROLLBACK` before the command is accepted. `IDENTIFY` reports the last judgement; it
   does not read the slot. A trial image that failed to confirm itself keeps 1: tag 9 reports
   integrity, and tag 10 reports the rollback.
   - **`ROLLBACK` to a slot whose trial failed** (point d). Intended: such a slot counts as
     good, and `ROLLBACK` to it is accepted. This is how 0064 item 3.1 reads from now on:
     "checks good" means tag 9 = 1, and a failed trial does not exclude it. Reason: the image
     is intact, the failure may have had a cause that is gone (a network down during the trial,
     for example), and the start is a trial again (0064 item 3.4), so a second failure returns
     to the image that sent `ROLLBACK`. The host learns of the earlier failure from tag 10 and
     the log, not from tag 9.
5. **What a test can expect** for a package that is not a runnable image of this firmware:
   - SHA-256 differs from the header: 55 at `UPDATE_VERIFY` (T-A1-59);
   - SHA-256 matches, but no image record, wrong hardware or kind, linked for the other slot,
     integrity check failing, entry point or stack pointer out of range, or record and header
     disagree: 55 at `UPDATE_VERIFY`, 55 at `UPDATE_ACTIVATE`, the running image and the boot
     record unchanged, tag 9 for the staging slot 0 (T-A1-88);
   - passes every check but does not run or does not confirm itself: started as a trial and
     rolled back (U-6, 0044; T-A1-68);
   - refused by the boot stage: only if the slot changed after a good `UPDATE_VERIFY` (a flash
     fault); the previous image starts and the rollback is logged (T-A1-90).
   `ROLLBACK` to a slot with tag 9 = 0 or 2 gets 55 (0064; T-A1-89); to a slot whose trial
   failed it is accepted and starts a trial (T-A1-89).

## Addendum: two more log words for a failed verify (proposed, 2026-11-22)

Firmware question, relayed by the openg7250 session, 2026-11-22. A failed `UPDATE_VERIFY`
can stop before its check 1 (the hash) can run. It names, in this order and before `hash`:

- `missing-blocks`: a block of the image has not been received, or was counted as not received
  again (point 2, "Blocks sent again"); the verify does not hash an incomplete image;
- `unreadable`: a received word of the staging slot cannot be read (0065), so the hash cannot
  be computed. This is the word item 1.1 of the image check already uses.

The order of a failed verify's checks is therefore: `missing-blocks`, `unreadable`, `hash`, the
rest of the image check (`no-record` to `vectors`), `header`. Each still gives status 55.
Test: T-A1-88 (a verify with the last block not sent: the log names `missing-blocks`, not
`hash`).

## Alternatives considered

- `UPDATE_VERIFY` checks the SHA-256 only, and the boot stage refuses what is not runnable: a
  host would learn of a bad package only after a reset, the radio would go through a rollback
  for a fault that was known before the switch, and the result would depend on a check the
  host cannot see.
- Fix the image record's layout in Annex M: the host never reads it (the package header is
  what it sends), so fixing it would only tie the firmware's build tools to the spec.
- Judge tag 9 on every `IDENTIFY`: a full read of a slot takes milliseconds of flash time on a
  command that hosts poll; the judgement points of item 4 cover every change of a slot.

## Consequences

- Spec A1.5 (integrity bullet), A1.8 ("Image check", "`UPDATE_VERIFY`", "Blocks sent again",
  "Boot-stage refusals", "Integrity in `IDENTIFY`", slot names in the `ROLLBACK` log line),
  Table A1-3 status 55, Table M-2 tag 9; tests T-A1-88, T-A1-89 and T-A1-90 (new).
- 0064 item 3.1 is read as point d states; 0064 is not reopened.

## Sources

- Decisions 0008, 0028, 0043, 0044, 0061, 0064, 0065; testbench finding SF-31 and the
  testbench points a–d on 0067 (relayed by the openg7250 session, 2026-11-21).
