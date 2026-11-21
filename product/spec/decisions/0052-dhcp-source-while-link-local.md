# 0052 — DHCP messages while link-local carry the link-local source address

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers an emulator
  question of 2026-11-21
- Date: 2026-11-21
- Builds on: 0035, 0041, 0049; [RFC2131] section 4.1, [RFC3927] section 1.9, [CH394].

## Context

RFC 2131 section 4.1 asks for IP source 0.0.0.0 on messages broadcast "prior to that client
obtaining its IP address". RFC 3927 section 2.11 requires the DHCP client to keep trying while a
link-local address is configured, and section 1.9 says the host must not alter its use of DHCP
because of that address; neither says which source address the retries use. (An earlier draft
of this decision cited section 1.9 for the retries; the rule is in 2.11.)
The CH394's source IP register is chip-wide; the P2 sockets stay open during a DHCP borrow
(only the management socket is borrowed, 0041), so clearing it would break them.

## Decision

1. While link-local, DHCPDISCOVER and DHCPREQUEST use the link-local address as IP source: a
   deliberate reading forced by the chip-wide source register, not a choice the RFCs make for
   us; a server identifies a broadcast DISCOVER by `chaddr` and option 61.
2. They carry `ciaddr` = 0.0.0.0 and the BROADCAST flag set, so the answer comes by broadcast and
   does not depend on the source address.
3. Before any address is held (the first exchange at power-on), the source is 0.0.0.0.
4. Spec A1.11; test T-A1-74.

## Sources

- Emulator and firmware questions via openg7250, 2026-11-21; [RFC2131] 4.1; [RFC3927] 1.9, 2.11
  (read 2026-11-21); [CH394]; decisions
  0035, 0041, 0049.
