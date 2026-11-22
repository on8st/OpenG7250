// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The image check of A1.8 (0067 point 1), streamed over the flash: one function for
//! `UPDATE_VERIFY` and for every start, so that both reach the same result. Every item is judged
//! over the length the image record gives, never over the length received: bytes after the image
//! (a torn word in the rest of the slot, bytes appended to a package) do not count here.

use g7250_layout::header::{self, Header};
use g7250_layout::{HEADER_OFFSET, Range, crc32};
use g7250_store::FlashIo;

/// The item of the image check that failed, in the order of A1.8; the word of the log lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    /// A flash word of the image cannot be read (double ECC error).
    Unreadable,
    /// No image record of this firmware.
    NoRecord,
    Hardware,
    Kind,
    /// The recorded length does not fit the slot.
    Length,
    /// Linked for another place.
    Slot,
    /// The CRC-32 over the whole image.
    Integrity,
    /// Stack pointer outside RAM, or entry point outside the slot.
    Vectors,
}

impl Check {
    pub const fn word(self) -> &'static str {
        match self {
            Check::Unreadable => "unreadable",
            Check::NoRecord => "no-record",
            Check::Hardware => "hardware",
            Check::Kind => "kind",
            Check::Length => "length",
            Check::Slot => "slot",
            Check::Integrity => "integrity",
            Check::Vectors => "vectors",
        }
    }
}

/// Bytes read at a time.
const CHUNK: usize = 512;

/// The image check for `kind` in `region`. `poll` is called after every chunk (the boot stage's
/// TX path check, A3.6b).
pub fn check<F: FlashIo>(io: &mut F, kind: header::Kind, region: Range, ram: Range, poll: &mut dyn FnMut()) -> Result<Header, Check> {
    let mut vec = [0u8; 8];
    io.read(region.start, &mut vec).map_err(|_| Check::Unreadable)?;
    let mut rec = [0u8; header::HEADER_LEN];
    io.read(region.start + HEADER_OFFSET, &mut rec).map_err(|_| Check::Unreadable)?;
    let h = Header::parse(&rec).ok_or(Check::NoRecord)?;
    if h.magic != header::MAGIC || h.header_version != header::HEADER_VERSION || h.header_len as usize != header::HEADER_LEN {
        return Err(Check::NoRecord);
    }
    // Readability is the first item, over the recorded length (as far as it lies in the slot).
    let crc = image_crc(io, region.start, h.length.min(region.len()), poll).map_err(|_| Check::Unreadable)?;
    if h.hw_id != header::HW_ID_G7250_MAIN {
        return Err(Check::Hardware);
    }
    if h.kind != kind as u8 {
        return Err(Check::Kind);
    }
    if h.length < HEADER_OFFSET + header::HEADER_LEN as u32 || h.length > region.len() {
        return Err(Check::Length);
    }
    if h.load_addr != region.start {
        return Err(Check::Slot);
    }
    if crc != h.crc {
        return Err(Check::Integrity);
    }
    let sp = u32::from_le_bytes([vec[0], vec[1], vec[2], vec[3]]);
    let pc = u32::from_le_bytes([vec[4], vec[5], vec[6], vec[7]]);
    if !(sp > ram.start && sp <= ram.end) || pc & 1 == 0 || !region.contains_span(pc & !1, 2) {
        return Err(Check::Vectors);
    }
    Ok(h)
}

/// The CRC of `length` bytes from `start`, the record's `crc` field taken as zero.
fn image_crc<F: FlashIo>(io: &mut F, start: u32, length: u32, poll: &mut dyn FnMut()) -> Result<u32, F::Error> {
    let at = HEADER_OFFSET as usize + header::CRC_FIELD;
    let mut crc = crc32::init();
    let mut buf = [0u8; CHUNK];
    let mut off = 0usize;
    while off < length as usize {
        let n = (length as usize - off).min(CHUNK);
        io.read(start + off as u32, &mut buf[..n])?;
        for i in at.max(off)..(at + 4).min(off + n) {
            buf[i - off] = 0;
        }
        crc = crc32::update(crc, &buf[..n]);
        off += n;
        poll();
    }
    Ok(crc32::finish(crc))
}
