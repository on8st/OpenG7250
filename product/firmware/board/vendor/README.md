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
