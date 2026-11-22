# G7250-P2 reference model

An executable model of a G7250 running the new firmware, built from the specification only
(decision 0019), so that the conformance tests, `g7250ctl` and AetherSDR's G7250 backend can be
tested before and alongside the real firmware.

- Source of truth: `product/spec/` (branch `spec/draft` at `0e075f2`: parts A and B, Tables
  A1-1 to A1-4, A3-0, B2-1, B4-1, B8-2, B8-3, Annex M, Annex P) and the cleared HIS at `d07627e`
  (HIS 85 safe state). Never the reverse-engineering side.
- Language: Rust, no dependencies (SHA-256, HMAC-SHA256 and CRC-32 are in `src/crypto.rs`,
  checked against FIPS 180-4, RFC 4231 and the CRC-32 check value). Chosen by ON8ST on
  2026-11-21 in the openg7250-tb session, replacing the earlier plan of Python. Builds offline.
- Licence: GPL-3.0-or-later, © 2026 ON8ST (decision 0018).

## Build and run

```
cargo build --release --offline
cargo test --offline              # the debug build: overflow checks on
cargo test --release --offline    # and the build the bench runs
target/release/g7250-refmodel --ip 127.0.0.1 --netmask 255.0.0.0 \
    --tx locked --control 127.0.0.1:7290 --endpoint-file /tmp/ep.json
```

It binds the eight level-1 ports of Table B2-1 (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)
on `--ip`, ticks every millisecond, and serves the fixture control socket of
`testbench/docs/testlist-format.md` section 4 on `--control`. `--endpoint-file` writes the
endpoint description the test bench reads.

Choices where the spec leaves room: a discarded packet counts under its own name and under
`drop_<port>` (B11.1); a request that gets a refusal (a run request during an update or on a slow
link) counts as discarded; a gap (B11.6) is a number newer than the one before by more than 1, and
the numbers start again at a takeover; every counter but `reset_<cause>`, `fault_<code>` and
`trip_count` starts at 0 at each start; a start with no reset flag clears those too and counts no
`reset_unknown` (it counts as power-on).

## Language (decisions 0033, 0037, proposed)

Rust, an independent implementation written from the spec only: it never uses the firmware's
protocol crate, and the firmware never uses this code (0037).

## Options

| Option | Meaning |
|---|---|
| `--tx locked` (default) | r1.0 before calibration: no calibration set, record `tx` bit 1 = 1 and bit 4 = 0, every arm rejected with `NOT_CALIBRATED` (B10.2 item 3, Annex P freeze rule), nothing keys, DUC0 stream reason 14, `ATU_STATE` 5 and `ATU_TUNE` = 1 `NOT_CALIBRATED`, `LIMIT_SWR` and `LIMIT_PA_TEMP` `NOT_CALIBRATED` |
| `--tx unlocked` | starts with a complete simulated calibration set (Annex P "Tests": simulated values): `tx` bit 4 = 1, arm accepted, a minimal keying model, for host-side tests of TX logic |
| `--cal none` | no calibration set at all: also DDC0 is not offered (record `streams` bit 0 = 0, stream reason 2; B6.1, 0042), as on a radio before P-RX-2 |
| `--cal rx` (default with `--tx locked`) | a simulated set that holds only P-RX-2: DDC0 offered, TX frozen |
| `--cal full` | as `--tx unlocked` |
| `--port-base N` (default 20000) | spec port p at local port N + p, control at N + 7290; the endpoint file carries `radio.connect` and the bench host ranges at N + 9000 … N + 9199. Refuses any N whose ports touch 1024–7251 or 30000–38299 (the emulator runner's), so the model and the runner never share a port; `0` = the spec ports, only when no emulator run can be going |
| `--f1 closed` (default) | the F1 gate of A1.8a item 3 (P-SYS-5) is closed: a foreign `UPDATE_BEGIN` gets 56, as on every radio today |
| `--f1 open` | the gate counts as passed and the streaming installer of A1.8a item 3 runs (for T-A1-54, -55, -58, -60 to -62 and `g7250ctl`, with a synthetic opaque image only): the hand-over log, installer state, 60 after the first erase, the 10-minute rule before it, the write session of 0077, the secret erased with the first sector; after a good activation the model answers nothing (a stock image runs), after a power loss in writing nothing either (no valid application). Fixture, only with this option: `installer.state {}` → `state` (`own`, `before-erase`, `writing`, `stock`, `nothing`), `sectors`, `bytes_written`, `secret_stored`. Always offered: `vendor.install {}` puts this firmware back as after the vendor path (no secret, a new first-secret window). Choices where A1.8a is silent: power lost with every sector written and the hash right counts as stock (vendor behaviour, not specified); `UPDATE_ACTIVATE` without the flag gets its 17 after authentication. Not modelled: RF present (59), flash errors and erase times, a power loss inside an erase or a write (a sector is written whole or not at all), the one-sector RAM buffer, the pins, PTT and KEY inputs and watchdog of the installer, and what the vendor bootloader does with the result. The endpoint's `model.f1_gate` says which |

A calibration set can also be written and cleared at run time with `SET_CALIBRATION` and
`CLEAR_CALIBRATION` (A1.12); record `tx` bit 4 follows from the next session on.

## What is modelled

| Spec | Modelled | Not modelled |
|---|---|---|
| B2, B3 | ports; discovery reply (Table B3-1), byte 4 rules, source-port-1024 drop, rate limits 10/s per source and 50/s total; Erase/Program ignored; unknown command ignored; Set IP (MAC match, subnet rule, reply; the sockets stay, the management channel sees the new address: A1.10) | limited/directed broadcast detection (the shell does not see the destination address) |
| B4 | record 1.1 (207 registers) with the r1.0 values of 0035/0036/0039 (Table A3-0 ranges, `tx` bit 4, `regmap_groups` 0x3F, `ddc_rates` 0x07, `telemetry`, `cal_info`), `CAP_REQUEST`, rate limits, unasked record once per session with the 1 s limit, read-only answers | — |
| B5 | session start by run, set-up phase (4 sources, LRU), restart by the same host with RX settings, stop, watchdog 1 s, takeover with three `TX_ARM_STATE` copies 10 ms apart and the repeated notice | link speed effects beyond the refusal |
| B6, B7 | DDC0 I&Q at the selected rate (238 zero samples per packet), refusals with `STREAM_STATUS` reasons, General packet fields of Table B7-1 that set reasons, non-default ports (`LINK_STATUS` bit 17, log) | samples (no receiver), wideband, microphone |
| B8 | every register of Table B8-2 (groups 0–5; sensor values 0 with `SENSOR_VALID` 0, `ANT_STATE` 0, `ATU_STATE` 5 until P-ATU-1); every write rejection of B8.4 with `WRITE_STATUS` and the counter; `NOT_OWNER` 1/100 ms; source port 7250 dropped; padding | — |
| B9 | High Priority status every 50 ms (1 ms keyed) from 1025, fields 0 | sensor values |
| B10 | arm handshake (token with boot counter, `ARM_MAGIC`, generation, no-op repeat), the reason order of B10.2 item 3, the freeze rule; with a calibration set: PTT edge with the 10 ms rule of B11.6a, PA enable, Table A3-0 ranges, mode class, keyed timeout | relays, settle, limits, faults, CW keyer, ATU tune cycle (a start once calibrated and armed is answered as the no-op), DUC FIFO, numbered mode (so `hp_stale`, `hp_resync`, `hp_release`, `duc_cw_drop` and `duc_fifo_drop` stay 0) |
| A1 | header, size and status precedence, stored replies (5 s, 4 hosts), challenge per host (10 s, once), HMAC-SHA256, write session (60 s, `RESTART` exempt), auth-failure limit (status 34), reply caps, first-secret window (300 s), every payload of Annex M for `IDENTIFY` (tags 1–25), `GET_STATS` (the 45 names of Table M-6, then the model's own; 0074), `GET_CHALLENGE`, `SET_FIRST_SECRET`, `CHANGE_SECRET`, `GET_CONFIG`/`SET_CONFIG` (Table M-3; region 0 only, 62 for S2/T3 before calibration, 49 beyond built-in), `READ_LOG`, `CLEAR_LOG`, `SWITCH_LEVEL`, `RESTART`, `FACTORY_RESET` (keeps the calibration set), `UPDATE_STATUS`, `RETURN_TO_STOCK`, `ENTER_VENDOR_RECOVERY`, `GET_CALIBRATION`, `SET_CALIBRATION`, `CLEAR_CALIBRATION` (A1.12); firmware update and `ROLLBACK` (A1.8), the log kept over a reset (A1.7), limits that fall due during flash work act before waiting requests (0073), an address or netmask change by `SET_CONFIG` or Set IP ends the write session (0074) | the sockets stay where they are bound after an address change; DHCP (the mode returns the start values, no renewal); faults: `fault_<code>` and `trip_count` stay 0, and after an unexpected stop (a software reset without a reason line) only the log line `start: unexpected stop` is there, no `WDT_RESET_AT_START` and no drop reason beyond `RESET` (0075); `icmp_unreach`, `gen1024_ignored` and `ll_unprobed` stay 0; a write that fails in flash |

Payload layouts are those of the spec's Annex M (0039), which adopted the model's first layouts.
Values pending hardware (Annex P) are SIMULATED in the model and marked in the code: range 10
power, SWR S2 and PA T3 (used only once a calibration set holds P-PWR-2), `status_fill`. Tests
must not assert them.

## The `hw.observe` fixture

The model reports a notional picture of the HIS 85 safe-state items (T/R lines, SPI1 chain A
bit 7, PA bias, ATU latches, codec reset) derived from its own state: safe whenever it is not
keyed. Against the emulator the same fixture reports the real emulated pins.
