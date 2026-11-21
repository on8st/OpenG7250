# G7250 main-unit firmware

New firmware for the main unit of the Xiegu G7250. It implements the network interface
G7250-P2 (level 1) and later TCI with on-board DSP (level 2), as specified in `product/spec/`.

- Status: **build phase, milestones F1–F2 in code, not yet run on the emulator or the radio.**
  F1: boot stage, A/B slots, flash guards, safe state, watchdog, console. F2: CH394 on SPI6,
  DHCP or link-local address, discovery and Set IP on port 1024. F3 (started): the transmit
  gate; every TX request is rejected with `NOT_CALIBRATED` until the measured values are in the
  spec (ON8ST, 2026-11-20). Open questions: `docs/questions.md`.
- Only the main unit is reflashed. The controller keeps its stock firmware, and CTRL stays idle
  (decision 0023).

## Layout of this folder

| Path | What | Target |
|---|---|---|
| `crates/layout` | flash layout, write guard, EEPROM guard, image header, slot choice | host + MCU |
| `crates/safety` | the RF safe state as data; the transmit gate (`tx`) | host + MCU |
| `crates/proto` | **the protocol crate shared with `g7250ctl` (0037)**: management channel (`mgmt`: header and framing checks of A1.4, commands and status codes with their priority, HMAC-SHA256 trailer, every Annex M payload, both directions), discovery, rate limits, network configuration, DHCP messages. `no_std`, no allocation. `g7250ctl` uses it as a path dependency (`product/firmware/crates/proto`) | host + MCU |
| `crates/ch394` | CH394 driver (W5500-compatible registers), UDP | host + MCU |
| `tools/imgtool` | seal, check and merge images | host |
| `board/common` | raw-register board support: safe state, watchdog, console, reset cause | MCU |
| `board/boot` | boot stage at 0x08020000 | MCU |
| `board/app` | application (slot A, or slot B with `--features slot-b`) | MCU |
| `board/vendor` | embassy-stm32 0.6.0 with one patch: no backup-domain access (`board/vendor/README.md`) | MCU |
| `docs/` | flash layout and boot, emulator run, open questions | |

## Build

Needs Rust stable with the `thumbv7em-none-eabihf` target (`rust-toolchain.toml`) and
`shasum`. `./build.sh` runs the host tests (`cargo test`), builds the boot stage and both
application builds, seals and checks them (`g7250-imgtool`: every loadable byte inside its own
region, never in sector 0) and writes `out/` (`docs/memory-layout.md`, "Outputs").

## Language and toolchain (decision 0033, proposed)

Rust, `no_std`, on Embassy (`embassy-stm32`). Hardware drivers are written against
`embedded-hal` traits, so that protocol and state code also builds and is tested on the host
(`cargo test`), with test inputs written from the spec only.

The reference model (`product/refmodel/`) is independent of this firmware (0037). It is never a
code dependency: no crate, file or test of the firmware imports, reads or copies from it. It
appears only as a network peer in black-box interoperability tests (the `testbench/` runs
against the model and against the firmware over the network). The protocol crate
(`crates/proto`) is shared with `g7250ctl`, not with the model.

## Inputs

| Input | What it gives |
|---|---|
| `product/spec/spec/network-interface.md` | the network interface: parts A (management, safety core) and B (G7250-P2) |
| `product/spec/spec/part-c-tci-profile.md` | the TCI profile (level 2) |
| `product/spec/spec/register-map.yaml` | the G7250 register map (generated from the spec) |
| `his/` in the umbrella repository, cleared commits only | hardware facts: parts, pins, buses, safe states |
| `product/spec/decisions/` | design choices, one file each |

Clean room: the firmware is built only from these inputs and public datasheets. Never from
the reverse-engineering side (`re/`), from disassembly or from stock firmware.

## Platform rules from the HIS

Every rule below is a MUST from the HIS (cleared at `d199d33`; first listed at `2a38b7f`). They are listed so that nobody starts
without them; the HIS section is the authority.

- **Image placement** (HIS 10): the image starts at 0x08020000 with its vector table there.
- **Leave the vendor path intact** (HIS 15): never write sector 0 (0x08000000–0x0801FFFF), never
  change the option bytes, treat the EEPROM at I²C1 0x50 as read-only, do not use the Backup
  SRAM or the RTC backup registers.
- **FPGA** (HIS 15): it configures itself; never touch its configuration flash.
- **Safe state first** (HIS 85): set the RF control pins to their safe state before anything else
  in the RF section, and return to it whenever not transmitting. Bias DAC channels 2, 7, 8 at 0
  when not transmitting.
- **CTRL idle** (HIS 70): never transmit on USART2; keep PD5 high or high-impedance, never low.
- **Power** (HIS 20): switching off is the Main's job (power key PH5, power hold PH9; both
  HYPOTHESIS). Never enable a PMIC rail the HIS lists as off, never raise a listed rail.
- **FPGA bus** (HIS 80): never send commands 0x05/0x06, 0x09/0x0A.

## Structure (decision 0028, spec A1.8)

- a **boot stage** at 0x08020000 that picks one of two image slots and is never updated over
  the network; trial boot and rollback come with the update path (the F1 boot stage writes no
  flash);
- two **image slots** of 256 KiB (sectors 2–3 and 4–5);
- the **store** (configuration, calibration set, secret, boot record, boot counter, log) in
  sectors 6 and 7, used in turn (A1.8 "Store location"; 0061);
- a **streaming installer** that runs from RAM and writes an owner-supplied stock image over
  the network (spec A1.8a item 3): not built yet.

The flash layout and the boot sequence are in `docs/memory-layout.md`.

## Before the first flash

Every image is first tested in full on the RE side's emulator, as a black box: the product side
delivers the ELF and the spec-based tests and gets back pass or fail and the image's own
console and network output only (decision 0034, proposed). What to run and what passes:
`docs/emulator-run.md`.

The rollback drill F1 on the real radio (HIS 15) must show that the vendor path through the
controller can install both a stock image and an image of this firmware.

## Licence

GPL-3.0-or-later, © 2026 ON8ST; Apache-2.0 only if a vendor SDK forbids GPL (decision 0018).
