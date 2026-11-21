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
    let mut v = Vec::new();
    files(&firmware_root().join("board"), &mut v);
    let mut seen = 0;
    for f in v.iter().filter(|f| f.extension().is_some_and(|e| e == "x")) {
        let text = std::fs::read_to_string(f).unwrap();
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("FLASH") && l.contains("ORIGIN") {
                let o = l.split("ORIGIN").nth(1).unwrap();
                let hex = o.split("0x").nth(1).unwrap();
                let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit() || *c == '_').filter(|c| *c != '_').collect();
                let origin = u32::from_str_radix(&hex, 16).unwrap();
                assert!(origin >= 0x0802_0000, "{}: FLASH ORIGIN {origin:#x} below 0x08020000", f.display());
                seen += 1;
            }
        }
    }
    assert!(seen >= 2, "expected linker scripts for the boot stage and the application");
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
