// SPDX-License-Identifier: GPL-3.0-or-later
//! Source scan: the firmware never touches the option bytes, the Backup SRAM or the RTC backup
//! registers ([HIS] HIS 15 rules 2 and 4), never sends the forbidden FPGA commands
//! ([HIS] HIS 80), and links every image at or above 0x08020000 ([HIS] HIS 10).
//!
//! The scan covers every `.rs` and `.x` file of the board workspace and the host crates, except
//! vendored third-party code; the one vendored crate is checked for its patch instead. It is a
//! backstop next to the guard types, not a replacement for them.
use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        if p.is_dir() {
            if name != "target" && name != "tests" && name != "vendor" {
                files(&p, out);
            }
        } else if name.ends_with(".rs") || name.ends_with(".x") {
            out.push(p);
        }
    }
}

fn firmware_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Identifiers of option-byte, backup-domain and flash-protection registers (STM32H7 PAC and
/// reference-manual names, lower case).
const FORBIDDEN: &[&str] = &[
    "optcr", "optsr_prg", "optsr_cur", "optkeyr", "optccr", "boot_prg", "prar_prg", "scar_prg",
    "wpsn_prg", "wpsgn", "bkpsram", "bkpr", "bkpram", "bdcr", "bkpsramen", "bren", "set_dbp",
    "program_option", "option_bytes",
];

#[test]
fn no_option_byte_or_backup_domain_access() {
    let mut v = Vec::new();
    files(&firmware_root().join("board"), &mut v);
    files(&firmware_root().join("crates"), &mut v);
    let me = file!();
    for f in v {
        if f.ends_with(me) {
            continue;
        }
        let text = std::fs::read_to_string(&f).unwrap().to_lowercase();
        for line in text.lines() {
            let code = line.split("//").next().unwrap();
            for id in FORBIDDEN {
                let hit = code.match_indices(id).any(|(i, _)| {
                    let before = code[..i].chars().last();
                    let after = code[i + id.len()..].chars().next();
                    let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
                    !ident(before) && !ident(after)
                });
                assert!(!hit, "{} uses forbidden identifier `{id}`: {line}", f.display());
            }
        }
    }
}

#[test]
fn every_linker_script_places_flash_at_or_above_0x08020000() {
    // The FLASH region of every linker script is generated from the layout crate (build.rs,
    // `@FLASH@`; 0062): no script holds its own FLASH address, and the ranges the build scripts
    // pass lie in the application area, never in the vendor bootloader's sectors 0-1.
    let mut v = Vec::new();
    files(&firmware_root().join("board"), &mut v);
    let mut seen = 0;
    for f in v.iter().filter(|f| f.extension().is_some_and(|e| e == "x")) {
        let text = std::fs::read_to_string(f).unwrap();
        if !text.contains("MEMORY") {
            continue;
        }
        assert_eq!(text.matches("@FLASH@").count(), 1, "{}: one generated FLASH line", f.display());
        assert!(!text.lines().any(|l| l.trim().starts_with("FLASH") && l.contains("ORIGIN")), "{}: a FLASH address of its own", f.display());
        seen += 1;
    }
    assert!(seen >= 3, "expected linker scripts for the boot stage and the two slots");
    for (build, ranges) in [("board/app/build.rs", &["SLOT_A", "SLOT_B"][..]), ("board/boot/build.rs", &["BOOT_STAGE"][..])] {
        let text = std::fs::read_to_string(firmware_root().join(build)).unwrap();
        for r in ranges {
            assert!(text.contains(&format!("g7250_layout::{r}")), "{build}: {r}");
        }
    }
    for r in [g7250_layout::SLOT_A, g7250_layout::SLOT_B, g7250_layout::BOOT_STAGE] {
        assert!(r.start >= 0x0802_0000 && g7250_layout::APPLICATION_AREA.contains_span(r.start, r.len()), "{r:?}");
    }
}

#[test]
fn vendored_embassy_keeps_backup_domain_patch() {
    let root = firmware_root().join("board");
    let bd = std::fs::read_to_string(root.join("vendor/embassy-stm32-0.6.0/src/rcc/bd.rs")).unwrap();
    let at = bd.find("G7250 PATCH (no-backup-domain)").expect("patch marker missing in vendored bd.rs");
    let after = &bd[at..];
    let ret = after.find("return None;").expect("early return missing");
    let unlock = after.find("unlock();").expect("unlock call not found");
    assert!(ret < unlock, "the early return must come before the backup-domain unlock");
    let cargo = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("embassy-stm32 = { path = \"vendor/embassy-stm32-0.6.0\" }"), "patch.crates-io entry missing");
    let code: String = cargo.lines().map(|l| l.split('#').next().unwrap()).collect::<Vec<_>>().join("\n");
    assert!(!code.contains("gpio-init-analog"), "gpio-init-analog must stay off");
}

#[test]
fn vendored_embassy_keeps_supply_patch() {
    let h = std::fs::read_to_string(firmware_root().join("board/vendor/embassy-stm32-0.6.0/src/rcc/h.rs")).unwrap();
    assert!(h.contains("G7250 PATCH (supply-once)"), "supply-once patch marker missing");
    // The PWR_CR3 write block (it starts with `use pac::pwr::vals::Sdlevel;`) must not be
    // compiled for the RM0468 parts.
    let at = h.find("use pac::pwr::vals::Sdlevel;").expect("supply write block not found");
    let before = &h[at.saturating_sub(120)..at];
    assert!(
        before.contains("#[cfg(any(pwr_h7rm0399, pwr_h7rm0455, pwr_h7rs))]"),
        "PWR_CR3 write block enabled for RM0468: {before}"
    );
    // No unbounded voltage-ready wait is left.
    assert!(!h.contains("while !PWR.csr1().read().actvosrdy() {}"), "unbounded ACTVOSRDY wait");
    assert!(!h.contains("while !PWR.d3cr().read().vosrdy() {}"), "unbounded VOSRDY wait");
}

#[test]
fn only_the_fpga_module_touches_gpioe() {
    // [HIS] HIS 80: the FPGA bus is driven only through board/app/src/fpga.rs, behind the
    // fpga_gate permit; no other board code may name GPIOE.
    let mut v = Vec::new();
    files(&firmware_root().join("board"), &mut v);
    for f in v.iter().filter(|f| f.extension().is_some_and(|e| e == "rs")) {
        // safe.rs maps every port, E included, but acts only on the safe-state table, and
        // g7250-safety's test fpga_bus_is_never_driven_by_the_safe_state proves that table has no
        // action on port E.
        if f.ends_with("board/app/src/fpga.rs") || f.ends_with("board/common/src/safe.rs") {
            continue;
        }
        let text = std::fs::read_to_string(f).unwrap();
        for line in text.lines() {
            let code = line.split("//").next().unwrap();
            assert!(!code.contains("GPIOE") && !code.contains("PE0") && !code.contains("Port::E"), "{} touches the FPGA bus: {line}", f.display());
        }
    }
}

/// Code lines of every board and crate `.rs` file (comments cut at `//`), with its path.
fn code_lines() -> Vec<(PathBuf, String)> {
    let mut v = Vec::new();
    files(&firmware_root().join("board"), &mut v);
    files(&firmware_root().join("crates"), &mut v);
    let mut out = Vec::new();
    for f in v.into_iter().filter(|f| f.extension().is_some_and(|e| e == "rs")) {
        let text = std::fs::read_to_string(&f).unwrap();
        for line in text.lines() {
            // Code only: the comment cut off, and string literals blanked (descriptions name pins).
            let code = line.split("//").next().unwrap();
            let mut kept = String::new();
            let mut in_str = false;
            for c in code.chars() {
                if c == '"' {
                    in_str = !in_str;
                } else if !in_str {
                    kept.push(c);
                }
            }
            out.push((f.clone(), kept));
        }
    }
    out
}

#[test]
fn only_the_key_path_calls_the_rf_hooks() {
    // docs/tx-lock-audit.md: Keyer::key needs a KeyPermit, but cw_element and atu_step do not;
    // they are called only by g7250_safety::keypath, between a successful key and the unkey.
    // Any other caller would bypass the key path.
    for (f, code) in code_lines() {
        if f.ends_with("crates/safety/src/keypath.rs") {
            continue;
        }
        for call in [".key(&", ".cw_element(", ".atu_step("] {
            assert!(!code.contains(call), "{} calls an RF hook outside the key path: {code}", f.display());
        }
    }
}

#[test]
fn rf_ports_and_rf_peripherals_are_named_only_by_the_safe_state() {
    // docs/tx-lock-audit.md: the RF control pins of [HIS] HIS 85 (ports A, C, D, F, G, H and
    // B14) are written only by board/common/src/safe.rs, at their safe-state levels; the RF
    // peripherals (SPI1 shift-register chains, SPI2 PA bias/drive DAC, DAC1 RF gain) are not used
    // at all. The pins given out are the CH394's (SPI6, PG12-PG15, PB5) and the console's
    // (USART1, PB6, PB7).
    const PORTS: &[&str] = &["GPIOA", "GPIOC", "GPIOD", "GPIOF", "GPIOG", "GPIOH", "Port::A", "Port::C", "Port::D", "Port::F", "Port::G", "Port::H"];
    const PERIPHERALS: &[&str] = &["SPI1", "SPI2", "DAC1", "pac::DAC", "p.DAC"];
    // PC13: CH394 INT# ([HIS] HIS 40, VERIFIED), an input.
    const PINS_ALLOWED: &[&str] = &["PG12", "PG13", "PG14", "PG15", "PB5", "PB6", "PB7", "PC13"];
    for (f, code) in code_lines() {
        let path = f.to_string_lossy();
        let allowed_ports = path.ends_with("board/common/src/safe.rs") || path.ends_with("crates/safety/src/safe_state.rs");
        if !allowed_ports {
            for p in PORTS {
                assert!(!code.contains(p), "{} names an RF port outside the safe state: {code}", f.display());
            }
        }
        for p in PERIPHERALS {
            let hit = code.match_indices(p).any(|(i, _)| !code[i + p.len()..].starts_with(|c: char| c.is_ascii_digit()));
            assert!(!hit, "{} uses an RF peripheral: {code}", f.display());
        }
        // Embassy pin singletons: `p.PXn` for a pin outside the allowed set.
        for (i, _) in code.match_indices("p.P") {
            let rest: String = code[i + 2..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            let is_pin = rest.len() >= 3 && rest.as_bytes()[1].is_ascii_uppercase() && rest[2..].chars().all(|c| c.is_ascii_digit());
            if is_pin {
                assert!(PINS_ALLOWED.contains(&rest.as_str()), "{} claims pin {rest}: {code}", f.display());
            }
        }
    }
}
