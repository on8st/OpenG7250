# Vendored crates

## embassy-stm32 0.6.0

- Source: crates.io `embassy-stm32` 0.6.0 (MIT OR Apache-2.0, https://github.com/embassy-rs/embassy),
  copied unmodified from the cargo registry; `Cargo.lock`, `.cargo_vcs_info.json` and `.cargo-ok`
  left out. Used through `[patch.crates-io]` in `board/Cargo.toml`.
- Licence: MIT OR Apache-2.0 (`license` field of its `Cargo.toml`). The crates.io package ships
  no licence files, so `LICENSE-MIT` and `LICENSE-APACHE` were added from the Embassy
  repository at tag `embassy-stm32-v0.6.0` (raw.githubusercontent.com/embassy-rs/embassy),
  unmodified; they carry the notice "Copyright (c) Embassy project contributors". The one
  changed file, `src/rcc/bd.rs`, is marked at the change (Apache-2.0 section 4(b)).
- One local change, marked `G7250 PATCH (no-backup-domain)` in `src/rcc/bd.rs`:
  `LsConfig::init` returns at once.

### Why

On the STM32H7, `embassy_stm32::init` always calls `LsConfig::init` (`src/rcc/h.rs`), whatever
the configuration. With any `LsConfig`, including `LsConfig::off()`, that function

1. sets `PWR_CR1.DBP` (lifts the backup-domain write protection),
2. writes `PWR_BDCR.BREN` (Backup SRAM retention; off unless asked), and
3. if `RCC_BDCR` does not already hold the RTC set-up it wants, resets the whole backup domain
   with `RCC_BDCR.BDRST`, which clears the RTC and its backup registers.

[HIS] HIS 15 rule 4 (`9d10e72`): new firmware MUST NOT use the Backup SRAM or the RTC backup
registers; the factory firmware reads the Backup SRAM at boot, and the bootloader's use of the
backup domain is unknown. Changing retention or resetting the domain would change what the
factory firmware finds after a return to stock (decision 0024 R1, 0028).

With the patch, init leaves the backup domain untouched and reports no RTC clock. The firmware
uses no RTC. The IWDG starts the LSI itself, so it does not depend on this function.

Found in a local smoke run of milestone F1, 2026-11-21: the application stalled in
`LsConfig::init` waiting for `LSIRDY`, which is how the call came to light.

### Check

`crates/layout/tests/forbidden.rs` (`vendored_embassy_keeps_backup_domain_patch`) fails if the
marker is missing.

### Second change: `G7250 PATCH (supply-once)` in `src/rcc/h.rs`

- On the RM0468 parts (STM32H72x/H73x) `init` no longer writes PWR_CR3. Its supply bits are
  write-once per power-on and a configuration that does not match the board can stop or harm the
  part ([HIS] HIS 10, `d199d33`). The G7250 boot stage sets them once, explicitly
  (`g7250_safety::supply`: keep if already the board's configuration, write only from the
  power-on value, otherwise hold the safe state), and the application calls `init` only after
  checking that PWR_CR3 holds that configuration and ACTVOSRDY is set.
- The waits on ACTVOSRDY and VOSRDY are bounded (2 000 000 polls); a timeout panics, and the
  G7250 panic handler puts the safe state back and lets the watchdog reset the MCU.
- `crates/layout/tests/forbidden.rs` (`vendored_embassy_keeps_supply_patch`) fails if the marker
  or the bound is missing.

### Third change: `G7250 PATCH (time-over-flash)` in `src/time_driver/gp16.rs` and `src/lib.rs`

- A module `g7250` with access to the driver's counter, its two half period flags and its period,
  so that the application counts the half periods of a flash operation. A sector erase runs with
  interrupts masked for seconds, and the driver's interrupt can count at most two of them
  (32.768 ms each) afterwards: time was lost, and it could step back. The rules are in
  `g7250_safety::timekeep`, host-tested. The driver's own code is not changed.
- `crates/layout/tests/forbidden.rs` (`vendored_embassy_keeps_time_patch`) fails if the marker or
  the application's use of it is missing.
