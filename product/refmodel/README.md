# G7250-P2 reference model

An executable model of a G7250 running the new firmware, built from the specification only
(decision 0019), so that the conformance tests, `g7250ctl` and AetherSDR's G7250 backend can be
tested before and alongside the real firmware.

- Source of truth: `product/spec/` (branch `spec/draft`, parts A and B, Tables B2-1, B4-1, B8-2,
  B8-3, A1-1 to A1-4) and the cleared HIS at `9d10e72` (HIS 85 safe state). Never the
  reverse-engineering side.
- Language: Rust, no dependencies (SHA-256, HMAC-SHA256 and CRC-32 are in `src/crypto.rs`,
  checked against FIPS 180-4, RFC 4231 and the CRC-32 check value). Chosen by ON8ST on
  2026-11-21 in the openg7250-tb session, replacing the earlier plan of Python. Builds offline.
- Licence: GPL-3.0-or-later, © 2026 ON8ST (decision 0018).

## Build and run

```
cargo build --release --offline
cargo test --offline
target/release/g7250-refmodel --ip 127.0.0.1 --netmask 255.0.0.0 \
    --tx locked --control 127.0.0.1:7290 --endpoint-file /tmp/ep.json
```

It binds the eight level-1 ports of Table B2-1 (1024, 1025, 1026, 1027, 1029, 1035, 7250, 7251)
on `--ip`, ticks every millisecond, and serves the fixture control socket of
`testbench/docs/testlist-format.md` section 4 on `--control`. `--endpoint-file` writes the
endpoint description the test bench reads.

| Option | Meaning |
|---|---|
| `--tx locked` (default) | r1.0 before calibration (ON8ST, 2026-11-20): record `tx` bit 1 = 0, every arm rejected, nothing keys, `ATU_TUNE` = 1 gets `NOT_CALIBRATED` |
| `--tx unlocked` | TX as after calibration: arm handshake and a minimal keying model, for host-side tests of TX logic |
| `--lock-reason spec` (default) | locked arm reject reason 11 `TX_NOT_IMPLEMENTED`, as the spec is written (B10.2 item 3) |
| `--lock-reason not-calibrated` | reason 12, the proposal of spec finding SF-01; not in the spec |

## What is modelled (T1)

| Spec | Modelled | Not modelled |
|---|---|---|
| B2, B3 | ports; discovery reply (Table B3-1), byte 4 rules, source-port-1024 drop, rate limits 10/s per source and 50/s total; Erase/Program ignored; unknown command ignored; Set IP (MAC match, subnet rule, reply; the address is recorded, not applied) | limited/directed broadcast detection (the shell does not see the destination address) |
| B4 | record 1.1 (207 registers), `CAP_REQUEST`, rate limits, unasked record once per session with the 1 s limit, read-only answers | — |
| B5 | session start by run, set-up phase (4 sources, LRU), restart by the same host with RX settings, stop, watchdog 1 s, takeover with three `TX_ARM_STATE` copies 10 ms apart and the repeated notice | link speed effects beyond the refusal |
| B6, B7 | DDC0 I&Q at the selected rate (238 zero samples per packet), refusals with `STREAM_STATUS` reasons, General packet fields of Table B7-1 that set reasons, non-default ports (`LINK_STATUS` bit 17, log) | samples (no receiver), wideband, microphone |
| B8 | every register of Table B8-2 that exists without sensors; groups 0, 1, 2, 4 present; every write rejection of B8.4 with `WRITE_STATUS` and the counter; `NOT_OWNER` 1/100 ms; source port 7250 dropped; padding | telemetry group 3, antenna group 5 (absent: HIS-11, HIS-06) |
| B9 | High Priority status every 50 ms (1 ms keyed) from 1025, fields 0 | sensor values |
| B10 | arm handshake (token with boot counter, `ARM_MAGIC`, generation, no-op repeat, `BAD_TOKEN`), locked state; unlocked: PTT edge with the 10 ms rule of B11.6a, PA enable, TX ranges, mode class, keyed timeout | relays, settle, limits, faults, CW keyer, ATU tune cycle, DUC FIFO, numbered mode |
| A1 | header, size and status precedence, stored replies (5 s, 4 hosts), challenge per host (10 s, once), HMAC-SHA256, write session (60 s, `RESTART` exempt), auth-failure limit (status 34), reply caps, first-secret window (300 s), `CHANGE_SECRET` masking, `IDENTIFY`, `GET_STATS`, `READ_LOG`, `CLEAR_LOG`, `SWITCH_LEVEL`, `RESTART`, `FACTORY_RESET`, `UPDATE_STATUS`, `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` (56) | `GET_CONFIG`, `SET_CONFIG`, the update transfer and `ROLLBACK`: answered with 56 (model gap) |

Payload layouts of the management commands are not fixed by the spec yet (A1: "fixed with the
reference model before M5"). The model's provisional layouts are in
[`docs/mgmt-payloads.md`](docs/mgmt-payloads.md), for the spec session to adopt or change.

Values the spec leaves TBD are PLACEHOLDERS in the model, marked in the code: TX band edges
(spec cites [G7250-MAN] only; finding SF-07), 50 MHz power, SWR S2 and PA T3 built-in values,
`status_fill`. Tests must not assert them.

## The `hw.observe` fixture

The model reports a notional picture of the HIS 85 safe-state items (T/R lines, SPI1 chain A
bit 7, PA bias, ATU latches, codec reset) derived from its own state: safe whenever it is not
keyed. Against the emulator the same fixture reports the real emulated pins.
