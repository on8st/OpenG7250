# 0082 — Change into DHCP mode while running

- Status: accepted (ON8ST, 2026-11-25, "Voorstel spec", relayed by the openg7250 session: the
  proposal as written, points 1 to 4, the text changes and test T-A1-100); proposed
  2026-11-25 on a firmware question relayed by the openg7250 session.
- Date: 2026-11-25
- Builds on: 0035 (DHCP with link-local fall-back), 0041 (borrows of the management socket),
  0048 and 0057 (start-up bound), 0049 (link-local probe, conflicts), 0052 (DHCP messages
  while link-local), 0022 §44 and §54 (address changes).

## Context

The firmware asked whether a running `SET_CONFIG` with a C-1 change into DHCP mode is an
address change with the probe bound of A1.11 (discovery and management closed for at most
10 s, T-A1-72), or whether the radio keeps its old address until a lease comes, closing the
management socket only for borrows of at most 2 s (A1.11, 0041).

The text decided part of it. A1.11: "While no lease is held ... the G7250 MUST use an IPv4
link-local address ... and keep trying DHCP"; so keeping a static address until a lease comes
breaks that rule when no server answers. It was silent on the rest: borrows were named only for
renewal, rebinding and the retries while no lease is held; the probe list did not name a mode
change; the source-address rule of 0052 covered only a link-local source. An immediate change
as at power-on would close discovery and management for the DHCP attempts and the probe, up to
6 + 9 s, beyond the 10 s of T-A1-72.

## Decision

1. The mode change is no address change in itself. After the reply the radio keeps the address
   it holds and does not end the session for the mode change.
2. From a link-local address (static mode on its link-local fall-back after a conflict, A1.11
   "Conflict on a DHCP or static address"), the radio keeps that address and is at once in the
   "no lease held" state of A1.11: DHCP runs by borrows of at most 2 s, and the lease is an
   address change (A1.11a). No probe.
3. From a static address, the radio keeps it while it tries DHCP by borrows of at most 2 s,
   with `ciaddr` 0.0.0.0, the BROADCAST flag and the static address as IP source (the 0052 rule
   widened to "the address held"), for at most 6 s, the DHCP attempts of A1.11 "Bound on
   start-up", with the channel free between borrows ("Bound on a borrow").
   - A lease within that time is an address change (A1.11a), unless it gives the same address
     and netmask, which changes nothing, as a renewal that keeps them.
   - With no lease, the radio chooses a link-local address, an address change of A1.11a with
     the probe of A1.11 (at most 10 s closed, as T-A1-72), and then keeps trying DHCP as A1.11
     says. The static address is never kept beyond those 6 s with no lease.
4. Already in DHCP mode, a `SET_CONFIG` that sets DHCP again changes nothing and starts no
   exchange.

## Alternatives considered

- (b) Keep the static address until a lease comes: breaks the MUST of A1.11 with no server.
- (c) An immediate change as at power-on (sockets closed, DHCP, then the probe): up to 15 s
  closed, beyond T-A1-72's 10 s and close to the host's 16 s retry window (A1.4).

## Cost

- Operator: none new; the radio stays reachable during the DHCP attempts.
- Firmware: the first exchange after the mode change runs by borrows with the static address
  as source, bounded at 6 s; then the existing paths.
- Host: none new; the session ends only when the address changes.

## Consequences

- Spec A1.11: the borrow list in the first paragraph; the probe list of "Link-local address";
  "DHCP messages while link-local" (the address held); a new bullet "Change into DHCP mode while
  running".
- Test T-A1-100 new.

## Sources

- Decisions 0022, 0035, 0041, 0048, 0049, 0052, 0057; [RFC2131], [RFC3927] as A1.11 cites
  them; the firmware question relayed by the openg7250 session, 2026-11-25.
