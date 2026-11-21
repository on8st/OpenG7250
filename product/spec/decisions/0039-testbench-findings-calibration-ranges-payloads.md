# 0039 — Calibration set, transmit ranges and management payloads (testbench findings SF-01 to SF-18)

- Status: proposed (to the repository owner, ON8ST); written by the product session from the
  testbench's review by test (openg7250-tb, `testbench/docs/spec-findings.md`, fork branch
  `tb/main` at `823d484`)
- Date: 2026-11-21
- Builds on: 0008, 0020 §23, §27, 0028, 0032, 0035, 0036; [G7250-MAN] "Parameters".

## Context

The testbench turned the spec into tests and reported 18 findings against `4551925`. Four were
already resolved by the v1.0 work (SF-01 in part, SF-02, SF-03, SF-14, SF-15). Severity A:
SF-01 (TX locked until calibrated), SF-05 (no way from locked to calibrated), SF-07 (transmit
ranges not tabulated), SF-12 (built-in S2 and T3 unknown).

## Decision (proposed)

1. **Calibration set** (SF-05, SF-12). The measured values of Annex P form a unit's calibration
   set, written with new management commands `GET_CALIBRATION`, `SET_CALIBRATION`,
   `CLEAR_CALIBRATION` (0x0060–0x0062), refused during a P2 session, while armed and during an
   update; stored in this firmware's flash, kept by a factory reset, lost by a return to stock.
   Record `tx` bit 4 is 1 when the set is complete. Limits that need the set (S2, T3) are
   refused until then: management status 62, P2 write reject reason `NOT_CALIBRATED`. New spec
   section A1.12, identification item I-7.
2. **Transmit ranges** (SF-07). [G7250-MAN] gives one list of transmit ranges and leaves local
   rules to the dealer. This version has one region, 0, with the ten ranges of the manual as
   Table A3-0; it is the factory default; any other region is refused with status 48. A
   narrower licence is set with the per-band power limit.
3. **Management payloads** (SF-08). New Annex M fixes every payload: the 16-byte challenge,
   the HMAC over challenge then message, `IDENTIFY` TLVs, configuration TLVs, the package header
   and the update, log and calibration layouts. It adopts the reference model's provisional
   layouts and adds what was missing.
4. **Smaller fixes**: TX locked rule in the B10 introduction (SF-01); `BAD_TOKEN` first in the
   arm reason order (SF-02); DUC I&Q while TX is not calibrated is discarded, stream reason 14
   (SF-04); `NOT_RUNNING` not set over P2 (SF-06); `FAULT_CLEAR` without a fault (SF-09);
   retransmission after a reset gets 33 (SF-10); `UNKNOWN_ADDRESS` answer value 0 (SF-11);
   `SWITCH_LEVEL` status order as Table A1-3 (SF-13); `WATCHDOG_STATE` bit 0 before a General
   packet (SF-16); uncalibrated telemetry: raw words filled, `cal_info` bits 0–4 = 0 (SF-18).
5. **Not changed**: SF-17 (MAC uniqueness stays pending hardware, P-NET-2). SF-18's proposal
   "telemetry = 0, group 3 absent" is not taken: the sensors exist ([HIS] HIS 85), and their
   validity is reported per value in `SENSOR_VALID`.

## Addendum: firmware questions and the tx bit 1 conflict (2026-11-21)

- **tx bit 1 in r1.0.** The firmware read bit 1 = 0 and reason 11; the reference model read
  bit 1 = 1 and reason 12. Settled: TX is part of r1.0 (0036), so every r1.0 firmware reports
  bit 1 = 1 and an uncalibrated r1.0 radio rejects the arm with 12 `NOT_CALIBRATED`; bit 1 = 0
  and reason 11 belong only to a build before M4 without a key path, and then bit 4 is 0 too.
  Record definition, B10 introduction and new test T-B4-18.
- **SQ-F2-2.** Where the Ethernet controller does not report the destination of a datagram,
  every Set IP is treated as unicast or directed broadcast: own subnet only, any source while
  on a link-local address. T-B3-32 rewritten.
- **SQ-F2-3.** After Set IP the netmask in use is kept if the new address is in its subnet,
  else 255.255.255.0; a gateway outside the new subnet is cleared. T-B3-34.
- **SQ-F3-3.** The boot counter shares the non-volatile store of configuration and calibration;
  without it record `tx` bit 4 stays 0.

## Consequences

- Spec: A1.5, A1.6 C-6, A1.9 step 3, A1.12 (new), A2.5a, Tables A1-2, A1-3, A3-0 (new),
  A3-2, B8.4, B8-2, B8-3, B10, B10.2, B10.7, record `cal_info`; Annex M (new); tests T-A1-63
  to T-A1-65, T-B10-42; part C C5.5.6 item 7.

## Sources

- `testbench/docs/spec-findings.md` (tb/main `823d484`); `product/refmodel/docs/mgmt-payloads.md`
  (same branch); [G7250-MAN] "Parameters"; [HIS] HIS 10, HIS 85 (`9d10e72`); decisions above.
