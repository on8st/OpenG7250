// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Non-volatile store of the G7250 main firmware in flash sector 7 (`g7250_layout::CONFIG`).
//!
//! Holds what must survive a restart: the boot record (0028 Decision 3), the boot counter of the
//! arm token (B10.2 item 5), the configuration (A1.6, validated as a whole, "stored in
//! non-volatile memory with a version and a check value"), the calibration set (A1.12) and the
//! management secret (A1.10). Firmware design; see `docs/memory-layout.md`, "Store".
//!
//! Format: append-only records, each a whole number of 32-byte flash words (the H7 programs a
//! 256-bit word once). Record = header (16 bytes) + payload, padded with 0xFF:
//!
//! | Bytes | Field |
//! |---|---|
//! | 0-3 | magic "G7ST" |
//! | 4 | kind ([`Kind`]) |
//! | 5 | format version of the payload |
//! | 6-7 | payload length |
//! | 8-11 | sequence number, +1 per record written |
//! | 12-15 | CRC-32 of bytes 0-11 and the payload |
//!
//! The newest valid record of a kind wins. A record that fails its CRC (a torn write) is skipped;
//! appending continues after the last written word. When the sector is full, the newest record
//! of each kind is kept in RAM, the sector is erased and they are written back (compaction).
//!
//! Every program and erase passes [`FlashGuard::check`] first: the store can never reach sector 0,
//! and it has no path to the option bytes, the EEPROM or the Backup SRAM ([HIS] HIS 15).
#![cfg_attr(not(test), no_std)]

use g7250_layout::crc32;
use g7250_layout::{CONFIG, FLASH_WORD, FlashGuard, Op, Refusal, Writer};

pub const MAGIC: u32 = u32::from_le_bytes(*b"G7ST");
pub const HEADER: usize = 16;
/// Largest payload of one record (a calibration set fits a 1000-byte request, A1.4).
pub const MAX_PAYLOAD: usize = 1024;

/// Record kinds. The values are part of the stored format; never renumber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    BootRecord = 1,
    BootCounter = 2,
    Config = 3,
    Calibration = 4,
    Secret = 5,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::BootRecord, Kind::BootCounter, Kind::Config, Kind::Calibration, Kind::Secret];
    fn from_u8(v: u8) -> Option<Kind> {
        Self::ALL.iter().copied().find(|k| *k as u8 == v)
    }
}

/// Flash access for the store: offsets are absolute addresses. The board implements it on the
/// HAL flash driver; tests on a RAM model that enforces write-once flash words.
pub trait FlashIo {
    type Error;
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), Self::Error>;
    /// Program whole flash words at a word-aligned address. Words must be erased.
    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), Self::Error>;
    /// Erase the whole sector that starts at `addr`.
    fn erase_sector(&mut self, addr: u32) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Io(E),
    /// The guard refused the operation (a bug: the store only works inside `CONFIG`).
    Guard(Refusal),
    TooLarge,
}

fn padded(len: usize) -> u32 {
    (((HEADER + len) as u32 + FLASH_WORD - 1) / FLASH_WORD) * FLASH_WORD
}

struct Found {
    addr: u32,
    seq: u32,
    len: u16,
    version: u8,
}

pub struct Store<F> {
    io: F,
    writer: Writer,
    /// Address of the next free flash word.
    end: u32,
    seq: u32,
}

impl<F: FlashIo> Store<F> {
    /// Scan the sector. `writer` is who writes: the application or the boot stage.
    pub fn open(io: F, writer: Writer) -> Result<Self, Error<F::Error>> {
        let mut s = Store { io, writer, end: CONFIG.start, seq: 0 };
        let mut addr = CONFIG.start;
        let mut last_written = CONFIG.start;
        while addr + FLASH_WORD <= CONFIG.end {
            let mut h = [0u8; HEADER];
            s.io.read(addr, &mut h).map_err(Error::Io)?;
            if h.iter().all(|&b| b == 0xFF) {
                // Erased word: either the end, or the gap after a torn write. A record never
                // starts with an erased word, so step one word.
                addr += FLASH_WORD;
                continue;
            }
            match s.check(addr, &h)? {
                Some(f) => {
                    s.seq = s.seq.max(f.seq);
                    addr += padded(f.len as usize);
                }
                None => addr += FLASH_WORD, // torn or foreign word: skip it
            }
            last_written = addr;
        }
        s.end = last_written;
        Ok(s)
    }

    pub fn release(self) -> F {
        self.io
    }

    /// Bytes still free in the sector.
    pub fn free(&self) -> u32 {
        CONFIG.end - self.end
    }

    fn check(&mut self, addr: u32, h: &[u8; HEADER]) -> Result<Option<Found>, Error<F::Error>> {
        if u32::from_le_bytes([h[0], h[1], h[2], h[3]]) != MAGIC || Kind::from_u8(h[4]).is_none() {
            return Ok(None);
        }
        let len = u16::from_le_bytes([h[6], h[7]]);
        if len as usize > MAX_PAYLOAD || addr + padded(len as usize) > CONFIG.end {
            return Ok(None);
        }
        let mut c = crc32::update(crc32::init(), &h[..12]);
        let mut buf = [0u8; 64];
        let mut at = 0usize;
        while at < len as usize {
            let n = (len as usize - at).min(buf.len());
            self.io.read(addr + (HEADER + at) as u32, &mut buf[..n]).map_err(Error::Io)?;
            c = crc32::update(c, &buf[..n]);
            at += n;
        }
        if crc32::finish(c) != u32::from_le_bytes([h[12], h[13], h[14], h[15]]) {
            return Ok(None);
        }
        Ok(Some(Found { addr, seq: u32::from_le_bytes([h[8], h[9], h[10], h[11]]), len, version: h[5] }))
    }

    fn newest(&mut self, kind: Kind) -> Result<Option<Found>, Error<F::Error>> {
        let mut best: Option<Found> = None;
        let mut addr = CONFIG.start;
        while addr + FLASH_WORD <= self.end {
            let mut h = [0u8; HEADER];
            self.io.read(addr, &mut h).map_err(Error::Io)?;
            match self.check(addr, &h)? {
                Some(f) => {
                    let step = padded(f.len as usize);
                    if h[4] == kind as u8 && best.as_ref().is_none_or(|b| f.seq > b.seq) {
                        best = Some(f);
                    }
                    addr += step;
                }
                None => addr += FLASH_WORD,
            }
        }
        Ok(best)
    }

    /// Read the newest record of `kind` into `buf`; returns (format version, length).
    pub fn get(&mut self, kind: Kind, buf: &mut [u8]) -> Result<Option<(u8, usize)>, Error<F::Error>> {
        let Some(f) = self.newest(kind)? else { return Ok(None) };
        let n = f.len as usize;
        if buf.len() < n {
            return Err(Error::TooLarge);
        }
        self.io.read(f.addr + HEADER as u32, &mut buf[..n]).map_err(Error::Io)?;
        Ok(Some((f.version, n)))
    }

    fn guard(&self, op: Op, addr: u32, len: u32) -> Result<(), Error<F::Error>> {
        FlashGuard::check(self.writer, op, addr, len).map_err(Error::Guard)
    }

    fn write_record(&mut self, kind: Kind, version: u8, payload: &[u8]) -> Result<(), Error<F::Error>> {
        self.seq = self.seq.wrapping_add(1);
        let mut h = [0u8; HEADER];
        h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[4] = kind as u8;
        h[5] = version;
        h[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        h[8..12].copy_from_slice(&self.seq.to_le_bytes());
        let c = crc32::finish(crc32::update(crc32::update(crc32::init(), &h[..12]), payload));
        h[12..16].copy_from_slice(&c.to_le_bytes());
        let total = padded(payload.len());
        self.guard(Op::Program, self.end, total)?;
        // Assemble word by word: header, payload, 0xFF padding.
        let mut word = [0xFFu8; FLASH_WORD as usize];
        let mut pos = 0usize; // position in the record
        let total_us = total as usize;
        let mut addr = self.end;
        while pos < total_us {
            for (i, w) in word.iter_mut().enumerate() {
                let p = pos + i;
                *w = if p < HEADER {
                    h[p]
                } else if p - HEADER < payload.len() {
                    payload[p - HEADER]
                } else {
                    0xFF
                };
            }
            self.io.program(addr, &word).map_err(Error::Io)?;
            addr += FLASH_WORD;
            pos += FLASH_WORD as usize;
        }
        self.end += total;
        Ok(())
    }

    /// Store a new record of `kind`. Compacts the sector first if it does not fit.
    pub fn put(&mut self, kind: Kind, version: u8, payload: &[u8]) -> Result<(), Error<F::Error>> {
        if payload.len() > MAX_PAYLOAD {
            return Err(Error::TooLarge);
        }
        if self.end + padded(payload.len()) > CONFIG.end {
            self.compact(Some(kind))?;
        }
        self.write_record(kind, version, payload)
    }

    /// Erase the sector and write back the newest record of every kind except `skip` (the kind
    /// about to be written anew) and except kinds dropped by [`Store::compact_without`].
    fn compact(&mut self, skip: Option<Kind>) -> Result<(), Error<F::Error>> {
        self.compact_keep(|k| Some(k) != skip)
    }

    /// Remove every record of the kinds for which `drop` is true (for example the secret at a
    /// factory reset or before a switch to stock, 0024 R5), by compaction.
    pub fn compact_without(&mut self, drop: impl Fn(Kind) -> bool) -> Result<(), Error<F::Error>> {
        self.compact_keep(|k| !drop(k))
    }

    fn compact_keep(&mut self, keep: impl Fn(Kind) -> bool) -> Result<(), Error<F::Error>> {
        // RAM copy of the newest record of each kind kept.
        let mut saved: [(u8, u16, [u8; MAX_PAYLOAD]); 5] = [(0, 0, [0; MAX_PAYLOAD]); 5];
        let mut have = [false; 5];
        for (i, k) in Kind::ALL.iter().enumerate() {
            if !keep(*k) {
                continue;
            }
            let mut b = [0u8; MAX_PAYLOAD];
            if let Some((v, n)) = self.get(*k, &mut b)? {
                saved[i] = (v, n as u16, b);
                have[i] = true;
            }
        }
        self.guard(Op::Erase, CONFIG.start, CONFIG.len())?;
        self.io.erase_sector(CONFIG.start).map_err(Error::Io)?;
        self.end = CONFIG.start;
        for (i, k) in Kind::ALL.iter().enumerate() {
            if have[i] {
                let (v, n, b) = saved[i];
                self.write_record(*k, v, &b[..n as usize])?;
            }
        }
        Ok(())
    }

    /// Increment the boot counter and return the new value (B10.2 item 5). Called once per
    /// start by the application.
    pub fn next_boot_counter(&mut self) -> Result<u32, Error<F::Error>> {
        let mut b = [0u8; 4];
        let n = match self.get(Kind::BootCounter, &mut b)? {
            Some((_, 4)) => u32::from_le_bytes(b).wrapping_add(1),
            _ => 1,
        };
        self.put(Kind::BootCounter, 1, &n.to_le_bytes())?;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g7250_layout::{SECTOR_SIZE, Slot};

    /// Sector 7 in RAM, with the rules of H7 flash: program only erased, aligned, whole words,
    /// each word once; erase whole sectors. Optionally fail after `budget` words (power loss).
    struct Sim {
        mem: std::vec::Vec<u8>,
        written: std::vec::Vec<bool>,
        budget: Option<usize>,
        erases: usize,
    }

    #[derive(Debug, PartialEq)]
    enum SimErr {
        PowerLost,
        Rule(&'static str),
    }

    impl Sim {
        fn new() -> Self {
            Sim { mem: vec![0xFF; SECTOR_SIZE as usize], written: vec![false; (SECTOR_SIZE / FLASH_WORD) as usize], budget: None, erases: 0 }
        }
    }

    impl FlashIo for Sim {
        type Error = SimErr;
        fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), SimErr> {
            let o = (addr - CONFIG.start) as usize;
            buf.copy_from_slice(&self.mem[o..o + buf.len()]);
            Ok(())
        }
        fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), SimErr> {
            if addr % FLASH_WORD != 0 || data.len() % FLASH_WORD as usize != 0 || !CONFIG.contains_span(addr, data.len() as u32) {
                return Err(SimErr::Rule("alignment or range"));
            }
            for (i, w) in data.chunks(FLASH_WORD as usize).enumerate() {
                if let Some(b) = self.budget.as_mut() {
                    if *b == 0 {
                        return Err(SimErr::PowerLost);
                    }
                    *b -= 1;
                }
                let wi = ((addr - CONFIG.start) / FLASH_WORD) as usize + i;
                if self.written[wi] {
                    return Err(SimErr::Rule("word programmed twice"));
                }
                self.written[wi] = true;
                self.mem[wi * 32..wi * 32 + 32].copy_from_slice(w);
            }
            Ok(())
        }
        fn erase_sector(&mut self, addr: u32) -> Result<(), SimErr> {
            assert_eq!(addr, CONFIG.start);
            self.mem.fill(0xFF);
            self.written.fill(false);
            self.erases += 1;
            Ok(())
        }
    }

    const APP: Writer = Writer::Application { running: Slot::A };

    #[test]
    fn empty_store() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        let mut b = [0u8; 16];
        assert_eq!(s.get(Kind::Config, &mut b).unwrap(), None);
        assert_eq!(s.free(), SECTOR_SIZE);
    }

    #[test]
    fn newest_wins_and_survives_reopen() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"first").unwrap();
        s.put(Kind::Calibration, 1, &[7u8; 300]).unwrap();
        s.put(Kind::Config, 1, b"second config").unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        let mut b = [0u8; 400];
        let (v, n) = s.get(Kind::Config, &mut b).unwrap().unwrap();
        assert_eq!((v, &b[..n]), (1, &b"second config"[..]));
        let (_, n) = s.get(Kind::Calibration, &mut b).unwrap().unwrap();
        assert_eq!(&b[..n], &[7u8; 300][..]);
    }

    #[test]
    fn boot_counter_counts_across_restarts() {
        let mut io = Sim::new();
        for want in 1..=5u32 {
            let mut s = Store::open(io, APP).unwrap();
            assert_eq!(s.next_boot_counter().unwrap(), want);
            io = s.release();
        }
    }

    #[test]
    fn torn_write_is_skipped_and_appending_goes_on() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"good").unwrap();
        let mut io = s.release();
        io.budget = Some(1); // power lost after the first word of the next record
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!(s.put(Kind::Config, 1, &[9u8; 100]), Err(Error::Io(SimErr::PowerLost)));
        let mut io = s.release();
        io.budget = None;
        let mut s = Store::open(io, APP).unwrap();
        let mut b = [0u8; 200];
        let (_, n) = s.get(Kind::Config, &mut b).unwrap().unwrap();
        assert_eq!(&b[..n], b"good");
        // The next record goes after the torn one; no word is programmed twice.
        s.put(Kind::Config, 1, b"after").unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        let (_, n) = s.get(Kind::Config, &mut b).unwrap().unwrap();
        assert_eq!(&b[..n], b"after");
    }

    #[test]
    fn compaction_keeps_newest_of_each_kind() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Calibration, 2, &[5u8; 900]).unwrap();
        s.put(Kind::Secret, 1, &[1u8; 32]).unwrap();
        for i in 0..5000u32 {
            s.next_boot_counter().unwrap();
            if i % 100 == 0 {
                s.put(Kind::Config, 1, &i.to_le_bytes()).unwrap();
            }
        }
        let io = s.release();
        assert!(io.erases >= 1);
        let mut s = Store::open(io, APP).unwrap();
        let mut b = [0u8; MAX_PAYLOAD];
        assert_eq!(s.get(Kind::BootCounter, &mut b).unwrap(), Some((1, 4)));
        assert_eq!(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), 5000);
        let (v, n) = s.get(Kind::Calibration, &mut b).unwrap().unwrap();
        assert_eq!((v, n, b[0]), (2, 900, 5));
        let (_, n) = s.get(Kind::Config, &mut b).unwrap().unwrap();
        assert_eq!(&b[..n], &4900u32.to_le_bytes());
        assert_eq!(s.get(Kind::Secret, &mut b).unwrap(), Some((1, 32)));
    }

    #[test]
    fn compact_without_drops_the_secret() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Secret, 1, &[1u8; 32]).unwrap();
        s.put(Kind::Config, 1, b"cfg").unwrap();
        s.compact_without(|k| k == Kind::Secret).unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        let mut b = [0u8; 64];
        assert_eq!(s.get(Kind::Secret, &mut b).unwrap(), None);
        assert!(s.get(Kind::Config, &mut b).unwrap().is_some());
    }

    #[test]
    fn too_large_and_guard() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        assert_eq!(s.put(Kind::Config, 1, &[0u8; MAX_PAYLOAD + 1]), Err(Error::TooLarge));
        // The boot stage may write the store too (it is in CONFIG, its only region).
        let mut s = Store::open(s.release(), Writer::BootStage).unwrap();
        s.put(Kind::BootRecord, 1, &[0u8; 16]).unwrap();
    }
}
