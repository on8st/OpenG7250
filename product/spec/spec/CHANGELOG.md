# Changelog — G7250-P2 network-interface spec

- © 2026 ON8ST, CC BY 4.0 (see `LICENSE.md`)

All dates 2026-11-19 unless stated. Decision numbers refer to `decisions/`.

## After 1.0 (2026-11-21 and 2026-11-22; not a version)

- 0067, 0068, 0069, 0070 and 0071 accepted by ON8ST on 2026-11-22, as they stand at `f3c15c1`.
- The 0067 addenda (log words; full store, status 64) and the 0068 addendum (build readings)
  accepted by ON8ST on 2026-11-22.
- The 0069 addendum (start check) accepted by ON8ST on 2026-11-22, as it stands at `78aa233`.
- 0072 (C-3 limits that may be lost) accepted in principle by ON8ST on 2026-11-22 and held
  for the point with Table M-3 (ending the lost state before the calibration set holds S2 and
  T3); the number is reserved, and none of its text is in the spec yet.
- 0073 accepted by ON8ST on 2026-11-22.

- 0067 (accepted 2026-11-22; testbench finding SF-31): what an image of this firmware must be. A1.8
  "Image check" (record, slot, length, own integrity check, entry point and stack pointer,
  every word readable; layout the firmware's), "`UPDATE_VERIFY`" (hash, then image check,
  then record–header agreement, each 55), "Boot-stage refusals", "Integrity in `IDENTIFY`"
  (tag 9: none, good, bad, and when each is judged) and "Packages that are not a runnable
  image". A1.5 integrity bullet, Table A1-3 status 55, Table M-2 tag 9. Tests T-A1-88,
  T-A1-89, T-A1-90.
- 0067, testbench points before acceptance: Context states the contract only, with no pointer
  into firmware documents; "Blocks sent again" (a block sent again with a new request id
  replaces the received data); slots named `A` or `B` in log lines, with the check names;
  `ROLLBACK` to a slot whose trial failed is accepted (tag 9 = 1) and starts a trial.
- Annex P, P-SYS-9: the persist block is stated as a firmware-chosen range of AXI SRAM
  (currently 0x2401FF80–0x2401FFFF, as [HIS] `069ea0b` names it), without a pointer into the
  firmware documents; its radio check is the spec's own test T-A3-60.
- 0067 (bench question on T-A1-88): the image check is judged over the length the image record
  gives; an intact image with bytes appended is refused with `header`, not `integrity`.
- 0068 (accepted 2026-11-22; bench question on T-A1-34, firmware input): the log ring is kept over a reset
  without power loss, each entry with a check word, sequence numbers continuing; the reason
  is logged before every commanded reset; a start entry `start after <cause>: log kept|lost`
  and the boot-stage lines follow each start. Survival through the stage before the boot
  stage is part of P-SYS-9; P-SYS-2 keeps the reset cause only. A1.7, P-SYS-2, P-SYS-9;
  T-A1-34 rewritten, T-A1-91 new.
- 0068, firmware questions: the boot stage writes the start entry and its own lines; the cause
  is named from the reset flags (order watchdog, software, power-on, brown-out, pin; `unknown`
  only with no flag), read once and cleared; tag 17 values; P-SYS-2 is only whether the flags
  are trustworthy on this radio.
- 0069 (accepted 2026-11-22; emulator flash-model finding): a request does at most one sector erase and
  programs at most one sector; its reply comes within `t_flash_reply` = 15 s; a host keeps
  retransmitting such requests every 500 ms for at least 16 s. A1.4, B2.3; T-B5-22 and
  T-A1-85 extended.
- 0069, firmware question: a request that ends in a commanded reset may do up to two sector
  erases after its reply and before the reset (reset within 17 s of the reply); `FACTORY_RESET`
  erases the old secret from both store sectors this way.
- 0069: `CHANGE_SECRET` writes the new secret by a compaction before its reply; the stale
  sector with the old secret is erased by the next compaction or by a start check (the
  non-live sector erased once, logged, if it holds a secret other than the live one). A1.10 "The old secret in flash"; T-A1-92 new.
- 0070 (accepted 2026-11-22; g7250ctl Q5): A1.1a says what `g7250ctl` must do against the reference model
  (every command, and its host-side behaviour tested) instead of passing the radio's tests.
- 0071 (accepted 2026-11-22; g7250ctl Q6): the package file, Table M-4 header followed by the image;
  name, writer and host checks (Annex M "Package file"). Test T-A1-93 new. Released builds
  carry a version not used before; development builds are told apart by the image's SHA-256.
- 0067 addendum (accepted 2026-11-22; firmware question): a failed `UPDATE_VERIFY` names `missing-blocks`
  or `unreadable` before `hash`. A1.8 "Log names"; T-A1-88 has a `missing-blocks` case.
- 0067 addendum (accepted 2026-11-22; firmware question): a full store during a transfer. New status 64
  (room made, nothing taken, send the block again with a new request id); the transfer goes on;
  55 still discards it; a verify makes room and completes. Table A1-3, A1.8; T-A1-88.
- 0068 addendum (accepted 2026-11-22; firmware build readings): power loss means a power-on or
  brown-out flag, or no flag at all, for the log and the counters; P-SYS-2's check shows that
  every reset without power loss sets a flag, before P-SYS-9 is set; oldest entry failing means
  log lost; the `ROLLBACK` line is logged last before the reset; fallback line `boot: fallback
  to slot <s> (active slot refused, no trial)`. A1.7, A1.8, A3.5, P-SYS-2; T-A1-90, T-A1-91.
- 0069 addendum (accepted 2026-11-22; firmware readings): after a fall-back the start check erases the
  sector with the failed newer generation (meant); the start check also erases a non-live
  sector holding data it cannot place in a complete record, so no part of an old secret
  survives the next start. A1.10; T-A1-92. The start check runs only with a live generation;
  the drill F1 checks that a second start erases nothing in sector 6 (P-SYS-5). "Complete
  generation": a readable commit word, or the first generation with at least one checked
  record of a closed list (secret, configuration, calibration); an erased sector never qualifies;
  only the start check is limited (other start writes compact as A1.6 says); the erase of the
  lost sector after the owner's first `SET_CONFIG` is an accepted consequence; a first generation
  is recognised by having no commit word (a double fault accepted).
- 0073 (accepted 2026-11-22; firmware question): time over flash work. Uptime and every limit
  counted on it include flash work (within 1 ms per request); the trial's 60 s, the
  first-secret window, the 10-minute abandonment, the write-session idle time and the
  stored-reply time act within 100 ms after they fall due, or after the flash work they fall
  due in, before queued management requests; a confirmation write started before 60 s counts;
  the P2 watchdog is not on the list. B2.3 "time over flash work", A1.8, A1.10; T-B5-22,
  T-A1-16 extended, T-A1-94 new.
- Lint L6 counts a test id as defined only in a table row whose first cell is the id, so that
  a row may cite another test; a second definition is still a finding (542 ids, as before).

## 1.0 — version 1.0, 2026-11-21 (tag `spec-v1.0`)

- Accepted by ON8ST on 2026-11-21. Contains every change listed under "After rc7" below;
  decisions 0001–0066 are all accepted.
- Headers of `network-interface.md`, part C and the AetherSDR annex set to version 1.0.
- What still needs the radio is listed in Annex P (Table P-1) with a safe placeholder per
  value; the radio checks are in HIS 90 and in T-B6-28.

## After rc7 (2026-11-20)

- HIS cleared (gate review 2) and read at `2a38b7f`; answer status per question in
  `open-questions-his.md`; impact in `analysis/his-2a38b7f-impact.md`; source key [HIS].
- 0027: the test bench is `testbench/` in the umbrella repository.
- 0028: first install once through the vendor path on the controller; return to stock by a
  one-way streaming installer over the network (status 60 added); no retained stock image,
  `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` reserved (status 56); own updates in two slots
  behind a boot stage of our own; FPGA never touched. A1.1a, A1.5, A1.8, A1.8a, Tables A1-2 and
  A1-3, T-A1-08, T-A1-52 to T-A1-59 rewritten, T-A1-60 added.

- 0029–0032: headless power and quiet idle; local PTT/KEY not used in v1.0; no audio in v1.0;
  own ATU control with bypass and write reject reason 11 `NOT_CALIBRATED` until calibrated.
  Tests T-A3-07, T-A3-42, T-B10-15, T-B10-37 marked "not in v1.0".

- 0035 (v1.0 freeze): no open hardware value left unnamed. New Annex P, Table P-1 (pending hardware parameters with
  safe placeholders); freeze rule: arm reject reason 12 `NOT_CALIBRATED` and record `tx` bit 4
  until the transmit parameters are set (B10.2, part C C5.5.6 item 7; T-B10-41). Settled from
  the HIS: C-1 default DHCP with link-local fallback [RFC3927]; A1.11 final; TRNG; MAC from the
  CH394; link-status register; socket split as 0004; 192 ksps maximum until P-NET-1; one ADC;
  Alex words and step attenuator ignored; sidetone and mic bytes ignored; local factory reset
  through the vendor path. Lint L9.
- 0036: transmit in r1.0, armable once calibrated. 0037 (proposed): `g7250ctl` in Rust.
  0038 (proposed): AetherSDR backend after F2; annex AetherSDR G-08, G-10, G-12, Q-O1 and the RFC
  draft updated.

- Review of everything since rc7 (independent review, 4 HIGH, about 12 MEDIUM): streaming
  installer keeps the A3.6 TX-side state, services the watchdog (P-SYS-7) and answers only the
  listed commands; abort before the first erase returns to this firmware with the secret, which
  is now erased with the first sector erase; the way back to stock is gated by F1 (P-SYS-5,
  status 56, T-A1-61); status 61 (image too large) and 17 (missing foreign flag); TX gate also
  covers P-TX-1, P-ATU-1, P-ANT-1; `ATU_STATE` 5 / TCI `unknown`; `NOT_CALIBRATED` precedence in
  A3.9b, part C C5.7, B10.2 order of arm reasons; register groups 3–5 always present; no
  microphone stream in v1.0 (B6.11); T-A3-58 for unused local PTT/KEY; footswitch absent in
  v1.0; citations corrected; tools: trace.py --check, lint L9 "per HIS-nn", L10.

- 0039 (proposed), from the testbench's review by test (SF-01..SF-18): calibration set with
  `GET_/SET_/CLEAR_CALIBRATION` (A1.12, I-7, status 62); one region with the manual's ten
  transmit ranges (Table A3-0); Annex M with every management payload (16-byte challenge, HMAC
  over challenge then message, IDENTIFY, configuration and calibration TLVs, package header);
  DUC I&Q discarded and stream reason 14 while not calibrated; `BAD_TOKEN` first; smaller fixes
  (SF-06, 09, 10, 11, 13, 16, 18). Tests T-A1-63..65, T-B10-42.

- 0040–0059, answers to firmware and testbench questions: see `product-handover.md`.
- 2026-11-21: ON8ST accepted 0033–0035 and 0037–0059; every decision 0001–0059 is now
  accepted.
- 0060 (proposed): P-RX-2 from the unit's factory calibration, confirmed on the radio; encoding
  in Table P-1; Annex P "P-RX-2 route"; T-B6-28; HIS-02 question on the EEPROM.
- 0061: the store is in flash sectors 6 and 7 (A1.8 "Store location", A1.12, P-SYS-5).
- 0061 addendum (SF-26): an image of this firmware too large for its slot gets 61; updates
  touch sectors 6 and 7 only as store writes; T-A1-77, T-A1-78.
- 0062 (proposed, SF-27): `IDENTIFY` tag 25 reports the flash layout, which the two SF-26
  tests now use; new test T-A1-79.
- Firmware questions of 2026-11-21: 0060 carries the two other tuning words from the capture
  and fixes the flag order (swap, then negate Q); `UPDATE_BEGIN` code order 51, 56, 63, 61, 52
  (0061 addendum).
- 0063 (proposed): update commands with no transfer get 54 or 55; a package header with a bad
  magic or version gets 17, an unsupported component 56; new tests T-A1-80 and T-A1-81.
- 0063 addendum: only update commands answered 0 or 1 restart the 10-minute timer; the state
  after `UPDATE_ABORT` is set out in A1.8; new test T-A1-82.
- 0064, 0065 (proposed), from the testbench run-sheet gaps: torn boot-record writes, `ROLLBACK`,
  flash erase timing and failure, power loss in store writes, flash ECC; source [STM32H72x];
  new tests T-A1-83 to T-A1-87 and T-B5-22.
- SF-28: the update-state test expects `BUSY` for a P2 arm during an update, not `MAINTENANCE`.
- 0066 (proposed, SF-29): A3.6b TX path check: configuration and pin level, every 5 ms, fault
  after 10 ms of mismatch; new test T-A3-61.
- 0066 addendum (SF-30): Table A3-6b lists the checked pins; SEND and TUNER join it with P-SYS-8.
- 0066 addendum: first check within 100 ms of the boot stage's first instruction; DAC rows by
  host test until a DAC driver exists.
- 0066 addendum: during flash work the check samples in the wait loop and runs at the end; fault
  within 20 ms after the end.
- P-SYS-9: the persist block's range is named as the firmware's design (now 128 bytes at
  0x2401FF80); radio check over the whole block.
- 2026-11-21: ON8ST accepted 0060 and the 0061 addendum.
- 2026-11-21: ON8ST accepted 0062–0066 with their addenda; every decision 0001–0066 is
  accepted. `t_erase_max` from [STM32H72x] (DS13311 Rev 5, Table 48): at most 4 s.

## rc7 — release candidate 7 (tag `spec-rc7`)

- ON8ST accepted 0025 and 0026 (2026-11-20). `product/tools/g7250ctl/` and `product/refmodel/`
  created with READMEs.

## rc6 — release candidate 6 (tag `spec-rc6`)

- 0023 note: ON8ST confirms option A; CTRL stays idle and the controller is not used as a
  front panel on either level (part A "Scope of the firmware", A3.7).
- 0025 (proposed, amends 0024): one installation path on the main unit for both directions,
  with no step that needs the controller; the foreign-image upload is the main mechanism;
  first install uses the same path, pending HIS-19, with an explicit open risk (A1.8, A1.8a
  "One installation path", items 1, 4, 9; T-A1-58, T-A1-59; HIS-19). No new management
  command.
- 0026: firmware management lives on the host side; the part A management
  protocol is the only protocol; the CLI `g7250ctl` (`product/tools/g7250ctl/`) is its reference
  implementation; AetherSDR follows on the same protocol (A1.1a).

## rc5 — release candidate 5 (tag `spec-rc5`)

- ON8ST reviewed rc4 and accepted 0020–0024 (2026-11-20).
- 0023: only the main-unit firmware is replaced; the controller keeps its stock
  firmware and is out of scope (part A "Scope of the firmware", B1.9).
- 0024: switching between stock and new firmware is reversible; the vendor's own
  update and recovery path is never disabled; an owner-supplied stock image can be installed
  from the new firmware; `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` management commands
  (A1.8a).
- `review-guide-rc4.md` for ON8ST's review.

## rc4 — release candidate 4

- Fixes from the full reviews of rc3 and the rc4 draft (0022 §74–§87, 0021 §43–§46): keyer
  elements and the CW hang delay count as keyed for RF-path purposes; CW break-in set/clear and
  CW with PTT (B10.5b); "RF while not keyed" in element gaps; restart by the run bit keeps the
  RX-side settings of the same host ([P2] "Detailed operation"); frequency encoding fixed to a
  phase word on a virtual 122.88 MHz clock; limit reaction times; write-session timer only
  refreshed by authenticated requests; one commanded reset at a time; TCI: TX source frozen
  while keyed, TX_ENABLE as permission, AetherSDR rewrites TCI session state after every P2
  session end, DUC0 is the effective TX frequency on level 1, level-1 CW hang bounded at 2 s.

## rc3 — release candidate 3

- Fixes from the reviews of rc2 (0022 §62–§73, 0021 §27–§42): PEP estimate from the DUC
  samples when the detector has no PEP; DUC FIFO bounded while unkeyed; TX settings reset at
  session end; bounded RF-gone waits; factory reset atomic; management reply caps; TCI keyed
  liveness table covering every source; AetherSDR never arms on a TX request; level-1 mode
  class follows the transmitted signal.

## rc2 — release candidate 2

- Fixes from the independent review of rc1 (0022 §33–§51) and of the rc2 draft (0022 §52–§61):
  RF path set for the TX frequency at every key (split included), set-up phase so the arm is
  reachable for standard P2 hosts, ordering of High Priority packets (numbered and unnumbered
  hosts), CWX bit meaning and key-down cap, ATU tune retries and state after a failed tune,
  commanded resets only after RF is gone (bounded wait), one conservative power mapping for
  every limit in force, DHCP address changes end the session, P2 accepted from the own subnet
  only, factory-reset scope.
- Tools: register map lists G7250 extensions and checks itself (lint L8).

## rc1 — release candidate 1 (tag `spec-rc1`)

Covers `network-interface.md` (parts A and B), `part-c-tci-profile.md` (part C),
`annex-aethersdr.md`, `glossary.md` and the generated `register-map.yaml` and
`traceability-auto.md`.

- Every requirement cites a source; every remaining TBD names a HIS question (or G6).
  Checked by `tools/spec_lint.py` (0 findings, 299 test ids), `tools/trace.py` (every
  decision cited or explained), `tools/budget.py` (all required scenarios within the 60 % SPI
  target).
- Contains proposed decisions 0020 (rc1 detail choices), 0021 (TCI profile choices) and 0022
  (fixes from two independent reviews and a byte-by-byte check against P2 V4.4), for review by
  ON8ST.
- Open: hardware values from the HIS (`open-questions-his.md`, HIS-01 to HIS-20).

## History of the drafts

| Part | Draft | What changed |
|------|-------|--------------|
| A, B | 1 | M1 sections: transport, discovery, session, RX IQ; annexes for P2 differences, clarifications and tests. |
| A, B | 2 | Aligned with accepted decisions 0003–0008: capability record, streams, telemetry, safety, management channel. |
| A, B | 3 | All open points without a HIS dependency resolved (0020); B11 error handling written; register addresses. |
| A, B | 4 | First independent review and P2 byte check fixed (0022 §1–§21). |
| A, B | 5 | Second review of the transmit side fixed (0022 §22–§32): key rule, relay switching sequence, ATU tune cycle, CW backstop, fault hold-off, `LIMIT_INHIBIT`. |
| C | 1 | TCI 2.1 profile: transport, initialization, command subset, `G7250_` extensions, multiple clients, safety. |
| C | 2 | Open points without a HIS dependency resolved (0021 §1–§17). |
| C | 3 | First review fixed (0021 §18–§26): keyed liveness without Pong, part B symbols verbatim, arm holder. |
| C | 4 | Second review fixed, rules referenced from part A/B (0022 §22–§32). |
| Annex | 1–2 | Changes needed in AetherSDR, from its public source; follow-ups from parts B and C. |
