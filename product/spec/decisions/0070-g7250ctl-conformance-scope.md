# 0070 — What g7250ctl must pass against the reference model

- Status: proposed (2026-11-22); answers question Q5 of the g7250ctl session (relayed by the
  openg7250 session, 2026-11-22)
- Date: 2026-11-22
- Builds on: 0019 (reference model), 0026 (g7250ctl as reference implementation), 0037 (Rust,
  shared protocol crate), 0063, 0067, 0069.

## Context

A1.1a says `g7250ctl` "MUST pass every A1 conformance test of Annex X3 against the reference
model". Most T-A1 tests check what the radio does, not what a host does, so a host tool cannot
pass them in any meaningful sense, and the host-side rules it implements are named nowhere as
tests.

## Decision

1. `g7250ctl` MUST drive every command of Table A1-2 against the reference model, and its
   host-side behaviour MUST be tested there:
   1. request ids and retransmission (A1.4): a new id per request, the same id and payload
      for a retransmission, every 500 ms, 5 times, and at least 16 s for a request that may do
      flash work (0069);
   2. authentication (A1.10): one challenge per authenticated request; the `CHANGE_SECRET` mask
      (new secret XOR HMAC-SHA256(old secret, "G7NS" ‖ challenge));
   3. the update sequence (A1.8): the package for the slot that is not running (tag 23); every
      block sent again in order, with new request ids, after a failed verify (0067); abort on
      a status that ends the transfer; the result read back with `IDENTIFY` after the reset;
   4. the package file of Annex M (0071): refused before `UPDATE_BEGIN` if it does not parse.
2. The T-A1 tests of Annex X3 stay tests of the radio (reference model, emulator or radio).
   `g7250ctl` MAY be the host that runs them; it does not "pass" them.
3. Test T-A1-93 (new) lists the host-side checks of point 1.

## Alternatives considered

- Keep "every A1 test": not meaningful for a host, and leaves the host rules untested.

## Consequences

- Spec A1.1a reworded; test T-A1-93.

## Sources

- Decisions 0019, 0026, 0037, 0067, 0069, 0071; g7250ctl session question Q5.
