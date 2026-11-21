# 0043 — Update packages are linked for one slot; no update during a trial boot

- Status: proposed (to the repository owner, ON8ST); answers a firmware design question of
  2026-11-21
- Date: 2026-11-21
- Builds on: 0008 (U-3 to U-6), 0028 (boot stage, two slots), 0039 (Annex M).

## Context

The firmware links each application image for one slot. An update is staged in the other
slot (A1.8), so the host must send the image linked for the slot that is not running. A1.8 and
the package header (Table M-4) did not say so.

## Options considered

- **A. One image per package, the header names its slot; the host picks (chosen).** For: no
  change to the transfer; the radio refuses a wrong package before writing. Against: a release
  ships two packages.
- **B. One package with both images.** Against: twice the transfer; the radio discards half.
- **C. Position-independent images.** Against: hard to guarantee for `no_std` Rust on Cortex-M
  with a vector table at a fixed address, and not needed.

## Decision (proposed)

1. Table M-4 byte 52 names the target slot (0 either, 1 A, 2 B); identification tag 23 names
   the running slot. `UPDATE_BEGIN` with an image for the running slot gets the new status 63
   before anything is written.
2. `UPDATE_BEGIN` during an unconfirmed trial boot gets 58: the update is still in progress until
   confirmation or rollback (firmware's choice, confirmed).
3. A flash write failure during a transfer discards it, answers 55, logs it and leaves the
   running image and boot record unchanged (firmware's choice, confirmed).

## Consequences

- Spec A1.8, Tables A1-3, M-2, M-4; test T-A1-67. `g7250ctl` reads tag 23 and sends the package
  for the other slot; a release ships both.

## Sources

- Firmware question via openg7250, 2026-11-21; decisions 0008, 0028, 0039.
