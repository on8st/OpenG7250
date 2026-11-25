## What this changes

<!-- One component per PR. Say which side and which paths. -->

- Side: RE / Gate (his) / Product / Shared (testbench) / Structure
- Paths:
- Stacked on: <!-- e.g. #1, or "none" -->

## Checklist

- [ ] `tools/leak-check.sh` passes
- [ ] English throughout (documents, code, comments, commit messages)
- [ ] No decrypted images, dumps, decompiler output or local paths
- [ ] Product side: nothing taken from `re/`; hardware facts only from a cleared `his/`
- [ ] `his/` changes: gate review in `re/analysis/docs/` with verdict: <!-- CLEARED / pending -->
- [ ] Decisions recorded where a rule or design choice changes
