# 0034 — Every firmware image is tested on the emulator, as a black box, before the radio

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session)
- Decided by: ON8ST (relayed by the openg7250 session, 2026-11-20)
- Date: 2026-11-20
- Product side of project decision 0007 (root `decisions/` of the umbrella repository, not in
  this clone). This file records only what the product side does and receives.

## Decision

1. Before any product firmware image runs on the radio, it is tested in full on the emulator
   of the RE side, as a black box.
2. **The product side delivers:** the image as an ELF file, and the tests to run, written from
   the spec (Annex X3, part C tests, `testbench/`).
3. **The RE side returns only:** pass or fail per test, and the image's own output: its console
   and its network traffic. Nothing about the emulator's internals or about stock behaviour
   comes back (clean room, AGENTS.md rules 1, 7).
4. This comes on top of the tests against the reference model (0019) and does not replace the
   hardware steps that need the radio (HIS 90, the F1 drill).

## Consequences

- `product/firmware/README.md` and `testbench/README.md` mention the emulator run as a gate
  before the radio.
- No effect on the network interface spec.

## Sources

- ON8ST, 2026-11-20, relayed by the openg7250 session; project decision 0007 (not read here).
- Decisions 0019, 0027, 0028; [HIS] HIS 90 (`9d10e72`).
