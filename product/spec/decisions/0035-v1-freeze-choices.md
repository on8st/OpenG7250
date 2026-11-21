# 0035 — Choices for the v1.0 freeze without the radio

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); written by the product
  session to finish the spec without the radio (ON8ST's go for items 1–5, 2026-11-20)
- Date: 2026-11-20
- Applies to: `spec/network-interface.md` (conventions, A1.6, A1.7, A1.10, A1.11, A3.6, B2,
  B4.8, B6, B10.2, Table B8-3, General and DUC-specific bytes, Annex P), part C (C5.5.6, C5.7,
  C5.8, C5.9, C6, C8), annex AetherSDR.
- Builds on: [HIS] up to `9d10e72` (HIS 10, 15, 40, 70, 80, 85, 90); decisions 0004, 0005,
  0028–0032.

## Context

79 values in the spec were "TBD" with a HIS id. The cleared HIS answers some of them; the rest
need the real radio, which is not delivered yet (HIS 90). A v1.0 must be frozen without it.

## Decision

1. **Pending hardware.** Every value that needs the radio gets a parameter id in the new Annex P
   (Table P-1) with a safe placeholder. "TBD" no longer appears; the text says "pending hardware
   (P-…; HIS-…)", or "pending G6" for level-2 values.
2. **Freeze rule.** TX stays disabled until all transmit parameters (groups P-RF, P-PWR, P-SEN,
   P-TIM) are set: record `tx` bit 4 = 0, and every arm is rejected with the new arm reject
   reason 12 `NOT_CALIBRATED` (B10.2, part C C5.5.6 item 7). ATU and antenna selection stay
   unavailable until P-ATU-1 and P-ANT-1 are set (0032).
3. **Values settled without the radio:**
   - C-1 factory default: DHCP, with an IPv4 link-local address [RFC3927] while no lease is held;
     A1.11 is final (the CH394 has no DHCP client, 0008 and [CH394]; [HIS] HIS 40 leaves network
     behaviour to the product spec).
   - Local factory reset: the headless product has no local input; the local path is a
     re-install through the vendor path, and the first start then uses the factory values.
   - Log: in RAM, size a firmware choice, lost at power loss; warm-reset survival pending
     (P-SYS-2).
   - Randomness: the MCU's hardware TRNG ([HIS] HIS 10).
   - MAC: from the CH394's registers ([HIS] HIS 40); uniqueness pending (P-NET-2).
   - CH394 link status register usable (this firmware owns the chip); socket buffer split as
     0004.
   - SPI: 192 ksps is the maximum DDC0 rate until 50 MHz or more is shown (P-NET-1).
   - General bytes 4–6, 17: one ADC; dither and random ignored in this version.
   - Alex enable and Alex words, step attenuator byte 1443: ignored (no identical meaning,
     [HIS] HIS 85).
   - Sidetone, line-in and microphone bytes: ignored (0031); hardware keyer not used (0030).

## Amendment after the review of 2026-11-21

- The TX gate also covers P-TX-1, P-ATU-1 and P-ANT-1; `ATU_STATE` 5 (unknown) while the ATU
  latches are unwritten; the way back to stock is closed until the drill F1 (P-SYS-5) shows the
  vendor path recovers an empty application area; new P-SYS-7 (watchdog) and P-SYS-8 (SEND and
  TUNER); status 61 for an image too large.

## Consequences

- Spec: conventions, Annex P, the places above; Table B8-3 arm reject reason 12; record `tx`
  bit 4; part C C5.5.6 item 7.
- Lint: "TBD" is not allowed any more; every P-id used is defined in Table P-1.

## Sources

- [HIS] HIS 10, 15, 40, 70, 80, 85, 90 (`9d10e72`); [RFC3927]; ON8ST, 2026-11-20 (go for
  items 1–5); decisions 0004, 0005, 0028–0032.
