# 0055 — Flash work: exempt from the burst rule, never while armed or keyed

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers a firmware
  follow-up on 0054, 2026-11-21
- Date: 2026-11-21
- Builds on: 0054, 0039 (calibration set), A1.6, A1.8, A1.10.

## Context

Flash writes and erases in the network task (management writes, a store compaction, an update
block, the boot or trial record) stall it, a sector erase for up to seconds. During that time
High Priority packets beyond the two that port 1027 holds can be lost (firmware estimate).

## Decision

1. The firmware's exemption is accepted: while a management write that writes flash, a store
   compaction or a firmware update is in progress, 0054's no-loss rule does not apply, and a P2
   session may end by its watchdog.
2. Added for safety: flash work never starts while TX is armed or keyed on any level.
   `SET_CONFIG` while armed first unkeys and disarms (A1.6); `SET_FIRST_SECRET` and
   `CHANGE_SECRET` are now refused with status 50 while armed or keyed, like the calibration
   commands and updates. A lost stop can then never leave the radio transmitting.
3. The radio starts no flash work on its own during a P2 session, except the trial-boot
   confirmation, during which no arm is possible; a compaction runs only inside the management
   write that needs it.
4. The host learns of it from the management reply, sent after the flash work, and from the update
   state.
5. T-B5-20 excludes flash work; new test T-B5-21.

## Sources

- Firmware via openg7250, 2026-11-21; decisions 0039, 0054.
