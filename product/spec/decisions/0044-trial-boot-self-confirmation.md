# 0044 — Self-confirmation of a trial boot

- Status: proposed (to the repository owner, ON8ST); answers a firmware question of
  2026-11-21 (7ee0b1c)
- Date: 2026-11-21
- Builds on: 0008 (U-6), 0020 §24 (60 s), 0022 §10, 0028 (boot stage), 0043.

## Decision (proposed)

The firmware's criterion is accepted and written into A1.8:

1. A trial image confirms itself, by writing the boot record, after at least 10 s with the
   network up (CH394 initialised, management socket open), the main loop running, the watchdog
   serviced and no class 2 fault. No host is needed.
2. Not confirmed 60 s after start: a commanded reset (A1.9), and the boot stage starts the
   previous image.
3. A reset of the trial image before confirmation, for any other reason, also leads to the
   previous image at the next start, because the boot record still marks the trial as
   unconfirmed.
4. Every rollback is logged. Test T-A1-68.

## Sources

- Firmware question via openg7250, 2026-11-21; decisions 0008, 0020 §24, 0022 §10, 0028, 0043.
