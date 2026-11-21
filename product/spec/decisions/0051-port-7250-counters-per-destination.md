# 0051 — Port 7250 sequence counters per destination

- Status: proposed (to the repository owner, ON8ST); answers an ambiguity found by test
  T-A2-11 (firmware and testbench, 2026-11-21)
- Date: 2026-11-21
- Builds on: 0022 §3, §29, §73 item 22, §83 item 4, §86 item 10; [P2] "Sequence Number".

## Context

B2.7 keeps a counter "for each local port and direction"; A2.4 step 1 gave the takeover copies
"the next sequence number of local port 7250". The firmware numbered an answer to a non-owner
with the owner's counter, so the owner's stream showed a gap; the bench expected none.

## Decision (proposed)

Reading (b), as the firmware proposed and openg7250 advised:

1. On port 7250 the direction is the destination. One counter for the owner's register
   destination (the session's counter, with the resets of B2.7).
2. One separate counter for all answers to non-owners (`NOT_OWNER`, `CAP_REQUEST` answers),
   reset only by a reset of the radio.
3. The takeover copies and the repeated `TAKEOVER` notice to an old owner continue the old
   session's counter towards that host.

## Consequences

- Spec B2.7, A2.4 step 1; tests T-A2-11 (rewritten) and a new non-owner counter test.

## Sources

- Firmware fw-9b89c0a and the testbench via openg7250, 2026-11-21; [P2]; decision 0022.
