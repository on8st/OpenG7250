# Decisions

Project-wide decisions for OpenG7250, one short file per decision.
Decisions that concern a single component live in that component's own
`decisions/` folder.

The product side works in a sparse clone without `re/`. A decision here that
affects the product is also recorded in `product/spec/decisions/`, without RE
detail.

## Conventions

- File name: `NNNN-short-title.md`, numbered in order, never renumbered.
- Start from [`TEMPLATE.md`](TEMPLATE.md).
- A decision is not edited after acceptance; a later decision supersedes it and
  both files say so.
- Status: `proposed`, `accepted`, `superseded by NNNN` or `rejected`.

## Index

| No. | Title | Status | Date |
|---|---|---|---|
| [0001](0001-umbrella-layout.md) | Umbrella layout | proposed | 2026-11-20 |
| [0002](0002-three-sides-and-gate.md) | Three sides and the HIS gate | proposed | 2026-11-20 |
| [0003](0003-product-work-tree.md) | Product work tree | proposed | 2026-11-20 |
| [0004](0004-no-legal-gate.md) | No legal-advice gate | proposed | 2026-11-20 |
| [0005](0005-stacked-prs.md) | Structure first, components as stacked pull requests | proposed | 2026-11-20 |
| [0006](0006-no-decrypted-hashes.md) | No hashes of decrypted images | proposed | 2026-11-20 |
| [0007](0007-emulator-as-test-bench.md) | The emulator as test bench for the product firmware | proposed | 2026-11-20 |
