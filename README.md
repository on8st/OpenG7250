# OpenG7250

Umbrella repository for all Xiegu G7250 work: reverse engineering of the stock
firmware, an emulator, a hardware-interface spec, a test bench, and a new
product: independent firmware speaking a documented network protocol.

Read [CLEANROOM.md](CLEANROOM.md) and [AGENTS.md](AGENTS.md) before working in
any part of this project. Then see [CONTRIBUTING.md](CONTRIBUTING.md),
[SECURITY.md](SECURITY.md) and [LICENSES.md](LICENSES.md).

## Layout

| Path | Side | Component | Content | Status |
|---|---|---|---|---|
| [`re/analysis/`](re/analysis/) | RE | — | Own RE work: decryption and Ghidra tooling, findings, memory map, protocol coverage | Present |
| `re/emu/` | RE | `g7250-emu` | Emulator running the stock Main and Control firmware | Planned submodule; integrated later |
| `his/` | Gate | `g7250-his` | Hardware-interface spec: hardware facts only | Planned submodule |
| `product/spec/` | Product | `g7250-spec` | Network interface spec based on the public OpenHPSDR protocols; details in `product/spec/` | Present (skeleton); content follows in a separate PR |
| `product/firmware/` | Product | `g7250-firmware` | New firmware, starting with a headless Main for AetherSDR | Later; never checked out in an RE work tree |
| `testbench/` | Shared | — | Automated test framework and design of the HIL test bench | Planned in the tree; written from `product/spec/` and `his/` only |
| `website/` | — | — | Project website | Possibly later (placeholder README) |
| `tools/` | Shared | — | Leak check, pre-commit hook, product-clone setup | Present |
| `decisions/` | — | — | Project-wide decisions | Present (0001–0006, proposed) |

## The three sides

```
   re/  ── understand the stock radio ──►  his/  ──►  product/
                                         (filter)     spec  ◄── host software (AetherSDR, …)
                                                      firmware
```

- **RE** (`re/`) studies the stock firmware and hardware. Findings become
  documents in `re/analysis/docs/`.
- **HIS** (`his/`) is the filter: only reviewed hardware facts pass from the RE
  side, never how the stock firmware does things.
- **Product** (`product/`) is what is being built. `spec/` defines how host
  software talks to the radio over the network; it is based on the public
  OpenHPSDR protocols and own design choices, not on Xiegu's protocols.
  `firmware/` implements `spec/` on the hardware described by `his/`.
  The product side does not read `re/`.
- **Test bench** (`testbench/`) tests against the specs as a black box, so it can
  be used against the emulator, the real radio and the new firmware alike. Tests
  that rely on stock-firmware behaviour stay in the emulator.

## External references

- [`fventuri/g7250-base`](https://github.com/fventuri/g7250-base) (Franco Venturi)
  is a separate project with its own methodology. It is a comparison source, not
  a part of this repository. Findings are shared with it through issues only; see
  [shared findings](re/analysis/docs/fventuri-findings.md) and
  [acknowledgments](re/analysis/ACKNOWLEDGMENTS.md).

## Getting started (RE side)

See [re/analysis/analysis/README.md](re/analysis/analysis/README.md) for the
decryption and decompilation workflow.
