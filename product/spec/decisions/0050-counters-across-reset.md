# 0050 — Fault counters across a reset: pending hardware, part of the TX gate

- Status: proposed (to the repository owner, ON8ST)
- Date: 2026-11-21
- Builds on: 0022 §27 (counters survive resets without power loss), 0035 (Annex P, freeze rule).

## Context

A3.5 requires the fault entry counters, the reset counters and `TRIP_COUNT` to survive `RESTART`
and every reset without power loss. The firmware keeps them in AXI SRAM. Whether that content
survives the vendor bootloader, which runs before the boot stage, is not known; it goes to the
HIS as an open item. The P-SYS-2 placeholder ("lost at every reset") contradicted the rule.

## Decision (proposed)

1. New Annex P parameter P-SYS-9: survival of the firmware's RAM content across a software reset
   and a watchdog reset, through the vendor bootloader; radio check by writing a pattern,
   resetting both ways and reading it back in the boot stage.
2. P-SYS-9 joins the freeze rule: TX stays locked until it is shown, so a lost count can never let
   a `RESTART` lift an escalation to class 2.
3. P-SYS-2 now covers the reset cause and the log only.
4. Test T-A3-60 (needs the radio).

## Sources

- openg7250, 2026-11-21; decisions 0022 §27, 0035; [HIS] HIS 10, HIS 90.
