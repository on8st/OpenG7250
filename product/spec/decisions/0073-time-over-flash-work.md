# 0073 — Time over flash work

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session); proposed
  2026-11-22; answers a firmware question on timers during flash work (relayed by the
  openg7250 session, 2026-11-22)
- Date: 2026-11-22
- Builds on: 0044 (trial self-confirmation), 0055 and 0065 (flash work), 0069 (one erase per
  request, `t_flash_reply`), 0020 §24 (write session, first-secret window).

## Context

A sector erase stalls the processor for up to `t_erase_max` = 4 s (B2.3, 0065), and a request
with flash work is answered within `t_flash_reply` = 15 s (0069). The accepted text counts
several limits on uptime (the trial's 60 s, the first-secret window, the 10-minute
abandonment, the write-session idle time, the stored-reply time) but does not say whether
uptime runs during flash work, nor when a limit that falls due during it acts, nor how it
relates to requests that arrived meanwhile. A time base that stops with interrupts off would
lose up to 8 s per erase and keep a window or a trial open longer than the text allows.

## Decision

1. **Uptime counts flash work.** Uptime (`IDENTIFY` tag 16, the log's timestamps) and every
   time limit counted on it MUST include the time spent in flash work, with interrupts off or
   not. The time of the flash work of one request, of a start's own store writes and of the
   trial self-confirmation MUST be counted to within 1 ms in total, whatever number of erase
   and program operations it holds. Over a run, uptime is then behind real time by at most
   1 ms per request with flash work, plus the tolerance of the clock it is counted from.
2. **The listed limits and their tolerance.** The trial limit of 60 s (A1.8), the
   first-secret window of 300 s (A1.10), the 10-minute abandonment (A1.8, A1.8a), the 60 s
   write-session idle time (A1.10) and the stored-reply time (A1.4) MUST NOT act before they
   fall due and MUST act within 100 ms after they fall due, in ordinary running too (a
   one-second tick is not enough). One that falls due during flash work MUST act within 100 ms
   after that work ends, before any management request received during the work is handled;
   such a request is judged by the state when it is handled, so a window or session that ended
   meanwhile does not stay open for it. Existing tests keep their wider tolerances (T-A1-82:
   ± 5 s); they lie within this.
3. **Trial confirmation.** A self-confirmation whose boot-record write started before the
   60 s and succeeds counts as confirmation; one that starts at or after 60 s does not (A1.8).
4. **Not on the list.** The P2 session watchdog (B5.6): flash work never overlaps an arm
   (B2.3), so over flash work it guards no TX, only the RX session, and B2.3 already lets the
   session end by its watchdog over an erase. The firmware MAY read the High Priority packets
   that waited through the work (port 1027 holds two, 0053) before it judges the watchdog, or
   judge the watchdog first; B5.6's 1100 ms bound holds outside flash work. Limits of transmit
   (A3.4) are not affected for the same reason. Work after the reply of a request that ends in
   a reset is followed by that reset, which ends every timer.
5. **Text.** B2.3 "Flash work" gains the bullet "Time over flash work"; A1.10 marks the
   first-secret window and A1.8 the trial's 60 s with a pointer to it.

## Alternatives considered

- Limits that stop during flash work: a window or trial would last longer than its stated
  time by the flash work, without bound over many requests.
- A test of the write-session idle time over another request's flash work: no case exists.
  The holder's verified requests refresh the idle time at receipt, every flash-work request
  needs the session (or is `SET_FIRST_SECRET`, with no secret and so no session), other
  hosts' writes get 35 without flash work, and 15 s < 60 s. T-A1-15 gains no case from this
  decision.

## Consequences

- B2.3 "Flash work": new bullet "Time over flash work". A1.10 first-secret window and A1.8
  trial: pointer to it.
- Tests: T-B5-22 (uptime over flash work), T-A1-16 (a failing `SET_FIRST_SECRET` at the end of
  the window), T-A1-94 new (trial and abandonment limits falling due during flash work).
- Host: a limit may act up to `t_flash_reply` + 100 ms late when it falls inside flash work,
  and a management request read after the work is judged then (it may find a window or
  session closed). A P2 session over a management write with an erase may end by its
  watchdog, as it may today.
- Firmware: a time base that keeps counting with interrupts off; due limits handled before
  queued management requests; a 100 ms tolerance for the listed limits in ordinary running.
- Operator: none in normal use; log timestamps and uptime stay true over updates and
  compactions.

## Sources

- Decisions 0044, 0053, 0055, 0065, 0069; [STM32H72x] Table 48 (erase time, through B2.3).
