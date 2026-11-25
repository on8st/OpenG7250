# 0002 — Three sides and the HIS gate

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: project

## Context

New firmware must be built without copying how the stock firmware works, while it still needs hardware knowledge found by reverse engineering.

## Decision

The project has three sides: RE (`re/`), the gate (`his/`) and the product (`product/`, `testbench/`). The HIS is the only path from RE to product and carries hardware facts only. A change to `his/` reaches the product side only after a gate review in `re/analysis/docs/` marks it cleared. Reviews are done by a reviewer who did not write the change. The product side asks questions through HIS questions; answers come back only through the HIS.

## Consequences

Every HIS change needs a review round. The product side may work with placeholders marked "pending hardware" while a question is open.
