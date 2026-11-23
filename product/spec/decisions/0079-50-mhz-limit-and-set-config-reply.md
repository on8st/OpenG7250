# 0079 — Built-in power limit at 50 MHz; the `SET_CONFIG` reply

- Status: accepted (ON8ST, 2026-11-23, relayed by the openg7250 session: option (a)); proposed
  2026-11-23; answers two firmware questions found while fixing T-A1-28, relayed by the
  openg7250 session.
- Date: 2026-11-23
- Builds on: 0022 §31 (C-3 never looser than the built-in value), 0039 (calibration set,
  status 62), 0074 (tags 9 and 10 absent without a set); Annex P P-PWR-2.

## Context

1. The capability record gives 0 for the 50 MHz `tx_band` entry while P-PWR-2 is pending, so
   `GET_CONFIG` shows 0 there. `SET_CONFIG` judged a band limit against 1000 (100 W) for every
   band, so a 50 MHz limit of 50 W was taken as a tightening although it is above the 0 shown.
   The spec gave the built-in value only as "50 MHz HIS-12" and the record entry as "pending
   hardware", with no number.
2. 0074 made `GET_CONFIG` leave out tags 9 and 10 until the calibration set holds S2 and T3.
   The reply row of `SET_CONFIG` in Table M-1 still read "every item, after the change".

## Decision

1. **(a) 0 W at 50 MHz until P-PWR-2.** For each band the built-in power limit is the maximum
   power of its `tx_band` entry (B4.8). At 50 MHz it is 0 W until P-PWR-2 gives the maximum
   power there: the record carries 0 in both power fields, `GET_CONFIG` tag 8 carries 0, and a
   C-3 band limit above 0 there gets 49; 0 is accepted. While the rating is unknown, 0 W is the
   only value that is "never beyond the hardware rating" (A3.4 rule 2), and record, `GET_CONFIG`
   and the check of a write then agree. TX there is closed as on any band at 0 W, and TX is
   `NOT_CALIBRATED` anyway (0036).
2. **The `SET_CONFIG` reply** (editorial) gives every item it has after the change, as
   `GET_CONFIG` gives it (tags 9 and 10 only once the calibration set holds S2 and T3).

## Alternatives considered

- (b) Treat the 50 MHz limit as S2 and T3 are treated (status 62, its TLV left out until set):
  it would change tag 8's "one TLV per band", and the value is a firmware value, not a
  calibration entry (A1.12 holds per-unit measurements).

## Cost

- A persistent 0 written for 50 MHz before P-PWR-2 stays in force after a build with the real
  value, so the band stays closed until the owner loosens it (allowed and logged, A1.6 C-3).
  This errs to the safe side; no rule is added for it.
- Host and firmware: none beyond the check against the record's value per band.

## Consequences

- Spec A1.6 C-3 bullet; A3.4c (the built-in value); B4.8 `tx_band` row; Table M-1 `SET_CONFIG`
  reply.
- Spec Table A3-2, Forward-power row: "0 W until P-PWR-2 (0079)" (ON8ST, 2026-11-23).
- Tests T-A1-28 and T-A1-96 extended.

## Sources

- Decisions 0022, 0036, 0039, 0074; Annex P P-PWR-2; [G7250-PUDX], [G7250-RIGPIX] (90 W at
  50 MHz, through B4.8).
