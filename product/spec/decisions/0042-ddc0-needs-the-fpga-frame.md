# 0042 — DDC0 is not offered until the FPGA configuration frame is understood

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session)
- Date: 2026-11-21
- Builds on: 0004 (one DDC), 0035 (Annex P, freeze rule); [HIS] HIS 80, HIS 85, HIS 90
  (`5292ba8`, gate review 8).

## Context

HIS 80 at `5292ba8`: DDC0 cannot run until byte 0 (status flags) and bytes 9–18 (per-rate
calibration among them) of the FPGA's 0x01 configuration frame are understood; tuning DDC0 or
setting its rate needs a full frame. Which tuning word drives DDC0, the I/Q order of the 0x04
samples and the bus timing are not known either. Byte 0 may hold a transmit enable, so any 0x01
write while it is not understood must happen with the RF section in its safe state. Reusing the
unit's factory calibration values would be another route; it needs a decision by ON8ST and is
not taken here.

## Decision

1. New Annex P parameter P-RX-2: the configuration frame's status byte and bytes 9–18, the
   spectral-inversion flag, the DDC0 tuning word, the I/Q order and the bus timing.
2. Until P-RX-2 is set, DDC0 is not offered: record `streams` bit 0 = 0, a DDC0 enable gets
   `STREAM_STATUS` reason 2, no DDC0 packets. Discovery, register map, management and sessions
   work as usual.
3. The firmware writes no configuration frame outside the A3.6 safe state.
4. P-RX-2 is also part of the TX gate of the freeze rule, because the frame may hold a transmit
   enable.
5. Not decided here: whether the unit's factory calibration values may be reused for bytes 9–18
   (ON8ST).

## Addendum: stream reason order (firmware question, 2026-11-21)

When several `STREAM_STATUS` reasons apply to one stream, the first in this order is reported:
10, 9, 8 (radio-wide); 1 (not enabled by the host); 13, 6, 11, 12, 5 (General-packet
refusals); 2, 14 (not offered or not calibrated); 3, 4, 7 (stream parameters). So before
P-RX-2 a DDC0 enable with an unsupported rate or sample size gets 2, and one with a
General-packet refusal gets that refusal. Table B8-3; tests T-B6-10, 18, 19, 20, T-B7-04.

## Consequences

- Spec B6.1, record `streams` bit 0, Annex P (P-RX-2, freeze rule, reception paragraph); test
  T-B6-27.
- Milestone M1 (RX IQ) on the radio waits for P-RX-2.

## Sources

- [HIS] HIS 80, HIS 85, HIS 90 (`5292ba8`); decisions 0004, 0035.
