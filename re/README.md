# re/: reverse engineering

Everything here studies the stock firmware and hardware of the G7250.

| Path | Content |
|---|---|
| `analysis/` | Decryption and Ghidra tooling, the public release package, findings in `analysis/docs/` |
| `emu/` | Emulator running the stock Main and Control images (added by its own PR) |

## Who may read this

Only the RE side. Anyone who reads `re/` does not write product material.
Product contributors never open, search or inspect `re/`, not even through git
history.

## Where results go

- Findings: `analysis/docs/`, as conclusions with addresses and sources,
  without code or listings.
- Hardware facts the product needs: a change to `his/`, which passes a gate
  review before the product side reads it.
- Generated output: the git-ignored `analysis/analysis/generated/` and
  `emu/.private/`, never committed.
