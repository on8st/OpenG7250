# 0074 — Management details from the g7250ctl runs

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session); proposed
  2026-11-22; answers questions of the g7250ctl session on the reference model (M1 to M4,
  relayed by the openg7250 session, 2026-11-22), and the "restart needed" value of the
  streaming installer before its first erase (testbench, same day)
- Date: 2026-11-22
- Builds on: 0020 §19, §24, §28; 0022 §4, §75, §86; 0039; 0043; 0058; 0063; 0067; 0068;
  0024 and 0028 (installer).

## Context

Running `g7250ctl` against the reference model found ten places where the accepted text
leaves a host's case open or unclear. For each, the accepted text already decides part of it;
this decision adds the rest. `GET_STATS` names only four counters (`hp_rx`, `ll_unprobed`,
`cw_speed_clamped`, `cw_hang_clamped`), although about 25 places require a count.

## Decision

1. **`UPDATE_ABORT` with no update state** (F3). It gets 0 with "restart needed" = 0 and
   changes nothing: no update state, RX and the P2 session go on, no timer, nothing erased;
   logged (A1.8 "No transfer in progress"). Otherwise a host whose activation reply was lost
   would stop RX on the trial image and, through the 10-minute restart, end the trial before
   it confirms. "Restart needed" is 1 if a `RESTART` is needed to leave the update state, and
   0 if there is no update state or in the streaming installer before its first erase, which
   restarts the main unit by itself (Table M-1; A1.8a item 3 "Before the first erase").
2. **Write failure in an update** (editorial). Afterwards no transfer is in progress, the
   update state remains until a restart, the 10-minute rule runs from the last update command
   answered 0, 1 or 64, and a new `UPDATE_BEGIN` is accepted; a failed `UPDATE_VERIFY` is not
   a write failure (A1.8 "Write failure").
3. **`GET_CONFIG` before S2 and T3** (F4). Tags 9 and 10 are left out until the calibration
   set holds S2 and T3; an absent tag means the limit is not known; never sent as 0
   (Table M-3, Table M-1).
4. **`GET_CALIBRATION` with no set** (Q7, editorial). Id zeros, date 0, source length 0, no
   entries: 37 bytes (Table M-1).
5. **`GET_STATS`** (F1). Table M-6 names every counter the spec requires (45 names). They
   come first, in table order, every one listed, also at 0; the firmware's own follow. The
   reply fits 1400 bytes (A1.4): names of at most 16 characters, at most 48 names
   (1200 bytes) in Table M-6. Each place that requires a count names its counter. Two kinds
   of count a fixed name cannot hold change: DUC I&Q from another source is one total
   (`duc_foreign`) with the source in the log at most once per second per source (B10.7);
   B11.1 counts per port (`drop_<port>`), the reason in the log at most once per second per
   port and reason. Gaps of B11.6 count from the start of the running session, or of the last
   one.
6. **End of the log** (Q8). `READ_LOG` sets flag bit 1 ("more blocks follow") when entries
   after the page exist at the time of the reply; a follower asks again from the next
   sequence number (A1.7).
7. **Opening the write session** (Q9). An authenticated write whose HMAC verifies opens it
   whatever its status, except 35 and the codes before authentication (16 to 19, 32 to 34);
   `RESTART` is exempt; `END_WRITE_SESSION` with no session opens none (A1.10). Refreshing was
   already decided so (0022 §75).
8. **`END_WRITE_SESSION`** (Q11). With no session held: 0, opens and ends nothing; from a
   non-holder: 35 (A1.10).
9. **Address change** (Q10). A change of address or netmask ends the write session and
   discards challenges and stored replies, logged; the `SET_CONFIG` that causes it is
   answered first; a DHCP renewal with the same address and netmask changes nothing (A1.10).
10. **Requests that end in a reset** (Q12). The radio cannot tell a retransmission from a
    replay, so nothing changes there. New host rule (A1.4): stop retransmitting at the first
    33 or once `IDENTIFY` answers after a silence, and learn from `IDENTIFY` (tags 16, 17,
    and for `UPDATE_ACTIVATE` and `ROLLBACK` tags 23 and 10) and the log whether the reset
    took place, before sending the request again with a new challenge.

## Alternatives considered

- Paging for `GET_STATS`: not needed while the named counters fit one reply; a later
  version that needs more adds paging then.
- One counter per source for foreign DUC I&Q: a table of fixed names cannot hold it.
- An exception for the 33 after a reset: the radio cannot tell a retransmission from a
  replay, so the host rule carries it.

## Consequences

Accepted text that changes (ten places):
- A1.8 "No transfer in progress" (0063): one more case (point 1).
- A1.8 "Write failure" (0043): one sentence added, editorial (point 2).
- A1.8a item 3 "Before the first erase" (0024, 0028): "restart needed" = 0 (point 1).
- Table M-1 rows `UPDATE_ABORT`, `GET_CONFIG`, `GET_CALIBRATION`, `GET_STATS` (points 1, 3,
  4, 5).
- Table M-3: one sentence (point 3).
- B10.7 (0022 §4): "counted per source" becomes one total plus the source in the log
  (point 5).
- B11.1 (0020 §19, §28): "counted per port and reason" becomes per port plus the reason in
  the log (point 5).
- A1.7 (L-3): flag bit 1 for `READ_LOG` (point 6).
- A1.10 "One writer at a time" (0020 §24, 0022 §75): opening, `END_WRITE_SESSION`, address
  change (points 7 to 9).
- A1.4: one host rule added; nothing the radio does changes (point 10).

Also: new Table M-6; each place that requires a count names its counter (A1.4, A1.10, A2.4,
A2.5, A3.5, B3.2, B3.9, B4.4, B8.5, B10.5b, B10.7, B11.6, B11.6a, B11.10).

Tests: T-A1-15, T-A1-62, T-A1-80, T-A1-85 and T-A1-93 extended; T-A1-95 to T-A1-98 new.

Cost: radio and reference model carry Table M-6 and flag bit 1 in `READ_LOG`. `g7250ctl`
handles absent tags 9 and 10, stops reading the log on bit 1 clear, takes a new challenge
after an address change, and follows the host rule of point 10. Operator: none.

## Sources

- Decisions 0020, 0022, 0024, 0028, 0039, 0043, 0058, 0063, 0067, 0068; g7250ctl session
  questions F1, F3, F4, Q7 to Q12.
