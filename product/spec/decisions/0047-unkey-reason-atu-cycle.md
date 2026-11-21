# 0047 — Unkey reason at the end of an ATU tune cycle

- Status: proposed (to the repository owner, ON8ST); answers a firmware question of 2026-11-21
- Date: 2026-11-21
- Builds on: 0022 §22, §38 (tune cycle), 0032 (own ATU control).

## Context

`TX_STATE` [23:16] had no reason for the end of an ATU tune cycle; the firmware used 1 (TX
request released) for both a tuned end and an abort. Reason 1 is right for an abort, which
releases the request, but misleading for a cycle that ends by itself.

## Decision (proposed)

1. New unkey reason 11, "ATU tune cycle ended by itself", for a cycle that ends tuned
   (`ATU_STATE` 3) or failed by itself (`ATU_STATE` 4).
2. An abort by `ATU_TUNE` = 0 or a stop request gives 1, with `ATU_STATE` 4.
3. The ATU tune time gives 4 with limit id `LIMIT_ATU_TIME` (unchanged); a keyed timeout, a limit
   or a drop keep their reasons.

## Consequences

- Spec A3.9b item 6, Table B8-2 `TX_STATE`, part C `G7250_TX_STATE`; test T-B10-47. The firmware
  changes its tuned end from 1 to 11.

## Sources

- Firmware question via openg7250, 2026-11-21; decisions 0022, 0032.
