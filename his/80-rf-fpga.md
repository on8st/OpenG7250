# HIS 80 — RF front-end / FPGA interface

**Evidence:** signals and command codes from g7250-base peripherals doc
(VERIFIED in the firmware analysis, i.e. **rests on disassembly**). Emulator
(re/emu D2): a fused GPIOE+FPGA model of this bus, built from the same
disassembly, is accepted by the factory image; this is consistency, not hardware
evidence, and the real FPGA version is unknown. The 0x01 byte offsets are
observed: they are the bytes the factory image writes over this bus in the
emulator (re/emu HIS-02 probe, with settings changes), which agrees with the
disassembly. The roles of the three words are inferred from the values written;
the FPGA's use of the words is not observed. The NCO clock and the absolute
sample rate are not measured.

The RF I/Q front end is an **FPGA reached over an 8-bit parallel bus on GPIOE**
(no FMC), driven by the MCU in software.

## Signals
| Signal | Pin | Use |
| --- | --- | --- |
| D0–D7 | PE0–PE7 | bidirectional data; the MCU switches the pin direction |
| CMD | PE8 (with PE9) | pulsed together to latch a command byte |
| STB | PE9 | per-byte strobe |
| DRDY | PE10 | rising edge signals sample data ready; PE10 can raise EXTI10 |

## Commands
| Code | Function |
| --- | --- |
| 0x00 | loopback/test register: a written byte reads back |
| 0x01 | 24-byte config frame: byte 0 status flags; three big-endian 32-bit NCO words at byte offsets 1, 5 and 19 (HIGH); bytes 9–18 other fields incl. per-rate calibration (meanings unknown); byte 23 most likely the rate index (MEDIUM). F4-Q2: the meaning of byte 0 and of bytes 9–18 and safe values for them are **not known**; the values the factory image writes are per-unit data and are not given here |
| 0x02 | status read (flag bits + three 16-bit values incl. ADC peak) |
| 0x03 | TX I/Q write — two signed 24-bit values per sample |
| 0x04 | RX I/Q read — two signed 24-bit values per sample, 8/4/2/1 per DRDY for rate index 0–3 |
| 0x05/0x06, 0x09/0x0A | exist (two pulse commands); role unknown. **MUST NOT** be sent |
| 0x08 | 3-byte FPGA version read |

## 0x01 frame and DDC0 (F4-Q2)

- **DDC0 cannot run until byte 0 and bytes 9–18 of the 0x01 frame are
  understood.** Tuning DDC0 or setting its rate needs a full frame, and safe
  values for those bytes are not known. The route is a measurement on the radio
  (`90-needs-hardware.md`). Reusing the unit's factory calibration values would
  be another route, but it needs a decision by ON8ST and is not open.
- **Safety:** any 0x01 write while byte 0 is not understood happens only with
  the RF section in the safe state of `85-rf-control.md`: byte 0 may hold a
  transmit enable, which is not excluded.

## Bus cycle (F4-Q1, F4-Q4, F4-Q5)

Evidence for this section: the bus routines of the factory image (firmware
analysis, **rests on disassembly**); no measurement on the radio, no FPGA
documentation. Timing values the factory image happens to produce are not
given here (stock timing).

- **Strobe polarity (F4-Q1):** STB (PE9) and CMD (PE8) are active-high
  pulses: a command byte is latched by raising CMD and STB and then lowering
  both; each data byte by raising STB and then lowering it (MEDIUM: inferred
  from the factory image's pin sequence; the FPGA's sampling edge is not
  documented).
- **Who drives D0–D7 (F4-Q1):** the MCU drives D0–D7 for the command byte and
  for data it writes (0x00, 0x01, 0x03). For a read (0x00 read-back, 0x02,
  0x04, 0x08) the MCU first switches D0–D7 to inputs, then the FPGA drives the
  byte while STB is high; the MCU samples D0–D7 during the high phase of STB
  (MEDIUM, same evidence). When the FPGA releases D0–D7 after a read strobe is
  **not known**: new firmware MUST switch D0–D7 back to outputs only after STB
  is low again, and should not drive them while any read is pending.
- **Set-up time, strobe width, turnaround time (F4-Q1):** **not known**; they
  need a measurement on the radio (`90-needs-hardware.md`).
- **0x04 RX read (F4-Q4):** each 24-bit value is read as three bytes, most
  significant byte first (big-endian), one STB per byte; two values per sample,
  so six strobes per sample (MEDIUM, same evidence). Which of the two values is
  I and which is Q is **not known**. Samples per DRDY: 8/4/2/1 for rate index
  0–3 (MEDIUM, as below).
- **DRDY (F4-Q5):** the active edge is most likely rising (MEDIUM: inferred
  from the interrupt edge the factory image chose, EXTI10 rising only; not
  measured). What the FPGA does when a read
  comes late (a missed or lost block, an overrun indication in the 0x02 status
  flags) is **not known**; the meaning of the 0x02 flag bits is not known.

## Timing / data
- **NCO:** 32-bit phase words, f × 2³² / f_clk (formula VERIFIED), with
  f_clk = 122.88 MHz as the phase-word scale (MEDIUM: inferred from the scale
  the factory image uses; not measured). The words are given for the first Nyquist zone (0 to f_clk/2).
  Whether the 0x01 frame carries a spectral-inversion flag, and where, is **not
  known** (F4-Q2).
- **Which NCO word is DDC0 (F4-Q3):** offsets 1 and 5 each set a receive-path
  tuning frequency (MEDIUM, see "Open"); which of them drives ADC0/DDC0 is
  **not known** and needs the capture of `90-needs-hardware.md`.
- **Sample rate:** rate index 0–3 → 384/192/96/48 kS/s I/Q; DRDY (PE10) at
  ~48 kHz (MEDIUM; absolute rate unmeasured).

## Open

- Strobe polarity and timing, and bus turnaround (who drives D0–D7 when): not
  measured. Bus contention is a hardware risk.
- 0x01 frame: the meaning of bytes 9–18 and safe values for the per-rate
  calibration bytes among them are unknown; the meaning of the flag bits in
  byte 0 is unknown; the position of the spectral-inversion flag in the frame
  is unknown.
- The three NCO words: offsets 1 and 5 each set a receive-path tuning frequency,
  independently of each other (MEDIUM: inferred from the values the factory
  image writes in the emulator; FPGA use not observed); offset
  19 is a third tuning word, most likely for the transmit path (HYPOTHESIS: the
  transmit side was not exercised). Which physical receive path each of offsets
  1 and 5 drives, and the transmit role of offset 19, need a capture on the
  radio including transmit (`90-needs-hardware.md`).
