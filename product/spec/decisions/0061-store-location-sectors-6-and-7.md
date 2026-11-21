# 0061 — The store is in flash sectors 6 and 7, checked in F1

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session)
- Date: 2026-11-21
- Builds on: 0024 R5 (no secret survives a return to stock), 0028 Decision 3 (the slot layout
  and the place of configuration are firmware design), 0035 (P-SYS-5), 0039 (calibration set);
  [HIS] HIS 15 rules 1–5 (`9d10e72`).

## Context

The spec required a non-volatile store for configuration (A1.6 C-4), the calibration set
(A1.12), the boot record (A1.8) and the boot counter (B10.2 item 5), but did not name where it
is; 0028 left that to firmware design. The firmware's layout (`product/firmware/docs/memory-layout.md`)
puts the store in sectors 6 and 7 (0x080C0000–0x080FFFFF), used in turn so that a compaction
never erases the only copy. HIS 15 rule 5 has an open check (HYPOTHESIS) whether the vendor
bootloader uses sector 6 as a staging area; the drill F1 settles it (P-SYS-5).

## Decision

1. The store is in flash sectors 6 and 7, used in turn, as the firmware has it. The spec names
   it in A1.8 ("Store location"); A1.12 item 2 and Annex P P-SYS-5 point to it.
2. F1 checks sector 6. If the vendor bootloader uses it only as staging during a vendor
   update, the store there is overwritten by an update that replaces this firmware anyway,
   which is acceptable (0024 R5). If it acts on sector 6 at every start, the store location is
   decided again.
3. Nothing else changes: no store in sector 0 or 1, the EEPROM, the Backup SRAM or the RTC
   backup registers ([HIS] HIS 15 rules 1, 3, 4).

## Addendum: updates keep clear of the store (testbench SF-26, 2026-11-21; proposed)

4. An image of this firmware longer than the slot it is linked to gets status 61 before
   anything is written; the slots end where the store begins. Order of the command-specific
   codes of `UPDATE_BEGIN` (firmware question, 2026-11-21): 51, 56, 63, 61, 52 (A1.8, Table
   A1-3 paragraph); so a too-large older image gets 61, and a foreign image before the F1 gate
   gets 56 whatever its size (as T-A1-61).
5. An update to an image of this firmware erases or writes sectors 6 and 7 only as a store
   write or compaction. Tests T-A1-77 (flash.log on the emulator) and T-A1-78. T-A1-55 (a
   foreign image may fill sectors 1–7, the one-way installation) is unchanged: a return to
   stock erases the store on purpose (0024 R5).

## Consequences

- Spec A1.8, A1.12 item 2, Annex P P-SYS-5; Table A1-2 status 61; T-A1-77, T-A1-78.

## Sources

- [HIS] HIS 15 (`9d10e72`); firmware design `product/firmware/docs/memory-layout.md`;
  decisions 0024, 0028, 0035, 0039.
