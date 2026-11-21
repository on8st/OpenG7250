# 0057 — Link-local without MACRAW, and the start-up bound corrected to 17 s

- Status: proposed (to the repository owner, ON8ST); answers two firmware items, 2026-11-21
  (b6f9e11)
- Date: 2026-11-21
- Amends: 0048 (15 s → 17 s), 0049.

## Context

1. Socket 0 may fail to open in MACRAW (a chip or driver fault); the spec required the RFC 3927
   probe but said nothing on this case.
2. 0048's "about 7 s" for the probe and announce left out the 2 s between the two announcements.
   By the RFC 3927 section 9 constants the claim with both announcements takes up to 9 s, so
   6 s of DHCP plus the claim reaches 15 s before the sockets open; the emulator saw 13.7–15.2 s.

## Decision (proposed)

1. Option (a), made visible: after three tries 10 ms apart the radio uses the chosen link-local
   address without a complete probe, so the management channel stays reachable; IP_CONFLI still
   guards it. It is logged ("link-local <address> used without a complete RFC 3927 probe"),
   counted (`ll_unprobed`), reported in `IDENTIFY` tag 24, and printed once on the debug console
   (`G7250 FAULT LINKLOCAL <address> UNPROBED`). Not an A3.5 fault: it does not touch transmit.
   (b) would leave the radio unreachable; (c) would block transmit for a network fault.
2. The start-up bound without DHCP becomes 17 s from link up (6 s DHCP + up to 9 s claim + 2 s
   margin); the text now names the constants. No behaviour changes.
3. Tests T-A1-69 (17 s) and T-A1-76 (MACRAW failure).

## Sources

- Firmware via openg7250, 2026-11-21; [RFC3927] section 9; [CH394]; decisions 0048, 0049.
