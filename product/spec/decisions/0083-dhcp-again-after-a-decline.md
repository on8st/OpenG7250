# 0083 — DHCP again after a decline

- Status: accepted (ON8ST, 2026-11-27, proposal (a), relayed by the openg7250 session);
  proposed 2026-11-27 on a test-bench question relayed by the openg7250 session.
- Date: 2026-11-27
- Builds on: 0035 (DHCP with link-local fall-back), 0041 (borrows of at most 2 s), 0049
  (conflicts, link-local probe).

## Context

A1.11 "Conflict on a DHCP or static address" (0049) says that after a conflict on a leased
address the G7250 sends DHCPDECLINE, uses a link-local address and "starts DHCP again", with
no time bound, so T-A1-73 could not judge when. The test bench asked for an upper bound of
10 s. [RFC2131] section 3.1 item 5 says the opposite: "the client MUST send a DHCPDECLINE
message to the server and restarts the configuration process. The client SHOULD wait a
minimum of ten seconds before restarting the configuration process" (read on rfc-editor.org,
2026-11-27). The 10 s are a minimum against looping, not a maximum.

## Decision

1. The first DHCPDISCOVER after the DHCPDECLINE MUST be sent no earlier than 10 s after it
   ([RFC2131] section 3.1 item 5) and no later than 12 s after it.
2. If the probe of the link-local address (at most 10 s, or later under the rate limit after
   10 conflicts, A1.11) ends after that, the DISCOVER is sent within 2 s of the end of the
   probe. The sockets are closed during the probe, so it cannot leave earlier; the 2 s are the
   borrow bound of 0041.
3. The retries then follow the back-off of A1.11 (from 4 s, doubling up to 64 s).
4. T-A1-73 checks it, with `ciaddr` 0.0.0.0, the BROADCAST flag and the link-local address as
   IP source (A1.11, 0052).

## Alternatives considered

- (b) Only the RFC minimum of 10 s, with no upper bound: T-A1-73 could check the minimum only.
- (c) No bound, as before.
- The test bench's "at most 10 s after the DECLINE": against the SHOULD of [RFC2131].

## Cost

- Firmware: a timer from the DHCPDECLINE; the exchange itself is unchanged.
- Operator and host: none; a leased address lost to a conflict comes back through DHCP at
  most about 12 s later, or 2 s after a late probe.

## Consequences

- Spec A1.11 "Conflict on a DHCP or static address".
- Test T-A1-73 extended.

## Sources

- [RFC2131] section 3.1 item 5; decisions 0035, 0041, 0049, 0052; the test-bench question
  relayed by the openg7250 session, 2026-11-27.
