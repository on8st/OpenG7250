# 0058 — Counter of High Priority packets received

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers a testbench
  request on T-B5-20, 2026-11-21
- Date: 2026-11-21
- Builds on: 0054, 0055; [CH394].

## Decision

1. `GET_STATS` counts every High Priority packet read from port 1027, from any source and before
   the owner check, as `hp_rx`. A lost packet then shows even when it repeats the state in force.
2. No receive-overflow counter per socket is defined: the [CH394] datasheet gives none for UDP
   sockets.
3. T-B5-20 checks that `hp_rx` rises by exactly the number of packets sent.

## Sources

- Testbench via openg7250, 2026-11-21; [CH394]; decisions 0054, 0055.
