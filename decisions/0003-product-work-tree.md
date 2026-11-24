# 0003 — Product work tree

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: product/spec | testbench | project

## Context

Product contributors must not see `re/`, but `re/` is in the same repository and its history.

## Decision

Product work happens in a separate sparse, blobless clone with `product` and `testbench` only, plus a read-only work tree of `his/` at a cleared commit (`tools/product-clone.sh`). Product agents also run with deny rules for RE paths and for git commands that could expose `re/` from history.

## Consequences

Not watertight against a determined reader, but it makes the separation explicit and verifiable. Moving the HIS work tree to a newly cleared commit is a deliberate step.
