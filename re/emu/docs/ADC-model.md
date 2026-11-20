# ADC model (Main ADC1)

`models/csharp/G7250Adc.cs` with `platforms/g7250_main_adc.repl`. It replaces
Renode 1.17's `STM32H7_ADC` (`adcM1S2`), whose raw values were wrong (DR read 2
for a set input voltage). Step 1c of `docs/PRODUCT-TESTBENCH-PLAN.md`.

Loaded by `product_run.py` (bench and product runs) and, since step 2, by
`m1_run.py` for every stock Main run: the PA plant drives its analog inputs
(`docs/PA-plant-model.md`).

Sources: ST RM0468 (public), ADC chapter, DMAMUX1 request table (adc1_dma = 9)
and interrupt table (ADC1_2 = 18). Channel numbers are the datasheet's INPx
inputs; the board pins per channel are in `his/85-rf-control.md` ("ADC1
metering inputs").

## What it models

- ADC1 registers ISR, IER, CR, CFGR, CFGR2, SMPR1/2, PCSEL, LTR/HTR, SQR1–4,
  DR and the rest as plain storage; ADC2 registers are stored but do not
  convert; the common registers CSR, CCR and CDR at +0x300.
- Power-up: DEEPPWD (set after reset), ADVREGEN with LDORDY, ADCAL completes
  at once, ADEN with ADRDY, ADDIS, ADSTART, ADSTP.
- Regular sequence of up to 16 ranks, single and continuous mode; one
  conversion per 1 / `conversionRate` of virtual time (default 20 kHz).
- Resolution 16, 14, 12, 10 and 8 bit, both RES encodings of RM0468.
- raw = round(V / Vref × (2^bits − 1)), clamped; Vref 3.3 V by default. Input
  voltages per channel come from `SetChannelVoltage(channel, volts)` (harness,
  test-bench fixture or a plant model); unset channels read 0 V.
- EOC, EOSMP, EOS; OVR when DR is not read before the next result without DMA
  (OVRMOD selects keep or overwrite); a DR read clears EOC. Interrupt line from
  IER.
- DMA: a request edge per conversion with DMNGT one-shot (01) or circular (11).

## Not modelled (documented assumptions)

Conversion time from sampling time and ADC clock (fixed rate instead),
external triggers (a sequence started with EXTEN ≠ 0 runs at the same fixed
rate and logs "not modelled: ADC1/CFGR EXTEN" once), injected conversions,
oversampling, offsets, analog watchdogs, differential inputs, dual mode, ADC2
conversions, calibration values, left alignment, ADC3.

## Self-test

`firmware-tests/bench/adc_test.c`, with the bench inputs INP1 = 0.5 V,
INP4 = 1.65 V, INP5 = 3.3 V: LDORDY, calibration, ADRDY; a software-read
12-bit sequence (620 and 2048); EOS and the end of a single sequence; 16-bit
full scale (0xFFFF); continuous conversions into a circular DMA ring; ADSTP and
ADDIS. 11 of 11 pass.
