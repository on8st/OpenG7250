# Emulator black-box run (decision 0034): what to run and what passes

The image: `out/g7250.elf` from `./build.sh` (boot stage at 0x08020000 plus the application in
slot A at 0x08040000; program headers only). Load it at its load addresses and start from the
vector table at 0x08020000, as for an application image ([HIS] HIS 10). Return per test: pass or
fail, the console output (USART1 TX, PB6, 115200 8N1) and the network traffic. Nothing else.

The checksums of the delivered files are in `out/SHA256SUMS`.

## F1: boot, banner, watchdog (no network needed)

| Id | Check | Pass |
|---|---|---|
| F1-01 | Console after start | Lines in this order: `OpenG7250 boot …`, `boot: reset cause …`, three `boot: start …` lines (the state the previous stage left, HIS 90), one supply line (`boot: supply PWR_CR3 set to SMPS->LDO 2.5 V (was the power-on value)` when PWR_CR3 starts at its power-on value 0x00000006, `… already SMPS->LDO 2.5 V, not written` when it starts at 0x2E; any other value: see F1-10), `boot: safe state held, watchdog 2000 ms`, `boot: starting slot A at 0x08040000`, `OpenG7250 main firmware … slot A (G7250-P2, board type 200)`, `app: reset cause …`, `app: safe state held; TX NOT_CALIBRATED: every TX request is rejected (record tx bit 1 = 1, bit 4 = 0); ATU bypass; watchdog 2000 ms`. |
| F1-02 | Alive | `app: alive N s` every 10 s, N increasing, for at least 60 s. |
| F1-03 | Watchdog refreshed | No reset during 60 s: no second `OpenG7250 boot` line (IWDG1 2 s, refreshed every 250 ms). |
| F1-04 | Safe state before anything else | Within 20 us of the boot stage's first instruction at 0x08020000 (0045; T-A3-06), and before any RCC clock-tree change or RAM initialisation: GPIO clocks on (AHB4ENR), then one step per port in this order: port A (PA10 high), H (PH10 high, PH11 low, PH9 high), B (PB14 low, PB5 low, PB10 low), F (PF4 high, PF6 low), C (PC7 low), D (PD12 low, PD13 high, PD5 input with pull-up), G (PG2 high, PG5 high: ATU strobes inactive, never pulsed). Per port: one BSRR write with all levels, then OTYPER, PUPDR (port D only), MODER; so each pin's level is written before it becomes an output, and PD5's pull-up is on before it leaves its analog reset mode. Then the full per-pin safe state again in table order (also speed and pulls), with no level change. ([HIS] HIS 85 `d199d33`; 0029) |
| F1-05 | Pins never driven | PA4, PA12, PA11, PH12, PH3–PH6 stay in their reset mode for the whole run. PD5 (USART2 TX to the controller) is an input with pull-up from the safe state on: never an output, never low (HIS 70, HIS 85; T-B10-L11); nothing is sent on USART2. PD13, PG2, PG5 go high once and never low (no strobe pulse, so no ATU latch is written). |
| F1-06 | No forbidden write | No flash write outside sectors 6 and 7 (0x080C0000–0x080FFFFF, the store, used in turn: A1.8 "Store location", 0061; the application writes one 32-byte record there at every start, the boot counter of B10.2 item 5, and compacts into the other sector when one is full); in particular nothing in 0x08000000–0x0801FFFF; no write to OPTKEYR, OPTCR or any `*_PRG` option-byte register (FLASH_ACR, KEYR, CR, SR, CCR are used for sectors 6 and 7); no I²C traffic to 0x50; no write to RCC_BDCR, PWR_CR1.DBP or PWR_BDCR; no access to the Backup SRAM (0x38800000). |
| F1-07 | No valid slot | With slot A erased (0xFF), the boot stage prints `boot: slot A refused: no-record` (and the same for B; an unreadable or corrupt slot names its check instead, 0067), then `boot: no bootable image in slot A or B; holding the safe state`, then `boot: idle N` once a second, and does not reset. |
| F1-08 | Corrupt slot | With one byte of slot A changed after loading, `boot: slot A refused: integrity`; slot A is not started; with a good slot B whose staging is not open, `boot: fallback to slot B (active slot refused, no trial)` and B starts (0067, 0068 addendum). Slot B is checked, and named, only when it is about to be started. |
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

Not yet: DDC0 I&Q (B6; `STREAM_STATUS` DDC0 reason 1), ADC words in the status packet. At a
takeover the old owner, armed or not, gets the final `TX_ARM_STATE` (`TAKEOVER`) three times
about 10 ms apart at its session's register destination, then one copy per DUC I&Q or register
packet for 5 s, at most once per 100 ms (A2.4); takeovers are counted (`GET_STATS` `takeovers`)
and logged `p2: takeover: session of A taken over by B (n more takeovers of this pair since the
last entry)`, at most once per second per pair.
`LINK_STATUS` bit 17 is set for a non-default radio port in a General packet (T-B7-03). TX:
record `tx` bit 1 = 1 (r1.0, 0036), bit 4 = 0; an arm that meets every other condition is
refused with 12 `NOT_CALIBRATED`, and no TX request keys (B10 intro). The key path
(crates/safety keypath: PTT and CWX edges, keyed timeout, CWX 1 s cap, maximum TX time, PA
enable and mode-class unkeys) runs on the owner's High Priority packets, but every edge is
refused before any pin is touched: `TX_STATE` reads bit 0 = 0, source 0, unkey reason 0, and
bit 4 = 1 (PA disabled) until the owner's General packet sets byte 58 bit [0]. The
DUC-specific packet (B10.6) is followed: break-in, CW select, the keyer bits, speed, weight,
hang delay (above 2000 ms clamped, logged `p2: CW hang delay <ms> clamped to 2000`, `GET_STATS`
`cw_hang_clamped`), keyer speed (above 60 wpm used as 60, logged `p2: CW speed <S> used as 60`,
`GET_STATS` `cw_speed_clamped`; one per packet, 0046 SF-23),
EER and the DUC0 format refusals; a packet while keyed waits for the unkey. The CWX keyer
(crates/safety cw, B10.5a "Keyer timing", 0046: unit 1200/S ms, weight 33-66 as a mark/space
shift, Curtis iambic A/B decided at the end of each space, element memory in mode B only (the opposite
paddle held during the mark), strict spacing 3 T from the end
of the mark; straight key with iambic clear; host tests T-B10-43 to T-B10-46) and the
hang delay exist but form nothing here. KEY keys nothing in v1.0 (Table B10-1, 0030). The TX frequency (High Priority bytes 329-332, phase word, B3.8) is followed: a request
outside the ranges of Table A3-0 does not key (`TX_STATE` bit 3); while keyed a retune in the
same `tx_band` entry is followed, another entry unkeys with reason 6, outside every range with
reason 4 and limit id 1 (A3.9). General byte 37 bit [3] = 0 refuses every request (B3.8). The
DUC FIFO and the underrun check exist (B10.5, B10.7), but with record `tx` bit 4 = 0 every DUC
I&Q packet is discarded and counted before it, and High Priority status byte 30 bit [2] stays 0. Not
yet: the RF-path relays and their bounded wait (A3.9a; P-RF-1..3, HIS-07), the ATU cycle,
LIMIT_STATUS, and the A3.9a key sequence on the pins.

## Link-local address and conflicts (A1.11; 0048, 0049)

With no DHCP lease after the start-up attempts, the radio claims a link-local address as RFC
3927 section 2 before any socket opens: its own address at 0.0.0.0, socket 0 in MACRAW, three
probes (sender 0.0.0.0, 1-2 s apart after a random 0-1 s), 2 s wait, two announcements 2 s
apart, then the sockets open. Console: `net: probing link-local <ip> (RFC 3927)`, `net:
link-local <ip> probed and announced`, and per conflict `net: link-local address conflict on
<ip> while probing; next candidate <ip>`. A lease end or DHCPNAK probes the address again with
every socket closed (the session ends, A1.11a). An IP_CONFLI (GINT bit 7) while running is
logged (`net: address conflict on <ip>`, and in the log): on a lease, DHCPDECLINE, link-local,
DHCP again after 10 s; on a link-local address, the next candidate; on a static address, the
previous configuration (C-4), or link-local if there is none. After 10 conflicts each new probe
waits 60 s, with no address and the sockets closed. If socket 0 cannot open in MACRAW (three tries,
10 ms apart), the address is used unprobed (0057): console `G7250 FAULT LINKLOCAL <ip> UNPROBED`
once, log `link-local <ip> used without a complete RFC 3927 probe`, `GET_STATS` `ll_unprobed`,
`IDENTIFY` tag 24 = 1; not an A3.5 fault. Start-up bound without DHCP: 17 s from link up (0057).
The emulator needs MACRAW on socket 0 and IP_CONFLI in its CH394 model; the MACRAW length header is read as the frame length alone
(`MACRAW_LEN_INCLUDES_HEADER`, to confirm).

## DHCP retry and renewal (A1.11, A1.11a)

Console: `net: DHCP lease L s from <server>; renewal at L/2 s`, or `net: no DHCP lease; link-local,
DHCP keeps trying (A1.11)`; later `net: address … (Dhcp | LinkLocal)` when the address changes.

| Id | Set-up | Pass |
|---|---|---|
| DH-01 | No DHCP server at power-on, one added after a minute | Link-local first; DISCOVER from 0.0.0.0:68 to 255.255.255.255:67 again after 4 s, then with back-off (8, 16, 32, 64 s; then every 64 s; each exchange waits 3 s); once the server answers, the radio takes the lease (an address change: a running session ends) |
| DH-02 | Lease of 120 s | At 60 s a REQUEST with ciaddr = the address in use, unicast to the server, no options 50/54; ACK with the same address changes nothing (a session goes on) |
| DH-03 | As DH-02, server silent at T1 | Retries after a pause (half the remaining time, at least 60 s); at T2 (105 s) the REQUEST is broadcast (rebinding) |
| DH-04 | Server answers the renewal with DHCPNAK, or no answer until the lease ends | The radio stops using the address: link-local, the session ends, DHCP starts over at once |
| DH-05 | Renewal ACK with another address | Address change under A1.11a: the session ends, the new address is used |
| DH-06 | Management requests during an exchange | Socket 7 is DHCP (port 68) for up to 3 s per exchange; management requests in that window get no answer (the host retransmits, A1.2); P2 sockets and the session watchdog keep running |

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
| ST-06 | Flash ECC: a word with a double ECC error (DBECCERR1, the read a bus error), if the emulator can model it: (a) in the live store sector, in a record's header or payload; (b) in the other slot (a power cut while a block was programmed); (c) in the newest commit word | No fault and no watchdog reset. (a) the store opens, the older record of that kind is used, writes go on after the word; (b) `boot: slot B refused: unreadable` and slot A starts; IDENTIFY tag 9 of that slot 2 (bad); ROLLBACK refused; (c) the previous generation stays live. See `docs/memory-layout.md`, "Flash ECC (R0)" |

"Every value present" can be read over the management channel once step 4 exists; until then
the console boot counter (monotonic, never back to 1) is the visible check.

## Update path (A1.8; 0043)

Files: `out/g7250-<version>-slot-a.g7pk`, `out/g7250-<version>-slot-b.g7pk` (0071): the Table M-4 header (53 bytes, the
`UPDATE_BEGIN` payload after the flags byte) followed by the sealed image for that slot. The
running image is in slot A (`IDENTIFY` tag 23 = 1), so the B package is the one to send; both
have the running version (accepted: not older). Console lines: `boot: record active … trial …`,
`boot: starting slot B as a trial`, `net: trial boot of slot B: arm refused (MAINTENANCE) until
confirmed`, `net: trial image confirmed (slot B)`, `boot: trial not confirmed: rolled back (U-6)`.

| Id | Do | Pass |
|---|---|---|
| UP-01 | `UPDATE_BEGIN` with the A package | 63, nothing written (slot B unchanged), no update state (T-A1-67) |
| UP-02 | `UPDATE_BEGIN` (B package), all blocks, `UPDATE_VERIFY`, `UPDATE_ACTIVATE` [0] | 0 each; verify returns the package's SHA-256; activate replies [0]; reset; boot stage starts slot B as a trial; confirmation after about 10 s; next start: slot B, no trial |
| UP-03 | After `UPDATE_BEGIN`: P2 run request; a register write; `CAP_REQUEST`; an arm | session ended; `STREAM_STATUS` reason 10 at most every 100 ms; write `BUSY`; record answered; arm `MAINTENANCE` (T-A1-23) |
| UP-04 | During the trial (before confirmation): RX session and an arm | RX works; arm `MAINTENANCE` (T-A1-24); `UPDATE_BEGIN` 58 |
| UP-05 | Trial whose network never comes up (no CH394 answer), or with a class 2 fault (force a safe-state pin) | no confirmation; reset at 60 s from the main task; boot stage rolls back to slot A and says so; `READ_LOG` after the start shows the rollback (T-A1-09, T-A1-68) |
| UP-05a | Reset the trial image before its confirmation (watchdog: stall the executor; power cut; a fault) | next start, and every start after: slot A, rolled back; slot B is never started again until a new activation (T-A1-68) |
| UP-06 | Power cut at any flash word or erase during blocks, verify or activate | next start: slot A, unchanged; a new update completes |
| UP-07 | Power cut during the boot stage's trial-attempt write or the confirmation | next start: trial again (attempt not recorded), slot A rolled back (attempt used, not confirmed), or slot B (confirmed) |
| UP-08 | Flash program error (FLASH_SR1 error flag) during a block | 55, logged, transfer discarded; slot A and the boot record unchanged (0043) |
| UP-09 | `UPDATE_BEGIN`, then only `UPDATE_STATUS` for 10 min | logged, restart into slot A (T-A1-34); the same after `UPDATE_ABORT` and after a verify without activate (T-A1-46) |
| UP-10 | `ROLLBACK` after UP-02 (running B, A valid) | reply, reset, slot A starts, not a trial |

## F3 status (transmit gate)

No network path to TX exists yet (no High Priority packet, no register map). What the image
shows: the banner line of F1-01, and that no pin of the TX path changes (F1-04, F1-05). The
gate itself is covered by host tests (`cargo test -p g7250-safety`).

## First radio (hardware only; the emulator starts at 0x08020000 and cannot decide these)

| Id | Check | Expected |
|---|---|---|
| RD-01 (T-A3-60, P-SYS-9; the log ring 0x2401E700–0x2401FF7F too, 0068: `start after software: log kept` after a `RESTART`) | TRIP_COUNT over a reset without power loss (A3.5; persist.rs; 0050: part of the freeze rule, TX stays locked until shown). Make `TRIP_COUNT` non-zero (any limit trip once the lock opens; until then a debugger write of the `.persist` block through `persist::set_trip_count` semantics), then a software reset (`RESTART`) and, separately, a watchdog reset. | After each: `app: TRIP_COUNT n (kept over the reset)` with the same n, and `TRIP_COUNT` reads n. If it reads `(power-on or not found: from 0)`, the stage before the boot stage overwrites 0x2401FF80-0x2401FFFF: an open HIS item gets the measured answer and the block moves to the address it gives. Pending hardware parameter. |
| RD-02 | MACRAW length header ([CH394] 12.2.7). With no DHCP server, capture the start-up probe and look at the console. | Three ARP probes and two announcements on the wire, each 60 bytes. If `net: link-local ... probed` never comes or the frames read wrong, the 2-byte length counts itself: set `MACRAW_LEN_INCLUDES_HEADER` (crates/ch394). |

| RD-03 (T-B5-20, 0054) | Busy-pass time on the radio. With a session running, send 20 High Priority packets 1 ms apart (no management write, compaction or update in progress, 0055), then read `GET_STATS`. | `busy_pass_max_us` well under 2000 (two packets held by port 1027's 4 KiB); about 900 expected at SPI6 16 MHz (estimate). `net_send_dropped` 0 with a live host. A value near or above 2000 means the margin of 0054 is not met: report it with the SPI clock. |
| RD-04 (B2.3, 0065 item 1) | Flash parallelism for the erase bound. The driver never writes FLASH_CR PSIZE (board/common flash.rs), so it erases and programs at the reset value. With a debugger, read FLASH_CR1 after reset and before the first store write, and note PSIZE (bits 5:4, RM0468 FLASH_CR). | PSIZE recorded. `T_ERASE_MAX_MS` = 4 s (DS13311 Rev 5, Table 48, the x8 row, the largest) bounds every PSIZE value, so nothing changes. A value other than the reset value the RM gives is a finding: report it to the HIS. |
