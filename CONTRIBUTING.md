# Contributing to OpenG7250

Thank you for helping. Read [CLEANROOM.md](CLEANROOM.md) first: the project is
split into sides, and which side you work on decides what you may read.

## Sides

| Side | Paths | May read |
|---|---|---|
| RE | `re/` | everything except product work in progress |
| Gate | `his/` | written by the RE side, cleared by a gate review |
| Product | `product/`, `testbench/` | `product/`, `testbench/`, a cleared `his/`, public sources; **never `re/`** |

If you have read anything under `re/` or the stock firmware, you are on the RE
side and do not write product material.

## Work trees

- RE: a full clone.
- Product: a separate sparse clone with `product` and `testbench`, plus a
  read-only work tree of `his/` at a cleared commit. `tools/product-clone.sh`
  sets this up.

## Branches and pull requests

- Work on a branch; never on `main`.
- One component per pull request. The structure lands first; component PRs
  are stacked on it and say so in their description.
- Fill in the pull request template, including the gate status for `his/`.
- A change to `his/` is not for the product side until a gate review in
  `re/analysis/docs/` marks it **cleared**.

## Commits

- Commit under your own identity (a GitHub noreply address is fine).
- English for documents, code, comments, commit messages and issues.
- Run `tools/leak-check.sh` before you push. `tools/pre-commit` installs it as a
  hook: `ln -s ../../tools/pre-commit .git/hooks/pre-commit`.

## What never goes into the repository

See [SECURITY.md](SECURITY.md): decrypted images, decompiler output, dumps,
traces, local paths and secrets.

## Questions across the gate

The product side asks the RE side through an issue with the "HIS question"
template. Answers come back only as a change to `his/` that passes the gate.
