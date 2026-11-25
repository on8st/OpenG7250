# 0001 — Umbrella layout

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: project

## Context

G7250 work was spread over separate local repositories (emulator, HIS, spec) next to this RE repository.

## Decision

OpenG7250 is the umbrella for all G7250 work: `re/` (analysis, emu), `his/`, `product/` (spec, firmware, refmodel, tools), `testbench/`, `website/`. Components are merged into the tree with their history rather than added as submodules. `fventuri/g7250-base` stays outside as a comparison source.

## Consequences

One repository to clone and review. Component histories are kept. The structure lands first (#1); components follow as separate pull requests.
