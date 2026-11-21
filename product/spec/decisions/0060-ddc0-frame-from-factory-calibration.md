# 0060 — DDC0 frame values from the unit's factory calibration, confirmed on the radio

- Status: proposed (to the repository owner, ON8ST); written on ON8ST's choice of route (b)
  confirmed by (a), relayed by the openg7250 session, 2026-11-21
- Date: 2026-11-21
- Builds on: 0035 (Annex P, freeze rule), 0039 (calibration set, A1.12), 0042 (DDC0 gate,
  point 5 left this open); [HIS] HIS 15 rule 3, HIS 80, HIS 90 (`5292ba8`).

## Context

DDC0 needs a complete FPGA configuration frame (command 0x01). Its status byte 0 and bytes 9–18
(per-rate calibration among them) have no known meaning or safe values, and the DDC0 tuning
word (offset 1 or 5), the I/Q order of the 0x04 samples and the spectral-inversion flag are not
known ([HIS] HIS 80). The values the factory firmware writes there are per-unit data; the HIS
does not give them ([HIS] HIS 80). The unit's factory calibration is kept in the 24Cxx EEPROM
at I²C1 0x50, which this firmware treats as read-only ([HIS] HIS 15 rule 3). HIS 80 named the
reuse of the factory calibration as a route that needs ON8ST's decision; 0042 point 5 left it
open.

Routes:

- (a) derive every value from measurement on the radio alone (captures with setting changes
  and a known signal, [HIS] HIS 90 rows F4-Q2, F4-Q3);
- (b) reuse the values the unit's factory calibration gives, without interpreting them more
  than needed;
- (b) confirmed by (a): ON8ST's choice.

## Decision (proposed)

1. **Route.** P-RX-2 is set from the unit's factory calibration in principle, and counts as
   set only after the confirmation of point 3 on that unit. If the confirmation fails, route
   (a) applies, in a new decision.
2. **What the factory calibration must give**, per unit, for each DDC0 rate this firmware
   offers (`ddc_rates`, B4.8):
   1. the complete 24-byte 0x01 frame the factory firmware writes in receive (not
      transmitting), at that rate, at two or more receive frequencies, so that the bytes that
      change with frequency (the tuning words) are told apart from those that do not;
   2. from these: the value of byte 0 in receive, bytes 9–18 for that rate, the rate index
      (byte 23, [HIS] HIS 80), and which tuning word (offset 1 or 5) changes with the receive
      frequency; a byte that changes with a setting other than rate and frequency is not
      reused until its dependence is known;
   3. its source: the logic-analyser capture of the frames the factory firmware writes on
      PE0–PE10 of that unit, passive probes only, under the signal-generator rule
      ([HIS] HIS 90). Reading the EEPROM directly instead is allowed only read-only
      ([HIS] HIS 15 rule 3) and only once the HIS describes where the frame values are kept
      (open question, HIS-02, added by this decision);
   4. a rate for which the factory firmware writes no frame is not offered (absent from
      `ddc_rates`).
   The values are per unit: they go into that unit's calibration set (A1.12) as the value of
   P-RX-2, not into the firmware image or this specification. Encoding (Table P-1): flags
   (u8: bit 0 I and Q swapped, bit 1 spectrum inverted, as found by point 3), DDC0 tuning-word
   offset (u8: 1 or 5), then per offered rate: rate index (u8, byte 23), byte 0 (u8), bytes
   9–18 (10 bytes). The firmware writes them as given and does not need to know what each bit
   means.
3. **Confirmation on the first radio** (route (a), before P-RX-2 counts as set). With a test
   build on the bench (as for the drill F1), the RF section in the safe state of A3.6, a dummy
   load on both antenna ports and the generator connected under the signal-generator rule
   ([HIS] HIS 90), at every offered rate:
   1. **No transmit.** Writing the reused frame keys nothing: forward power stays below
      `P_rf_off` (P-RF-3) or the power meter shows none, and the SEND output stays released.
      This confirms that byte 0 holds no transmit enable in the reused value.
   2. **Tuning word and frequency.** A tone at f₀ + Δ, with DDC0 tuned to f₀ by High Priority
      bytes 9–12 (B6.8), appears at +Δ in DDC0 within 10 Hz + 1 ppm of f₀; this confirms the
      tuning word, f_clk ([HIS] HIS 80) and the scale of B3.8.
   3. **I/Q order and spectral inversion.** With the flags of point 2 applied, the tone at
      f₀ + Δ appears above the centre and one at f₀ − Δ below it, and the image of each is at
      least 30 dB below the tone.
   4. **Level per rate.** The same tone gives the same DDC0 level within 1 dB at every
      offered rate.
   5. **Stability.** The results repeat after a power cycle.
   The results are logged with the calibration set's source and date (A1.12 item 2) and given
   to the HIS for its HIS 90 record. Figures in items 2–4 are proposed and may be tightened
   once the first radio has been measured.
4. **Until confirmed, nothing changes.** The freeze rule (Annex P) and the DDC0 gate (0042)
   stay as they are: P-RX-2 not set, record `streams` bit 0 = 0, a DDC0 enable gets
   `STREAM_STATUS` reason 2, record `tx` bit 4 = 0, and no configuration frame is written
   outside the A3.6 safe state. P-RX-2 is written into a unit's calibration set only with the
   values that passed point 3 on that unit; the calibration source (A1.12 item 2) names the
   capture and the confirmation.

## Alternatives considered

- Route (a) alone: needs every byte understood from measurements; slower, and the factory
  values are the best starting point for per-unit calibration bytes.
- Route (b) without confirmation: would put unknown bits on the FPGA bus of a transmitter
  without a check that they key nothing and tune correctly.

## Consequences

- Annex P: P-RX-2 row and a paragraph "P-RX-2 route"; 0042 point 5 decided here.
- Open question to the HIS (HIS-02): where the factory calibration keeps the 0x01 frame values,
  if it does, and in what layout.
- Test T-B6-28 (new) for the confirmation, run on the radio.

## Sources

- [HIS] HIS 15, HIS 80, HIS 90 (`5292ba8`); decisions 0035, 0039, 0042.
