# 0056 — A malformed command payload is status 17, in the framing group

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers a firmware
  precedence question, 2026-11-21 (b8d1ade)
- Date: 2026-11-21
- Builds on: 0020 §27, 0022 §51, 0039 (Annex M), 0055.

## Decision

1. A payload that does not have the size or structure Annex M gives for its command counts as
   malformed, status 17, in the framing group of Table A1-3: before authentication and every
   state check (36, 35, 58, 57, 50). It cannot be read, so there is nothing to check its state
   against. The firmware already does this.
2. A payload that decodes but carries a value that is not allowed is a command-specific fault
   (48, 49, 51–56, 59–63), after the state checks.
3. Spec A1.4 "Malformed payload"; test T-A1-75. T-B5-21 uses well-formed payloads and is not
   affected.

## Sources

- Firmware via openg7250, 2026-11-21; decisions 0020, 0022, 0039, 0055.
