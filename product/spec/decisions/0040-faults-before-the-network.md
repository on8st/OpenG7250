# 0040 — Faults found before the network are reported on the debug console only

- Status: proposed (to the repository owner, ON8ST); answers firmware question SQ-F1-4
- Date: 2026-11-21
- Builds on: 0028 (boot stage), 0035 (freeze rule); [HIS] HIS 10, HIS 40, HIS 85, HIS 90
  (`6e2208a`).

## Context

The firmware checks the MCU's core supply configuration before anything else: it can be
written only once per power-on, and one that does not match the board wiring can harm the part
([HIS] HIS 10). On a mismatch or a time-out the boot stage holds the safe state before any
network, so a host cannot be told through identification.

## Options considered

- **(a) Console only (chosen).** The boot stage reports a fixed line on the debug console
  (USART1, [HIS] HIS 90) and stays in the safe state. For: nothing runs on a supply the
  firmware believes wrong. Against: the owner sees only that the radio does not appear on the
  network, unless a console is connected.
- **(b) Minimal network at the reset voltage scale, for IDENTIFY only.** Against: runs the
  network chip and the network code on a supply the firmware believes wrong. Rejected.
- **(c) A stored fault code shown by a later boot.** Against: needs a flash write on that
  supply, and a later boot that finds the same fault stops at the same place. Rejected for r1.0.

## Decision (proposed)

Option (a), spec A3.5a: safe state of A3.6, CH394 held in reset, no flash write; one ASCII line
per second on USART1 TX (PB6) at 115200 8N1:
`G7250 FAULT SUPPLY <PWR_CR3 as 8 hex digits> <MISMATCH|TIMEOUT>` CR LF, hex in upper case
(for example `G7250 FAULT SUPPLY 0100002E MISMATCH`); until power is removed.
The way out is the vendor path (A1.8a item 9). Test T-A3-59.

## Consequences

- Spec A3.5a (new), test T-A3-59.
- `g7250ctl` and AetherSDR documentation: a radio that never appears on the network may be in
  this state; check the console or reinstall through the vendor path.

## Sources

- Firmware session question SQ-F1-4 (via openg7250, 2026-11-21); [HIS] HIS 10, HIS 40, HIS 85,
  HIS 90 (`6e2208a`); decisions 0028, 0035.
