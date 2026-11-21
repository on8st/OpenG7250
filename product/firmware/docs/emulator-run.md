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
| F1-04 | Safe state before anything else | The first GPIO writes after reset, in this order: PA10 high, PH10 high, PB14 low, PF4 high, PF6 low, PC7 low, PH11 low, PD12 low, PD13 high, PG2 high, PG5 high (ATU strobes inactive, never pulsed), PB5 low, PB10 low, PH9 high (each pin: level written first, then made an output), PD5 input with pull-up (output register high and pull-up set before the pin leaves its analog reset mode). Before any RCC clock-tree change. ([HIS] HIS 85 `d199d33`; 0029) |
| F1-05 | Pins never driven | PA4, PA12, PA11, PH12, PH3–PH6 stay in their reset mode for the whole run. PD5 (USART2 TX to the controller) is an input with pull-up from the safe state on: never an output, never low (HIS 70, HIS 85; T-B10-L11); nothing is sent on USART2. PD13, PG2, PG5 go high once and never low (no strobe pulse, so no ATU latch is written). |
| F1-06 | No forbidden write | No flash write outside sector 7 (0x080E0000–0x080FFFFF, the store; since step 2 the application writes one 32-byte record there at every start, the boot counter of B10.2 item 5); in particular nothing in 0x08000000–0x0801FFFF; no write to OPTKEYR, OPTCR or any `*_PRG` option-byte register (FLASH_ACR, KEYR, CR, SR, CCR are used for sector 7); no I²C traffic to 0x50; no write to RCC_BDCR, PWR_CR1.DBP or PWR_BDCR; no access to the Backup SRAM (0x38800000). |
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

## Step 2: management channel, read commands (UDP 7251)

Expected console after the discovery line: `net: management listening on UDP 7251 (protocol 1.0)`;
before it, `app: boot counter N` (N + 1 after every start).

Built: `IDENTIFY` (Table M-2; also from outside the subnet), `GET_STATS`, `GET_CHALLENGE`,
`GET_CONFIG` (factory configuration until a write exists), `GET_CALIBRATION` (no set: id zeros,
date 0); framing and status order of A1.4/A1.3; repeat cache; one challenge per host for 10 s;
first-secret window (300 s from network up, reported in tag 12); source rule (own subnet only,
except `IDENTIFY`; never a broadcast source); reply caps (20/s per source, 100/s total, 50/s
`IDENTIFY`, 10/s `IDENTIFY` off-subnet); 5 authentication failures in 10 s → 34.

Expected answers that look like failures but are this step's scope: every authenticated request
gets 32 (no trailer) or 33 (no secret can be set yet); `SET_FIRST_SECRET`, `READ_LOG`,
`UPDATE_STATUS` and all writes get 56 until step 4. `GET_CHALLENGE` gets 56 if the RNG does not
deliver (HSI48 or RNG not modelled).

## Step 3: P2 session, capability record, register map

Console after the discovery line: `net: P2 ports 1025 1026 1027 1029 1035 7250 open`; per session
`p2: session of <ip>:<port> started[ (restart)][ (takeover)]` and `p2: session of … ended (Stop |
Watchdog | Takeover | LinkDown | address change)`.

Built: all eight sockets of Table B2-1 (1028 never); General on 1024 (byte 4 = 0x00, ≥ 60
bytes), DDC-specific 1025, DUC-specific 1026, High Priority 1027 (≥ 1444 bytes; shorter C&C
discarded, B11.3); set-up phase (B5.1a), run bit start, owner IP + port, stop, takeover by any
C&C of another host, watchdog 1 s (only when General byte 38 bit 0 is set or no General yet),
restart by the same host (B5.4), refusal below 100 Mbit/s (B5.8); discovery byte 4 = 0x03 for
other hosts while running; REDISCOVERY drop. Register map on 7250 (B8): `CAP_REQUEST` from any
host → echo + whole record (207 registers) in one packet, the unasked record once per session,
every register of Table B8-2 readable by write, the write rules of B8.4/B8.5 (non-owner
`NOT_OWNER` once per 100 ms, no answer to source port 7250). High Priority status (60 bytes,
from 1025) every 50 ms while running; all fields 0 except the sequence number.

Not yet: DDC0 I&Q (B6; `STREAM_STATUS` DDC0 reason 1), DUC I&Q (ignored), ADC words in the
status packet, the three-times notice to an old owner (sent once, and only when it was armed),
`LINK_STATUS` bit 17. TX: arm refused with 11 `TX_NOT_IMPLEMENTED` (record `tx` bit 1 = 0), as
agreed until the key path exists.

## Store: watchdog- and power-loss-safe compaction

Image: `out/test/g7250-compact.elf` (test build: compacts the store at every start; never for
the radio). Console: `app: boot counter N`, `app: TEST compaction: live sector S generation G`,
`app: TEST compaction done: live sector S' generation G+1`, with S' alternating 7 → 6 → 7 …

What the emulator should force, and what passes:

| Id | Force | Pass |
|---|---|---|
| ST-01 | Sector erase time at the STM32H725/735 datasheet maximum (FLASH_SR1 BSY/QW held for that long after START) | No watchdog reset during the erase; the compaction ends; the next start shows the next generation and N + 1 |
| ST-02 | Erase time beyond 10 s (BSY never clears) | No hang, no watchdog reset loop: `app: TEST compaction FAILED (…Erase(…))`, the radio goes on running on the old sector; next start: same generation, N + 1 |
| ST-03 | Power cut during the erase of the other sector (any point) | Next start: same generation as before, every value present, N ≥ the last value |
| ST-04 | Power cut after each flash word written during steps 2-3 (copy, commit) | Next start: generation G or G+1, every value present, N never goes back |
| ST-05 | During ST-01, check the bus | No instruction fetch from 0x08000000–0x080FFFFF between START and the end of BSY/QW; interrupts masked; IWDG KR written with 0xAAAA during the poll |
| ST-06 | Flash ECC fault on reading a word of a half-erased sector, if the emulator can model it | Known residual risk (see `docs/memory-layout.md`, "Residual risk"); please report what happens |

"Every value present" can be read over the management channel once step 4 exists; until then
the console boot counter (monotonic, never back to 1) is the visible check.

## F3 status (transmit gate)

No network path to TX exists yet (no High Priority packet, no register map). What the image
shows: the banner line of F1-01, and that no pin of the TX path changes (F1-04, F1-05). The
gate itself is covered by host tests (`cargo test -p g7250-safety`).
