// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Flash layout of the G7250 main unit, and the hard guards that keep the vendor path intact.
//!
//! Sources: [HIS] HIS 10 (image placement), HIS 15 (rules 1-5, Main flash) at `9d10e72`;
//! decision 0028 Decision 3 (own boot stage plus A/B slots). The layout itself is firmware
//! design (0028: "The slot sizes, the location of configuration and log, and the boot record
//! format are firmware design"); see `docs/memory-layout.md`.
//!
//! Every flash write and erase in this firmware goes through [`FlashGuard::check`]. The guard
//! refuses, for every writer, anything that touches sector 0 (0x0800_0000..0x0802_0000) or lies
//! outside the application area. Option bytes are not memory-mapped flash; they are protected by
//! never using the option-byte registers at all (see the source scan in `tests/forbidden.rs`).
#![no_std]

pub mod crc32;
pub mod eeprom;
pub mod header;
pub mod select;

/// Start of the Main's flash bank (STM32H72x/H73x, single bank).
pub const FLASH_BASE: u32 = 0x0800_0000;
/// End (exclusive) of the Main's flash: 1 MiB, sectors 0-7 of 128 KiB ([HIS] HIS 15).
pub const FLASH_END: u32 = 0x0810_0000;
/// Erase granularity ([HIS] HIS 15: "erased in 128 KiB sectors", 0028 Context).
pub const SECTOR_SIZE: u32 = 0x2_0000;
/// Programming granularity of the H7 flash: one 256-bit flash word (RM0468).
pub const FLASH_WORD: u32 = 32;

/// A half-open address range `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

impl Range {
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
    pub const fn len(&self) -> u32 {
        self.end - self.start
    }
    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
    /// True if `[addr, addr+len)` lies wholly inside this range. Overflow counts as outside.
    pub const fn contains_span(&self, addr: u32, len: u32) -> bool {
        match addr.checked_add(len) {
            Some(end) => addr >= self.start && end <= self.end,
            None => false,
        }
    }
    /// True if `[addr, addr+len)` shares at least one byte with this range. Overflow counts as
    /// overlapping (fail closed).
    pub const fn overlaps_span(&self, addr: u32, len: u32) -> bool {
        if len == 0 {
            return false;
        }
        match addr.checked_add(len) {
            Some(end) => addr < self.end && end > self.start,
            None => true,
        }
    }
}

/// Sector 0: the vendor bootloader. Never written, never erased ([HIS] HIS 15 rule 1).
pub const VENDOR_BOOTLOADER: Range = Range::new(0x0800_0000, 0x0802_0000);
/// Sectors 1-7: the application area ([HIS] HIS 15, Main flash).
pub const APPLICATION_AREA: Range = Range::new(0x0802_0000, 0x0810_0000);
/// Sector 1: the boot stage of this firmware, where the vendor bootloader starts an application
/// ([HIS] HIS 10). Never updated over the network (0028 Decision 3; 0008 U-7).
pub const BOOT_STAGE: Range = Range::new(0x0802_0000, 0x0804_0000);
/// Sectors 2-3: image slot A (256 KiB).
pub const SLOT_A: Range = Range::new(0x0804_0000, 0x0808_0000);
/// Sectors 4-5: image slot B (256 KiB).
pub const SLOT_B: Range = Range::new(0x0808_0000, 0x080C_0000);
/// Sector 6: kept free. [HIS] HIS 15 rule 5 lists an open check whether the vendor bootloader
/// uses sector 6 as a staging area (HYPOTHESIS); until the F1 drill settles it, nothing of ours
/// lives there.
pub const RESERVED_S6: Range = Range::new(0x080C_0000, 0x080E_0000);
/// Sector 7: configuration, boot record and log of this firmware.
pub const CONFIG: Range = Range::new(0x080E_0000, 0x0810_0000);

/// Offset of the image header from the start of a slot or of the boot stage. The vector table
/// sits at offset 0 ([HIS] HIS 10); the H72x vector table is shorter than 0x400 bytes.
pub const HEADER_OFFSET: u32 = 0x400;

/// One of the two image slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    A,
    B,
}

impl Slot {
    pub const fn range(self) -> Range {
        match self {
            Slot::A => SLOT_A,
            Slot::B => SLOT_B,
        }
    }
    pub const fn other(self) -> Slot {
        match self {
            Slot::A => Slot::B,
            Slot::B => Slot::A,
        }
    }
}

/// Who asks to write. Each writer has its own allowed set; sector 0 is in none of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Writer {
    /// The running application, updating the other slot (A1.8) or its configuration and log.
    Application { running: Slot },
    /// The boot stage, writing only the boot record in the configuration sector.
    BootStage,
    /// The streaming installer for an owner-supplied stock image (0028 Decision 2; [HIS] HIS 15
    /// "Installing the factory firmware": "writes only sectors 1-7").
    StockInstaller,
}

/// Why a write or erase was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The span touches sector 0, the vendor bootloader ([HIS] HIS 15 rule 1).
    VendorBootloader,
    /// The span is not wholly inside the Main's application area.
    OutsideApplicationArea,
    /// The span is inside the application area but not in a region this writer may write.
    NotAllowedForWriter,
    /// Zero length, or not aligned to the flash word / sector as the operation requires.
    Misaligned,
}

/// Write or erase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Program,
    Erase,
}

/// The hard guard. Stateless; every flash driver call goes through it.
pub struct FlashGuard;

impl FlashGuard {
    /// Check an absolute-address operation. `Ok(())` means the operation may proceed.
    pub const fn check(writer: Writer, op: Op, addr: u32, len: u32) -> Result<(), Refusal> {
        // Rule 1 first, for every writer, before anything else is looked at.
        if VENDOR_BOOTLOADER.overlaps_span(addr, len) {
            return Err(Refusal::VendorBootloader);
        }
        if len == 0 {
            return Err(Refusal::Misaligned);
        }
        if !APPLICATION_AREA.contains_span(addr, len) {
            return Err(Refusal::OutsideApplicationArea);
        }
        let align = match op {
            Op::Program => FLASH_WORD,
            Op::Erase => SECTOR_SIZE,
        };
        if addr % align != 0 || len % align != 0 {
            return Err(Refusal::Misaligned);
        }
        let allowed = match writer {
            Writer::Application { running } => {
                running.other().range().contains_span(addr, len) || CONFIG.contains_span(addr, len)
            }
            Writer::BootStage => CONFIG.contains_span(addr, len),
            Writer::StockInstaller => true, // sectors 1-7, already checked above
        };
        if allowed { Ok(()) } else { Err(Refusal::NotAllowedForWriter) }
    }

    /// Same check for an offset relative to [`FLASH_BASE`], as flash drivers use.
    pub const fn check_offset(writer: Writer, op: Op, offset: u32, len: u32) -> Result<(), Refusal> {
        match FLASH_BASE.checked_add(offset) {
            Some(addr) => Self::check(writer, op, addr, len),
            None => Err(Refusal::OutsideApplicationArea),
        }
    }
}

// Compile-time sanity of the layout itself.
const _: () = {
    assert!(BOOT_STAGE.start == 0x0802_0000); // [HIS] HIS 10
    assert!(BOOT_STAGE.end == SLOT_A.start);
    assert!(SLOT_A.end == SLOT_B.start);
    assert!(SLOT_B.end == RESERVED_S6.start);
    assert!(RESERVED_S6.end == CONFIG.start);
    assert!(CONFIG.end == FLASH_END);
    assert!(SLOT_A.len() == SLOT_B.len());
    assert!(SLOT_A.start % SECTOR_SIZE == 0 && SLOT_B.start % SECTOR_SIZE == 0);
    assert!(VENDOR_BOOTLOADER.end == APPLICATION_AREA.start);
};
