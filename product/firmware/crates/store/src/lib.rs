// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Non-volatile store of the G7250 main firmware in flash sectors 6 and 7
//! (`g7250_layout::STORE_SECTORS`), used in turn.
//!
//! Holds what must survive a restart: the boot record (0028 Decision 3), the boot counter of the
//! arm token (B10.2 item 5), the configuration (A1.6, "stored in non-volatile memory with a
//! version and a check value"), the calibration set (A1.12) and the management secret (A1.10).
//! Firmware design; see `docs/memory-layout.md`, "Store".
//!
//! Records: each a whole number of 32-byte flash words (the H7 programs a 256-bit word once).
//! Record = header (16 bytes) + payload, padded with 0xFF:
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
//! Generations: a [`Kind::Commit`] record (payload: generation, u32) marks a sector as complete.
//! The live sector is the one whose newest commit has the highest generation; with no commit
//! anywhere (a fresh unit, or a store written before commits existed) sector 7 is live as
//! generation 0. Records are appended to the live sector; the newest valid record of each kind
//! wins; a torn record fails its CRC and is skipped. A flash word that cannot be read at all (a
//! double ECC error, [`FlashIo::unreadable`]) is skipped the same way, and never written again.
//!
//! Compaction, when the live sector is full: (1) erase the other sector, (2) copy the newest
//! record of every kind into it, (3) write a commit with the next generation, (4) switch to it.
//! The old sector is not touched; it is erased only by the next compaction. So a reset or a power
//! loss at any step leaves either the old committed sector (steps 1-3, before the commit word is
//! complete) or the new one (after) as the live sector, with every value present: no step ever
//! erases the only copy. Tests: `tests` below cut power at every flash word and during the erase.
//!
//! Every program and erase passes [`FlashGuard::check`] first: the store can reach only sectors
//! 6 and 7, never sector 0, and has no path to the option bytes, the EEPROM or the Backup SRAM
//! ([HIS] HIS 15).
#![cfg_attr(not(test), no_std)]

use g7250_layout::crc32;
use g7250_layout::{FLASH_WORD, FlashGuard, Op, Range, Refusal, STORE_SECTORS, Writer};

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
    /// End of a compaction: the sector holds a complete generation (payload: generation, u32).
    Commit = 6,
}

impl Kind {
    /// The kinds that carry data (copied by a compaction).
    pub const DATA: [Kind; 5] = [Kind::BootRecord, Kind::BootCounter, Kind::Config, Kind::Calibration, Kind::Secret];
    fn from_u8(v: u8) -> Option<Kind> {
        Self::DATA.iter().copied().chain([Kind::Commit]).find(|k| *k as u8 == v)
    }
}

/// Flash access for the store: addresses are absolute. The board implements it on the HAL flash
/// driver (erase from RAM, watchdog refreshed); tests on a RAM model with write-once words.
pub trait FlashIo {
    type Error;
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), Self::Error>;
    /// `e` from [`FlashIo::read`] means a flash word in the range could not be read (on the H7: a
    /// double ECC error, a word torn by a power loss). The scan skips such a word like a torn
    /// record instead of failing.
    fn unreadable(_e: &Self::Error) -> bool {
        false
    }
    /// Program whole flash words at a word-aligned address. Words must be erased.
    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), Self::Error>;
    /// Erase the whole sector that starts at `addr`.
    fn erase_sector(&mut self, addr: u32) -> Result<(), Self::Error>;
}

impl<T: FlashIo + ?Sized> FlashIo for &mut T {
    type Error = T::Error;
    fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), Self::Error> {
        (**self).read(addr, buf)
    }
    fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), Self::Error> {
        (**self).program(addr, data)
    }
    fn erase_sector(&mut self, addr: u32) -> Result<(), Self::Error> {
        (**self).erase_sector(addr)
    }
    fn unreadable(e: &Self::Error) -> bool {
        T::unreadable(e)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Io(E),
    /// The guard refused the operation (a bug: the store only works inside its sectors).
    Guard(Refusal),
    TooLarge,
}

fn padded(len: usize) -> u32 {
    (((HEADER + len) as u32 + FLASH_WORD - 1) / FLASH_WORD) * FLASH_WORD
}

#[derive(Clone, Copy)]
struct Found {
    addr: u32,
    kind: Kind,
    seq: u32,
    len: u16,
    version: u8,
}

/// What a scan of one sector found.
#[derive(Clone, Copy, Debug, Default)]
struct SectorScan {
    /// Highest committed generation in the sector.
    generation: Option<u32>,
    /// First free word after the last written one.
    end: u32,
    /// Highest sequence number.
    seq: u32,
    /// Any valid record at all.
    any: bool,
}

pub struct Store<F> {
    io: F,
    writer: Writer,
    /// Index into `STORE_SECTORS` of the live sector.
    live: usize,
    generation: u32,
    /// Address of the next free flash word in the live sector.
    end: u32,
    seq: u32,
}

impl<F: FlashIo> Store<F> {
    /// Scan both sectors and pick the live one. `writer` is who writes: the application or the
    /// boot stage.
    pub fn open(io: F, writer: Writer) -> Result<Self, Error<F::Error>> {
        let mut s = Store { io, writer, live: 1, generation: 0, end: STORE_SECTORS[1].start, seq: 0 };
        let a = s.scan(STORE_SECTORS[0])?;
        let b = s.scan(STORE_SECTORS[1])?;
        let (live, scan) = match (a.generation, b.generation) {
            (Some(x), Some(y)) => {
                if x > y {
                    (0, a)
                } else {
                    (1, b)
                }
            }
            (Some(_), None) => (0, a),
            (None, Some(_)) => (1, b),
            // No commit anywhere: a fresh unit, or a store written before generations existed
            // (records only in sector 7). Sector 7 is live as generation 0.
            (None, None) => (1, b),
        };
        s.live = live;
        s.generation = scan.generation.unwrap_or(0);
        s.end = scan.end;
        s.seq = a.seq.max(b.seq);
        Ok(s)
    }

    pub fn release(self) -> F {
        self.io
    }

    /// Bytes still free in the live sector.
    pub fn free(&self) -> u32 {
        STORE_SECTORS[self.live].end - self.end
    }

    /// Generation of the live sector (0 before the first compaction).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Index of the live sector (0 = sector 6, 1 = sector 7).
    pub fn live_sector(&self) -> usize {
        self.live
    }

    /// The header at `addr`; `None` if its flash word cannot be read.
    fn read_header(&mut self, addr: u32) -> Result<Option<[u8; HEADER]>, Error<F::Error>> {
        let mut h = [0u8; HEADER];
        match self.io.read(addr, &mut h) {
            Ok(()) => Ok(Some(h)),
            Err(e) if F::unreadable(&e) => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn check(&mut self, r: Range, addr: u32, h: &[u8; HEADER]) -> Result<Option<Found>, Error<F::Error>> {
        if u32::from_le_bytes([h[0], h[1], h[2], h[3]]) != MAGIC {
            return Ok(None);
        }
        let Some(kind) = Kind::from_u8(h[4]) else { return Ok(None) };
        let len = u16::from_le_bytes([h[6], h[7]]);
        if len as usize > MAX_PAYLOAD || addr + padded(len as usize) > r.end {
            return Ok(None);
        }
        let mut c = crc32::update(crc32::init(), &h[..12]);
        let mut buf = [0u8; 64];
        let mut at = 0usize;
        while at < len as usize {
            let n = (len as usize - at).min(buf.len());
            match self.io.read(addr + (HEADER + at) as u32, &mut buf[..n]) {
                Ok(()) => {}
                // An unreadable word in the payload: the record is lost, as a torn one.
                Err(e) if F::unreadable(&e) => return Ok(None),
                Err(e) => return Err(Error::Io(e)),
            }
            c = crc32::update(c, &buf[..n]);
            at += n;
        }
        if crc32::finish(c) != u32::from_le_bytes([h[12], h[13], h[14], h[15]]) {
            return Ok(None);
        }
        Ok(Some(Found { addr, kind, seq: u32::from_le_bytes([h[8], h[9], h[10], h[11]]), len, version: h[5] }))
    }

    /// Walk the valid records of a sector, calling `f` for each, and return what was found.
    fn walk(&mut self, r: Range, mut f: impl FnMut(&mut Self, Found) -> Result<(), Error<F::Error>>) -> Result<SectorScan, Error<F::Error>> {
        let mut out = SectorScan { end: r.start, ..Default::default() };
        let mut addr = r.start;
        while addr + FLASH_WORD <= r.end {
            let Some(h) = self.read_header(addr)? else {
                // Unreadable word (ECC): written, so not free; skip it like a torn word.
                addr += FLASH_WORD;
                out.end = addr;
                continue;
            };
            if h.iter().all(|&b| b == 0xFF) {
                // Erased word: the end, or the gap after a torn write. A record never starts
                // with an erased word, so step one word.
                addr += FLASH_WORD;
                continue;
            }
            match self.check(r, addr, &h)? {
                Some(found) => {
                    out.any = true;
                    out.seq = out.seq.max(found.seq);
                    if found.kind == Kind::Commit && found.len == 4 {
                        let mut g = [0u8; 4];
                        self.io.read(found.addr + HEADER as u32, &mut g).map_err(Error::Io)?;
                        let g = u32::from_le_bytes(g);
                        out.generation = Some(out.generation.map_or(g, |x| x.max(g)));
                    }
                    addr += padded(found.len as usize);
                    f(self, found)?;
                }
                None => addr += FLASH_WORD, // torn, foreign, partly erased or unreadable: skip it
            }
            out.end = addr;
        }
        Ok(out)
    }

    fn scan(&mut self, r: Range) -> Result<SectorScan, Error<F::Error>> {
        self.walk(r, |_, _| Ok(()))
    }

    fn newest(&mut self, kind: Kind) -> Result<Option<Found>, Error<F::Error>> {
        let r = STORE_SECTORS[self.live];
        let mut best: Option<Found> = None;
        self.walk(r, |_, f| {
            if f.kind == kind && best.is_none_or(|b| f.seq > b.seq) {
                best = Some(f);
            }
            Ok(())
        })?;
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

    /// Write one record at `at` in sector `r`; returns the address after it.
    fn write_at(&mut self, r: Range, at: u32, kind: Kind, version: u8, payload: &[u8]) -> Result<u32, Error<F::Error>> {
        let total = padded(payload.len());
        if at + total > r.end {
            return Err(Error::TooLarge);
        }
        self.seq = self.seq.wrapping_add(1);
        let mut h = [0u8; HEADER];
        h[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        h[4] = kind as u8;
        h[5] = version;
        h[6..8].copy_from_slice(&(payload.len() as u16).to_le_bytes());
        h[8..12].copy_from_slice(&self.seq.to_le_bytes());
        let c = crc32::finish(crc32::update(crc32::update(crc32::init(), &h[..12]), payload));
        h[12..16].copy_from_slice(&c.to_le_bytes());
        self.guard(Op::Program, at, total)?;
        let mut word = [0xFFu8; FLASH_WORD as usize];
        let mut pos = 0usize;
        let mut addr = at;
        while pos < total as usize {
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
        Ok(addr)
    }

    /// Store a new record of `kind`. Compacts first if it does not fit in the live sector.
    pub fn put(&mut self, kind: Kind, version: u8, payload: &[u8]) -> Result<(), Error<F::Error>> {
        if payload.len() > MAX_PAYLOAD || kind == Kind::Commit {
            return Err(Error::TooLarge);
        }
        if self.end + padded(payload.len()) > STORE_SECTORS[self.live].end {
            self.compact_keep(|_| true)?;
        }
        self.end = self.write_at(STORE_SECTORS[self.live], self.end, kind, version, payload)?;
        Ok(())
    }

    /// Remove every record of the kinds for which `drop` is true (for example the secret at a
    /// factory reset or before a switch to stock, 0024 R5), by a compaction that leaves them out.
    /// The old sector still holds them until the next compaction erases it; a caller that must
    /// destroy a kind for good calls this twice.
    pub fn compact_without(&mut self, drop: impl Fn(Kind) -> bool) -> Result<(), Error<F::Error>> {
        self.compact_keep(|k| !drop(k))
    }

    /// Force a compaction now (management or test).
    pub fn compact(&mut self) -> Result<(), Error<F::Error>> {
        self.compact_keep(|_| true)
    }

    fn compact_keep(&mut self, keep: impl Fn(Kind) -> bool) -> Result<(), Error<F::Error>> {
        // (0) Copy the newest record of every kind kept to RAM.
        let mut saved: [(u8, u16, [u8; MAX_PAYLOAD]); 5] = [(0, 0, [0; MAX_PAYLOAD]); 5];
        let mut have = [false; 5];
        for (i, k) in Kind::DATA.iter().enumerate() {
            if !keep(*k) {
                continue;
            }
            let mut b = [0u8; MAX_PAYLOAD];
            if let Some((v, n)) = self.get(*k, &mut b)? {
                saved[i] = (v, n as u16, b);
                have[i] = true;
            }
        }
        // (1) Erase the other sector. The live sector is not touched.
        let target = 1 - self.live;
        let r = STORE_SECTORS[target];
        self.guard(Op::Erase, r.start, r.len())?;
        self.io.erase_sector(r.start).map_err(Error::Io)?;
        // (2) Copy.
        let mut at = r.start;
        for (i, k) in Kind::DATA.iter().enumerate() {
            if have[i] {
                let (v, n, b) = saved[i];
                at = self.write_at(r, at, *k, v, &b[..n as usize])?;
            }
        }
        // (3) Commit with the next generation; (4) switch.
        let g = self.generation.wrapping_add(1);
        at = self.write_at(r, at, Kind::Commit, 1, &g.to_le_bytes())?;
        self.live = target;
        self.generation = g;
        self.end = at;
        Ok(())
    }

    /// Increment the boot counter and return the new value (B10.2 item 5). Called once per
    /// start by the application, after the safe state and the supply check.
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
    use g7250_layout::{SECTOR_SIZE, STORE, Slot};

    /// Sectors 6 and 7 in RAM, with the rules of H7 flash: program only erased, aligned, whole
    /// words, each word once; erase whole sectors. Power can be cut after `budget` flash words,
    /// or in the middle of an erase (`erase_cut`: the sector is left half erased). With
    /// `ecc_on_cut` the word being programmed when the power goes (or the word at the erase
    /// boundary) is left unreadable, as a double ECC error on the H7; `bad` marks such words.
    #[derive(Clone)]
    struct Sim {
        mem: std::vec::Vec<u8>,
        written: std::vec::Vec<bool>,
        bad: std::vec::Vec<bool>,
        ecc_on_cut: bool,
        budget: Option<usize>,
        erase_cut: bool,
        erases: usize,
        programs: usize,
    }

    #[derive(Debug, PartialEq)]
    enum SimErr {
        PowerLost,
        Ecc,
        Rule(&'static str),
    }

    impl Sim {
        fn new() -> Self {
            let n = STORE.len() as usize;
            Sim { mem: vec![0xFF; n], written: vec![false; n / 32], bad: vec![false; n / 32], ecc_on_cut: false, budget: None, erase_cut: false, erases: 0, programs: 0 }
        }
        /// Power comes back: no more failures.
        fn power_on(mut self) -> Self {
            self.budget = None;
            self.erase_cut = false;
            self
        }
    }

    impl FlashIo for Sim {
        type Error = SimErr;
        fn read(&mut self, addr: u32, buf: &mut [u8]) -> Result<(), SimErr> {
            let o = (addr - STORE.start) as usize;
            if !buf.is_empty() && self.bad[o / 32..=(o + buf.len() - 1) / 32].iter().any(|&b| b) {
                return Err(SimErr::Ecc);
            }
            buf.copy_from_slice(&self.mem[o..o + buf.len()]);
            Ok(())
        }
        fn unreadable(e: &SimErr) -> bool {
            *e == SimErr::Ecc
        }
        fn program(&mut self, addr: u32, data: &[u8]) -> Result<(), SimErr> {
            if addr % FLASH_WORD != 0 || data.len() % FLASH_WORD as usize != 0 || !STORE.contains_span(addr, data.len() as u32) {
                return Err(SimErr::Rule("alignment or range"));
            }
            for (i, w) in data.chunks(FLASH_WORD as usize).enumerate() {
                if let Some(b) = self.budget.as_mut() {
                    if *b == 0 {
                        if self.ecc_on_cut {
                            let wi = ((addr - STORE.start) / FLASH_WORD) as usize + i;
                            self.written[wi] = true;
                            self.bad[wi] = true;
                            // Half the bits made it: still neither erased nor the data.
                            self.mem[wi * 32..wi * 32 + 16].copy_from_slice(&w[..16]);
                        }
                        return Err(SimErr::PowerLost);
                    }
                    *b -= 1;
                }
                let wi = ((addr - STORE.start) / FLASH_WORD) as usize + i;
                if self.written[wi] {
                    return Err(SimErr::Rule("word programmed twice"));
                }
                self.written[wi] = true;
                self.mem[wi * 32..wi * 32 + 32].copy_from_slice(w);
                self.programs += 1;
            }
            Ok(())
        }
        fn erase_sector(&mut self, addr: u32) -> Result<(), SimErr> {
            assert!(addr == STORE_SECTORS[0].start || addr == STORE_SECTORS[1].start);
            let o = (addr - STORE.start) as usize;
            let n = SECTOR_SIZE as usize;
            if self.erase_cut {
                // Half erased: the first half is 0xFF, the rest keeps its old content (and may
                // still hold old commits).
                self.mem[o..o + n / 2].fill(0xFF);
                for w in &mut self.written[o / 32..(o + n / 2) / 32] {
                    *w = false;
                }
                self.bad[o / 32..(o + n / 2) / 32].fill(false);
                if self.ecc_on_cut {
                    // The word at the boundary was being erased: unreadable.
                    self.bad[(o + n / 2) / 32] = true;
                    self.written[(o + n / 2) / 32] = true;
                }
                return Err(SimErr::PowerLost);
            }
            self.mem[o..o + n].fill(0xFF);
            for w in &mut self.written[o / 32..(o + n) / 32] {
                *w = false;
            }
            self.bad[o / 32..(o + n) / 32].fill(false);
            self.erases += 1;
            Ok(())
        }
    }

    const APP: Writer = Writer::Application { running: Slot::A };

    fn get(s: &mut Store<Sim>, k: Kind) -> Option<std::vec::Vec<u8>> {
        let mut b = [0u8; MAX_PAYLOAD];
        s.get(k, &mut b).unwrap().map(|(_, n)| b[..n].to_vec())
    }

    #[test]
    fn empty_store_starts_in_sector_7_generation_0() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (1, 0));
        assert_eq!(get(&mut s, Kind::Config), None);
        assert_eq!(s.free(), SECTOR_SIZE);
    }

    #[test]
    fn newest_wins_and_survives_reopen() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"first").unwrap();
        s.put(Kind::Calibration, 1, &[7u8; 300]).unwrap();
        s.put(Kind::Config, 1, b"second config").unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"second config");
        assert_eq!(get(&mut s, Kind::Calibration).unwrap(), vec![7u8; 300]);
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
        for ecc in [false, true] {
            let mut sim = Sim::new();
            sim.ecc_on_cut = ecc;
            let mut s = Store::open(sim, APP).unwrap();
            s.put(Kind::Config, 1, b"good").unwrap();
            let mut io = s.release();
            io.budget = Some(1);
            let mut s = Store::open(io, APP).unwrap();
            assert_eq!(s.put(Kind::Config, 1, &[9u8; 100]), Err(Error::Io(SimErr::PowerLost)));
            let mut s = Store::open(s.release().power_on(), APP).unwrap();
            assert_eq!(get(&mut s, Kind::Config).unwrap(), b"good");
            s.put(Kind::Config, 1, b"after").unwrap();
            let mut s = Store::open(s.release(), APP).unwrap();
            assert_eq!(get(&mut s, Kind::Config).unwrap(), b"after");
        }
    }

    #[test]
    fn unreadable_words_anywhere_are_skipped_r0() {
        // A word with a double ECC error in a record's header, in its payload, and in the commit
        // of the other sector: the scan goes on, the older value of that kind wins, the store
        // keeps working and never programs the bad word.
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"old").unwrap();
        s.compact().unwrap(); // sector 6, generation 1
        s.put(Kind::Calibration, 1, &[7u8; 100]).unwrap();
        let cal_at = s.end - padded(100);
        s.put(Kind::Config, 1, b"new").unwrap();
        let cfg_at = s.end - padded(3);
        s.put(Kind::Calibration, 1, &[8u8; 100]).unwrap();
        let cal2_at = s.end - padded(100);
        let mut io = s.release();
        let w = |a: u32| ((a - STORE.start) / FLASH_WORD) as usize;
        io.bad[w(cfg_at)] = true; // header of the newest config
        io.bad[w(cal2_at) + 2] = true; // payload of the newest calibration
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (0, 1));
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"old");
        assert_eq!(get(&mut s, Kind::Calibration).unwrap(), vec![7u8; 100]);
        assert!(cal_at < cfg_at);
        s.put(Kind::Config, 1, b"again").unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"again");
        // A compaction carries the readable values over; the bad words go with the erase.
        s.compact().unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"again");
        assert_eq!(get(&mut s, Kind::Calibration).unwrap(), vec![7u8; 100]);
        // An unreadable commit word in the newer sector: the older generation stays live.
        let gen_before = s.generation();
        let live = s.live_sector();
        s.compact().unwrap();
        let commit_word = s.end - padded(4);
        let mut io = s.release();
        io.bad[w(commit_word)] = true;
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (live, gen_before));
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"again");
    }

    #[test]
    fn compactions_alternate_sectors_and_keep_everything() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Calibration, 2, &[5u8; 900]).unwrap();
        s.put(Kind::Secret, 1, &[1u8; 32]).unwrap();
        for i in 0..10_000u32 {
            s.next_boot_counter().unwrap();
            if i % 100 == 0 {
                s.put(Kind::Config, 1, &i.to_le_bytes()).unwrap();
            }
        }
        assert!(s.generation() >= 2, "at least two compactions");
        let io = s.release();
        assert!(io.erases >= 2);
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!(get(&mut s, Kind::BootCounter).unwrap(), 10_000u32.to_le_bytes());
        assert_eq!(get(&mut s, Kind::Calibration).unwrap(), vec![5u8; 900]);
        assert_eq!(get(&mut s, Kind::Config).unwrap(), 9900u32.to_le_bytes());
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), vec![1u8; 32]);
    }

    /// A store just before a compaction: the live sector full, values set.
    fn full_store() -> (Sim, u32) {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"config v1").unwrap();
        s.put(Kind::Calibration, 1, &[3u8; 700]).unwrap();
        s.put(Kind::Secret, 1, &[9u8; 32]).unwrap();
        let mut n = 0;
        while s.free() >= 32 {
            n = s.next_boot_counter().unwrap();
        }
        (s.release(), n)
    }

    fn check_all(io: Sim, counter_at_least: u32) -> Store<Sim> {
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"config v1");
        assert_eq!(get(&mut s, Kind::Calibration).unwrap(), vec![3u8; 700]);
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), vec![9u8; 32]);
        let c = u32::from_le_bytes(get(&mut s, Kind::BootCounter).unwrap().try_into().unwrap());
        assert!(c >= counter_at_least, "boot counter went back: {c} < {counter_at_least}");
        s
    }

    #[test]
    fn power_loss_at_every_word_of_a_compaction_loses_nothing() {
        for ecc in [false, true] {
            let (mut base, n) = full_store();
            base.ecc_on_cut = ecc;
            // How many words does the compaction plus the triggering record take?
            let mut probe = Store::open(base.clone(), APP).unwrap();
            let before = probe.io.programs;
            probe.next_boot_counter().unwrap();
            let words = probe.io.programs - before;
            assert!(words > 5);
            for cut in 0..=words {
                let mut io = base.clone();
                io.budget = Some(cut);
                let mut s = Store::open(io, APP).unwrap();
                let r = s.next_boot_counter();
                let io = s.release().power_on();
                // Every value is there after the restart, whatever the cut.
                let mut s = check_all(io, n);
                // And the store goes on working.
                let m = s.next_boot_counter().unwrap();
                assert!(m > n);
                if cut == words {
                    assert_eq!(r, Ok(n + 1));
                }
            }
        }
    }

    #[test]
    fn power_loss_during_the_erase_loses_nothing() {
        let (base, n) = full_store();
        // First compaction erases sector 6 (fresh), the second one sector 7 with old data in it.
        for (round, ecc) in [(0, false), (1, false), (0, true), (1, true)] {
            let mut io = base.clone();
            io.ecc_on_cut = ecc;
            if round == 1 {
                let mut s = Store::open(io, APP).unwrap();
                while s.generation() < 1 {
                    s.next_boot_counter().unwrap();
                }
                while s.free() >= 32 {
                    s.next_boot_counter().unwrap();
                }
                io = s.release();
            }
            let n0 = {
                let mut s = Store::open(io.clone(), APP).unwrap();
                u32::from_le_bytes(get(&mut s, Kind::BootCounter).unwrap().try_into().unwrap())
            };
            io.erase_cut = true;
            let mut s = Store::open(io, APP).unwrap();
            assert_eq!(s.next_boot_counter(), Err(Error::Io(SimErr::PowerLost)));
            let s = check_all(s.release().power_on(), n0.max(n));
            let mut s = Store::open(s.release(), APP).unwrap();
            s.next_boot_counter().unwrap();
        }
    }

    #[test]
    fn older_commit_left_in_a_half_erased_sector_does_not_win() {
        // Build generation 2 in sector 7 (6 -> gen 1, 7 -> gen 2), then cut the erase of sector 6
        // for generation 3: sector 6's gen-1 commit may survive in its second half.
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"keep me").unwrap();
        s.compact().unwrap();
        s.put(Kind::Config, 1, b"newer").unwrap();
        s.compact().unwrap();
        assert_eq!((s.live_sector(), s.generation()), (1, 2));
        let mut io = s.release();
        io.erase_cut = true;
        let mut s = Store::open(io, APP).unwrap();
        assert!(s.compact().is_err());
        let mut s = Store::open(s.release().power_on(), APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (1, 2));
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"newer");
    }

    #[test]
    fn store_from_before_generations_is_read_and_migrated() {
        // Step-2 images wrote records to sector 7 without commits.
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Secret, 1, &[4u8; 32]).unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (1, 0));
        s.compact().unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (0, 1));
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), vec![4u8; 32]);
    }

    #[test]
    fn compact_without_drops_a_kind() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Secret, 1, &[1u8; 32]).unwrap();
        s.put(Kind::Config, 1, b"cfg").unwrap();
        s.compact_without(|k| k == Kind::Secret).unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!(get(&mut s, Kind::Secret), None);
        assert!(get(&mut s, Kind::Config).is_some());
    }

    #[test]
    fn too_large_commit_kind_and_guard() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        assert_eq!(s.put(Kind::Config, 1, &[0u8; MAX_PAYLOAD + 1]), Err(Error::TooLarge));
        assert_eq!(s.put(Kind::Commit, 1, &[0u8; 4]), Err(Error::TooLarge));
        let mut s = Store::open(s.release(), Writer::BootStage).unwrap();
        s.put(Kind::BootRecord, 1, &[0u8; 16]).unwrap();
    }
}
