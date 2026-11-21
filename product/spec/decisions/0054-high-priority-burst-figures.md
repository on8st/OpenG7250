# 0054 — High Priority bursts the radio must take without loss

- Status: proposed (to the repository owner, ON8ST); answers a firmware point on T-B5-20,
  2026-11-21 (cb92100)
- Date: 2026-11-21
- Builds on: 0053 (buffer split); [CH394]; Annex P P-NET-1.

## Context

Reading one 1444-byte datagram over SPI at about 17 MHz takes about 0.7 ms; on a 100 Mbit/s wire
it arrives in about 0.12 ms. Port 1027 holds two High Priority packets (0053). Ten packets truly
back to back at wire speed overflow it whatever the firmware does, so T-B5-20 needed figures.

## Decision (proposed)

1. On port 1027 the radio MUST process without loss (a) two packets back to back at wire speed
   when no third arrives within 2 ms of the first, and (b) any sequence with at least 1 ms between
   packets.
2. The figures hold at the SPI clock known to work and do not depend on P-NET-1; a faster clock
   adds margin only.
3. A host SHOULD NOT send High Priority packets closer than 1 ms apart, except a stop or run
   straight after another packet.
4. T-B5-20: the pair at wire speed (both orders), and 20 packets at 1 ms.

## Sources

- Firmware via openg7250, 2026-11-21; [CH394]; decision 0053.
