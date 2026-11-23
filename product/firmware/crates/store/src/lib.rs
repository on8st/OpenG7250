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
    /// The slot whose staging has not ended with a good `UPDATE_VERIFY` (A1.8; 0067).
    Staging = 7,
    /// A factory reset was begun (A1.6a): the next start finishes it (0069).
    Wipe = 8,
    /// First record of a compaction: the generation number and the address of its commit word
    /// (payload: number u32, address u32), the number's second place, programmed before the
    /// commit (0072 point 2).
    Begin = 9,
    /// The C-3 limits of a generation ([`Limits`]), written by every compaction and never
    /// appended (0072 point 1).
    Limits = 10,
    /// A C-3 state record (A1.6 "Keeping the judgment"; payload: number u32, then [`Limits`]),
    /// appended to the live sector before a newer uncommitted generation is erased.
    LimitsState = 11,
}

impl Kind {
    /// The kinds that carry data (copied by a compaction).
    pub const DATA: [Kind; 7] = [Kind::BootRecord, Kind::BootCounter, Kind::Config, Kind::Calibration, Kind::Secret, Kind::Staging, Kind::Wipe];
    fn from_u8(v: u8) -> Option<Kind> {
        Self::DATA.iter().copied().chain([Kind::Commit, Kind::Begin, Kind::Limits, Kind::LimitsState]).find(|k| *k as u8 == v)
    }
}

/// Bands of the C-3 power limit (Table M-3 tag 8; at most 16 `tx_band` entries, B4.8).
pub const LIMIT_BANDS: usize = 16;
/// Bytes of an encoded [`Limits`].
pub const LIMITS_LEN: usize = 1 + 4 + 2 * LIMIT_BANDS + 2 + 2;

/// The C-3 items (A1.6 C-3) as the store keeps them, or "lost" (0072 point 5). `None` is "not
/// tightened": the built-in value. Every item is a maximum, so the stricter of two values is the
/// smaller, and "not tightened" (stored as all ones) is looser than any value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub lost: bool,
    pub max_tx_time_ms: Option<u32>,
    pub band: [Option<u16>; LIMIT_BANDS],
    pub swr_s2_x100: Option<u16>,
    pub pa_temp_t3: Option<u16>,
}

impl Limits {
    /// The factory state: nothing tightened.
    pub const BUILTIN: Limits = Limits { lost: false, max_tx_time_ms: None, band: [None; LIMIT_BANDS], swr_s2_x100: None, pa_temp_t3: None };
    pub const LOST: Limits = Limits { lost: true, ..Limits::BUILTIN };

    pub fn encode(&self) -> [u8; LIMITS_LEN] {
        let mut b = [0xFFu8; LIMITS_LEN];
        b[0] = self.lost as u8;
        if let Some(t) = self.max_tx_time_ms {
            b[1..5].copy_from_slice(&t.to_le_bytes());
        }
        for (i, l) in self.band.iter().enumerate() {
            if let Some(l) = l {
                b[5 + 2 * i..7 + 2 * i].copy_from_slice(&l.to_le_bytes());
            }
        }
        if let Some(v) = self.swr_s2_x100 {
            b[37..39].copy_from_slice(&v.to_le_bytes());
        }
        if let Some(v) = self.pa_temp_t3 {
            b[39..41].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    /// `None` for a payload of another length or flag: such a record is not readable.
    pub fn decode(b: &[u8]) -> Option<Limits> {
        if b.len() != LIMITS_LEN || b[0] > 1 {
            return None;
        }
        let u16_at = |i: usize| Some(u16::from_le_bytes([b[i], b[i + 1]])).filter(|&v| v != u16::MAX);
        let t = u32::from_le_bytes([b[1], b[2], b[3], b[4]]);
        let l = Limits {
            lost: b[0] == 1,
            max_tx_time_ms: Some(t).filter(|&t| t != u32::MAX),
            band: core::array::from_fn(|i| u16_at(5 + 2 * i)),
            swr_s2_x100: u16_at(37),
            pa_temp_t3: u16_at(39),
        };
        Some(if l.lost { Limits::LOST } else { l })
    }

    /// Each item the stricter of the two; lost if either is (0072 point 3).
    pub fn stricter(&self, o: &Limits) -> Limits {
        fn min<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(x), Some(y)) => Some(x.min(y)),
                (x, None) => x,
                (None, y) => y,
            }
        }
        if self.lost || o.lost {
            return Limits::LOST;
        }
        Limits {
            lost: false,
            max_tx_time_ms: min(self.max_tx_time_ms, o.max_tx_time_ms),
            band: core::array::from_fn(|i| min(self.band[i], o.band[i])),
            swr_s2_x100: min(self.swr_s2_x100, o.swr_s2_x100),
            pa_temp_t3: min(self.pa_temp_t3, o.pa_temp_t3),
        }
    }
}

/// How C-3 was judged at open (A1.6 "C-3 at a start"), for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Judged {
    /// (1) A failed generation counted: each item the stricter of it and the live value.
    FailedGeneration,
    /// (2) A C-3 state record of the live sector.
    StateRecord,
    /// (3) The C-3 record of the committed live generation.
    Committed,
    /// (4) No committed generation live: the built-in values.
    Factory,
}

/// Room kept in the live sector for C-3 state records (A1.6 "Keeping the judgment": "The store
/// MUST always have room"): two, so that a second one fits after a first left the sector full.
const STATE_ROOM: u32 = 2 * (((HEADER + 4 + LIMITS_LEN) as u32 + FLASH_WORD - 1) / FLASH_WORD) * FLASH_WORD;

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
    /// A C-3 state record did not read back as written: the erase it was to protect is not done.
    ReadBack,
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

/// What one sector says about its generation (0072 point 2; A1.6 "C-3 at a start").
#[derive(Clone, Copy, Debug, Default)]
struct GenInfo {
    /// The readable number of its begin record, and the address of its commit word.
    begin: Option<(u32, u32)>,
    /// A valid commit record.
    committed: bool,
    /// The begin record's commit word is neither erased nor a valid commit of its number: a failed
    /// generation (0065 point 5).
    failed: bool,
    /// The newest readable C-3 record.
    limits: Option<Limits>,
    /// The highest readable number: begin, commit and state records.
    max_number: u32,
    /// The highest number of a readable C-3 state record, and the newest such record's limits.
    state_max: Option<u32>,
    state_newest: Option<Limits>,
}

/// What the sector that is not live holds that the start check must erase ([`Store::stale`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stale {
    Nothing,
    /// A record of the kind asked for, other than the live one.
    Other,
    /// Words that are not erased and in no complete, checked record.
    Unrecognised,
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
    /// Unreadable flash words (double ECC error) met by the scans since open.
    unreadable: u32,
    /// The count after the scans of open: logged once per start (0065 point 5).
    unreadable_at_open: u32,
    fell_back: bool,
    /// C-3 as judged at open or set since (A1.6 "C-3 at a start"; 0072).
    limits: Limits,
    judged: Judged,
    /// The highest generation number readable in either sector at open, or written since.
    number: u32,
}

impl<F: FlashIo> Store<F> {
    /// Scan both sectors and pick the live one. `writer` is who writes: the application or the
    /// boot stage.
    pub fn open(io: F, writer: Writer) -> Result<Self, Error<F::Error>> {
        let mut s = Store { io, writer, live: 1, generation: 0, end: STORE_SECTORS[1].start, seq: 0, unreadable: 0, unreadable_at_open: 0, fell_back: false, limits: Limits::BUILTIN, judged: Judged::Factory, number: 0 };
        let a = s.scan(STORE_SECTORS[0])?;
        let unreadable_a = s.unreadable;
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
        let (other, bad) = if live == 0 { (b, s.unreadable - unreadable_a) } else { (a, unreadable_a) };
        s.fell_back = other.any && other.generation.is_none() && other.seq > scan.seq && bad > 0;
        s.live = live;
        s.generation = scan.generation.unwrap_or(0);
        s.end = scan.end;
        s.seq = a.seq.max(b.seq);
        s.unreadable_at_open = s.unreadable;
        s.judge()?;
        Ok(s)
    }

    /// C-3 at a start (A1.6 "C-3 at a start"; 0072 point 3). The live value: the newest readable
    /// state record of the live sector; else the C-3 record of the committed live generation, lost
    /// if it cannot be read; else (no committed generation live) the built-in values. A failed
    /// generation in either sector counts if its number is readable and higher than the live
    /// generation's (0 if it is not committed) and than every state record's: each item then the
    /// stricter of it and the live value, lost if its C-3 record cannot be read.
    fn judge(&mut self) -> Result<(), Error<F::Error>> {
        let unreadable = self.unreadable;
        let l = self.gen_info(STORE_SECTORS[self.live])?;
        let o = self.gen_info(STORE_SECTORS[1 - self.live])?;
        self.unreadable = unreadable;
        self.number = l.max_number.max(o.max_number).max(self.generation);
        let threshold = self.generation.max(l.state_max.unwrap_or(0));
        let committed = self.generation > 0;
        let (mut v, mut j) = match (l.state_newest, committed) {
            (Some(st), _) => (st, Judged::StateRecord),
            (None, true) => (l.limits.unwrap_or(Limits::LOST), Judged::Committed),
            (None, false) => (Limits::BUILTIN, Judged::Factory),
        };
        for g in [Some(o), (!committed).then_some(l)].into_iter().flatten() {
            if let (true, Some((n, _))) = (g.failed, g.begin) {
                if n > threshold {
                    v = v.stricter(&g.limits.unwrap_or(Limits::LOST));
                    j = Judged::FailedGeneration;
                }
            }
        }
        self.limits = v;
        self.judged = j;
        Ok(())
    }

    /// C-3 as judged at open or set since; [`Limits::lost`] closes TX (0072 point 5).
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Which case of "C-3 at a start" gave [`Self::limits`] at open.
    pub fn judged(&self) -> Judged {
        self.judged
    }

    /// The payload of a checked record, `None` if a word of it cannot be read.
    fn payload<const N: usize>(&mut self, f: &Found) -> Result<Option<[u8; N]>, Error<F::Error>> {
        let mut b = [0u8; N];
        if f.len as usize != N {
            return Ok(None);
        }
        match self.io.read(f.addr + HEADER as u32, &mut b) {
            Ok(()) => Ok(Some(b)),
            Err(e) if F::unreadable(&e) => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    fn gen_info(&mut self, r: Range) -> Result<GenInfo, Error<F::Error>> {
        let mut g = GenInfo::default();
        let mut state_seq = None;
        let mut limits_seq = None;
        let mut begin_seq = None;
        self.walk(r, |s, f| {
            match f.kind {
                Kind::Begin => {
                    if let Some(p) = s.payload::<8>(&f)? {
                        let n = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                        g.max_number = g.max_number.max(n);
                        if begin_seq.is_none_or(|q| f.seq > q) {
                            begin_seq = Some(f.seq);
                            g.begin = Some((n, u32::from_le_bytes([p[4], p[5], p[6], p[7]])));
                        }
                    }
                }
                Kind::Commit => {
                    if let Some(p) = s.payload::<4>(&f)? {
                        g.committed = true;
                        g.max_number = g.max_number.max(u32::from_le_bytes(p));
                    }
                }
                Kind::Limits => {
                    if let Some(l) = s.payload::<LIMITS_LEN>(&f)?.and_then(|p| Limits::decode(&p)) {
                        if limits_seq.is_none_or(|q| f.seq > q) {
                            limits_seq = Some(f.seq);
                            g.limits = Some(l);
                        }
                    }
                }
                Kind::LimitsState => {
                    if let Some(p) = s.payload::<{ 4 + LIMITS_LEN }>(&f)? {
                        if let Some(l) = Limits::decode(&p[4..]) {
                            let n = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                            g.max_number = g.max_number.max(n);
                            g.state_max = Some(g.state_max.map_or(n, |m| m.max(n)));
                            if state_seq.is_none_or(|q| f.seq > q) {
                                state_seq = Some(f.seq);
                                g.state_newest = Some(l);
                            }
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })?;
        // The commit word the begin record names: erased (a cut compaction), the commit of that
        // number, or anything else (failed).
        if let Some((n, at)) = g.begin {
            g.failed = if at % FLASH_WORD != 0 || at < r.start || at + FLASH_WORD > r.end {
                true
            } else {
                match self.read_header(at)? {
                    None => true,
                    Some(h) if h.iter().all(|&b| b == 0xFF) => false,
                    Some(h) => match self.check(r, at, &h)? {
                        Some(f) if f.kind == Kind::Commit => self.payload::<4>(&f)? != Some(n.to_le_bytes()),
                        _ => true,
                    },
                }
            };
        }
        Ok(g)
    }

    /// A C-3 state record with `number` and the limits in force, in the live sector, read back
    /// (A1.6 "Keeping the judgment"). Uses the room kept for it ([`STATE_ROOM`]); with none left
    /// (more than two in a row, each after a fault) it is not written.
    fn write_state(&mut self, number: u32) -> Result<(), Error<F::Error>> {
        let r = STORE_SECTORS[self.live];
        let mut p = [0u8; 4 + LIMITS_LEN];
        p[..4].copy_from_slice(&number.to_le_bytes());
        p[4..].copy_from_slice(&self.limits.encode());
        if self.room() < padded(p.len()) {
            return Ok(());
        }
        self.end = self.write_at(r, self.end, Kind::LimitsState, 1, &p)?;
        let unreadable = self.unreadable;
        let back = self.gen_info(r)?;
        self.unreadable = unreadable;
        if back.state_newest != Some(self.limits) || back.state_max.is_none_or(|m| m < number) {
            return Err(Error::ReadBack);
        }
        Ok(())
    }

    /// Flash words with a double ECC error that the scan at open met in both sectors (each
    /// treated as a torn record and never programmed again): the start-up logs this once
    /// (0065 point 5; T-A1-87). 0 on a store with none.
    pub fn unreadable_at_open(&self) -> u32 {
        self.unreadable_at_open
    }

    pub fn release(self) -> F {
        self.io
    }

    /// Bytes still free in the live sector for records other than the factory-reset mark and the
    /// C-3 state records: one flash word is always kept for that mark ([`Self::wipe`]), and room
    /// for two state records ([`STATE_ROOM`]).
    pub fn free(&self) -> u32 {
        let room = self.room();
        let kept = FLASH_WORD + STATE_ROOM;
        if room > kept { room - kept } else { 0 }
    }

    /// Bytes not yet written in the live sector, the word kept for the mark included.
    fn room(&self) -> u32 {
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
                // An unreadable word in the payload: the record is lost, as a torn one. (The scan
                // reaches that word as a header next and counts it there, once.)
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
                self.unreadable += 1;
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

    /// Store a new record of `kind`. Compacts first if it does not fit in the live sector, with
    /// one word left there for the factory-reset mark (a mark itself may take that word).
    pub fn put(&mut self, kind: Kind, version: u8, payload: &[u8]) -> Result<(), Error<F::Error>> {
        if payload.len() > MAX_PAYLOAD || matches!(kind, Kind::Commit | Kind::Begin | Kind::Limits | Kind::LimitsState) {
            return Err(Error::TooLarge);
        }
        if padded(payload.len()) > if kind == Kind::Wipe { self.room() } else { self.free() } {
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

    /// Make sure that `bytes` of records can be appended without an erase: compacts now if the
    /// live sector has less room (0069: one erase per request, at a request of the caller's
    /// choice).
    pub fn reserve(&mut self, bytes: u32) -> Result<(), Error<F::Error>> {
        if self.free() < bytes { self.compact() } else { Ok(()) }
    }

    /// Force a compaction now (management or test).
    pub fn compact(&mut self) -> Result<(), Error<F::Error>> {
        self.compact_keep(|_| true)
    }

    /// Write a record by a compaction into the other sector, which never holds an older record
    /// of its kind then; the sector with the old one is no longer live (the secret: A1.10 "The
    /// old secret in flash"; 0069).
    pub fn replace(&mut self, kind: Kind, version: u8, payload: &[u8]) -> Result<(), Error<F::Error>> {
        if payload.len() > MAX_PAYLOAD || !Kind::DATA.contains(&kind) {
            return Err(Error::TooLarge);
        }
        self.compact_with(|_| true, Some((kind, version, payload)))
    }

    /// A factory reset (A1.6a): remove the kinds for which `drop` is true from both sectors and
    /// write `kind` new, as `remake` makes it from its live payload: two compactions, one erase
    /// each. The first does both, so that a cut leaves the old items or the new ones. A mark in the
    /// live sector, written first (no erase) and left out of the first compaction, lets the next
    /// start finish what a failed erase or a power loss left undone (A1.6a "Atomicity": the reset
    /// has started; T-A1-39: finished at the next start). [`Self::put`] always leaves a word for
    /// the mark. A live sector with no word left (written by an earlier firmware) gets the mark by
    /// a compaction that keeps everything; the drop and the remake are then the second erase, and
    /// the sector left with the dropped kinds is erased by the start check of the next start
    /// ([`Self::stale`]), as 0069 allows no third erase. A cut in that first compaction leaves
    /// the old items and no mark.
    pub fn wipe(&mut self, drop: impl Fn(Kind) -> bool, kind: Kind, remake: impl Fn(Option<&[u8]>, &mut [u8]) -> Option<usize>) -> Result<(), Error<F::Error>> {
        if self.room() < FLASH_WORD {
            self.compact_with(|_| true, Some((Kind::Wipe, 1, &[1])))?;
            return self.finish_wipe(drop, kind, remake);
        }
        self.put(Kind::Wipe, 1, &[1])?;
        self.finish_wipe(drop, kind, remake)?;
        self.compact()
    }

    /// A wipe was begun and its first compaction did not end.
    pub fn wipe_pending(&mut self) -> Result<bool, Error<F::Error>> {
        Ok(self.newest(Kind::Wipe)?.is_some())
    }

    /// The first compaction of a wipe. The other sector still holds what it dropped; the start
    /// check erases it ([`Self::stale`], [`Self::erase_stale`]).
    pub fn finish_wipe(&mut self, drop: impl Fn(Kind) -> bool, kind: Kind, remake: impl Fn(Option<&[u8]>, &mut [u8]) -> Option<usize>) -> Result<(), Error<F::Error>> {
        let (mut old, mut new) = ([0u8; MAX_PAYLOAD], [0u8; MAX_PAYLOAD]);
        let n = self.get(kind, &mut old)?.map(|(_, n)| n);
        let len = remake(n.map(|n| &old[..n]), &mut new);
        self.compact_with(|k| k != Kind::Wipe && k != kind && !drop(k), len.map(|l| (kind, 1, &new[..l])))
    }

    /// The live sector holds a complete generation (A1.10; 0069 addendum): one with a readable
    /// commit word, or the first generation (generation 0, which has none) with a checked record
    /// of this closed list: secret, configuration, calibration. What a start writes before its
    /// start check (the boot counter, the boot record) does not count, nor do the staging and the
    /// wipe mark: with the commit of the only generation in sector 6 unreadable, an erased sector
    /// 7 is live as generation 0 and would soon hold those.
    fn live_complete(&mut self) -> Result<bool, Error<F::Error>> {
        if self.generation > 0 {
            return Ok(true);
        }
        let mut counts = false;
        self.walk(STORE_SECTORS[self.live], |_, f| {
            counts |= matches!(f.kind, Kind::Secret | Kind::Config | Kind::Calibration);
            Ok(())
        })?;
        Ok(counts)
    }

    /// The start check (A1.10 "The old secret in flash"; 0069 and its addendum): what the sector
    /// that is not live holds that must go, judged over every word of it. A record of `kind` other
    /// than `live` (any, if the live sector has none) comes first; then any word, not erased, that
    /// is in no complete, checked record (torn, unreadable, foreign: some bytes of a secret may be
    /// there). Never without a complete generation in the live sector ([`Self::live_complete`]):
    /// the other sector may then hold the only copy of the store.
    pub fn stale(&mut self, kind: Kind, live: Option<&[u8]>) -> Result<Stale, Error<F::Error>> {
        if !self.live_complete()? {
            return Ok(Stale::Nothing);
        }
        let r = STORE_SECTORS[1 - self.live];
        let mut out = Stale::Nothing;
        let mut addr = r.start;
        while addr + FLASH_WORD <= r.end {
            let mut w = [0u8; FLASH_WORD as usize];
            match self.io.read(addr, &mut w) {
                Ok(()) if w.iter().all(|&b| b == 0xFF) => {}
                Ok(()) => {
                    let mut h = [0u8; HEADER];
                    h.copy_from_slice(&w[..HEADER]);
                    if let Some(f) = self.check(r, addr, &h)? {
                        if f.kind == kind {
                            let mut b = [0u8; 64];
                            let n = f.len as usize;
                            if n > b.len() || self.io.read(addr + HEADER as u32, &mut b[..n]).is_err() || live != Some(&b[..n]) {
                                return Ok(Stale::Other);
                            }
                        }
                        addr += padded(f.len as usize);
                        continue;
                    }
                    out = Stale::Unrecognised;
                }
                Err(e) if F::unreadable(&e) => out = Stale::Unrecognised,
                Err(e) => return Err(Error::Io(e)),
            }
            addr += FLASH_WORD;
        }
        Ok(out)
    }

    /// Write new C-3 limits, and `new` if given, by a compaction into a new generation, never
    /// appended (A1.6 C-3; 0072 point 1): the one erase of the request.
    pub fn write_limits(&mut self, new: Option<(Kind, u8, &[u8])>, limits: Limits) -> Result<(), Error<F::Error>> {
        if new.is_some_and(|n| n.2.len() > MAX_PAYLOAD || !Kind::DATA.contains(&n.0)) {
            return Err(Error::TooLarge);
        }
        self.compact_full(|_| true, new, limits)
    }

    /// Erase the sector that is not live. For the start check only after
    /// [`Self::stale`]; otherwise the first step of a compaction. If that sector holds a
    /// generation without a valid commit whose number is readable and higher than the live
    /// generation's and every state record's, a C-3 state record goes first (A1.6 "Keeping the
    /// judgment").
    pub fn erase_stale(&mut self) -> Result<(), Error<F::Error>> {
        let r = STORE_SECTORS[1 - self.live];
        let unreadable = self.unreadable;
        let o = self.gen_info(r)?;
        if let (Some((n, _)), false) = (o.begin, o.committed) {
            let l = self.gen_info(STORE_SECTORS[self.live])?;
            if n > self.generation.max(l.state_max.unwrap_or(0)) {
                self.write_state(n)?;
            }
        }
        self.unreadable = unreadable;
        self.guard(Op::Erase, r.start, r.len())?;
        self.io.erase_sector(r.start).map_err(Error::Io)
    }

    /// At open the sector with the newest records had no readable commit and unreadable words:
    /// the previous generation is used (0065 point 5).
    pub fn fell_back(&self) -> bool {
        self.fell_back
    }

    fn compact_keep(&mut self, keep: impl Fn(Kind) -> bool) -> Result<(), Error<F::Error>> {
        self.compact_with(keep, None)
    }

    fn compact_with(&mut self, keep: impl Fn(Kind) -> bool, new: Option<(Kind, u8, &[u8])>) -> Result<(), Error<F::Error>> {
        self.compact_full(keep, new, self.limits)
    }

    /// A compaction: begin record, the kinds kept (or the new record), the C-3 record with
    /// `limits` (the judged ones, unless a C-3 write sets new ones), the commit (0072 points 1, 2).
    fn compact_full(&mut self, keep: impl Fn(Kind) -> bool, new: Option<(Kind, u8, &[u8])>, limits: Limits) -> Result<(), Error<F::Error>> {
        // (1) Erase the other sector. The live sector is not touched.
        let target = 1 - self.live;
        self.erase_stale()?;
        let r = STORE_SECTORS[target];
        // Where the commit word will be: after the begin record, the records and the C-3 record.
        let mut size = padded(8) + padded(LIMITS_LEN);
        for k in Kind::DATA {
            if let Some((_, _, p)) = new.filter(|n| n.0 == k) {
                size += padded(p.len());
            } else if keep(k) {
                if let Some(f) = self.newest(k)? {
                    size += padded(f.len as usize);
                }
            }
        }
        let g = self.number.max(self.generation).wrapping_add(1);
        let commit_at = r.start + size;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&g.to_le_bytes());
        b[4..].copy_from_slice(&commit_at.to_le_bytes());
        let mut at = self.write_at(r, r.start, Kind::Begin, 1, &b)?;
        // (2) Copy the newest record of every kind kept from the live sector, or write the new one.
        for k in Kind::DATA {
            if let Some((_, v, p)) = new.filter(|n| n.0 == k) {
                at = self.write_at(r, at, k, v, p)?;
            } else if keep(k) {
                let mut b = [0u8; MAX_PAYLOAD];
                if let Some((v, n)) = self.get(k, &mut b)? {
                    at = self.write_at(r, at, k, v, &b[..n])?;
                }
            }
        }
        at = self.write_at(r, at, Kind::Limits, 1, &limits.encode())?;
        if at != commit_at {
            return Err(Error::TooLarge); // a record changed under the copy: a bug
        }
        // (3) Commit with the next generation; (4) switch.
        at = self.write_at(r, at, Kind::Commit, 1, &g.to_le_bytes())?;
        self.live = target;
        self.generation = g;
        self.number = g;
        self.end = at;
        self.limits = limits;
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
        assert_eq!(s.free(), SECTOR_SIZE - FLASH_WORD - STATE_ROOM);
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
        assert_eq!(s.unreadable_at_open(), 2, "both bad words counted once at open (T-A1-87 log)");
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"old");
        assert_eq!(s.unreadable_at_open(), 2, "reads after open do not change it");
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
            let g0 = Store::open(base.clone(), APP).unwrap().generation();
            for cut in 0..=words {
                let mut io = base.clone();
                io.budget = Some(cut);
                let mut s = Store::open(io, APP).unwrap();
                let r = s.next_boot_counter();
                let io = s.release().power_on();
                // Every value is there after the restart, whatever the cut, and the generation
                // never goes back (0065 point 3; T-A1-86).
                let mut s = check_all(io, n);
                assert!(s.generation() >= g0, "ecc {ecc} cut {cut}: generation {} < {g0}", s.generation());
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
    fn power_loss_at_every_word_of_an_append_keeps_the_old_value_or_the_new_one() {
        // A multi-word record over an older one of the same kind: after a cut at any word, the
        // value read is the old one (cut before the commit) or the new one (complete), never a
        // mix; the other kinds are untouched; the store goes on working (both cut modes).
        let new = [0x5Au8; 300];
        for ecc in [false, true] {
            let mut sim = Sim::new();
            sim.ecc_on_cut = ecc;
            let mut s = Store::open(sim, APP).unwrap();
            s.put(Kind::Config, 1, b"old config").unwrap();
            s.put(Kind::Secret, 1, &[9u8; 32]).unwrap();
            let base = s.release();
            let mut probe = Store::open(base.clone(), APP).unwrap();
            let before = probe.io.programs;
            probe.put(Kind::Config, 1, &new).unwrap();
            let words = probe.io.programs - before;
            assert!(words >= 10, "{words}");
            for cut in 0..=words {
                let mut io = base.clone();
                io.budget = Some(cut);
                let mut s = Store::open(io, APP).unwrap();
                let done = s.put(Kind::Config, 1, &new).is_ok();
                let mut s = Store::open(s.release().power_on(), APP).unwrap();
                let got = get(&mut s, Kind::Config).unwrap();
                if done {
                    assert_eq!(got, new.to_vec());
                } else {
                    assert_eq!(got, b"old config", "ecc {ecc} cut {cut}/{words}");
                }
                assert_eq!(get(&mut s, Kind::Secret).unwrap(), vec![9u8; 32]);
                s.put(Kind::Config, 1, b"after").unwrap();
                let mut s = Store::open(s.release(), APP).unwrap();
                assert_eq!(get(&mut s, Kind::Config).unwrap(), b"after");
            }
        }
    }

    #[test]
    fn a_second_power_loss_during_the_recovery_from_the_first_loses_nothing() {
        // A cut in a compaction, then on the next start another cut in the next write (which
        // compacts again or appends): every value is still there and the boot counter never
        // goes back (both cut modes; every first cut, a spread of second cuts).
        for ecc in [false, true] {
            let (mut base, n) = full_store();
            base.ecc_on_cut = ecc;
            let mut probe = Store::open(base.clone(), APP).unwrap();
            let before = probe.io.programs;
            probe.next_boot_counter().unwrap();
            let words = probe.io.programs - before;
            for first in 0..=words {
                let mut io = base.clone();
                io.budget = Some(first);
                let mut s = Store::open(io, APP).unwrap();
                let _ = s.next_boot_counter();
                let after_first = s.release().power_on();
                for second in [0, 1, 2, words / 2, words.saturating_sub(1), words] {
                    let mut io = after_first.clone();
                    io.budget = Some(second);
                    if let Ok(mut s) = Store::open(io, APP) {
                        let _ = s.next_boot_counter();
                        io = s.release();
                    } else {
                        continue;
                    }
                    let mut s = check_all(io.power_on(), n);
                    assert!(s.next_boot_counter().unwrap() > n, "ecc {ecc} cuts {first}/{second}");
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

    /// The payload bytes appear anywhere in a sector.
    fn holds(io: &Sim, sector: usize, what: &[u8]) -> bool {
        let o = (STORE_SECTORS[sector].start - STORE.start) as usize;
        io.mem[o..o + SECTOR_SIZE as usize].windows(what.len()).any(|w| w == what)
    }

    const OLD: [u8; 32] = [0x91; 32];
    const NEW: [u8; 32] = [0x37; 32];

    /// A store with the old secret and some other records.
    fn with_secret() -> Sim {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"config v1").unwrap();
        s.put(Kind::Secret, 1, &OLD).unwrap();
        s.put(Kind::BootCounter, 1, &7u32.to_le_bytes()).unwrap();
        s.release()
    }

    /// The start check as the application does it; whether it erased.
    fn start_check(s: &mut Store<Sim>) -> bool {
        let live = get(s, Kind::Secret);
        let stale = s.stale(Kind::Secret, live.as_deref()).unwrap();
        if stale != Stale::Nothing {
            s.erase_stale().unwrap();
        }
        stale != Stale::Nothing
    }

    #[test]
    fn the_old_secret_is_gone_after_the_next_start_t_a1_92() {
        let mut s = Store::open(with_secret(), APP).unwrap();
        let e = s.io.erases;
        s.replace(Kind::Secret, 1, &NEW).unwrap();
        assert_eq!(s.io.erases, e + 1, "one erase before the reply");
        let (live, stale) = (s.live_sector(), 1 - s.live_sector());
        assert!(holds(&s.io, live, &NEW) && !holds(&s.io, live, &OLD));
        assert!(holds(&s.io, stale, &OLD) && !holds(&s.io, stale, &NEW));
        // The next start.
        let mut s = Store::open(s.release(), APP).unwrap();
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), NEW);
        assert_eq!(get(&mut s, Kind::Config).unwrap(), b"config v1");
        assert!(start_check(&mut s));
        assert!(!holds(&s.io, 0, &OLD) && !holds(&s.io, 1, &OLD));
        // The one after: nothing to do. A compaction then leaves the same secret in both sectors;
        // the start check leaves that.
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(!start_check(&mut s));
        s.compact().unwrap();
        s.compact().unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(holds(&s.io, 0, &NEW) && holds(&s.io, 1, &NEW));
        assert!(!start_check(&mut s), "only the live secret");
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), NEW);
        // Or the next compaction before any start erases it.
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.replace(Kind::Secret, 1, &NEW).unwrap();
        s.compact().unwrap();
        assert!(!holds(&s.io, 0, &OLD) && !holds(&s.io, 1, &OLD));
    }

    #[test]
    fn power_loss_at_every_word_of_a_secret_change_leaves_one_of_the_two() {
        for ecc in [false, true] {
            let mut base = with_secret();
            base.ecc_on_cut = ecc;
            let mut p = Store::open(base.clone(), APP).unwrap();
            p.replace(Kind::Secret, 1, &NEW).unwrap();
            let words = p.io.programs - base.programs;
            for cut in 0..=words + 1 {
                let mut io = base.clone();
                if cut == words + 1 {
                    io.erase_cut = true;
                } else {
                    io.budget = Some(cut);
                }
                let mut s = Store::open(io, APP).unwrap();
                let done = s.replace(Kind::Secret, 1, &NEW).is_ok();
                let mut s = Store::open(s.release().power_on(), APP).unwrap();
                assert_eq!(get(&mut s, Kind::Secret).unwrap(), if done { NEW } else { OLD }, "ecc {ecc} cut {cut}");
                assert_eq!(get(&mut s, Kind::Config).unwrap(), b"config v1");
                start_check(&mut s);
                let mut s = Store::open(s.release(), APP).unwrap();
                assert_eq!(get(&mut s, Kind::Secret).unwrap(), if done { NEW } else { OLD });
                let gone: &[u8] = if done { &OLD } else { &NEW };
                assert!(!holds(&s.io, 0, gone) && !holds(&s.io, 1, gone), "ecc {ecc} cut {cut}");
            }
        }
    }

    #[test]
    fn a_store_that_fell_back_keeps_its_generation_t_a1_92() {
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.replace(Kind::Secret, 1, &NEW).unwrap();
        assert!(!s.fell_back());
        let commit = s.end - padded(4);
        let mut io = s.release();
        io.bad[((commit - STORE.start) / FLASH_WORD) as usize] = true;
        let mut s = Store::open(io, APP).unwrap();
        assert!(s.fell_back());
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), OLD, "the stale generation is the live one");
        // What the check erases is the other sector, never the generation in use.
        assert!(start_check(&mut s));
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(!s.fell_back());
        assert_eq!(get(&mut s, Kind::Secret).unwrap(), OLD);
        // An old unreadable word in the sector that is not live is no fall-back.
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.compact().unwrap();
        s.compact().unwrap();
        let mut io = s.release();
        let o = (STORE_SECTORS[0].start - STORE.start) as usize / 32;
        for w in o..o + 12 {
            io.bad[w] = true;
        }
        assert!(!Store::open(io, APP).unwrap().fell_back());
    }

    use g7250_proto::mgmt::payload::{Config, MAX_ALLOW, MAX_BANDS, MAX_CONFIG};

    /// Every item of Table M-3 set to something that is not the factory value.
    fn owner_config() -> Config {
        let mut c = Config {
            address_mode: Some(1),
            address: Some([192, 168, 7, 20]),
            netmask: Some([255, 255, 255, 0]),
            gateway: Some([192, 168, 7, 1]),
            default_level: Some(2),
            region: Some(1),
            max_tx_time_ms: Some(60_000),
            swr_s2_x100: Some(210),
            pa_temp_t3: Some(650),
            allow_count: Some(MAX_ALLOW as u8),
            allow: [([192, 168, 7, 0], 24); MAX_ALLOW],
            ..Default::default()
        };
        for (i, b) in c.band_limit.iter_mut().enumerate() {
            *b = (i != 5).then_some(100 + i as u16); // one band without a limit; 0 closes a band
        }
        c.band_limit[2] = Some(0);
        c
    }

    fn encoded(c: &Config) -> std::vec::Vec<u8> {
        let mut b = [0u8; MAX_CONFIG];
        let n = c.encode(&mut b).unwrap();
        b[..n].to_vec()
    }

    /// The configuration as a start uses it (mgmt_init.rs).
    fn config_at_start(s: &mut Store<Sim>) -> Config {
        Config::stored(get(s, Kind::Config).as_deref()).unwrap_or(Config::factory())
    }

    fn limits(c: &Config) -> (Option<u32>, [Option<u16>; MAX_BANDS], Option<u16>, Option<u16>) {
        (c.max_tx_time_ms, c.band_limit, c.swr_s2_x100, c.pa_temp_t3)
    }

    /// A store before a factory reset: the owner's configuration, OLD as the secret.
    fn owner_store() -> Sim {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, &encoded(&owner_config())).unwrap();
        s.put(Kind::Secret, 1, &OLD).unwrap();
        s.put(Kind::BootCounter, 1, &7u32.to_le_bytes()).unwrap();
        s.put(Kind::Calibration, 1, &[0x3C; 200]).unwrap();
        s.release()
    }

    fn drops(k: Kind) -> bool {
        k == Kind::Secret
    }

    fn reset(s: &mut Store<Sim>) -> Result<(), Error<SimErr>> {
        s.wipe(drops, Kind::Config, Config::after_factory_reset)
    }

    /// The next start, as the application does it.
    fn start(s: &mut Store<Sim>) {
        if s.wipe_pending().unwrap() {
            s.finish_wipe(drops, Kind::Config, Config::after_factory_reset).unwrap();
        }
        start_check(s);
    }

    /// After a factory reset: the factory values with the owner's C-3 limits; the rest kept.
    fn check_reset(s: &mut Store<Sim>, owner: &Config) {
        let c = config_at_start(s);
        assert_eq!(limits(&c), limits(owner), "C-3 kept");
        let f = Config::factory();
        assert_eq!(Config { max_tx_time_ms: f.max_tx_time_ms, band_limit: f.band_limit, swr_s2_x100: f.swr_s2_x100, pa_temp_t3: f.pa_temp_t3, ..c }, f, "the rest factory");
        assert_eq!(get(s, Kind::Secret), None);
        assert_eq!(get(s, Kind::BootCounter).unwrap(), 7u32.to_le_bytes());
        assert_eq!(get(s, Kind::Calibration).unwrap(), [0x3C; 200]);
        assert!(!holds(&s.io, 0, &OLD) && !holds(&s.io, 1, &OLD));
        assert!(!s.wipe_pending().unwrap());
    }

    #[test]
    fn a_factory_reset_keeps_the_c3_limits_t_a1_39() {
        let owner = owner_config();
        let mut s = Store::open(owner_store(), APP).unwrap();
        let e = s.io.erases;
        reset(&mut s).unwrap();
        assert_eq!(s.io.erases - e, 2, "at most two erases after the reply");
        check_reset(&mut s, &owner);
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(!start_check(&mut s), "nothing left to do");
        check_reset(&mut s, &owner);
        // Again: the same.
        reset(&mut s).unwrap();
        let mut s = Store::open(s.release(), APP).unwrap();
        check_reset(&mut s, &owner);
        // Each C-3 item alone, the others at their factory values.
        let one: [fn(&mut Config); 4] = [|c| c.max_tx_time_ms = Some(1), |c| c.band_limit[MAX_BANDS - 1] = Some(0), |c| c.swr_s2_x100 = Some(151), |c| c.pa_temp_t3 = Some(1)];
        for set in one {
            let mut c = Config::factory();
            set(&mut c);
            let mut s = Store::open(owner_store(), APP).unwrap();
            s.put(Kind::Config, 1, &encoded(&c)).unwrap();
            reset(&mut s).unwrap();
            assert_eq!(config_at_start(&mut s), c);
        }
        // A record that does not decode, one that is too long, and none: the built-in limits.
        for rec in [Some(std::vec![7u8, 9, 0]), Some(std::vec![0u8; MAX_CONFIG + 2]), None] {
            let mut s = Store::open(Sim::new(), APP).unwrap();
            s.put(Kind::Secret, 1, &OLD).unwrap();
            if let Some(r) = &rec {
                s.put(Kind::Config, 1, r).unwrap();
            }
            assert_eq!(config_at_start(&mut s), Config::factory());
            reset(&mut s).unwrap();
            assert_eq!(config_at_start(&mut s), Config::factory());
            assert_eq!(get(&mut s, Kind::Secret), None);
        }
        // The fixed sizes hold the largest record.
        assert!(encoded(&Config { band_limit: [Some(u16::MAX); MAX_BANDS], ..owner }).len() <= MAX_CONFIG);
    }

    #[test]
    fn a_cut_factory_reset_never_loosens_a_limit_and_is_finished_0069() {
        let owner = owner_config();
        let mut p = Store::open(owner_store(), APP).unwrap();
        let w = p.io.programs;
        reset(&mut p).unwrap();
        let words = p.io.programs - w;
        for ecc in [false, true] {
            // A cut at every word, and at the first and at the second erase.
            for cut in 0..=words + 2 {
                let mut base = owner_store();
                base.ecc_on_cut = ecc;
                let mut s = Store::open(base, APP).unwrap();
                match cut.checked_sub(words + 1) {
                    None => {
                        s.io.budget = Some(cut);
                        assert_eq!(reset(&mut s).is_ok(), cut == words);
                    }
                    Some(0) => {
                        s.io.erase_cut = true;
                        assert!(reset(&mut s).is_err());
                    }
                    Some(_) => {
                        s.put(Kind::Wipe, 1, &[1]).unwrap();
                        s.finish_wipe(drops, Kind::Config, Config::after_factory_reset).unwrap();
                        s.io.erase_cut = true;
                        assert!(s.compact().is_err());
                    }
                }
                let cut_io = s.release().power_on();
                // The limits hold at the next start, before and after it finishes the reset.
                let mut s = Store::open(cut_io.clone(), APP).unwrap();
                assert_eq!(limits(&config_at_start(&mut s)), limits(&owner), "ecc {ecc} cut {cut}");
                let (pending, e, w) = (s.wipe_pending().unwrap(), s.io.erases, s.io.programs);
                start(&mut s);
                assert!(s.io.erases - e <= 2, "at most two erases");
                let finish = s.io.programs - w;
                let mut s = Store::open(s.release(), APP).unwrap();
                if cut == 0 {
                    // Cut before the mark was written: the reset did not begin.
                    assert!(!pending);
                    assert_eq!((get(&mut s, Kind::Secret).unwrap(), config_at_start(&mut s)), (OLD.to_vec(), owner));
                    continue;
                }
                check_reset(&mut s, &owner);
                // The finish itself cut, at every word and at its erase: finished by the start after.
                for again in 0..=finish {
                    if !pending {
                        break;
                    }
                    let mut s = Store::open(cut_io.clone(), APP).unwrap();
                    if again == finish {
                        s.io.erase_cut = true;
                    } else {
                        s.io.budget = Some(again);
                    }
                    assert!(s.finish_wipe(drops, Kind::Config, Config::after_factory_reset).is_err());
                    let mut s = Store::open(s.release().power_on(), APP).unwrap();
                    assert_eq!(limits(&config_at_start(&mut s)), limits(&owner), "ecc {ecc} cut {cut}/{again}");
                    start(&mut s);
                    let mut s = Store::open(s.release(), APP).unwrap();
                    check_reset(&mut s, &owner);
                }
            }
        }
    }

    // --- 0072: C-3 at a start ---

    fn lim(power: u16) -> Limits {
        let mut l = Limits::BUILTIN;
        l.band[0] = Some(power);
        l.max_tx_time_ms = Some(1000 * power as u32);
        l
    }

    /// The address of the newest record of `kind` in `sector`.
    fn addr_of(s: &mut Store<Sim>, sector: usize, kind: Kind) -> u32 {
        let mut at = None;
        s.walk(STORE_SECTORS[sector], |_, f| {
            if f.kind == kind {
                at = Some(f.addr);
            }
            Ok(())
        })
        .unwrap();
        at.unwrap()
    }

    fn addr_of_live(s: &mut Store<Sim>, kind: Kind) -> u32 {
        let live = s.live_sector();
        addr_of(s, live, kind)
    }

    fn spoil(io: &mut Sim, addr: u32) {
        io.bad[((addr - STORE.start) / FLASH_WORD) as usize] = true;
    }

    fn reopen(s: Store<Sim>) -> Store<Sim> {
        Store::open(s.release().power_on(), APP).unwrap()
    }

    #[test]
    fn limits_encode_and_stricter() {
        for l in [Limits::BUILTIN, Limits::LOST, lim(5), Limits { swr_s2_x100: Some(150), pa_temp_t3: Some(700), band: [Some(0); LIMIT_BANDS], ..lim(1) }] {
            assert_eq!(Limits::decode(&l.encode()), Some(l));
        }
        assert_eq!(Limits::decode(&[2; LIMITS_LEN]), None);
        assert_eq!(Limits::decode(&[0; LIMITS_LEN - 1]), None);
        let (a, b) = (lim(5), Limits { swr_s2_x100: Some(150), ..lim(9) });
        assert_eq!(a.stricter(&b), Limits { swr_s2_x100: Some(150), ..lim(5) }, "per item; not tightened is looser than a value");
        assert_eq!(a.stricter(&Limits::LOST), Limits::LOST);
        assert_eq!(Limits::BUILTIN.stricter(&a), a);
    }

    #[test]
    fn a_c3_write_is_a_compaction_and_holds_t_a1_86() {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (Limits::BUILTIN, Judged::Factory));
        s.put(Kind::Config, 1, b"c1").unwrap();
        let e = s.io.erases;
        s.write_limits(None, lim(50)).unwrap();
        assert_eq!(s.io.erases - e, 1, "one erase");
        let mut s = reopen(s);
        assert_eq!((s.limits(), s.judged(), get(&mut s, Kind::Config).unwrap()), (lim(50), Judged::Committed, b"c1".to_vec()));
        // Every compaction carries the limits in force, a plain one too.
        s.compact().unwrap();
        s.put(Kind::Config, 1, b"c1 again").unwrap();
        let mut s = reopen(s);
        assert_eq!((s.limits(), s.judged()), (lim(50), Judged::Committed));
        // Loosened back: the store keeps what is written.
        s.write_limits(Some((Kind::Config, 1, b"c1 new")), lim(80)).unwrap();
        let mut s = reopen(s);
        assert_eq!((s.limits(), get(&mut s, Kind::Config).unwrap()), (lim(80), b"c1 new".to_vec()));
        assert!(s.put(Kind::Limits, 1, &lim(1).encode()).is_err(), "never appended");
    }

    /// A store with lim(50) committed in generation 1 and config "old".
    fn c3_base() -> Sim {
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.write_limits(Some((Kind::Config, 1, b"old")), lim(50)).unwrap();
        s.release()
    }

    #[test]
    fn a_cut_c3_write_never_loosens_and_stays_after_the_start_check_t_a1_86() {
        let mut p = Store::open(c3_base(), APP).unwrap();
        let w = p.io.programs;
        p.write_limits(Some((Kind::Config, 1, b"new")), lim(30)).unwrap();
        let words = p.io.programs - w;
        for ecc in [false, true] {
            for cut in 0..words {
                let mut io = c3_base();
                io.ecc_on_cut = ecc;
                let mut s = Store::open(io, APP).unwrap();
                s.io.budget = Some(cut);
                assert!(s.write_limits(Some((Kind::Config, 1, b"new")), lim(30)).is_err());
                let mut s = reopen(s);
                let (l, c) = (s.limits(), get(&mut s, Kind::Config).unwrap());
                assert_eq!(c, b"old", "ecc {ecc} cut {cut}: not committed, the other items old");
                // Old, or (the commit word torn) the stricter of old and new; never looser, never lost.
                let commit_word = ecc && cut == words - 1;
                assert_eq!(l, if commit_word { lim(30) } else { lim(50) }, "ecc {ecc} cut {cut}");
                // The start check, then a cut in its erase, then the next start: the same.
                let had = start_check(&mut s);
                let mut s = reopen(s);
                assert_eq!(s.limits(), l, "ecc {ecc} cut {cut}: after the start check (erased: {had})");
                let mut io = Store::open(c3_base(), APP).unwrap();
                io.io.budget = Some(cut);
                io.io.ecc_on_cut = ecc;
                let _ = io.write_limits(Some((Kind::Config, 1, b"new")), lim(30));
                let mut s = reopen(io);
                if s.stale(Kind::Secret, None).unwrap() != Stale::Nothing {
                    s.io.erase_cut = true;
                    assert!(s.erase_stale().is_err());
                    let s = reopen(s);
                    assert_eq!(s.limits(), l, "ecc {ecc} cut {cut}: erase cut");
                }
            }
        }
    }

    #[test]
    fn an_unreadable_c3_record_is_lost_until_rewritten_t_a1_87() {
        let mut s = Store::open(c3_base(), APP).unwrap();
        let at = addr_of_live(&mut s, Kind::Limits);
        let mut io = s.release();
        spoil(&mut io, at);
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (Limits::LOST, Judged::Committed));
        // Every compaction carries the lost state, until a C-3 write.
        s.compact().unwrap();
        let mut s = reopen(s);
        assert_eq!(s.limits(), Limits::LOST);
        s.write_limits(None, lim(70)).unwrap();
        let s = reopen(s);
        assert_eq!(s.limits(), lim(70));
    }

    #[test]
    fn the_generation_number_in_two_places_t_a1_87() {
        // The commit word unreadable, the other sector erased: the failed generation counts.
        let mut s = Store::open(c3_base(), APP).unwrap();
        s.erase_stale().unwrap();
        let live = s.live_sector();
        let at = addr_of(&mut s, live, Kind::Commit);
        let mut io = s.release();
        spoil(&mut io, at);
        let s = Store::open(io, APP).unwrap();
        assert_ne!(s.live_sector(), live);
        assert_eq!((s.limits(), s.judged()), (lim(50), Judged::FailedGeneration), "never the built-in values");
        // The begin record unreadable: the commit still makes the generation live.
        let mut s = Store::open(c3_base(), APP).unwrap();
        let at = addr_of_live(&mut s, Kind::Begin);
        let mut io = s.release();
        spoil(&mut io, at);
        let s = Store::open(io, APP).unwrap();
        assert_eq!((s.generation(), s.limits(), s.judged()), (1, lim(50), Judged::Committed));
    }

    /// Generation 1 (lim(50)) live in sector 6 and generation 2 (lim(30), "new") in sector 7 with
    /// its commit word unreadable, and with its C-3 record unreadable too if `c3`.
    fn failed_newer(c3: bool) -> Sim {
        let mut s = Store::open(c3_base(), APP).unwrap();
        s.write_limits(Some((Kind::Config, 1, b"new")), lim(30)).unwrap();
        let (c, l) = (addr_of_live(&mut s, Kind::Commit), addr_of_live(&mut s, Kind::Limits));
        let mut io = s.release();
        spoil(&mut io, c);
        if c3 {
            spoil(&mut io, l);
        }
        io
    }

    #[test]
    fn a_failed_generation_gives_the_stricter_values_and_a_state_record_keeps_them_t_a1_92() {
        for c3 in [false, true] {
            let want = if c3 { Limits::LOST } else { lim(30) };
            let mut s = Store::open(failed_newer(c3), APP).unwrap();
            assert!(s.fell_back());
            assert_eq!((s.limits(), s.judged(), get(&mut s, Kind::Config).unwrap()), (want, Judged::FailedGeneration, b"old".to_vec()));
            // The start check erases the failed generation; first a state record. A cut at its
            // word, in the erase, after it, in the next compaction and after it: never looser.
            let w = s.io.programs;
            assert!(start_check(&mut s));
            assert_eq!(s.io.programs - w, 2, "a state record of two words");
            let s = reopen(s);
            assert_eq!((s.limits(), s.judged()), (want, Judged::StateRecord), "c3 {c3}");
            for cut in 0..2 {
                let mut s = Store::open(failed_newer(c3), APP).unwrap();
                s.io.budget = Some(cut);
                assert!(s.erase_stale().is_err());
                let s = reopen(s);
                assert_eq!(s.limits(), want, "c3 {c3}: state record cut at word {cut}");
            }
            let mut s = Store::open(failed_newer(c3), APP).unwrap();
            s.io.erase_cut = true;
            assert!(s.erase_stale().is_err());
            let mut s = reopen(s);
            assert_eq!(s.limits(), want, "c3 {c3}: erase cut");
            // The next compaction (a write with no C-3 item), cut at every word, then whole.
            let base = s.release();
            let mut p = Store::open(base.clone(), APP).unwrap();
            let w = p.io.programs;
            p.write_limits(None, p.limits()).unwrap();
            for cut in 0..p.io.programs - w {
                let mut s = Store::open(base.clone(), APP).unwrap();
                s.io.budget = Some(cut);
                let _ = s.put(Kind::Config, 1, b"x").and_then(|_| s.compact());
                let s = reopen(s);
                assert_eq!(s.limits(), want, "c3 {c3}: compaction cut at {cut}");
            }
            let mut s = Store::open(base, APP).unwrap();
            s.compact().unwrap();
            let s = reopen(s);
            assert_eq!((s.limits(), s.judged()), (want, Judged::Committed), "c3 {c3}: carried by the committed generation");
        }
    }

    #[test]
    fn a_factory_reset_carries_c3_and_the_lost_state_t_a1_39() {
        for lost in [false, true] {
            let mut s = Store::open(owner_store(), APP).unwrap();
            s.write_limits(None, lim(40)).unwrap();
            if lost {
                let at = addr_of_live(&mut s, Kind::Limits);
                let mut io = s.release();
                spoil(&mut io, at);
                s = Store::open(io, APP).unwrap();
            }
            let want = if lost { Limits::LOST } else { lim(40) };
            assert_eq!(s.limits(), want);
            reset(&mut s).unwrap();
            let mut s = reopen(s);
            assert_eq!((s.limits(), get(&mut s, Kind::Secret)), (want, None), "lost {lost}");
        }
    }

    #[test]
    fn which_failed_generation_counts_t_a1_92() {
        // Looser than the live value: the stricter, so the live one.
        let mut s = Store::open(c3_base(), APP).unwrap();
        s.write_limits(None, lim(80)).unwrap();
        let at = addr_of_live(&mut s, Kind::Commit);
        let mut io = s.release();
        spoil(&mut io, at);
        let s = Store::open(io, APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (lim(50), Judged::FailedGeneration));
        // Not above the live sector's state record: it does not count (here its C-3 record is
        // unreadable, so it would close TX).
        let mut s = Store::open(failed_newer(true), APP).unwrap();
        s.limits = lim(30);
        s.write_state(2).unwrap();
        let s = reopen(s);
        assert_eq!((s.limits(), s.judged()), (lim(30), Judged::StateRecord));
        // After a fall-back, L holds a state record; a tightening commits N above it; N's commit
        // word unreadable: N counts, its stricter values, not the state record's.
        let mut s = Store::open(failed_newer(false), APP).unwrap();
        assert!(start_check(&mut s));
        s.write_limits(None, lim(20)).unwrap();
        assert!(s.generation() > 2, "above every number readable, the state record's included");
        let at = addr_of_live(&mut s, Kind::Commit);
        let mut io = s.release();
        spoil(&mut io, at);
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (lim(20), Judged::FailedGeneration));
        // A state record before N's sector is erased, and a cut at each point keeps them.
        let base = s.release();
        let mut s = Store::open(base.clone(), APP).unwrap();
        assert!(start_check(&mut s));
        assert_eq!(reopen(s).limits(), lim(20));
        for cut in 0..2 {
            let mut s = Store::open(base.clone(), APP).unwrap();
            s.io.budget = Some(cut);
            assert!(s.erase_stale().is_err());
            assert_eq!(reopen(s).limits(), lim(20), "cut {cut}");
        }
        // A failed generation in each sector and none live: both taken.
        let mut s = Store::open(c3_base(), APP).unwrap();
        s.erase_stale().unwrap();
        let first = addr_of_live(&mut s, Kind::Commit);
        s.write_limits(None, lim(30)).unwrap();
        let second = addr_of_live(&mut s, Kind::Commit);
        let mut io = s.release();
        spoil(&mut io, first);
        spoil(&mut io, second);
        let s = Store::open(io, APP).unwrap();
        assert_eq!((s.generation(), s.limits(), s.judged()), (0, lim(30), Judged::FailedGeneration));
    }

    #[test]
    fn an_older_generation_never_counts_and_foreign_data_is_factory_t_a1_92() {
        // G live, G-1 in the other sector with its commit word unreadable: G's values.
        let mut s = Store::open(c3_base(), APP).unwrap();
        s.write_limits(None, lim(40)).unwrap();
        let older = 1 - s.live_sector();
        let at = addr_of(&mut s, older, Kind::Commit);
        let mut io = s.release();
        spoil(&mut io, at);
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (lim(40), Judged::Committed));
        let w = s.io.programs;
        assert!(start_check(&mut s), "erased as unrecognised data");
        assert_eq!(s.io.programs, w, "no state record for an older generation");
        // Foreign data in both sectors: the built-in values.
        let mut io = Sim::new();
        io.mem.fill(0x5A);
        io.written.fill(true);
        let s = Store::open(io, APP).unwrap();
        assert_eq!((s.limits(), s.judged()), (Limits::BUILTIN, Judged::Factory));
    }

    /// The owner's store with the live sector filled by `put` as far as it goes, and with
    /// `last_word` also its last word (a store written by a firmware that kept no word free).
    fn full_owner_store(last_word: bool) -> Sim {
        let mut s = Store::open(owner_store(), APP).unwrap();
        while s.free() > 0 {
            s.put(Kind::Staging, 1, &[]).unwrap();
        }
        assert_eq!((s.generation(), s.room()), (0, FLASH_WORD + STATE_ROOM));
        if last_word {
            while s.room() > 0 {
                s.end = s.write_at(STORE_SECTORS[s.live], s.end, Kind::Staging, 1, &[]).unwrap();
            }
        }
        s.release()
    }

    #[test]
    fn a_factory_reset_of_a_full_store_is_marked_and_finished_a1_6a() {
        let owner = owner_config();
        // put leaves the word for the mark: two erases, and a cut at any word after the mark
        // (none of the reset's records written yet, or some) is finished at the next start.
        let mut p = Store::open(full_owner_store(false), APP).unwrap();
        let (e, w) = (p.io.erases, p.io.programs);
        reset(&mut p).unwrap();
        assert_eq!(p.io.erases - e, 2, "at most two erases after the reply");
        check_reset(&mut p, &owner);
        for cut in 1..p.io.programs - w {
            let mut s = Store::open(full_owner_store(false), APP).unwrap();
            s.io.budget = Some(cut);
            assert!(reset(&mut s).is_err());
            let mut s = Store::open(s.release().power_on(), APP).unwrap();
            // Marked until the first compaction commits; that compaction leaves the mark out.
            assert!(s.wipe_pending().unwrap() || s.generation() == 1, "cut {cut}: marked");
            assert_eq!(limits(&config_at_start(&mut s)), limits(&owner));
            start(&mut s);
            let mut s = Store::open(s.release(), APP).unwrap();
            check_reset(&mut s, &owner);
        }
        // No word left: the mark comes with a first compaction, the drop with the second; the
        // start check of the next start erases the sector left with the old secret.
        let mut s = Store::open(full_owner_store(true), APP).unwrap();
        let e = s.io.erases;
        reset(&mut s).unwrap();
        assert_eq!(s.io.erases - e, 2, "at most two erases after the reply");
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(!s.wipe_pending().unwrap());
        assert!(start_check(&mut s), "the old secret erased at the next start");
        check_reset(&mut s, &owner);
        // A cut at every word: in the first compaction the old items stay and no mark (the
        // accepted gap of a store with no word left); after its commit, marked and finished at
        // the next start.
        let mut p = Store::open(full_owner_store(true), APP).unwrap();
        let w = p.io.programs;
        reset(&mut p).unwrap();
        for cut in 0..p.io.programs - w {
            let mut s = Store::open(full_owner_store(true), APP).unwrap();
            s.io.budget = Some(cut);
            assert!(reset(&mut s).is_err());
            let mut s = Store::open(s.release().power_on(), APP).unwrap();
            assert_eq!(limits(&config_at_start(&mut s)), limits(&owner));
            if s.generation() == 0 {
                assert!(!s.wipe_pending().unwrap());
                assert_eq!((get(&mut s, Kind::Secret).unwrap(), config_at_start(&mut s)), (OLD.to_vec(), owner), "cut {cut}");
                continue;
            }
            assert!(s.wipe_pending().unwrap(), "cut {cut}: marked");
            start(&mut s);
            let mut s = Store::open(s.release(), APP).unwrap();
            start_check(&mut s);
            check_reset(&mut s, &owner);
        }
    }

    #[test]
    fn no_erase_at_start_without_a_complete_live_generation_0069() {
        // The only generation is in sector 6 and sector 7 is erased (by the start check after a
        // first change of the secret); then the commit word of sector 6 becomes unreadable, and
        // sector 7 is live as generation 0. Nothing may erase sector 6 at a start.
        let mut s = Store::open(Sim::new(), APP).unwrap();
        s.put(Kind::Config, 1, b"config v1").unwrap();
        s.put(Kind::Secret, 1, &OLD).unwrap();
        s.replace(Kind::Secret, 1, &NEW).unwrap();
        assert_eq!((s.live_sector(), s.generation()), (0, 1));
        let commit = s.end - padded(4);
        let mut s = Store::open(s.release(), APP).unwrap();
        assert!(start_check(&mut s));
        assert!(!s.io.written[(SECTOR_SIZE / FLASH_WORD) as usize..].iter().any(|&w| w), "sector 7 erased");
        let mut io = s.release();
        io.bad[((commit - STORE.start) / FLASH_WORD) as usize] = true;
        let sector6 = io.mem[..SECTOR_SIZE as usize].to_vec();
        let same = |s: &Store<Sim>| s.io.mem[..SECTOR_SIZE as usize] == sector6[..];
        let mut s = Store::open(io, APP).unwrap();
        assert_eq!((s.live_sector(), s.generation(), get(&mut s, Kind::Secret)), (1, 0, None));
        assert!(!start_check(&mut s) && same(&s));
        // Nor with what a start writes before its start check, and other records that do not
        // count, in sector 7, at this start and the next ones.
        for k in [Kind::BootCounter, Kind::BootRecord, Kind::Staging, Kind::Wipe] {
            s.put(k, 1, &[1, 0, 0, 0]).unwrap();
            assert!(!start_check(&mut s) && same(&s), "{k:?}");
            s = Store::open(s.release(), APP).unwrap();
            assert!(!start_check(&mut s) && same(&s), "{k:?}");
        }
        // The owner's first record of the list: the next start erases sector 6 (accepted).
        for k in [Kind::Config, Kind::Calibration, Kind::Secret] {
            let mut t = Store::open(s.io.clone(), APP).unwrap();
            t.put(k, 1, &[0x11; 32]).unwrap();
            let mut t = Store::open(t.release(), APP).unwrap();
            assert!(start_check(&mut t), "{k:?}");
            assert!(!holds(&t.io, 0, &NEW));
            assert_eq!(get(&mut t, k).unwrap(), [0x11; 32]);
        }
        // An empty store, and foreign data in both sectors: nothing either.
        assert!(!start_check(&mut Store::open(Sim::new(), APP).unwrap()));
        let mut io = Sim::new();
        io.mem.fill(0x5A);
        io.written.fill(true);
        assert!(!start_check(&mut Store::open(io, APP).unwrap()));
    }

    #[test]
    fn a_failed_first_change_is_erased_after_a_fall_back_to_the_first_generation_0069() {
        // The first generation (no commit word) holds only the first secret, only configuration,
        // or only calibration; the newer generation with NEW has an unreadable commit word. The
        // first generation stays in use, and NEW is in neither sector after the start check.
        for k in [Kind::Secret, Kind::Config, Kind::Calibration] {
            let mut s = Store::open(Sim::new(), APP).unwrap();
            s.put(k, 1, &OLD).unwrap();
            s.replace(Kind::Secret, 1, &NEW).unwrap();
            let commit = s.end - padded(4);
            let mut io = s.release();
            io.bad[((commit - STORE.start) / FLASH_WORD) as usize] = true;
            let mut s = Store::open(io, APP).unwrap();
            assert_eq!((s.live_sector(), s.generation()), (1, 0));
            assert!(s.fell_back());
            s.next_boot_counter().unwrap();
            assert!(start_check(&mut s), "{k:?}");
            assert!(!holds(&s.io, 0, &NEW) && !holds(&s.io, 1, &NEW), "{k:?}");
            let mut s = Store::open(s.release(), APP).unwrap();
            assert_eq!(get(&mut s, k).unwrap(), OLD, "{k:?}");
            assert!(!start_check(&mut s));
        }
    }

    /// What the start check says, without erasing.
    fn stale(s: &mut Store<Sim>) -> Stale {
        let live = get(s, Kind::Secret);
        s.stale(Kind::Secret, live.as_deref()).unwrap()
    }

    #[test]
    fn unrecognised_data_in_the_other_sector_is_erased_at_start_0069() {
        // Generation 1 in sector 6, sector 7 erased.
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.compact().unwrap();
        s.erase_stale().unwrap();
        assert_eq!((s.live_sector(), stale(&mut s)), (0, Stale::Nothing), "fully erased");
        let clean = s.release();
        let n = (SECTOR_SIZE / FLASH_WORD) as usize;
        // One word that is not erased, at the start, in the middle and at the end of the sector:
        // foreign in its first or only in its second half, or unreadable.
        for at in [n, n + 1, n + n / 2, 2 * n - 1] {
            for what in 0..3 {
                let mut io = clean.clone();
                io.written[at] = true;
                match what {
                    0 => io.mem[at * 32] = 0xFE,
                    1 => io.mem[at * 32 + 31] = 0x7F,
                    _ => io.bad[at] = true,
                }
                let mut s = Store::open(io, APP).unwrap();
                assert_eq!(stale(&mut s), Stale::Unrecognised, "word {at} {what}");
                assert!(start_check(&mut s));
                assert!(s.io.mem[n * 32..].iter().all(|&b| b == 0xFF) && !s.io.bad[at]);
                let mut s = Store::open(s.release(), APP).unwrap();
                assert_eq!(stale(&mut s), Stale::Nothing, "once");
                assert_eq!(get(&mut s, Kind::Secret).unwrap(), OLD);
            }
        }
        // Complete records with no other secret are left, and not judged by their number or kind.
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.put(Kind::Calibration, 1, &[7u8; 333]).unwrap();
        s.compact().unwrap();
        assert_eq!(stale(&mut s), Stale::Nothing);
        // A word after the complete records; and another secret comes first.
        let mut t = Store::open(s.io.clone(), APP).unwrap();
        t.io.mem[(2 * n - 1) * 32] = 0;
        assert_eq!(stale(&mut t), Stale::Unrecognised);
        t.put(Kind::Secret, 1, &NEW).unwrap();
        assert_eq!(stale(&mut t), Stale::Other);
    }

    #[test]
    fn no_byte_of_the_old_secret_is_left_after_a_cut_erase_t_a1_92() {
        // The stale sector with the old secret, its erase cut at every word of the secret's
        // record (the words before the cut erased; the one at the cut unreadable, or not).
        let mut s = Store::open(with_secret(), APP).unwrap();
        s.replace(Kind::Secret, 1, &NEW).unwrap();
        let stale_at = STORE_SECTORS[1 - s.live_sector()].start;
        let first = ((stale_at - STORE.start) / FLASH_WORD) as usize;
        let base = s.release();
        let found = (first..first + 16).filter(|w| base.mem[w * 32..w * 32 + 32].iter().any(|&b| b == OLD[0])).collect::<std::vec::Vec<_>>();
        assert_eq!(found.len(), 2, "the record has two words");
        // Another secret comes first, also after a word that is not recognised.
        let mut io = base.clone();
        io.mem[first * 32] ^= 1;
        assert_eq!(stale(&mut Store::open(io, APP).unwrap()), Stale::Other);
        let mut seen = [false; 2];
        for cut in first..first + 16 {
            for ecc in [false, true] {
                let mut io = base.clone();
                io.mem[first * 32..cut * 32].fill(0xFF);
                io.written[first..cut].fill(false);
                io.bad[cut] = ecc;
                let mut s = Store::open(io, APP).unwrap();
                assert_eq!(get(&mut s, Kind::Secret).unwrap(), NEW);
                let v = stale(&mut s);
                let whole = cut < found[0] || cut == found[0] && !ecc;
                if whole {
                    assert_eq!(v, Stale::Other, "cut {cut} ecc {ecc}");
                } else if cut <= found[1] {
                    assert_eq!(v, Stale::Unrecognised, "cut {cut} ecc {ecc}");
                }
                seen[whole as usize] = true;
                start_check(&mut s);
                assert!(!holds(&s.io, 0, &OLD[..8]) && !holds(&s.io, 1, &OLD[..8]), "cut {cut} ecc {ecc}");
                assert_eq!(stale(&mut s), Stale::Nothing);
            }
        }
        assert_eq!(seen, [true; 2]);
    }
}
