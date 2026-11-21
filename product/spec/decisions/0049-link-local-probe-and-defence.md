# 0049 — Link-local probe on socket 0, defence by giving the address up

- Status: proposed (to the repository owner, ON8ST); answers firmware questions of 2026-11-21
- Date: 2026-11-21
- Amended by 0057 (start-up bound 17 s; MACRAW failure).
- Builds on: 0008, 0017 (socket budget), 0035, 0041, 0048; [RFC3927], [RFC2131], [CH394].

## Context

[CH394] (CH394DS1 V1.2): only socket 0 can use MACRAW; the chip raises IP_CONFLI (GINT bit 7)
when it sees another device with its address, without saying how or when. All eight sockets are
in use.

## Decision (proposed)

1. **Q1** Yes: when a link-local address is chosen while running (after a lease end, DHCPNAK or
   conflict, each an address change of A1.11a that ends the session anyway), the probe runs on
   socket 0 in MACRAW, and discovery and the other sockets are closed for at most 10 s.
2. **Q2** Yes: defence by [RFC3927] section 2.5 option (a) is conformant: give the address up at
   once on IP_CONFLI, no defending ARP, choose and probe a new one; after 10 conflicts wait 60 s
   (section 9). Every conflict is logged.
3. **Q3** Confirmed: DHCP attempts (at most 6 s) and probe and announce (about 7 s) run one after
   the other; 0048's 15 s assumes that.
4. A conflict on a leased address: DHCPDECLINE, link-local, DHCP again. On a static address:
   fall back to the previous configuration (A1.6 C-4), else link-local. Each an A1.11a change.

## Consequences

- Spec A1.11; tests T-A1-70 (probe), T-A1-71 (announce), T-A1-72 (conflict while link-local),
  T-A1-73 (conflict on DHCP or static address).

## Sources

- Firmware questions via openg7250, 2026-11-21; [RFC3927] sections 2, 2.5, 9; [RFC2131] section
  3.1; [CH394]; decisions 0008, 0017, 0035, 0041, 0048.
