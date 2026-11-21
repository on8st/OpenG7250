# TX lock audit

Read this before the first calibrated build. It lists every path in the firmware that could
reach the PA, its bias and drive, the T/R lines, the RF control pins of [HIS] HIS 85, the
FPGA bus ([HIS] HIS 80), or the radio side of a key (`BoardRf::key`, `cw_element`, `atu_step`).
For each path it names the guard and where it is.

State at the time of writing (fw/main after 04d184d): no path is open. Line numbers are those of
that tree; the item names stay valid if lines move.

## 1. The calibration set: the root of the lock

| Guard | Where | What it enforces |
|---|---|---|
| `MEASURED = Measured::NONE` | `crates/safety/src/tx.rs:159` | This build has no measured values. |
| `Measured::complete_for_tx` | `crates/safety/src/tx.rs:134` | Every freeze-rule item must be present: bias words, drive map, power detector, SWR limits, temperature, supply, `P_rf_off`, T/R timing, the underrun time (P-TIM-2, 2-500 ms, `:143`) and the AXI SRAM retention (P-SYS-9, 0050, `:144`). |
| `Calibration` is sealed | `crates/safety/src/tx.rs:164`, private `_sealed` `:167` | A `Calibration` is made only by `Calibration::from_measured` (`:171`), and only from a complete set. |
| `TxCore` built from `MEASURED` | `board/app/src/main.rs:199` | The only `TxCore` in the image. |
| Compile-time assertion | `board/app/src/main.rs:32` | A build whose `MEASURED` is complete does not compile until the A3.9a key sequence is written in `BoardRf::key`. |
| Host tests | `tx.rs:526` `this_build_is_not_calibrated`; `tx.rs:615` `every_freeze_rule_item_is_needed_p_sys_9_and_p_tim_2_included` | The set is empty, and leaving out any single item keeps it incomplete. |

Record `tx` bit 4 (`tx.rs:356`) and every arm (`NOT_CALIBRATED`, 12) follow from the same
`Calibration`.

## 2. Keying: `KeyPermit`

| Path | Guard | Where |
|---|---|---|
| Make a `KeyPermit` | Only `TxCore::tx_request` makes one. It needs the `Calibration` (`:413`), the arm and every key condition; ATU tuning also needs the ATU parameters (`:426`). The private field `_cal: &Calibration` (`:183`) keeps any other code from building one. | `crates/safety/src/tx.rs:181`, `:412`, `:430` |
| `Keyer::key(&KeyPermit)` | It takes a permit, so it cannot be called without one. It is called only in `KeyPath::try_key`. | trait `crates/safety/src/keypath.rs:121`; call `:378` |
| `Keyer::cw_element(on)` | It needs no permit. It is called only from `KeyPath::run_keyer` while `source` is set (after a successful `key`). | `keypath.rs:401` |
| `Keyer::atu_step(now)` | It needs no permit. It is called only from `KeyPath::tick` while the ATU cycle holds the key. | `keypath.rs:500` |
| Callers of the three hooks | Build check (new): `only_the_key_path_calls_the_rf_hooks`. No `.key(&`, `.cw_element(` or `.atu_step(` anywhere outside `keypath.rs`. | `crates/layout/tests/forbidden.rs:165` |
| Host test | `this_build_keys_nothing_and_never_calls_the_keyer`: with the empty set, no request edge, DUC setting or ATU write reaches the keyer. | `keypath.rs:719` |

## 3. The board side of a key

| Item | Today | Where |
|---|---|---|
| `BoardRf::key` | Writes no pin; it reasserts the safe state and answers "not keyed", so `TX_STATE` bit 0 stays 0. | `board/app/src/main.rs:41` |
| `BoardRf::cw_element` | Empty (the CW waveform needs the DUC behind the closed FPGA bus). | `main.rs:47` |
| `BoardRf::atu_step` | Answers "failed" at once (the ATU latches stay unwritten, P-ATU-1). | `main.rs:50` |
| `BoardRf::unkey` | `safe::reassert`: the safe-state levels only. | `main.rs:35` |

## 4. RF control pins and RF peripherals ([HIS] HIS 85)

| Path | Guard | Where |
|---|---|---|
| T/R lines, chain output enables (PA10, PH10), PA DAC reset (PB14), ATU strobes (PD13, PG2, PG5), PD12 | Written only from the `SAFE_STATE` table, at their safe levels: in `__pre_init` before RAM initialisation, again at start, and at every unkey. | table `crates/safety/src/safe_state.rs` (`SAFE_STATE`); `board/common/src/safe.rs:69` `apply_at_reset`, `:101` `apply`, `:115` `reassert`; `board/boot/src/main.rs:42` |
| The plan changes no other pin | Host test `reset_plan_gives_the_table_and_touches_nothing_else`. | `safe_state.rs:301` |
| Any other code naming ports A, C, D, F, G, H, or claiming a pin other than the CH394's (SPI6, PG12-PG15, PB5) and the console's (PB6, PB7) | Build check (new): `rf_ports_and_rf_peripherals_are_named_only_by_the_safe_state`. | `crates/layout/tests/forbidden.rs:180` |
| SPI1 (relay shift-register chains), SPI2 (PA bias and drive DAC), DAC1 (RF gain, PA4) | Not used anywhere. PA4 is left untouched (no safe level known). Same build check. | `forbidden.rs:180` |
| SEND and TUNER outputs (HIS-09), KEY and PTT inputs | Not driven, not used in v1.0 (0030). | `safe_state.rs` (`LeaveUntouched` entries) |

## 5. FPGA bus ([HIS] HIS 80)

| Path | Guard | Where |
|---|---|---|
| Any bus access | Every function of `board/app/src/fpga.rs` takes a `BusPermit` (e.g. `Bus::open`, `:40`). | `crates/safety/src/fpga_gate.rs:59` |
| Make a `BusPermit` | Only `fpga_gate::permit()`, which needs `CLEARED` (radio) or, with the feature `emulator-fpga-bus`, `EMULATOR`. Both are `None`. Both are compile-time constants, so nothing received over the network can open the gate. | `fpga_gate.rs:51`, `:55`, `:65`; test `gate_holds_by_default` `:95` |
| GPIOE named elsewhere | Build check `only_the_fpga_module_touches_gpioe`. | `crates/layout/tests/forbidden.rs:118` |
| The safe state drives no FPGA pin | Host test `fpga_bus_is_never_driven_by_the_safe_state`. | `safe_state.rs:230` |
| DUC (TX IQ) and the CW waveform | Need the bus, so they cannot run. DUC I&Q is discarded before the FIFO while record `tx` bit 4 = 0 (B10.7). | `board/app/src/p2.rs` `duc_iq` |

## 6. Paths only a comment protected, now checked

1. `cw_element` and `atu_step` take no permit. They are now covered by the build check of
   section 2 (`forbidden.rs:165`).
2. Nothing stopped new code from claiming an RF pin or SPI1, SPI2 or DAC1. That is now the
   build check of section 4 (`forbidden.rs:180`).

Both run with `cargo test` (build.sh, "host tests").

## 7. Before the first calibrated build (checklist)

- Every `Measured` field is set from a cleared HIS or spec value, with the commit citing it.
  P-SYS-9 is set only after first-radio check RD-01 (T-A3-60).
- `BoardRf::key` implements the A3.9a sequence: relays and settle, SEND and the RF delay, bias,
  then drive. Only then remove the assertion at `main.rs:32`. This needs a reviewer.
- `BoardRf::cw_element` and `atu_step` are written against the DUC (FPGA gate) and the ATU
  parameters (P-ATU-1).
- The RF pins and peripherals that the key sequence needs are added to the allowed lists of
  `forbidden.rs:180`, each with its HIS citation, and no other entry.
- `fpga_gate::CLEARED` is set only from a cleared HIS 80 parameter set.
- Done: the net loop empties socket 4 (DUC I&Q, port 1029) and socket 3 (High Priority) on
  every pass, two datagrams each, which is what their 4 KiB holds (Table B2-2; 0053;
  `HP_DUC_PER_PASS`). Before the DUC goes live, measure the pass time with the DUC stream on:
  the socket must be read within about 2.5 ms (two packets at 192 ksps).
