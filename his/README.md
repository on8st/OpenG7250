# his/ — G7250 hardware-interface specification (HIS)

The HIS is the **gate** of OpenG7250: the only channel through which knowledge
from the RE side reaches the product side. It records **hardware facts only**
(parts, buses, addresses, pins, wiring, electrical values, the FPGA's bus
interface), never how the stock firmware does things.

## Role in the umbrella

```
re/analysis/   RE findings on the stock firmware and hardware      (RE side)
re/emu/        emulator + harness; observes the hardware interface (RE side)
     │  hardware facts, reviewed at the gate ▼
his/           THIS spec — hardware facts only                      (gate)
     │  ▼
product/firmware/  new firmware; reads his/ for hardware facts      (product side)
product/spec/      the network interface, protocol-neutral towards the
                   stock firmware (public OpenHPSDR-based); not derived from his/
```

Each fact enters only after the gate review (kept on the RE side) confirms it
is a hardware fact and not stock behaviour or configuration. Every section states
its evidence: an emulator observation, a public datasheet, or, where nothing
else exists yet, that the fact rests on disassembly.

## Status

Imported from the former `g7250-his` repository (tag `his-draft0`). **Cleared at
72591aa by gate review 9 (short check)**; reviews in `re/analysis/docs/his-gate-review-2.md`
to `his-gate-review-9.md`, kept on the RE side. The product side may read `his/` at
that commit; later changes go through the gate first.

## Sections

- `00-overview.md`      — scope, conventions, process
- `10-reset-clock.md`   — MCU classes, image placement
- `15-stock-recovery.md` — keeping the factory firmware reinstallable
- `20-power.md`         — power hold, power key, PMIC and rails
- `30-i2c-devices.md`   — I²C bus and address map
- `40-ethernet.md`      — CH394 LAN controller
- `50-audio.md`         — codecs and audio path
- `60-display-input.md` — display, SDRAM, touch, keys, encoders
- `70-interlink.md`     — Main↔Control serial link (wiring)
- `80-rf-fpga.md`       — FPGA bus on GPIOE
- `85-rf-control.md`    — Main RF control: T/R, filters, PA bias DAC, metering, antennas; safe state
- `90-needs-hardware.md` — open points that need the real radio; safe order of the hardware steps

## Rules

No stock firmware code, no decompiler output, no stock behaviour. Follow the
umbrella [`CLEANROOM.md`](../CLEANROOM.md) and [`AGENTS.md`](../AGENTS.md).
Local commits on branches; push and PR only with an explicit go from the
repository owner (umbrella `AGENTS.md` rule 4).
