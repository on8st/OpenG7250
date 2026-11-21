// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Image header of this firmware: 64 bytes at [`crate::HEADER_OFFSET`] from the start of an
//! image, after the vector table. Firmware design (0028 Decision 3: the boot record format is
//! firmware design). Little-endian, the MCU's own byte order; this header never goes on the
//! wire as such (the management channel carries its own fields, spec A1.8).
//!
//! The linker reserves the header with `length` and `crc` zero; `g7250-imgtool seal` fills
//! them after the link. The CRC covers the image from its first byte to `length`, with the
//! `crc` field taken as zero.

use crate::{Range, crc32};

pub const MAGIC: u32 = u32::from_le_bytes(*b"G7FW");
pub const HEADER_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 64;

/// Hardware id in our wrapper (spec A1.8 "target hardware id"): the G7250 main unit.
pub const HW_ID_G7250_MAIN: u32 = u32::from_le_bytes(*b"G725");

/// Byte offset of the `crc` field inside the header.
pub const CRC_FIELD: usize = 12;
/// Byte offset of the `length` field inside the header.
pub const LENGTH_FIELD: usize = 8;

/// What kind of image this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    BootStage = 1,
    Application = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct Header {
    pub magic: u32,
    pub header_version: u16,
    pub header_len: u16,
    /// Image length in bytes from the image's first byte (vector table) on.
    pub length: u32,
    pub crc: u32,
    /// [31:24] major, [23:16] minor, [15:0] build (as capability record `fw_version`, B4.8).
    pub fw_version: u32,
    pub hw_id: u32,
    /// The address the image is linked for (slot start, or the boot stage start).
    pub load_addr: u32,
    pub kind: u8,
    /// Non-zero for a beta build (discovery byte 23, B3.5).
    pub beta: u8,
    pub _reserved: [u8; 2],
    /// Source revision as ASCII, NUL padded (for example a short git id).
    pub revision: [u8; 16],
    pub _pad: [u8; 16],
}

const _: () = assert!(core::mem::size_of::<Header>() == HEADER_LEN);

pub const fn version(major: u8, minor: u8, build: u16) -> u32 {
    ((major as u32) << 24) | ((minor as u32) << 16) | build as u32
}

const fn rev(s: &str) -> [u8; 16] {
    let b = s.as_bytes();
    let mut out = [0u8; 16];
    let mut i = 0;
    while i < b.len() && i < 16 {
        out[i] = b[i];
        i += 1;
    }
    out
}

impl Header {
    /// The header as the linker places it: `length` and `crc` still zero.
    pub const fn unsealed(kind: Kind, load_addr: u32, fw_version: u32, beta: bool, revision: &str) -> Self {
        Self {
            magic: MAGIC,
            header_version: HEADER_VERSION,
            header_len: HEADER_LEN as u16,
            length: 0,
            crc: 0,
            fw_version,
            hw_id: HW_ID_G7250_MAIN,
            load_addr,
            kind: kind as u8,
            beta: beta as u8,
            _reserved: [0; 2],
            revision: rev(revision),
            _pad: [0; 16],
        }
    }

    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < HEADER_LEN {
            return None;
        }
        let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        let u16_at = |o: usize| u16::from_le_bytes([b[o], b[o + 1]]);
        let mut revision = [0u8; 16];
        revision.copy_from_slice(&b[32..48]);
        let mut pad = [0u8; 16];
        pad.copy_from_slice(&b[48..64]);
        Some(Self {
            magic: u32_at(0),
            header_version: u16_at(4),
            header_len: u16_at(6),
            length: u32_at(8),
            crc: u32_at(12),
            fw_version: u32_at(16),
            hw_id: u32_at(20),
            load_addr: u32_at(24),
            kind: b[28],
            beta: b[29],
            _reserved: [b[30], b[31]],
            revision,
            _pad: pad,
        })
    }
}

/// Why an image is not bootable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    NoHeader,
    BadMagic,
    BadHeaderVersion,
    WrongHardware,
    WrongKind,
    WrongLoadAddress,
    BadLength,
    BadCrc,
    BadVector,
}

/// CRC over `image[..length]` with the header's `crc` field taken as zero.
pub fn image_crc(image: &[u8], length: usize) -> u32 {
    let h = crate::HEADER_OFFSET as usize;
    let crc_at = h + CRC_FIELD;
    let mut c = crc32::init();
    c = crc32::update(c, &image[..crc_at]);
    c = crc32::update(c, &[0, 0, 0, 0]);
    c = crc32::update(c, &image[crc_at + 4..length]);
    crc32::finish(c)
}

/// Check that `image` (the bytes of a slot, starting at the slot's first byte) holds a sealed,
/// bootable application linked for `slot`.
pub fn validate(image: &[u8], kind: Kind, region: Range, ram: Range) -> Result<Header, Invalid> {
    let h = crate::HEADER_OFFSET as usize;
    let hdr = Header::parse(image.get(h..).ok_or(Invalid::NoHeader)?).ok_or(Invalid::NoHeader)?;
    if hdr.magic != MAGIC {
        return Err(Invalid::BadMagic);
    }
    if hdr.header_version != HEADER_VERSION || hdr.header_len as usize != HEADER_LEN {
        return Err(Invalid::BadHeaderVersion);
    }
    if hdr.hw_id != HW_ID_G7250_MAIN {
        return Err(Invalid::WrongHardware);
    }
    if hdr.kind != kind as u8 {
        return Err(Invalid::WrongKind);
    }
    if hdr.load_addr != region.start {
        return Err(Invalid::WrongLoadAddress);
    }
    let len = hdr.length as usize;
    if len < h + HEADER_LEN || len > region.len() as usize || len > image.len() {
        return Err(Invalid::BadLength);
    }
    if image_crc(image, len) != hdr.crc {
        return Err(Invalid::BadCrc);
    }
    // Initial stack pointer in RAM, reset vector inside the image, Thumb bit set.
    let sp = u32::from_le_bytes([image[0], image[1], image[2], image[3]]);
    let pc = u32::from_le_bytes([image[4], image[5], image[6], image[7]]);
    if !(sp > ram.start && sp <= ram.end) || pc & 1 == 0 || !region.contains_span(pc & !1, 2) {
        return Err(Invalid::BadVector);
    }
    Ok(hdr)
}

/// Seal an image in place: fill `length` and `crc`. Used by `g7250-imgtool`.
pub fn seal(image: &mut [u8]) -> Result<u32, Invalid> {
    let h = crate::HEADER_OFFSET as usize;
    if image.len() < h + HEADER_LEN {
        return Err(Invalid::NoHeader);
    }
    let len = image.len();
    image[h + LENGTH_FIELD..h + LENGTH_FIELD + 4].copy_from_slice(&(len as u32).to_le_bytes());
    let crc = image_crc(image, len);
    image[h + CRC_FIELD..h + CRC_FIELD + 4].copy_from_slice(&crc.to_le_bytes());
    Ok(crc)
}

const fn w(o: &mut [u8; HEADER_LEN], at: usize, v: &[u8]) {
    let mut i = 0;
    while i < v.len() {
        o[at + i] = v[i];
        i += 1;
    }
}

/// Raw header bytes, for placing the header in the linker section.
pub const fn to_bytes(h: &Header) -> [u8; HEADER_LEN] {
    let mut o = [0u8; HEADER_LEN];
    w(&mut o, 0, &h.magic.to_le_bytes());
    w(&mut o, 4, &h.header_version.to_le_bytes());
    w(&mut o, 6, &h.header_len.to_le_bytes());
    w(&mut o, 8, &h.length.to_le_bytes());
    w(&mut o, 12, &h.crc.to_le_bytes());
    w(&mut o, 16, &h.fw_version.to_le_bytes());
    w(&mut o, 20, &h.hw_id.to_le_bytes());
    w(&mut o, 24, &h.load_addr.to_le_bytes());
    o[28] = h.kind;
    o[29] = h.beta;
    w(&mut o, 32, &h.revision);
    o
}
