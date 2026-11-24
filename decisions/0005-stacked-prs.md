# 0005 — Structure first, components as stacked pull requests

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: project

## Context

The structure and the components are large and of different kinds; one pull request would be hard to review.

## Decision

The structure lands in one pull request (#1). Each component follows in its own pull request, opened as a draft and stacked on #1 (and on another component where they touch the same lines). A component PR leaves draft once the structure is merged and its own open items are done.

## Consequences

Until #1 is merged, stacked diffs also show its commits; each description says which paths to review.
