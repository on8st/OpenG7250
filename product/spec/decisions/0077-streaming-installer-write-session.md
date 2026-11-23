# 0077 — The streaming installer: write session, power loss, identification

- Status: accepted (ON8ST, 2026-11-22, relayed by the openg7250 session); proposed
  2026-11-22; answers questions from the reference model's installer and the testbench on
  A1.8a item 3 (relayed by the openg7250 session, 2026-11-22). Not urgent: the gate of
  A1.8a item 3 is closed in this version.
- Date: 2026-11-22
- Builds on: 0020 §24 and 0022 §75 (write session), 0022 §86 item 7 (repeats), 0024 and 0028
  (streaming installer), 0069 (`t_flash_reply`), 0073 (time over flash work). The "restart
  needed" value of `UPDATE_ABORT` before the first erase is in 0074.

## Context

A1.8a item 3 lists the commands the installer answers and says nothing on the write session;
`END_WRITE_SESSION` gets 58 there. After the first erase the transfer must be completed, so a
holder that dies, or a tool that restarts on a new source port, would leave every other host
with 35, for ever if the installer keeps no idle timer. The installer also erases and
programs a whole sector at the last block of it, which delays that block's reply by up to
`t_flash_reply` = 15 s.

## Decision

1. **Write session in the installer.** It continues there: held by the host whose foreign
   `UPDATE_BEGIN` was accepted, ending after 60 s idle as A1.10 says; `END_WRITE_SESSION` is
   answered as A1.10 says and joins the installer's commands. Another host may then continue
   the transfer. Before the first erase the session also ends with the restart.
2. **The idle time pauses during the holder's own request** (A1.10). From receipt to reply the
   idle time does not run, in the installer and in this firmware alike, so a holder waiting for
   a reply delayed by flash work never loses the session. This is the one exception to 0073's
   "a limit that falls due during flash work acts within 100 ms after it ends": its list says
   that the write-session idle time does not run during the holder's own request. Repeats
   answered from the stored reply still neither refresh nor pause it (A1.4; 0022 §86 item 7).
3. **Power lost with every sector written** (editorial). "No way back": the main unit may have
   no valid application, or a stock image that was not checked by `UPDATE_VERIFY` and that the
   vendor bootloader may start; either way the path to a checked image is the vendor path.
   What the vendor bootloader then starts is vendor behaviour, not specified here (A1.8a
   "Clean room"; 0025); no test.
4. **Identification and the first secret in the installer** (editorial). From the first erase
   `IDENTIFY` reports no secret set (tag 11 = 0) and no first-secret window (tag 12); the
   installer opens none, so `SET_FIRST_SECRET` gets 36 (Table A1-3 order: 36 before 58), also
   before the first erase; the transfer goes on with the secret the installer keeps in RAM.

## Alternatives considered

- No write session in the installer: any host could interleave blocks into a one-way
  transfer.
- Relying on arithmetic (15 s reply within 60 s idle) instead of the pause: it holds today but
  breaks as soon as a write takes longer; the pause makes it hold whatever a write takes.

## Consequences

Accepted text that changes:
- A1.8a item 3 "Installer state" (0024, 0028): `END_WRITE_SESSION` added to the command list;
  two sentences added (points 1 and 4).
- A1.8a item 3 "No way back" (0028): one clause reworded, editorial (point 3).
- A1.10 "One writer at a time" (0020 §24, 0022 §75): the idle time pauses during the holder's
  own request (point 2); refreshing and the repeat rule are unchanged.
- B2.3 "time over flash work" (0073): the list names the pause.

Tests: T-A1-60 (identification, 36, 58, the write session over the installer, and the 57 s
spacing (corrected by 0078) around the block that completes a sector) and T-A1-15 (a stretched compaction from
the holder) extended. T-A1-50 is unchanged: its requests do no long flash work.

Cost: the installer keeps the 60 s idle timer, answers `END_WRITE_SESSION` and pauses the idle
time while it executes the holder's request; this firmware has the same pause. A broken-off
transfer after the first erase can be taken over after `END_WRITE_SESSION` or at most 60 s,
instead of never.

## Sources

- Decisions 0020, 0022, 0024, 0025, 0028, 0069, 0073, 0074.
