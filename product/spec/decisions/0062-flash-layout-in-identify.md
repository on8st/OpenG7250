# 0062 — The flash layout is reported in IDENTIFY (tag 25)

- Status: proposed (to the repository owner, ON8ST); answers testbench finding SF-27,
  2026-11-21
- Date: 2026-11-21
- Builds on: 0028 Decision 3 (slot layout is firmware design), 0043 (slot-linked images,
  tag 23), 0061 and its addendum (store in sectors 6 and 7; slot size and status 61);
  [HIS] HIS 15 (128 KiB sectors).

## Context

T-A1-77 needs to know which sectors form the staging slot and the store, and T-A1-78 needs
the slot size. A1.8 leaves the slot layout to the firmware, and identification reported none
of it, so a host or the bench could only take it from the firmware's documents.

Options:

- (a) a new `IDENTIFY` tag with the layout; the layout stays the firmware's but is visible;
- (b) fix the layout in the spec (slot A from 0x08040000 and slot B from 0x08080000, 256 KiB
  each, store in sectors 6 and 7).

## Decision (proposed)

1. Option (a), as the testbench advised. `IDENTIFY` tag 25, length 6, in 128 KiB sectors
   ([HIS] HIS 15): slot A first sector (u8), number of sectors (u8); slot B the same; the
   store the same. A1.5 and Annex M Table M-2.
2. The firmware keeps the layout as a design choice (0028), within what the spec already
   requires: sector 0 is the vendor bootloader's and sector 1 the boot stage's (A1.8), the store
   is in sectors 6 and 7 (0061), and the regions do not overlap.
3. The slot size that status 61 checks against (0061 addendum) is the slot's number of
   sectors from tag 25 times 128 KiB.
4. Tests: T-A1-77 and T-A1-78 take the layout from tag 25; new T-A1-79 checks the tag itself.

## Alternatives considered

- (b): would freeze a firmware design choice into the interface for no gain to a host. The
  layout can still change with a later boot stage without a spec change.

## Consequences

- Spec A1.5, Annex M Table M-2 tag 25, T-A1-77, T-A1-78, T-A1-79.

## Sources

- [HIS] HIS 15 (`9d10e72`); decisions 0028, 0043, 0061; testbench `docs/spec-findings.md`
  SF-27.
