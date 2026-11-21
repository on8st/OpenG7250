# Answers to the firmware's HIS questions F4-Q1..Q5 (DDC0 stream, RE side)

RE background for the HIS 80 and HIS 90 changes on his/next. Questions:
`product/firmware/docs/questions.md`, "HIS questions for the DDC0 stream"
(g7250-fw tree). Sources: g7250-base `main_unit/docs/peripherals.md` (FPGA
bus section, former data pin, historical name), and a reading of the factory
image's bus routines on the RE side (local, image outside any repository).
Conclusions only, no code or listings.

| Id | HIS (interface fact) | RE evidence | Kept on the RE side |
| --- | --- | --- | --- |
| F4-Q1 polarity | STB and CMD active-high pulses (MEDIUM) | The command-latch routine sets PE8, then PE9, then resets both in one BSRR write; the byte-strobe helpers set and reset PE9. | — |
| F4-Q1 direction | MCU drives D0–D7 for command and writes; switches them to inputs for reads; FPGA drives during STB high (MEDIUM) | The bus routines rewrite GPIOE_MODER between an all-outputs and a D0–D7-inputs value; reads sample GPIOE_IDR between STB set and STB reset. | The two MODER values used. |
| F4-Q1 timing | Set-up, width, turnaround not known (open, radio) | The factory routines add no delays: the strobe width and set-up are whatever the instruction timing gives. | That observation (stock timing). |
| F4-Q2 | Byte 0 and bytes 9–18 meaning and safe values not known; spectral-inversion flag position not known | peripherals.md: byte 0 status flags; bytes 9–18 per-rate calibration bytes taken from the unit's calibration block; byte 23 rate index; the spectral-inversion flag is used by the factory RX reader, i.e. in the MCU's processing, so whether the frame carries it at all is open. The previous HIS 80 sentence "plus a spectral-inversion flag in the frame" claimed more than the evidence and is reworded. | The values the factory image writes (per-unit calibration data, stock defaults). |
| F4-Q3 | DDC0's word not known; f_clk 122.88 MHz MEDIUM | peripherals.md: the word at the main-VFO slot goes to offset 1, the other VFO's to offset 5 (HIS-02 run 5 agrees); which physical receive path each drives is not in the code. | Which VFO slot maps to which offset (stock software structure). |
| F4-Q4 | Big-endian 24-bit values, one STB per byte, 6 strobes per sample (MEDIUM); I/Q order not known | The RX read routine shifts the first byte read to bits 23–16, the second to 15–8, the third to 7–0, sign-extends, and does this twice per sample into two slots. Which slot is I is decided later in the DSP, not in the read. | — |
| F4-Q5 | DRDY rising edge (HIGH); late-read behaviour and 0x02 flag meaning not known | EXTI10 configured rising-edge only. The "FPGA buffer underrun" message is the factory firmware's own software flag for an unconsumed block, not a bit the FPGA reports; the two 0x02 flag bits are read into the TRX struct with no known meaning. | The firmware's underrun logic (stock behaviour). |

Emulator-only evidence: the fused GPIOE+FPGA model (re/emu, step 2) follows
HIS 80 plus the hypotheses of `re/emu/docs/FPGA-model-notes.md`; the factory
image accepts it. That is consistency with a model, not evidence about the
FPGA, and none of the HIS statements above rests on it.
