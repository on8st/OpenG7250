# PA bias DAC and PA plant (Main)

> Sources cited as "g7250-base `<path>`" refer to the former data pin
> (historical name; see `docs/REPO_LAYOUT.md`, "Historical names").

`models/csharp/G7250PaPlant.cs` with `platforms/g7250_main_pa_plant.repl`,
loaded by `m1_run.py` (every stock Main boot) and `product_run.py`. It lets
the boot-time PA bias trim converge, so the stock Main reaches its superloop
(test A, `harness/g7250emu/superloop_run.py`).

## Since step 2 (PA): bus and pin level

Step 2 of `docs/PRODUCT-TESTBENCH-PLAN.md` (2026-11-21). The model now works
only through the hardware interface described in `his/85-rf-control.md`:

1. **Bias DAC on SPI2** (8-channel 10-bit serial DAC of the BU2506FV type):
   decodes the DAC words, latches on the LD rising edge (PB12), clears all
   outputs while RESET is low (PB14).
2. **PA plant**: drain current Idd from the bias codes: static current plus,
   per bias stage, gain × (code − dead zone) above the dead zone. Monotonic and
   zero extra current at code 0.
3. **Analog outputs**: the plant sets the input voltages of the ADC1 channels
   on the generic ADC model (`docs/ADC-model.md`), by the pins his/85 lists:
   PA1_C INP1 supply, PF11 INP2 forward, PA6 INP3 reflected, PC4 INP4 Idd,
   PB1 INP5 PA NTC, PC5 INP8 EALC, PA2 INP14 ALC. It republishes on every
   latched DAC word and on RESET.

The earlier metering feed, which wrote the scan results into a fixed RAM
buffer of the stock firmware and pulsed its DMA interrupt, is **removed**. The
firmware now reads the values through its own ADC and DMA set-up, on the
generic DMA and ADC models. The model contains no stock buffer address and no
stock sequence.

Fixture hooks for the test bench (`sensor.set` later): `SetSupplyVolts`,
`SetNtcPinVolts`, `SetRfPinVolts`, `GetDacCode(channel)`, `Idd`.

## From the analysis and the HIS (used as data)

| Fact | Source | Tag |
| --- | --- | --- |
| DAC word: 4-bit address (bit-reversed channel) << 2 \| value bits 9..8, then bits 7..0; LD high on PB12 latches; RESET low on PB14 forces all outputs low | his/85; Rohm BU2505FV/BU2506FV datasheet; g7250-base `main_unit/docs/peripherals.md` SPI2 | VERIFIED (format), HIGH (part family) |
| Bias channels: 2 driver, 8 final 1, 7 final 2; 10-bit | his/85; g7250-base `main_unit/docs/peripherals.md` | VERIFIED |
| ADC1 metering pins and channels | his/85 "ADC1 metering inputs"; ST datasheet pin table | VERIFIED |

## Assumed (HYPOTHESIS, placeholders, not measured)

| Parameter | Value | Note |
| --- | --- | --- |
| Static Idd | 0.05 A | |
| Driver dead zone / gain | 450 codes / 0.5 A per 150 codes | |
| Final dead zones / gains | 800 codes / 0.9 A per 150 codes | must keep the trim target below the 10-bit clamp |
| Board scaling to the pins | Idd 0.008059 A and supply 0.004835 V per 12-bit LSB at 3.3 V full scale; NTC at mid scale = 25 °C | values of the RE analysis; not in the HIS yet, so a bench `sensor.set` in physical units needs them in the HIS first |
| Supply | 13.8 V | |
| Forward, reflected, EALC, ALC | 0 V | no RF |

The plant is shaped for the trim loop to converge, not to predict real bias
codes. Real values need measurements on the radio.

## Result

2026-11-20 (metering feed): all three stages "Success!", superloop reached.
2026-11-21 (step 2, through the generic ADC and DMA): all three stages
"Success!", "Init Radio Server", 214 IWDG refreshes in the superloop test.
