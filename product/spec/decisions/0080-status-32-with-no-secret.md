# 0080 — Status 32 with no HMAC or no secret set

- Status: accepted (ON8ST, 2026-11-23, relayed by the openg7250 session, with the counter
  `mg_auth_required`); proposed 2026-11-23; answers a gap found by g7250ctl's run against the
  reference model, relayed by the openg7250 session.
- Date: 2026-11-23
- Builds on: 0020 §27 (status codes), 0022 §11 (challenge per host, stored replies), 0022 §73
  item 21 (status 34), 0039 (retransmission after a reset), 0074 (host rule after a reset,
  Table M-6), 0077 (the installer reports no secret set).

## Context

Status 32 "authentication required" was named only in Table A1-3; no sentence said when it is
sent. A1.9 step 3 and the host rule of A1.4 named only 33 for a retransmission after a reset,
but after a `FACTORY_RESET` no secret is set and there is nothing to verify an HMAC against.
The reference model answered 32, and g7250ctl accepted both.

## Decision

1. A request for a command marked "yes" in the Auth column of Table A1-2 gets 32 and is not
   executed when it has no HMAC trailer (flag bit 0 clear), or when no secret is set, whatever
   its trailer holds. 33 is sent only when a secret is set and the HMAC does not verify with
   it, or the challenge is missing, spent or expired.
2. A 32 is not an authentication failure: it does not count toward the limit of status 34, it
   uses up no challenge and it does not refresh the write session. It is counted in
   `mg_auth_required` (Table M-6, now 46 names; the name has 16 characters, within the bounds
   of 0074), not logged one by one.
3. In the installer (A1.8a item 3) the secret is the one kept in RAM, so a wrong HMAC there gets
   33 although `IDENTIFY` tag 11 reports no secret set (0077).
4. A1.9 step 3: a retransmission after the reset gets 33, or 32 when no secret is set any more.
   A1.4 "Requests that end in a reset": a host stops at the first 33 or 32.
5. Precedence: unchanged. Table A1-3 already orders 32, 33, 34, 36 before every other code;
   `SET_FIRST_SECRET` keeps 36 and `GET_CHALLENGE` stays unauthenticated.
6. Test T-A1-99.

## Alternatives considered

- 33 with no secret set: it tells the host its secret is wrong, and it counts toward 34. Five
  retransmissions of a `FACTORY_RESET` within 10 s would lock the host out of `GET_CHALLENGE` and
  of authenticated requests just when it should set the first secret. With no secret there is
  nothing to guess.
- A log line for every 32 instead of a counter: not chosen (ON8ST, 2026-11-23).

## Cost

- Firmware: the order of the checks, and one counter.
- Host: a 32 means "set a first secret" or "add the HMAC"; g7250ctl stops retransmitting a reset
  request at 32 as at 33.
- Operator: none.

## Consequences

- Spec A1.10, new bullet "No secret, no HMAC".
- Spec A1.9 step 3 (0039) and A1.4 "Requests that end in a reset" (0074).
- Spec Table M-6 (0074): intro (46 names) and row `mg_auth_required`.
- Test T-A1-99 (new).

## Sources

- Decisions 0020, 0022, 0039, 0074, 0077; g7250ctl's run against the reference model (relayed
  by the openg7250 session, 2026-11-23).
