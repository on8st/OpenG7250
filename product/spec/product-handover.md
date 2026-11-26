# Product handover (g7250-spec → product/spec/)

Date: 2026-11-19. Source of everything below: the start brief and the discussion with ON8ST
on 2026-11-19, as recorded in `decisions/`.

## Where things stand

- Spec release candidate **rc4** is published on the fork as branch `spec/draft` and tag
  `spec-rc4` (fd03ea5). Work continues on local `spec/draft` tracking `origin/spec/draft`.
  `spec/rc1` and `spec-rc1` stay at 93c247b. A pushed tag is never moved: the next one gets
  a new name (`spec-rc5`). Fetch only named refs (no `--all`, no `--tags`), so `his/integrate`
  is never fetched.
- ON8ST's review: `review-guide-rc4.md` lists the 18 choices that need the owner's judgement.
- rc4 passed repeated independent reviews; the last verification found OPEN HIGH 0,
  OPEN MEDIUM 0. Remaining LOW clarifications for a later pass: which key starts the zero-run
  count, the DUC sample tap before TX DC/IQ correction (HIS-18), leakage below `P_rf_off`
  (HIS-11), the zero-sample rule on level 2, CWX request going to 0 through bit [0], DUC rate
  change advice, and one key-latency definition with the DUC FIFO between the A3.1a key and
  the PTT key.
- Files: `spec/network-interface.md` (parts A, B), `spec/part-c-tci-profile.md`,
  `spec/annex-aethersdr.md`, `spec/glossary.md`, `spec/CHANGELOG.md`, generated
  `spec/register-map.yaml` and `spec/traceability-auto.md`; `design-note.md` draft 4 (rc4).
- Checks: `python3 tools/spec_lint.py` (0 findings, 483 test ids), `python3 tools/trace.py`,
  `python3 tools/budget.py`, `python3 tools/regmap.py` (`--check` to verify).
- Level 1 is G7250-P2: openHPSDR Protocol 2, board type 200 (0016, 0002).
- Clean room: do not fetch, check out or read `his/` or the `his/integrate` branch until the RE
  session reports that the HIS has passed its gate (it has not yet).

## Decisions

| #    | Title | Status |
|------|-------|--------|
| 0001 | Structure of the design note and the network-interface spec | accepted |
| 0002 | Board type of G7250-P2: 200 (0xC8), chosen ourselves; openHPSDR to be informed | accepted; notice sent as TAPR/OpenHPSDR-Firmware#6 |
| 0003 | Capability block: short P2 discovery reply (board type 200, never 254/255) + read-only capability record in the G7250 register map | accepted |
| 0004 | Streams: one DDC at 48–384 ksps (384 only if SPI ≥ ~50 MHz), TX IQ 24-bit 192 ksps, mic only if HIS-17, no wideband yet; unsupported requests refused | accepted |
| 0005 | Control fields on P2: standard fields where identical; G7250 functions and TX safety in a G7250 register map (P2 memory-mapped registers) | accepted |
| 0006 | Telemetry: standard P2 status fields where identical, calibrated and formula-compatible; G7250 sensors and state in register map | accepted |
| 0007 | Watchdog and safety: token-based TX arm, 13 disarm events, keyed timeout 300 ms, takeover always allowed but disarms, limits table, two fault classes | accepted |
| 0008 | Management channel: own UDP port on the 8th socket, challenge-response writes, signed updates with rollback, never keys the radio | accepted |
| 0009 | Stay as close to the standard protocol as possible (now P2, see 0016) | accepted |
| 0010 | No compatibility modes for now (HL2 rejected, ANAN out of scope for now) | accepted |
| 0011 | Two-level product interface: L1 G7250-P2 (was G7250-P1, see 0016) / L2 TCI with on-board DSP. Single owner, shared safety core, L2 only after gate G6 | accepted |
| 0012 | Every integration supports full functionality including TX; no SoapySDR module | accepted |
| 0013 | AetherSDR is the initial integration route for level 1; no separate bridge | accepted |
| 0014 | One TCI profile (spec part C) for AetherSDR on level 1 and the firmware on level 2 | accepted |
| 0015 | Host that does not know the board type: RX only; TX needs an arm in the G7250 register map | accepted |
| 0016 | Level 1 uses openHPSDR Protocol 2 instead of Protocol 1; AetherSDR P2 support built by us if needed | accepted. Revises 0001, 0009, 0011; 0005 and 0015 rewritten |
| 0017 | G7250-P2 offers no DDC audio stream (port 1028); frees one of the 8 CH394Q sockets | accepted |
| 0018 | Licences: documents CC BY 4.0, firmware GPL-3.0-or-later (Apache-2.0 if the vendor SDK forbids GPL), AetherSDR backend GPL-3.0; © ON8ST | accepted |
| 0019 | Reference model on the product side (`product/refmodel/`); shared test bench stays black-box | accepted |
| 0020 | rc1 detail choices: ports 7250 (register map) and 7251 (management), address map, codes, timers, B11 error handling | accepted (2026-11-20) |
| 0021 | TCI profile choices: port 50001, PROTOCOL ExpertSDR3,2.1, DEVICE G7250, TX policy, mode class from MODULATION, FLOAT32 IQ, LAN restrictions | accepted (2026-11-20) |
| 0022 | rc1 review fixes: RF path frozen while keyed, takeover procedure, non-owner TX IQ discarded, fault classes, limit tiers, SWR fold-back, level-switch procedure, rate limits | accepted (2026-11-20) |
| 0023 | Controller left alone (option A): CTRL idle, controller not a front panel | accepted |
| 0024 | Switching between stock and new firmware: foreign-image upload, vendor path intact, retained stock image, vendor recovery | accepted; amended by 0025 |
| 0025 | One installation path on the main unit, both directions, no controller step; open risk R5 (HIS-19) | accepted (2026-11-20) |
| 0026 | Firmware management on the host: spec protocol, CLI `g7250ctl` as reference, AetherSDR on the same protocol | accepted (2026-11-20) |
| 0027 | Test bench is the `testbench/` folder of the umbrella repository, not a submodule | accepted |
| 0028 | After the HIS: first install once via the controller; return to stock by streaming installer; A/B with own boot stage; no code in the spec phase | accepted (2026-11-20) |
| 0029 | Headless: power hold kept, no power-off command, quiet idle; standby later | accepted (2026-11-20) |
| 0030 | Local PTT and key inputs on the main unit not used in v1.0 | accepted (2026-11-20) |
| 0031 | No audio through the main unit's codec in v1.0 | accepted (2026-11-20) |
| 0032 | Own ATU control; bypass and `NOT_CALIBRATED` until its parameters are measured | accepted (2026-11-20) |
| 0033 | Firmware in Rust, `no_std`, on Embassy; host-tested protocol and state code | accepted (2026-11-21) |
| 0034 | Every firmware image tested as a black box on the emulator before the radio (product side of project decision 0007) | accepted (2026-11-21) |
| 0035 | v1.0 freeze without the radio: Annex P (pending hardware), `NOT_CALIBRATED`, values settled from the HIS | accepted (2026-11-21) |
| 0036 | Transmit is part of r1.0, armable only once calibrated | accepted (2026-11-20) |
| 0037 | `g7250ctl` in Rust, sharing the protocol crate with the firmware; reference model independent; test bench Python | accepted (2026-11-21) |
| 0038 | AetherSDR G7250 backend in C++/Qt after firmware F2, offered through the RFC | accepted (2026-11-21) |
| 0039 | Testbench findings SF-01..18: calibration set and commands (A1.12), transmit ranges Table A3-0, management payloads Annex M | accepted (2026-11-21) |
| 0040 | Faults before the network (supply check) reported on the debug console only | accepted (2026-11-21) |
| 0041 | DHCP exchanges borrow the management socket, at most 2 s at a time | accepted (2026-11-21) |
| 0042 | DDC0 not offered until the FPGA configuration frame is understood (P-RX-2) | accepted (2026-11-21) |
| 0043 | Update packages linked for one slot (header byte 52, status 63); no update during a trial boot | accepted (2026-11-21) |
| 0044 | Self-confirmation of a trial boot: 10 s with network up and main loop running; 60 s limit; reset before confirmation rolls back | accepted (2026-11-21) |
| 0045 | Safe state holds from 20 µs after the boot stage starts; before that outside the spec (P-SYS-1) | accepted (2026-11-21) |
| 0046 | CW keyer timing: unit 1200/S ms, weight as mark/space shift, Curtis iambic A/B, strict spacing, tolerance | accepted (2026-11-21) |
| 0047 | Unkey reason 11 when an ATU tune cycle ends by itself; abort gives 1 | accepted (2026-11-21) |
| 0048 | Sockets open within 5 s (DHCP) or 15 s (no DHCP) of link up; 17 s by 0057 | accepted (2026-11-21) |
| 0049 | Link-local probe on socket 0 (MACRAW), defence by giving the address up; conflicts on DHCP/static addresses | accepted (2026-11-21) |
| 0050 | Fault counters across a reset: pending hardware P-SYS-9, part of the TX gate | accepted (2026-11-21) |
| 0051 | Port 7250 sequence counters per destination (owner, non-owners, old owner) | accepted (2026-11-21) |
| 0052 | DHCP messages while link-local carry the link-local source, ciaddr 0, BROADCAST flag | accepted (2026-11-21) |
| 0053 | Normative socket buffer split, Table B2-2, powers of two (port 1027: 4 KB in, 2 KB out for wideband) | accepted (2026-11-21) |
| 0054 | High Priority bursts without loss: two at wire speed, or any number at >= 1 ms | accepted (2026-11-21) |
| 0055 | Flash work exempt from the burst rule; never while armed or keyed; secret writes refused while armed | accepted (2026-11-21) |
| 0056 | A malformed command payload is status 17, in the framing group | accepted (2026-11-21) |
| 0057 | Link-local used unprobed (visible) if MACRAW fails; start-up bound 17 s | accepted (2026-11-21) |
| 0058 | GET_STATS `hp_rx`: High Priority packets read from port 1027 | accepted (2026-11-21) |
| 0059 | Unasked capability record may come in up to three parts | accepted (2026-11-21) |
| 0060 | DDC0 frame values (P-RX-2) from the unit's factory calibration, confirmed by measurement on the radio (T-B6-28); gate unchanged until then | accepted (2026-11-21) |
| 0061 | The store is in flash sectors 6 and 7, used in turn; sector 6 checked in F1; addendum: own images too large for their slot get 61, updates touch the store only as store writes | accepted (2026-11-21), addendum too |
| 0062 | Flash layout (slots A, B and the store) reported in `IDENTIFY` tag 25; layout stays firmware design | accepted (2026-11-21) |
| 0063 | Update codes: no transfer in progress gives 54 (block) or 55 (verify, activate); unreadable package header 17, unsupported component 56 | accepted (2026-11-21) |
| 0064 | Torn boot-record writes; `ROLLBACK` needs a confirmed image and a good other slot (else 55), starts it as a trial | accepted (2026-11-21) |
| 0065 | Flash work: no watchdog or pin change up to `t_erase_max`; failure is 55 and `INTERNAL_ERROR`; store survives power loss; ECC errors are torn records | accepted (2026-11-21) |
| 0066 | TX path check (`TX_PATH_NOT_BLOCKED`): MCU configuration and the level at the pin, every 5 ms, fault after 10 ms of mismatch | accepted (2026-11-21) |
| 0067 | Image check: `UPDATE_VERIFY` checks hash, image check and record against header (55); boot-stage refusals; `IDENTIFY` tag 9 none, good, bad and when judged | accepted (2026-11-22); addenda (log words; full store, status 64) accepted (2026-11-22) |
| 0068 | Log kept over a reset without power loss; reason logged before every commanded reset; start entry `start after <cause>: log kept` or `log lost`; survival is part of P-SYS-9 | accepted (2026-11-22); addendum (build readings) accepted (2026-11-22) |
| 0069 | Reply time of requests with flash work: one sector erase per request, reply within 15 s, host retransmits for 16 s | accepted (2026-11-22); addendum (start check) accepted (2026-11-22) |
| 0070 | g7250ctl: drives every Table A1-2 command against the reference model; host-side behaviour tested (T-A1-93) | accepted (2026-11-22) |
| 0071 | Package file: Table M-4 header followed by the image, `.g7pk`, written by the firmware build | accepted (2026-11-22) |
| 0072 | C-3 limits that may be lost: a C-3 write always compacts; a failed newer generation gives the stricter values; a state record keeps the judgment; unreadable C-3 closes TX until every C-3 item is written again | accepted in principle (2026-11-22); accepted (2026-11-23) |
| 0073 | Time over flash work: uptime and the listed limits count flash work; a limit due during it acts within 100 ms after it, before queued management requests | accepted (2026-11-22) |
| 0074 | Management details from the g7250ctl runs: `UPDATE_ABORT` with no update state, `GET_STATS` counters of Table M-6, `READ_LOG` bit 1, write-session opening and end, address change, host rule after a reset | accepted (2026-11-22) |
| 0075 | Unexpected stop of the firmware: RF control pins safe within 20 µs, log line, reset within 10 ms; a software reset without a reason line counts as `WDT_RESET_AT_START` | accepted (2026-11-22) |
| 0076 | Radio drill F3 (P-SYS-10): power cuts in store writes, the unreadable word, kept RAM, one bus fault; the drill needs its own go; the RM0468 fixed case is open | accepted as text (2026-11-22); the drill needs its own go from ON8ST |
| 0077 | Streaming installer: the write session continues there with `END_WRITE_SESSION`; the idle time pauses during the holder's own request; power loss with every sector written; `SET_FIRST_SECRET` gets 36 | accepted (2026-11-22) |
| 0078 | Evidence of a commanded reset: a commanded-reset word in kept RAM; reads of kept RAM never stop the start; test timings; counter details and the definition of a gap | accepted (2026-11-22) |
| 0079 | Built-in power limit at 50 MHz 0 W until P-PWR-2 (a limit above 0 gets 49); the `SET_CONFIG` reply gives the items as `GET_CONFIG` does | accepted (2026-11-23) |
| 0080 | Status 32 for an authenticated command without an HMAC trailer or with no secret set; 33 only with a secret set (installer: the secret in RAM); a 32 is no authentication failure; counter `mg_auth_required` | accepted (2026-11-23) |
| 0081 | Flash work that never ends: the 55 within 15 s only for flash work that ends with an error flag or within its bound; a late end gets 55 within 0.5 s of its end; never-ending work takes the stall path (watchdog reset, no reply) | accepted (2026-11-24, reading (a) 2026-11-25) |
| 0082 | Change into DHCP mode while running: no address change in itself; from link-local the address is kept, from static for at most 6 s of DHCP attempts by borrows, then a lease or a probed link-local address | accepted (2026-11-25) |

All numbers 0001–0082 are now in use.

## Open questions for ON8ST

Decided on 2026-11-20 (ON8ST):
- **Review of rc4 completed** by ON8ST (2026-11-20).
- **Emu receipt:** the openg7250 relay ("received: ON8ST acceptance of 0020–0024 and
  HIS-17/18/19 questions") counts as the emu session's receipt (ON8ST, 2026-11-20). Emu's own
  receipt followed, relayed verbatim by openg7250: "I have received ON8ST's acceptance of
  decisions 0020–0024 and the HIS sub-questions HIS-17, HIS-18 and HIS-19. They are being handled
  through the HIS and its gate." The rc plan
  (steps 1–7) is complete; spec-rc7 is on the fork (b9132bc), `spec/draft` at 4902a37.
- 0025 and 0026 **accepted** (2026-11-20) after review on the decision page
  (https://claude.ai/artifact/F9UDf2htv7qiBHUoeb5jDv); R5 of 0025 is decided when the HIS
  answers.
- **Go** to push the follow-up commits on `spec/draft` with tag `spec-rc6` (done), and with
  tag `spec-rc7` for the READMEs and the acceptance of 0025/0026.
- **Go** to create `product/tools/g7250ctl/` (0026) and `product/refmodel/` (0019); the
  openg7250 session adds them to the product-side sparse checkout.
- 0020, 0021, 0022, 0023 and 0024 are **accepted** ("Akkoord met alles", after his review of
  rc4). 0023: controller out of scope; 0024: switching between stock and new firmware.
- `.claude/settings.json` stays as it is (the deny rules naming the decrypted-firmware folder
  remain, also in the history).
- **Go** for the RE session to push branch `spec/rc1` and tag `spec-rc4` to the fork
  (on8st/OpenG7250); done as branch `spec/draft`.
- **Go** (2026-11-20) to push the follow-up commits on `spec/draft` with tag `spec-rc5`. The spec PR to Guru-RF follows after PR #1, with a separate go.

Still open:
1. The HIS is cleared and read at `2a38b7f` (read-only worktree `g7250-product-his`). Its
   answers are processed in `analysis/his-2a38b7f-impact.md` and 0028. Open points of that
   analysis (§4: power key, local PTT/key, audio, ATU) wait for ON8ST; §5 lists the questions
   back to the HIS.
2. Go for an AetherSDR RFC issue (annex Q-O1); shared P2 refactor vs. self-contained backend
   (Q-O2).
3. The test bench as the `testbench/` folder of the umbrella repository (0019, 0027). `product/refmodel/` and `product/tools/g7250ctl/`
   exist with READMEs (ae85316).

Watch TAPR/OpenHPSDR-Firmware#6 for a reply on board type 200.

## Open questions for the HIS

In `open-questions-his.md` (HIS-01 to HIS-19).

## Next step

1. (Done: 0013, `sources.md`.) Still open in `sources.md`: read the Radioddity product page
   (HTTP 429 on 2026-11-19), read the TCI 2.1 PDF and check its terms, find a manufacturer
   manual.
2. (Done: `open-questions-his.md`.)
3. (Done: `design-note.md` draft 1; review by ON8ST.)
4. (Done, accepted: 0003, 0004, 0006, 0007, 0008.) Next: update the spec draft to the
   accepted decisions (B4 capability block, sample rates, telemetry, transmit, register map,
   B3.9 Set IP now accepted as P2), and update the illustrated design note.
5. (Draft 1 done: `spec/network-interface.md`.)
