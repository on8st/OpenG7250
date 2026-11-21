# Emulator black-box run (decision 0034): what to run and what passes

The image: `out/g7250.elf` from `./build.sh` (boot stage at 0x08020000 plus the application in
slot A at 0x08040000; program headers only). Load it at its load addresses and start from the
vector table at 0x08020000, as for an application image ([HIS] HIS 10). Return per test: pass or
fail, the console output (USART1 TX, PB6, 115200 8N1) and the network traffic. Nothing else.

The checksums of the delivered files are in `out/SHA256SUMS`.

## F1: boot, banner, watchdog (no network needed)

| Id | Check | Pass |
|---|---|---|
| F1-01 | Console after start | Lines in this order: `OpenG7250 boot …`, `boot: reset cause …`, three `boot: start …` lines (the state the previous stage left, HIS 90), one supply line (`boot: supply PWR_CR3 set to SMPS->LDO 2.5 V (was the power-on value)` when PWR_CR3 starts at its power-on value 0x00000006, `… already SMPS->LDO 2.5 V, not written` when it starts at 0x2E; any other value: see F1-10), `boot: safe state held, watchdog 2000 ms`, `boot: starting slot A at 0x08040000`, `OpenG7250 main firmware … slot A (G7250-P2, board type 200)`, `app: reset cause …`, `app: safe state held; TX NOT_CALIBRATED: every TX request is rejected (record tx bit 1 = 0, bit 4 = 0); ATU bypass; watchdog 2000 ms`. |
| F1-02 | Alive | `app: alive N s` every 10 s, N increasing, for at least 60 s. |
| F1-03 | Watchdog refreshed | No reset during 60 s: no second `OpenG7250 boot` line (IWDG1 2 s, refreshed every 250 ms). |
| F1-04 | Safe state before anything else | The first GPIO writes after reset, in this order: PA10 high, PH10 high, PB14 low, PF4 high, PF6 low, PC7 low, PH11 low, PD12 low, PD13 high, PG2 high, PG5 high (ATU strobes inactive, never pulsed), PB5 low, PB10 low, PH9 high (each pin: level written first, then made an output). Before any RCC clock-tree change. ([HIS] HIS 85 `d199d33`; 0029) |
| F1-05 | Pins never driven | PA4, PA12, PA11, PH12, PD5 (USART2 TX), PH3–PH6 stay in their reset mode for the whole run; nothing is sent on USART2. PD13, PG2, PG5 go high once and never low (no strobe pulse, so no ATU latch is written). |
| F1-06 | No forbidden write | No write to flash 0x08000000–0x0801FFFF; no write to any FLASH register except FLASH_ACR (no KEYR, OPTKEYR, OPTCR, CR, *_PRG); no I²C traffic to 0x50; no write to RCC_BDCR, PWR_CR1.DBP or PWR_BDCR; no access to the Backup SRAM (0x38800000). |
| F1-07 | No valid slot | With slot A erased (0xFF), the boot stage prints `boot: slot A not bootable: BadMagic` (and the same for B), then `boot: no bootable image in slot A or B; holding the safe state`, then `boot: idle N` once a second, and does not reset. |
| F1-08 | Corrupt slot | With one byte of slot A changed after loading, `boot: slot A not bootable: BadCrc`; slot A is not started. |
| F1-10 (T-A3-59) | Supply fault before the network (A3.5a; 0040) | With PWR_CR3 starting at another supply configuration (for example 0x00000002), or with ACTVOSRDY never set: after the banner and the `boot: start …` lines, exactly `G7250 FAULT SUPPLY <PWR_CR3, 8 upper-case hex digits> MISMATCH` (or `TIMEOUT`) CR LF, once per second, nothing else; PWR_CR3 not written; safe state held; PB5 (CH394 RST#) stays low; the IWDG is not started by the image (only refreshed); no flash write; no application start; no discovery or management answer. |
| F1-09 | Panic or hang | Not provoked from outside; covered by the watchdog: any reset prints the boot banner again with reset cause `watchdog`. |

## F2: CH394, address, discovery (needs the CH394 model on SPI6, CS PG15, RST PB5)

Expected console after F1-01: `net: CH394 answers, version register …`, `net: MAC …`,
`net: link … Mbit/s …`, `net: address … mask … gateway … (Dhcp | LinkLocal)`,
`net: discovery listening on UDP 1024 (board G7250-P2, type 200)`.

| Spec test | Runs in F2 | Note |
|---|---|---|
| T-B3-01, -02, -03, -04, -05, -06 | yes | -03 needs the directed broadcast of the address in use |
| T-B3-09, -10, -11, -12, -13, -16, -24, -30 | yes | -12/-30: byte 13 = 1 (version 0.1), byte 23 ≠ 0 (beta build) |
| T-B3-14 | byte 20 = 1 only | no DDC streams yet |
| T-B3-18, -20, -26, -33 | yes | |
| T-B3-31 | replies only | at most 10/s per source; the `GET_STATS` count does not exist yet |
| T-B3-25 | partly | address applied and answered; not stored across restart (no configuration store yet); "while running/armed" not testable yet |
| T-B3-21 | yes | only port 1024 is open in F2 (plus 68 during DHCP at start) |
| T-B3-32 | expected to fail | second half: see spec question SQ-F2-2 |
| T-B3-07, -08, -15, -17, -19, -22, -23, -27, -28, -29 | no | need the P2 session (B5), streams or the arm |
| DHCP | yes | with a DHCP server on the emulated LAN: DISCOVER and REQUEST from 0.0.0.0:68 to 255.255.255.255:67 with the broadcast flag; the leased address is used. Without one: a 169.254.x.y address after about 9 s. |

## F3 status (transmit gate)

No network path to TX exists yet (no High Priority packet, no register map). What the image
shows: the banner line of F1-01, and that no pin of the TX path changes (F1-04, F1-05). The
gate itself is covered by host tests (`cargo test -p g7250-safety`).
