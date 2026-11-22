// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! `g7250-imgtool`: seal and check images of the G7250 main firmware.
//!
//! ```text
//! g7250-imgtool seal  <boot|app-a|app-b> <elf> [--bin <out.bin>]
//! g7250-imgtool check <boot|app-a|app-b> <elf>
//! g7250-imgtool merge <out.elf> <boot.elf> <app.elf>...
//! ```
//!
//! `merge` writes one ELF with the load segments of all inputs (entry point of the first), for
//! a loader that takes a single file, such as the emulator black-box run (0034). It refuses
//! inputs whose flash segments overlap.
//!
//! `seal` fills `length` and `crc` of the image header (g7250-layout `header`) in the ELF, in
//! place, and can write the flat image. Both commands run the placement check first: every
//! loadable byte lies in the image's own flash region (never in sector 0, [HIS] HIS 15 rule 1;
//! at or above 0x08020000, HIS 10), RAM segments carry no flash bytes of their own, and the
//! flash bytes form one gap-free run from the region start.

use g7250_layout::header::{self, Kind};
use g7250_layout::{BOOT_STAGE, Range, SLOT_A, SLOT_B, VENDOR_BOOTLOADER};
use std::process::ExitCode;

mod elf;

pub fn target(name: &str) -> Option<(Kind, Range)> {
    match name {
        "boot" => Some((Kind::BootStage, BOOT_STAGE)),
        "app-a" => Some((Kind::Application, SLOT_A)),
        "app-b" => Some((Kind::Application, SLOT_B)),
        _ => None,
    }
}

/// RAM the images may use (AXI SRAM, board/*/memory*.x).
pub const RAM: Range = Range::new(0x2400_0000, 0x2402_0000);
/// SRAM4 (D3), for BDMA buffers only; zero-fill or no-load sections only.
pub const SRAM4: Range = Range::new(0x3800_0000, 0x3800_4000);
/// Backup SRAM: never used ([HIS] HIS 15 rule 4).
pub const BACKUP_SRAM: Range = Range::new(0x3880_0000, 0x3880_1000);

/// The flat flash image of an ELF, and where each flash byte came from in the file.
pub struct Flat {
    pub bytes: Vec<u8>,
    /// (address, file offset, length) of every flash run.
    pub runs: Vec<(u32, usize, usize)>,
}

pub fn flatten(file: &[u8], region: Range) -> Result<Flat, String> {
    let segs = elf::load_segments(file)?;
    let mut runs = Vec::new();
    for s in &segs {
        // The load address (LMA) is where the bytes land in flash.
        if s.filesz > 0 {
            if VENDOR_BOOTLOADER.overlaps_span(s.paddr, s.filesz) {
                return Err(format!("segment at {:#010x} touches sector 0 (vendor bootloader)", s.paddr));
            }
            if !region.contains_span(s.paddr, s.filesz) {
                return Err(format!(
                    "segment at {:#010x}+{:#x} is outside its region {:#010x}..{:#010x}",
                    s.paddr, s.filesz, region.start, region.end
                ));
            }
            runs.push((s.paddr, s.offset as usize, s.filesz as usize));
        }
        // The run-time address of RAM segments (.data, .bss) must be in RAM.
        if s.memsz > 0 && s.vaddr != s.paddr && !RAM.contains_span(s.vaddr, s.memsz) {
            return Err(format!("RAM segment at {:#010x}+{:#x} is outside RAM", s.vaddr, s.memsz));
        }
        if s.memsz > 0 && BACKUP_SRAM.overlaps_span(s.vaddr, s.memsz) {
            return Err(format!("segment at {:#010x} is in the Backup SRAM", s.vaddr));
        }
        if s.filesz == 0 && s.memsz > 0 && !RAM.contains_span(s.vaddr, s.memsz) && !SRAM4.contains_span(s.vaddr, s.memsz) {
            return Err(format!("zero-fill segment at {:#010x} is outside RAM", s.vaddr));
        }
    }
    runs.sort_by_key(|r| r.0);
    let mut next = region.start;
    let mut bytes = Vec::new();
    for &(addr, off, len) in &runs {
        if addr != next {
            return Err(format!("gap or overlap at {next:#010x} (next segment at {addr:#010x})"));
        }
        bytes.extend_from_slice(file.get(off..off + len).ok_or("segment beyond end of file")?);
        next = addr + len as u32;
    }
    if bytes.is_empty() {
        return Err("no flash bytes".into());
    }
    Ok(Flat { bytes, runs })
}

/// Write bytes of the flat image back into the ELF file at the place they came from.
fn patch(file: &mut [u8], flat: &Flat, region: Range, at: usize, len: usize) {
    let addr = region.start + at as u32;
    for &(a, off, l) in &flat.runs {
        if addr >= a && addr + len as u32 <= a + l as u32 {
            let o = off + (addr - a) as usize;
            file[o..o + len].copy_from_slice(&flat.bytes[at..at + len]);
            return;
        }
    }
    panic!("header not inside one segment");
}

pub fn seal_elf(file: &mut Vec<u8>, kind: Kind, region: Range) -> Result<Vec<u8>, String> {
    let mut flat = flatten(file, region)?;
    let h = g7250_layout::HEADER_OFFSET as usize;
    let hdr = header::Header::parse(flat.bytes.get(h..).ok_or("image shorter than its header")?).ok_or("no header")?;
    if hdr.magic != header::MAGIC || hdr.kind != kind as u8 || hdr.load_addr != region.start {
        return Err("header missing or for another kind or region".into());
    }
    header::seal(&mut flat.bytes).map_err(|e| format!("{e:?}"))?;
    patch(file, &flat, region, h + header::LENGTH_FIELD, 8);
    header::validate(&flat.bytes, kind, region, RAM).map_err(|e| format!("sealed image does not validate: {e:?}"))?;
    Ok(flat.bytes)
}

/// The name of a package file (0071 point 2; a convention, hosts read the header).
pub fn package_name(v: u32, region: Range) -> String {
    format!("g7250-{}.{}.{}-slot-{}.g7pk", v >> 24, (v >> 16) & 0xFF, v & 0xFFFF, if region == SLOT_A { "a" } else { "b" })
}

/// A package file: the Table M-4 header, then the image (0071).
pub fn package(image: &[u8], v: u32, region: Range) -> Vec<u8> {
    use g7250_proto::mgmt::payload::{COMPONENT_MAIN, HARDWARE_ID_G7250, Package, Version};
    use sha2::Digest;
    let pkg = Package {
        header_version: 1,
        component: COMPONENT_MAIN,
        hardware_id: HARDWARE_ID_G7250,
        min_hw_revision: 0,
        version: Version { major: (v >> 24) as u8, minor: (v >> 16) as u8, build: v as u16 },
        length: image.len() as u32,
        sha256: sha2::Sha256::digest(image).into(),
        signature_len: 0,
        target_slot: if region == SLOT_A { 1 } else { 2 },
    };
    let mut b = pkg.encode().to_vec();
    b.extend_from_slice(image);
    b
}

/// Break one item of the image check in a sealed image and seal it again, so that only that item
/// fails. Returns what the check must then report.
pub fn bad_image(image: &mut [u8], region: Range, case: &str) -> Option<header::Invalid> {
    let h = g7250_layout::HEADER_OFFSET as usize;
    let want = match case {
        "bad-hw" => {
            image[h + 20] ^= 1; // hw_id
            header::Invalid::WrongHardware
        }
        "bad-kind" => {
            image[h + 28] = Kind::BootStage as u8;
            header::Invalid::WrongKind
        }
        "bad-entry" => {
            let other = if region == SLOT_A { SLOT_B } else { SLOT_A };
            image[4..8].copy_from_slice(&((other.start + 0x501) | 1).to_le_bytes()); // reset vector
            header::Invalid::BadVector
        }
        _ => return None,
    };
    header::seal(image).ok()?;
    Some(want)
}

fn run(args: &[String]) -> Result<(), String> {
    let usage = "usage: g7250-imgtool <seal|check> <boot|app-a|app-b> <elf> [--bin <out>] | merge <out> <elf>... | package <app-a|app-b> <bin> <out-dir> | test-package <app-a|app-b> <bin> <out> <bad-hw|bad-kind|bad-entry>";
    if let [c, out, ins @ ..] = args {
        if c == "merge" {
            let files: Vec<Vec<u8>> =
                ins.iter().map(|p| std::fs::read(p).map_err(|e| format!("{p}: {e}"))).collect::<Result<_, _>>()?;
            if files.is_empty() {
                return Err(usage.into());
            }
            let merged = elf::merge(&files)?;
            std::fs::write(out, merged).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: merged {} images", files.len());
            return Ok(());
        }
    }
    let (cmd, tgt, path) = match args {
        [c, t, p, ..] => (c.as_str(), t.as_str(), p.as_str()),
        _ => return Err(usage.into()),
    };
    let (kind, region) = target(tgt).ok_or(usage)?;
    let mut file = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    match cmd {
        "check" => {
            let flat = flatten(&file, region)?;
            let hdr = header::validate(&flat.bytes, kind, region, RAM).map_err(|e| format!("{path}: {e:?}"))?;
            println!(
                "{path}: ok, {} bytes at {:#010x}, crc {:#010x}, version {:#010x}",
                hdr.length, region.start, hdr.crc, hdr.fw_version
            );
        }
        "package" => {
            // Update package (A1.8; 0043): the Table M-4 header (the UPDATE_BEGIN payload after
            // the flags byte, 53 bytes) followed by the sealed image of one slot.
            let [_, _, _, dir] = args else { return Err(usage.into()) };
            if kind != Kind::Application {
                return Err("a package carries an application image".into());
            }
            let hdr = header::validate(&file, kind, region, RAM).map_err(|e| format!("{path}: {e:?}"))?;
            let b = package(&file[..hdr.length as usize], hdr.fw_version, region);
            let out = &format!("{dir}/{}", package_name(hdr.fw_version, region));
            std::fs::write(out, &b).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: package, {} image bytes, version {:#010x}", hdr.length, hdr.fw_version);
        }
        "test-package" => {
            // Test only, never shipped (T-A1-88; 0067): a package that UPDATE_BEGIN accepts and
            // whose SHA-256 is right, with an image that fails one item of the image check.
            let [_, _, _, out, case] = args else { return Err(usage.into()) };
            if kind != Kind::Application {
                return Err("a package carries an application image".into());
            }
            let hdr = header::validate(&file, kind, region, RAM).map_err(|e| format!("{path}: {e:?}"))?;
            let mut image = file[..hdr.length as usize].to_vec();
            let want = bad_image(&mut image, region, case).ok_or_else(|| format!("{case}: no such test case; {usage}"))?;
            match header::validate(&image, kind, region, RAM) {
                Err(e) if e == want => {}
                r => return Err(format!("{out}: expected {want:?}, the image check gives {r:?}")),
            }
            let b = package(&image, hdr.fw_version, region);
            std::fs::write(out, &b).map_err(|e| format!("{out}: {e}"))?;
            println!("{out}: TEST package ({case}): the image check must fail with {want:?}");
        }
        "seal" => {
            let bytes = seal_elf(&mut file, kind, region)?;
            std::fs::write(path, &file).map_err(|e| format!("{path}: {e}"))?;
            if let [_, _, _, flag, out] = args {
                if flag == "--bin" {
                    std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
                }
            }
            println!("{path}: sealed, {} bytes", bytes.len());
        }
        _ => return Err(usage.into()),
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("g7250-imgtool: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g7250_layout::header::{Header, Invalid};

    #[test]
    fn test_packages_fail_one_item_with_a_right_hash_t_a1_88() {
        use sha2::Digest;
        for (slot, region) in [(1u8, SLOT_A), (2, SLOT_B)] {
            let mut good = vec![0x5Au8; 3000];
            good[0..4].copy_from_slice(&0x2401_0000u32.to_le_bytes());
            good[4..8].copy_from_slice(&(region.start + 0x501).to_le_bytes());
            let h = header::to_bytes(&Header::unsealed(Kind::Application, region.start, header::version(0, 1, 1), true, "test"));
            let o = g7250_layout::HEADER_OFFSET as usize;
            good[o..o + h.len()].copy_from_slice(&h);
            header::seal(&mut good).unwrap();
            for (case, want) in [("bad-hw", Invalid::WrongHardware), ("bad-kind", Invalid::WrongKind), ("bad-entry", Invalid::BadVector)] {
                let mut img = good.clone();
                assert_eq!(bad_image(&mut img, region, case), Some(want));
                assert_eq!(header::validate(&img, Kind::Application, region, RAM), Err(want), "{case}: that item and no other");
                let b = package(&img, header::version(0, 1, 1), region);
                assert_eq!(b.len(), 53 + img.len());
                let p = g7250_proto::mgmt::payload::Package::decode(&b).unwrap();
                assert_eq!((p.hardware_id, p.component, p.target_slot, p.length as usize), (0x00C8, 1, slot, img.len()));
                assert_eq!(p.sha256, <[u8; 32]>::from(sha2::Sha256::digest(&b[53..])));
            }
            assert_eq!(bad_image(&mut good, region, "other"), None);
        }
    }

    #[test]
    fn package_file_0071() {
        assert_eq!(package_name(header::version(0, 1, 1), SLOT_A), "g7250-0.1.1-slot-a.g7pk");
        assert_eq!(package_name(header::version(1, 20, 300), SLOT_B), "g7250-1.20.300-slot-b.g7pk");
        // The 53-byte header, then the image, nothing else.
        let img = [0xA5u8; 1500];
        let b = package(&img, header::version(0, 1, 1), SLOT_B);
        assert_eq!(b.len(), 53 + u32::from_be_bytes(b[14..18].try_into().unwrap()) as usize);
        assert_eq!((&b[..4], b[4], &b[50..53]), (&b"G7PK"[..], 1, &[0, 0, 2][..]));
        assert_eq!(b[53..], img);
    }
}
