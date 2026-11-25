# 0081 — Flash work that never ends

- Status: accepted (ON8ST, 2026-11-24, "Beperk de 55", relayed by the openg7250 session;
  reading (a) of point 1 chosen on 2026-11-25, "(a) 0,5 s na einde").
- Date: 2026-11-25
- Builds on: 0055 (flash work), 0065 point 2 (flash work that fails), 0069 point 2 (reply
  within `t_flash_reply` = 15 s).

## Context

0065 bounds a failed erase or program at 2 × `t_erase_max`, so that "firmware that does not
stall" can report the failure (0065, Alternatives), and 0069 asks for the 55 within 15 s.
T-A1-85, T-A1-15, T-A1-16 and T-A1-94 test it with an erase that does not end.

The MCU has a single flash bank of 1 MiB, and an instruction fetch or read from that bank waits
until a running erase or program of it ends (STMicroelectronics DS13311 and RM0468, public ST
documents, as relayed by the openg7250 session on 2026-11-24; not read by the spec session, no
section cited; `sources.md` [STM32H72x]). With an erase that never ends, no code from flash can
run: the firmware gives up after 2 × `t_erase_max`, but the core stalls on the next fetch from
flash and the watchdog resets it. The emulator shows this: the bound ends at 8.000 s, one fetch
stalls, the IWDG resets 2 s later; afterwards the store and the boot record are as before, and
`WDT_RESET_AT_START` is reported.

## Decision

1. The 55 within `t_flash_reply` = 15 s (0069) holds for flash work that ends with an error flag
   or within its bound. Flash work that ends late, after the firmware has given up, is
   answered with 55 within 0.5 s of its end. That is within 15 s unless the give-up came close
   to it, as with a compaction whose last program hangs (4 s + 2.4 s + 8 s = 14.4 s).
2. Flash work that never ends takes the existing stall path of B2.3 (0065): the watchdog resets
   the processor, no reply is sent, the store and the boot record are as before the
   operation, and the next start reports `WDT_RESET_AT_START`. A retransmission after the
   restart gets 33, or 32 with no secret set (A1.4, A1.9; 0080).
3. No change to the firmware.

## Alternatives considered

- (b) 15 s for every late end: it needs a shorter give-up, or a self-reset instead of the
  reply after a late end past 14.5 s; both are firmware changes, against point 3.
- Keep the 55 for work that never ends: no firmware on this MCU can send it while the flash
  bank is busy; the code would have to run from RAM until the reply is out.

## Cost

- Operator: none new; a hung flash shows as a watchdog reset (class 1, clearable).
- Firmware: none.
- Host: a request with flash work may get no reply; the 16 s of retransmission (A1.4) then
  end without one, and the host learns the outcome after the restart (log, `IDENTIFY`,
  `GET_CONFIG`). A late end past the host's 16 s is learned the same way.

## Consequences

- Spec A1.4 (host rule for requests with flash work); B2.3 "one erase per request, and the
  reply time" and "flash work that fails".
- Tests T-A1-85 (a late end and an end that never comes, split), T-A1-15, T-A1-16 and T-A1-94
  (their hung erase becomes one that ends 100 ms after the give-up; the timings still hold).
- `sources.md` [STM32H72x]: the single-bank fact, relayed, not read.

## Sources

- STMicroelectronics DS13311 and RM0468 (public), as relayed; decisions 0055, 0065, 0069,
  0080; the emulator's run relayed by the openg7250 session, 2026-11-24.
