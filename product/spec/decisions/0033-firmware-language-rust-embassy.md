# 0033 — Firmware language: Rust, no_std, on Embassy

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session)
- Decided by: ON8ST (relayed by the openg7250 session, 2026-11-20)
- Date: 2026-11-20
- Applies to: `product/firmware/` (build phase, 0028 Decision 4); `product/refmodel/` and
  `testbench/` as host-side test targets.

## Context

The main-unit firmware runs on an STM32H72x-class MCU ([HIS] HIS 00, HIS 10). It exposes
network parsers to the LAN (G7250-P2, the management channel, TCI) and must hold the RF section
in a safe state at all times ([HIS] HIS 85). Protocol and state code should be testable on a
host before any hardware exists (0019, 0027).

## Options considered

### A. Rust, `no_std`, on Embassy (`embassy-stm32`) (chosen)

- For: memory safety in code that parses untrusted network input.
- For: drivers written against `embedded-hal` traits, so protocol and state code builds and is
  tested on the host (`cargo test`), against `testbench/` and `product/refmodel/`.
- For: the RF safe state and the TX conditions can be expressed in types (for example, a
  transmit path that can only be built from an armed, calibrated state).
- Against: less existing openHPSDR server code to draw on.

### B. C

- For: most openHPSDR server software is C; the vendor HAL is C.
- Against: no memory safety for the network parsers; host testing needs more scaffolding.

## Decision

Option A. Rust, `no_std`, on Embassy with `embassy-stm32`; hardware drivers against
`embedded-hal` traits; protocol and state code tested on the host with `cargo test`.

- AetherSDR's G7250 backend stays C++/Qt (AetherSDR's own language; annex AetherSDR §4).
- The language of `g7250ctl` is still open.
- Licence as 0018 (GPL-3.0-or-later).

## Consequences

- `product/firmware/README.md` names the language and toolchain.
- No effect on the network interface spec.

## Sources

- ON8ST, 2026-11-20, relayed by the openg7250 session.
- [HIS] HIS 00, HIS 10, HIS 85 (`9d10e72`); decisions 0018, 0019, 0027, 0028.
