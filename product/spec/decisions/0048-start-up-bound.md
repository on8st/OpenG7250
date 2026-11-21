# 0048 — Bound from link up to open sockets

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers testbench
  finding SF-24
- Date: 2026-11-21
- Amended by 0057 (start-up bound 17 s; MACRAW failure).
- Builds on: 0035 (DHCP with link-local fallback), 0041 (DHCP socket borrow); [RFC3927].

## Decision

Counted from link up (the CH394 reports the link), the management and P2 sockets are open
within 5 s when a DHCP server answers the first DISCOVER, and within 15 s when none answers
(at most 6 s of DHCP attempts, then the RFC 3927 probe and announce, about 7 s at most by its
section 9 constants). Power-on to link up is not part of the bound. The current firmware's
6.5 s is inside it. Spec A1.11, test T-A1-69.

## Sources

- Testbench finding SF-24 via openg7250, 2026-11-21; [RFC3927] section 9; decisions 0035, 0041.
