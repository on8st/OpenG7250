# 0052 — DHCP messages while link-local carry the link-local source address

- Status: proposed (to the repository owner, ON8ST); answers an emulator question of 2026-11-21
- Date: 2026-11-21
- Builds on: 0035, 0041, 0049; [RFC2131] section 4.1, [RFC3927] section 1.9, [CH394].

## Context

RFC 2131 section 4.1 asks for IP source 0.0.0.0 on messages broadcast before the client has an
IP address. While link-local, the radio has an address, and RFC 3927 lets it go on with DHCP.
The CH394's source IP register is chip-wide; the P2 sockets stay open during a DHCP borrow
(only the management socket is borrowed, 0041), so clearing it would break them.

## Decision (proposed)

1. While link-local, DHCPDISCOVER and DHCPREQUEST use the link-local address as IP source.
2. They carry `ciaddr` = 0.0.0.0 and the BROADCAST flag set, so the answer comes by broadcast and
   does not depend on the source address.
3. Before any address is held (the first exchange at power-on), the source is 0.0.0.0.
4. Spec A1.11; test T-A1-74.

## Sources

- Emulator question via openg7250, 2026-11-21; [RFC2131] 4.1; [RFC3927] 1.9; [CH394]; decisions
  0035, 0041, 0049.
