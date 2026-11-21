# 0038 — The AetherSDR G7250 backend: C++/Qt, after firmware milestone F2, offered through the RFC

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); recorded as advice on
  ON8ST's request, 2026-11-20
- Date: 2026-11-20
- Builds on: 0013, 0016, annex AetherSDR §4 (option B with C as fallback; option D rejected).

## Decision

1. The G7250 P2 backend is written in C++/Qt, inside AetherSDR, on its `IRadioBackend` seam.
2. A separate product-side session writes it, after firmware milestone F2.
3. It is offered upstream through the RFC (draft in `drafts/rfc-aethersdr-g7250-backend.md`);
   posting the RFC needs ON8ST's go.

## Consequences

- Annex AetherSDR §6: Q-O1 timing follows this decision.
- The RFC draft's suggested timing ("once the reference model and g7250ctl show the management
  channel working") becomes "after firmware milestone F2".

## Sources

- ON8ST, 2026-11-20, relayed by the openg7250 session; decisions 0013, 0016, 0018.
