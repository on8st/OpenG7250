# 0063 — Status codes for an unreadable package header and for update commands with no transfer

- Status: accepted (ON8ST, 2026-11-21, relayed by the openg7250 session); answers two testbench
  readings, 2026-11-21
- Date: 2026-11-21
- Builds on: 0008 (U-2, U-3), 0039 (Annex M, Table M-4), 0056 (malformed payload is 17), 0061
  addendum (`UPDATE_BEGIN` order 51, 56, 63, 61, 52).

## Context

The reference model answers `UPDATE_BLOCK` with no transfer in progress with 54, and
`UPDATE_VERIFY` and `UPDATE_ACTIVATE` with 55. It answers a package header with a wrong magic,
header version or component with 17. The spec stated neither case.

## Decision

1. **No transfer in progress** (no accepted `UPDATE_BEGIN`, or after `UPDATE_ABORT` or an
   activation): `UPDATE_BLOCK` gets 54 (block out of order: no block is expected);
   `UPDATE_VERIFY` and `UPDATE_ACTIVATE` get 55, as an activation before a good verify already
   does (T-A1-55). For these commands it is the first command-specific check, so a block with
   a bad CRC and no transfer gets 54, not 53. The general codes of Table A1-3 come first. No new
   code is needed; the reference model is right.
2. **Package header.** A wrong magic or an unknown header version means the header cannot be
   read: 17, in the framing group (0056); only header version 1 is defined. A readable header
   with a component other than 1 is a well-formed value that this firmware does not support:
   56, at the place of 56 in the `UPDATE_BEGIN` order (51, 56, 63, 61, 52). The reference model
   is right for the magic and the version, and must give 56 for the component.

## Addendum: timers and the state after `UPDATE_ABORT` (firmware question, 2026-11-21)

3. Only an update command answered with status 0 or 1 restarts the 10-minute timer of A1.8
   (`UPDATE_STATUS` and a second `UPDATE_ABORT` excluded, point 4). A
   refused update command (54, 55, 53, 58 and every other non-zero status) and a repeat
   answered from the stored reply neither restart nor extend any update timer. Otherwise a
   stray command from a confused host could keep the radio in the update state, without RX,
   for as long as it keeps sending.
4. After `UPDATE_ABORT` no transfer is in progress, but the update state remains until a
   restart (as A1.8 already said, T-A1-23). The timer runs from the abort reply and ends in the
   restart into the running image unless `RESTART` comes first. Until then: `UPDATE_BEGIN`
   and `ROLLBACK` 58; `UPDATE_BLOCK` 54; `UPDATE_VERIFY`, `UPDATE_ACTIVATE` 55; a second
   `UPDATE_ABORT` 0 with no effect, the timer included; `UPDATE_STATUS` idle. Test T-A1-82.

## Alternatives considered

- A new code "no transfer in progress": it would add a code for a host error that 54 and 55
  already describe well enough.
- 17 for a wrong component: the component is read correctly; 0056 keeps 17 for payloads that
  cannot be read.

## Consequences

- Spec A1.8 ("Unreadable or unsupported header", "No transfer in progress"); tests T-A1-80,
  T-A1-81. Reference model: component ≠ 1 gives 56.

## Sources

- Decisions 0008, 0039, 0056, 0061; Annex M Table M-4.
