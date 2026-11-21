# HIS gate review 8 — his/next 7cfd57e (DDC0 answers F4-Q1..Q5)

Scope: `git diff e34122a 7cfd57e -- his` (his/80, his/90). RE background:
`re/analysis/docs/his-f4-ddc0-answers.md`. I also read g7250-base
`main_unit/docs/peripherals.md` (FPGA bus and VersaClock 6) and `his/30`,
`his/85` and `his/90` at 7cfd57e for context. The gate rule applies (only the
FPGA's bus interface passes; stock timing, sequences, defaults and data stay
on the RE side), together with decisions D1 and D2 from review 1 and
decisions 0023 and 0025.

## The five points raised by the RE session

### 1. "Bus cycle": interface only, apart from the DRDY tag

- Strobe polarity, who drives D0–D7 for command, write and read, sampling
  while STB is high, and big-endian 24-bit values at one strobe per byte are
  all bus signalling. The RE background keeps out the two GPIOE_MODER values,
  the CMD-then-STB set order, and the observation that the routines add no
  delays. Those are exactly the stock details. The section gives no timing
  value, and set-up, strobe width and turnaround are left open. OK.
- "MUST switch D0–D7 back to outputs only after STB is low again, and should
  not drive them while any read is pending" is a conservative rule against
  bus contention that follows from "release time not known". It is not stock
  behaviour. OK.
- **R1 (required). DRDY "active edge is rising (HIGH: the factory image
  configures EXTI10 for the rising edge only)".** This is the same pattern as
  the ATU strobe in review 6 (P3): the interface fact is inferred from a stock
  configuration choice. With a roughly 48 kHz block signal, reading on the
  other edge, half a period later, could also work. So the setting shows what
  stock chose, not which edge the FPGA intends as "data ready". Change it to
  **MEDIUM**, worded as an inference ("inferred from the edge the factory
  image uses; the FPGA's timing is not documented"). Change the 90 row
  ("MEDIUM/HIGH") to MEDIUM.

### 2. F4-Q3: f_clk 122.88 MHz — keep it as the word scale, drop "half of 245.76 MHz"

Where the number comes from:

- The primary evidence is the scale the factory image uses for the phase
  words, f × 2³² / 122.88 MHz (g7250-base peripherals.md, FPGA bus section).
  The scale of a register word is part of the FPGA's bus interface, like the
  word format and the offsets. Inferred from stock code, so MEDIUM. This may
  pass.
- "Half of a 245.76 MHz clock-generator output" rests on the VersaClock 6
  dividers that stock writes, plus an *assumed* 26 MHz crystal. In `his/30`
  that row is tagged **HYPOTHESIS** (and "122.88 MHz not excluded"). A
  MEDIUM statement cannot cite a HYPOTHESIS as its basis. A board frequency
  derived from stock clock settings and an assumed crystal is also what D1
  rules out.

**R2 (required).** In 80, write roughly: "f_clk = 122.88 MHz as the scale of
the phase words (MEDIUM: inferred from the scale the factory image uses; the
physical FPGA clock is not recorded, see `30-i2c-devices.md`, HYPOTHESIS)."
Drop "half of a 245.76 MHz clock-generator output". In this form it is an
interface constant, not a clock-tree fact, and does not clash with D1.

### 3. F4-Q2: byte 0 and bytes 9–18 "not known", factory values kept out — the right line

Agreed. The values are per-unit calibration data and stock defaults. Under D2
the content of the calibration stays on the RE side. A power-on value would
not help: nothing in the sources shows what the FPGA holds before the first
0x01 frame, or that it works with it. Writing that it does would be a guess,
not a hardware fact.

Consequence for the product: DDC0 cannot be run on the radio as specified
until these bytes are understood or measured. The HIS should say so plainly,
so that the product side does not fill in values.

Two routes I see, neither of which is the gate's to decide:
- **(a) Measurement.** This is in the 90 rows already: capture with the
  factory firmware while settings change. It shows which bytes change with
  rate and frequency and which stay constant per unit, which is an interface
  fact.
- **(b) Reuse of factory calibration.** D2 already allows the HIS to say
  "factory calibration is stored at EEPROM 0x0800" as data on the device, if
  the first product must reuse factory calibration. Telling the product which
  calibration bytes go into frame bytes 9–18 would be a stock data mapping,
  and so a D2 revisit by ON8ST.

**R3 (required, safety).** Byte 0 holds status flags of unknown meaning, in a
frame that also carries the word at offset 19 (transmit, HYPOTHESIS). The HIS
must say that any 0x01 write with byte 0 or bytes 9–18 not understood happens
with the RF section in the safe state of `85-rf-control.md` (T/R lines at RX
levels, PA DAC in reset, chain A output disabled). A byte-0 bit enabling
transmit inside the FPGA cannot be excluded. Add one sentence to 80, next to
F4-Q2, together with the "DDC0 cannot run until …" sentence above.

### 4. Spectral-inversion rewording — correct

The earlier sentence ("plus a spectral-inversion flag in the frame") claimed
more than the evidence: per the RE background the flag is used in the MCU's
RX processing, and nothing shows it travels in the frame. "Not known whether
the frame carries it, and where" claims less and matches the sources. OK.

### 5. F4-Q5: "underrun" — no stock behaviour got in

The HIS does not use the word. It says only that the FPGA's behaviour on a
late read and the meaning of the 0x02 flags are not known. The firmware's
underrun logic stays on the RE side. OK.

## Further findings

- **R4 (required, safety, 90).** The two new F4 rows connect a signal
  generator to an antenna input, while the radio runs the factory firmware.
  That conflicts with the existing step rule in 90 ("a dummy load on both ANT1
  and ANT2"), which covers the HIS-19/F1 steps but not these rows. A transmit
  into a generator can destroy it. Give the F4 capture rows their own rule:
  - the generator is connected through an attenuator rated for full transmit
    power (or a generator input protected against reverse power);
  - the other antenna port carries a dummy load;
  - PTT, the keys and any CAT or network transmit command are not used;
  - the probes on PE0–PE10 are passive (high-impedance inputs, no injection).
- F4-Q3 "offsets 1 and 5 each set a receive-path tuning frequency (MEDIUM)" is
  consistent with review 5. There is no VFO or RIT leak, and the slot mapping
  stays on the RE side. OK.
- F4-Q4: I/Q order left open, and "six strobes per sample" is consistent with
  the command table. OK.

## Verdict

**CHANGES.** Required:
- R1: DRDY edge MEDIUM, worded as an inference, in 80 and 90.
- R2: f_clk as the phase-word scale (MEDIUM), with "half of 245.76 MHz"
  dropped.
- R3: the 0x01-write safety sentence, and "DDC0 cannot run until byte 0 and
  bytes 9–18 are understood".
- R4: a step-safety rule for the F4 captures with a signal generator.

Points 1, 3 (the line itself), 4 and 5 pass as written.
