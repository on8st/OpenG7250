# HIS 85 — Main RF control (T/R, filters, PA bias, metering)

**Why this section exists.** Under decision 0023 the first product is the Main
firmware, and the Main drives the RF section: T/R switching, the filter relays
and the PA bias DAC. Pins left at reset defaults or driven by accident can key
the PA or bias it without drive, with no protection. Even an RX-only build MUST
hold these parts in the safe state below.

**Evidence:** g7250-base `main_unit/docs/peripherals.md` (SPI1, SPI2, ADC1 and
DAC1, GPIO pin map) and `main_unit/docs/flow.md` (power-off), **rest on
disassembly**; the DAC word format matches the Rohm BU2505FV/BU2506FV datasheet.
Tags per line. Nothing here is measured on hardware.

## Safe state at reset

New Main firmware MUST establish this state before anything else in the RF
section, and return to it whenever it is not transmitting.

| Part | Pins | Safe state | Tag |
| --- | --- | --- | --- |
| T/R lines | PF4, PF6, PC7, PH11 | RX levels: PF4 high; PF6, PC7, PH11 low | levels VERIFIED; function of each line HYPOTHESIS |
| SPI1 chain A (TX low-pass bank + T/R) | latch PA11, output enable PA10 | output enable inactive (PA10 high) until a frame with bit 7 clear has been latched | HIGH |
| SPI1 chain B (RX band-pass, preamp, attenuator) | latch PH12, output enable PH10 | output enable inactive (PH10 high) until a known frame has been latched | HIGH |
| ATU latches | CS/latch PD13, PG2, PG5 | leave unwritten; no safe value is known | HIGH (pins); safe value unknown |
| SPI2 PA DAC | RESET PB14 | PB14 low forces all outputs to VrefL; bias channels 2, 7, 8 at 0 whenever not transmitting | VERIFIED |
| DAC1 OUT1 | PA4 | RF-gain control voltage; safe level unknown | VERIFIED (pin) |
| PD12 | PD12 | unknown output (TIM4_CH1 capable); keep low | HIGH (pin); role unknown |
| PA12 | PA12 | unknown output; no safe level known | VERIFIED (output); role unknown |
| CH394 | RST# PB5 | held low (in reset) until the product brings it up (`40-ethernet.md`) | VERIFIED |
| Main codec | reset PB10 | held in reset unless used (`50-audio.md`) | VERIFIED |
| USART2 TX | PD5 | idle level, never low (`70-interlink.md`) | VERIFIED (pin) |

The output-enable lines PA10 and PH10 are active low: high disables the
shift-register outputs (HIGH: interpretation of the power-off writes). The chip type behind
them is of the 74HC595 kind (HYPOTHESIS).

## SPI1 — filter and switch shift registers (HIGH)

- SPI1 is transmit-only: SCK PG11, MOSI PD7 (AF5), no MISO; 8-bit frames,
  SPI mode 0.
- **Chain A** (8 bit; latch PA11, OE PA10): bits 0–6 select one TX low-pass
  filter each (7 bands, HF to 6 m; HIGH); **bit 7 is a T/R relay or PA-enable
  line** (HIGH). Bit 7 MUST be clear in every frame while not transmitting.
- **Chain B** (24 bit, 3 bytes MSB first; latch PH12, OE PH10): RX band-pass
  and preselector segments, preamp selector, attenuator (HIGH; exact bit
  meanings MEDIUM to HYPOTHESIS).
- **ATU latches** (8 bit each, the CS line is the latch): PD13 (network style,
  bypass/enable), PG2 and PG5 (the two reactance banks) (HIGH).
- **ATU strobe idle level (F1-Q5):** the strobes idle high (HIGH: the factory
  image drives PD13, PG2 and PG5 push-pull high from its GPIO set-up). Whether
  the board has pull resistors on them is **not known**; between reset and the
  firmware's GPIO set-up the MCU pins are high-impedance, so a floating strobe
  cannot be excluded (`90-needs-hardware.md`).

## Antennas

- Two antenna ports on the rear, ANT1 and ANT2 (SO-239, 50 Ω) (VERIFIED,
  teardown video and manual).
- The antenna selection is most likely bits 4–5 of the PD13 ATU latch
  (MEDIUM). Like the other ATU bits, it has no known safe value; leave the latch
  unwritten until this is confirmed.

## SPI2 — PA bias and drive DAC (VERIFIED)

- Part: 8-channel 10-bit serial DAC of the BU2506FV type (family HIGH, part
  MEDIUM). Bus: SCK PB13, MOSI PB15 (transmit-only, mode 0); load strobe LD
  PB12 (high latches the word); RESET PB14 (active low, forces all outputs to
  VrefL).
- Channel map:

| Ch | Function |
| ---: | --- |
| 1 | drive level |
| 2 | driver-stage bias |
| 3 | fan |
| 4 | unused |
| 5 | unknown (enable or reference, HYPOTHESIS) |
| 6 | power reference |
| 7 | final stage 2 bias |
| 8 | final stage 1 bias |

- Bias channels 2, 7 and 8 MUST be 0 whenever the radio is not transmitting.
  Safe bias values for transmit are not part of this spec (they depend on
  per-unit calibration and must be established by measurement).

## PTT and key inputs (HIGH)

- PH6: PTT input. PH3, PH4: CW key or paddle contacts.
- RX-only firmware MUST NOT transmit on any change of these inputs.

## ADC1 metering inputs (VERIFIED pins)

| Pin | Quantity |
| --- | --- |
| PF11 | forward-power detector |
| PA6 | reflected-power detector |
| PC4 | PA supply current (Idd) |
| PB1 | PA temperature (NTC) |
| PA1_C | supply voltage (Vext) |
| PC5 | EALC |
| PA2 | ALC |

These inputs are needed for any protection (SWR, current, temperature) before
transmit is enabled. Conversion constants and detector laws are not part of
this spec; they must be established by calibration on the radio.

## Open

- OE polarity (both chains) is taken only from the power-off path; strobe and
  latch timing of the shift registers and the DAC: per datasheet once the parts
  are identified on the board.
- Function of each T/R line, of DAC channel 5, of PD12 and of PA12.
- Pin state between reset and the product's GPIO init (and while the bootloader
  runs): the MCU pins are high-impedance. Whether board pull-ups hold PA10/PH10
  inactive and the T/R lines at RX levels is not known. New firmware SHOULD set
  these pins first in its startup.
- Safe values for the ATU latches and for DAC1 OUT1.
