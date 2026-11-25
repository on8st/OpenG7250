# Security and leak policy

## Never commit

- Decrypted firmware images, flash or memory dumps, decompiler output, traces.
  Generated RE output stays in the git-ignored `re/analysis/analysis/generated/`
  and `re/emu/.private/`.
- Hashes of decrypted images, unless a decision allows a specific file
  (see `decisions/` and `tools/leak-allow.txt`).
- Local paths that name a person or machine (home folders, user names).
- Credentials, tokens, keys of any account.
- Binary vendor files outside `re/analysis/firmware/` (the public, encrypted
  release package).

`tools/leak-check.sh` enforces these rules locally and in CI.

## Hardware safety

Work on a real radio follows `his/90-needs-hardware.md`: read before write,
never write the bootloader region or the option bytes, dummy loads on every
antenna port.

## Reporting a leak

If something from the list above was pushed, open an issue titled
"Leak report" **without** repeating the leaked content, or contact the
repository owner directly. The owner decides on a history rewrite.
