# 0006 — No hashes of decrypted images

- Status: proposed
- Date: 2026-11-20
- Decided by: ON8ST (proposal to the repository owner)
- Affects: project

## Context

The emulator rule says hashes of decrypted images are never committed. Upstream RE documents in `re/analysis/` do contain such a hash.

## Decision

Proposal: new material never contains hashes of decrypted images or their local paths; they stay in git-ignored local configuration. Existing upstream files are listed in `tools/leak-allow.txt` until the repository owner decides on them. `tools/leak-check.sh` enforces the rule in CI.

## Consequences

The repository owner decides whether the existing upstream hashes stay. Hashes of the public, encrypted release package are fine.
