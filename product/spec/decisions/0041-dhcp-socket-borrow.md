# 0041 — DHCP exchanges borrow the management socket, at most 2 s at a time

- Status: proposed (to the repository owner, ON8ST); answers a firmware question of 2026-11-21
  (f22aa3b, DHCP per A1.11)
- Date: 2026-11-21
- Builds on: 0008 (socket budget), 0017, 0020 §24 (host retransmission), 0035 (DHCP with
  link-local fallback).

## Context

All eight CH394 sockets are in use (0008, 0017). A1.11 let lease renewals borrow the
management socket "for the few seconds the exchange takes". The firmware also retries DISCOVER
while it is link-local (4 s, backing off to every 64 s) and lends the socket for 3 s per
exchange. A host retransmits a management request every 500 ms, up to 5 times (A1.4), so it
gives up after about 2.5 s: a 3 s borrow can make a request fail outright.

## Decision (proposed)

1. Every DHCP exchange after power-on (renewal, rebinding, and the retries while no lease is
   held) borrows the management socket, listening on UDP port 68. The firmware's reading is
   confirmed for the link-local retries.
2. One borrow lasts at most 2 s; an exchange not finished then is abandoned and retried at the
   next interval.
3. Retries while no lease is held start at 4 s and double up to at most every 64 s (RFC 2131
   section 4.1), so between two borrows the channel is free for at least 4 s.
4. A request lost during a borrow is not an error; the host's retransmission covers it.

## Consequences

- Spec A1.2 (exception wording) and A1.11 (borrow rule); test T-A1-66.
- The firmware shortens its exchange from 3 s to at most 2 s.

## Sources

- [RFC2131] section 4.1; firmware question via openg7250, 2026-11-21; decisions 0008, 0017,
  0020 §24, 0035.
