# 0037 — g7250ctl in Rust, sharing the protocol crate with the firmware; an independent reference model

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session)
- Decided by: ON8ST (2026-11-20 advice; amended 2026-11-21 on the independence of the reference
  model, relayed by the openg7250 session)
- Date: 2026-11-20, amended 2026-11-21
- Builds on: 0019 (reference model), 0026 (g7250ctl as reference implementation), 0027
  (test bench), 0033 (firmware in Rust), 0034 (emulator run).

## Decision

1. `g7250ctl` is written in Rust and ships as one static binary for Linux, macOS and Windows.
2. **The firmware and `g7250ctl` share one Rust protocol crate**: they are the two ends of the
   same management channel.
3. **`product/refmodel/` is an independent Rust implementation**, written by the testbench
   session from the spec only. It never uses the firmware's protocol crate, and the firmware
   side never uses the reference model's code.
4. **The test bench stays Python 3, standard library only,** and uses neither the crate nor the
   reference model's code.

## Reason

Two independent readings of the spec catch each other's mistakes; a shared crate would hide a
mistake in both. The cost is that the protocol layout exists twice in Rust; a difference shows
up as failing tests, because the test bench runs against the reference model and against the
firmware image on the emulator (0034).

## Consequences

- `product/tools/g7250ctl/README.md`, `product/refmodel/README.md` and `testbench/README.md`
  state these rules.
- The earlier "Note on independence" (refmodel sharing the crate, the test bench the only
  independent check) is replaced by item 3.

## Sources

- ON8ST, 2026-11-20 and 2026-11-21, relayed by the openg7250 session; decisions 0019, 0026, 0027,
  0033, 0034.
