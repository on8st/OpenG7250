// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Transfer of a new image into the other slot (A1.8, U-2 to U-5, U-8; Table M-1, M-4).
//!
//! The package carries one image: a sealed application image linked for the slot it is staged
//! in (the other slot; `IDENTIFY` tag 23 tells the host which slot runs). Table M-4 byte 52
//! names that slot; a package for the running slot gets 63 at `UPDATE_BEGIN` (0043). `UPDATE_VERIFY` checks the
//! SHA-256 of the package header and then the image header (load address = the other slot,
//! length, image CRC, version = the package's, vectors) in one pass over the staged bytes, so
//! that an image linked for the wrong slot is refused before activation.
//!
//! Blocks are written in order (resumable: a block already received is answered without a write;
//! a gap gets 54). A slot sector is erased when the first block in it arrives. Every program and
//! erase passes [`FlashGuard`] as `Writer::Application { running }`: the running slot, the boot
//! stage and sector 0 are out of reach.

use g7250_layout::header::{self, Header};
use g7250_layout::{FLASH_WORD, FlashGuard, Op, Range, SECTOR_SIZE, Slot, Writer, crc32};
use g7250_store::{FlashIo, Store};
use sha2::{Digest, Sha256};

/// Block data size (Table M-1 `UPDATE_BLOCK`; `UPDATE_BEGIN` reply).
pub const BLOCK: u32 = 512;
/// An update with no update command other than `UPDATE_STATUS` for this long is abandoned (A1.8).
pub const ABANDON_MS: u64 = 10 * 60 * 1000;
/// Target hardware id of the G7250 (Table M-4 bytes 6-7).
pub const HARDWARE_ID: u16 = 0x00C8;
pub const COMPONENT_MAIN: u8 = 1;
pub const PACKAGE_VERSION: u8 = 1;

/// Why an update command is refused: the management status code (Table A1-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Refusal {
    Malformed = 17,
    WrongHardware = 51,
    Older = 52,
    BadCrc = 53,
    OutOfOrder = 54,
    VerifyFailed = 55,
    NotSupported = 56,
    /// During an unconfirmed trial boot: staging would erase the image to return to.
    UpdateInProgress = 58,
    TooLarge = 61,
    /// The package's image is linked for the running slot (0043).
    RunningSlot = 63,
}

/// `UPDATE_BEGIN` (flags and Table M-4), as plain values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Offer {
    pub foreign: bool,
    pub downgrade: bool,
    pub header_version: u8,
    pub component: u8,
    pub hardware_id: u16,
    pub min_hw_revision: u16,
    /// `header::version(major, minor, build)`.
    pub version: u32,
    pub length: u32,
    pub sha256: [u8; 32],
    pub signature_len: u16,
    /// Table M-4 byte 52: 0 either slot, 1 slot A, 2 slot B (0043).
    pub target_slot: u8,
}

/// Table M-4 byte 52 for a slot.
pub const fn slot_code(s: Slot) -> u8 {
    match s {
        Slot::A => 1,
        Slot::B => 2,
    }
}

/// What the running firmware is.
#[derive(Clone, Copy, Debug)]
pub struct Running {
    pub slot: Slot,
    pub version: u32,
    /// Hardware revision, 0 if not known (HIS-15).
    pub hw_revision: u16,
    /// The running image is an unconfirmed trial.
    pub trial: bool,
    /// RAM an application may put its initial stack in (as the boot stage checks it).
    pub app_ram: Range,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// No update since the start.
    Idle,
    Receiving,
    Verified,
    /// Aborted, abandoned, or a flash write failed: the staging area is discarded; the update
    /// state lasts until a restart (A1.8).
    Discarded,
    /// The boot record names the new image as the trial; a commanded reset follows.
    Activated,
}

pub struct Stager {
    phase: Phase,
    offer: Option<Offer>,
    received: u32,
    count: u32,
    last_ms: u64,
    /// Why the last UPDATE_VERIFY failed, for the log (A1.7 "every update step"): "missing
    /// blocks", "hash", "image" (the image and record checks; 0067 will name each).
    verify_failed: Option<&'static str>,
}

impl Default for Stager {
    fn default() -> Self {
        Self::new()
    }
}

impl Stager {
    pub const fn new() -> Self {
        Stager { phase: Phase::Idle, offer: None, received: 0, count: 0, last_ms: 0, verify_failed: None }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The update state of A1.8 holds (from an accepted `UPDATE_BEGIN` until a restart): no P2
    /// session, no arm (`MAINTENANCE`), STREAM_STATUS reason 10, register writes `BUSY`.
    pub fn in_progress(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// `UPDATE_STATUS` (Table M-1): state 0 idle, 1 receiving, 2 verified; blocks; count.
    pub fn status(&self) -> (u8, u32, u32) {
        let s = match self.phase {
            Phase::Receiving => 1,
            Phase::Verified | Phase::Activated => 2,
            Phase::Idle | Phase::Discarded => 0,
        };
        (s, self.received, self.count)
    }

    /// No update command other than `UPDATE_STATUS` for 10 min while in the update state (also
    /// after `UPDATE_ABORT` and after `UPDATE_VERIFY` without `UPDATE_ACTIVATE`): the caller
    /// discards, logs and restarts into the running image (A1.8; T-A1-34, T-A1-46).
    pub fn abandoned(&self, now: u64) -> bool {
        matches!(self.phase, Phase::Receiving | Phase::Verified | Phase::Discarded) && now.saturating_sub(self.last_ms) >= ABANDON_MS
    }

    /// `UPDATE_BEGIN`. Nothing is written here (U-2). Returns the block count. The same package
    /// again while receiving resumes the transfer (U-3); another one starts it anew.
    pub fn begin(&mut self, o: &Offer, run: &Running, now: u64) -> Result<u32, Refusal> {
        // During a trial boot, and after UPDATE_ABORT or an activation until the restart (the
        // update state remains): 58 (0043; A1.8 "After UPDATE_ABORT", 0063 addendum).
        if run.trial || matches!(self.phase, Phase::Discarded | Phase::Activated) {
            return Err(Refusal::UpdateInProgress);
        }
        // Magic and header version are framing (17 before authentication, payload.rs; 0063);
        // checked here again for an Offer made another way.
        if o.header_version != PACKAGE_VERSION || o.signature_len != 0 || o.length == 0 || o.target_slot > 2 {
            return Err(Refusal::Malformed);
        }
        if o.hardware_id != HARDWARE_ID || (o.min_hw_revision != 0 && o.min_hw_revision > run.hw_revision) {
            return Err(Refusal::WrongHardware);
        }
        // 56: a readable header with a component other than 1 (0063), or a foreign image (the
        // stock image path, A1.8a item 3, stays closed until P-SYS-5 is set); after 51, before
        // 63, 61 and 52.
        if o.component != COMPONENT_MAIN || o.foreign {
            return Err(Refusal::NotSupported);
        }
        // An image for the running slot cannot be staged (0043; T-A1-67). 0 ("either slot")
        // passes here; UPDATE_VERIFY still checks the image's load address.
        if o.target_slot == slot_code(run.slot) {
            return Err(Refusal::RunningSlot);
        }
        // Longer than the slot it is linked to (the staging slot): 61, after 51 and 63 (A1.8
        // "Slot size and the store", 0061 addendum). The slot size is its sector count x 128 KiB
        // (IDENTIFY tag 25, 0062): the slots end where the store begins, so no image reaches it.
        if o.length > run.slot.other().range().len() {
            return Err(Refusal::TooLarge);
        }
        if o.version < run.version && !o.downgrade {
            return Err(Refusal::Older);
        }
        // Answered with 0: the timer restarts (A1.8 "What restarts the timer"). A refused
        // command never restarts or extends it.
        self.last_ms = now;
        let count = o.length.div_ceil(BLOCK);
        let resume = self.phase == Phase::Receiving && self.offer == Some(*o);
        if !resume {
            self.offer = Some(*o);
            self.received = 0;
            self.count = count;
        }
        self.phase = Phase::Receiving;
        Ok(count)
    }

    /// `UPDATE_BLOCK` `n` with its data; `crc_ok` from the request. Returns blocks received.
    pub fn block<F: FlashIo>(&mut self, io: &mut F, run: &Running, n: u32, crc_ok: bool, data: &[u8], now: u64) -> Result<u32, Refusal> {
        // No transfer in progress: 54, before the CRC (53), nothing changes (0063).
        if self.phase != Phase::Receiving {
            return Err(Refusal::OutOfOrder);
        }
        let o = self.offer.ok_or(Refusal::OutOfOrder)?;
        if !crc_ok {
            return Err(Refusal::BadCrc);
        }
        if n >= self.count || n > self.received {
            return Err(Refusal::OutOfOrder);
        }
        let expect = if n + 1 == self.count { o.length - n * BLOCK } else { BLOCK };
        if data.len() as u32 != expect {
            return Err(Refusal::Malformed);
        }
        if n < self.received {
            self.last_ms = now; // answered with 0
            return Ok(self.received); // a repeat after a lost reply (resumable, U-3)
        }
        if self.write_block(io, run, n, data).is_err() {
            self.phase = Phase::Discarded;
            return Err(Refusal::VerifyFailed);
        }
        self.received += 1;
        self.last_ms = now; // answered with 0
        Ok(self.received)
    }

    fn write_block<F: FlashIo>(&mut self, io: &mut F, run: &Running, n: u32, data: &[u8]) -> Result<(), ()> {
        let w = Writer::Application { running: run.slot };
        let slot = run.slot.other().range();
        let at = slot.start + n * BLOCK;
        if (at - slot.start) % SECTOR_SIZE == 0 {
            FlashGuard::check(w, Op::Erase, at, SECTOR_SIZE).map_err(|_| ())?;
            io.erase_sector(at).map_err(|_| ())?;
        }
        let mut buf = [0xFFu8; BLOCK as usize];
        buf[..data.len()].copy_from_slice(data);
        let len = (data.len() as u32).div_ceil(FLASH_WORD) * FLASH_WORD;
        FlashGuard::check(w, Op::Program, at, len).map_err(|_| ())?;
        io.program(at, &buf[..len as usize]).map_err(|_| ())
    }

    /// `UPDATE_VERIFY` (U-4): SHA-256 over the staged image, compared with the package; then the
    /// image header (as the boot stage will check it). Returns the computed hash.
    pub fn verify<F: FlashIo>(&mut self, io: &mut F, run: &Running, now: u64) -> Result<[u8; 32], Refusal> {
        // No transfer in progress: 55, nothing changes (0063).
        if !matches!(self.phase, Phase::Receiving | Phase::Verified) {
            return Err(Refusal::VerifyFailed);
        }
        let o = match (self.phase, self.offer) {
            (Phase::Receiving | Phase::Verified, Some(o)) if self.received == self.count => o,
            _ => {
                self.verify_failed = Some("missing blocks");
                return Err(Refusal::VerifyFailed);
            }
        };
        let slot = run.slot.other().range();
        let Ok((hash, crc, head)) = scan(io, slot.start, o.length) else {
            self.verify_failed = Some("unreadable");
            return Err(Refusal::VerifyFailed);
        };
        if hash != o.sha256 {
            self.verify_failed = Some("hash");
            return Err(Refusal::VerifyFailed);
        }
        let ok = check_head(&head, crc, &o, slot, run.app_ram);
        if !ok {
            self.verify_failed = Some("image");
            return Err(Refusal::VerifyFailed);
        }
        self.verify_failed = None;
        self.phase = Phase::Verified;
        self.last_ms = now; // answered with 0
        Ok(hash)
    }

    /// Why the last verify of a transfer failed (`None` after a good one): for the log.
    pub fn verify_failure(&self) -> Option<&'static str> {
        self.verify_failed
    }

    /// The staged image's version (from the package header), for the log.
    pub fn staged_version(&self) -> Option<u32> {
        self.offer.map(|o| o.version)
    }

    /// `UPDATE_ACTIVATE` (U-5): only after a good verify. Writes the boot record (one record,
    /// atomic); the caller then makes the commanded reset (A1.9).
    pub fn activate<F: FlashIo>(&mut self, store: &mut Store<F>, run: &Running, now: u64) -> Result<(), Refusal> {
        // No transfer in progress (or not verified): 55, nothing changes (0063).
        if self.phase != Phase::Verified {
            return Err(Refusal::VerifyFailed);
        }
        crate::boot::activate(store, run.slot).map_err(|_| Refusal::VerifyFailed)?;
        self.last_ms = now; // answered with 0
        self.phase = Phase::Activated;
        Ok(())
    }

    /// `UPDATE_ABORT`: the staging area is discarded (the slot keeps whatever was written; it is
    /// never activated). The update state lasts until a restart; the timer runs from this reply
    /// and ends in the restart into the running image (A1.8 "After UPDATE_ABORT"). A second
    /// UPDATE_ABORT is answered 0 and has no effect, the timer included (0063 addendum).
    /// Returns whether this abort had an effect.
    pub fn abort(&mut self, now: u64) -> bool {
        if self.phase == Phase::Discarded {
            return false;
        }
        self.last_ms = now;
        if self.phase != Phase::Activated {
            self.phase = Phase::Discarded;
            self.offer = None;
            self.received = 0;
            self.count = 0;
        }
        true
    }

    /// Whether `ROLLBACK` may go ahead as far as the update is concerned: only with a confirmed
    /// running image, not during a trial boot or in the update state (staging may have
    /// overwritten the other slot): 58 (A1.8 "ROLLBACK", 0064 point 3.1; "After UPDATE_ABORT",
    /// 0063 addendum). The caller then checks the other slot (55).
    pub fn rollback_allowed(&self, run: &Running) -> Result<(), Refusal> {
        if run.trial || self.in_progress() { Err(Refusal::UpdateInProgress) } else { Ok(()) }
    }
}

/// The first bytes of an image the checks need: vectors and the header.
const HEAD: usize = header::HEADER_LEN + g7250_layout::HEADER_OFFSET as usize;

/// One pass over the staged image: SHA-256, the image CRC (header CRC field as zero), the head.
fn scan<F: FlashIo>(io: &mut F, start: u32, length: u32) -> Result<([u8; 32], u32, [u8; HEAD]), F::Error> {
    let crc_at = g7250_layout::HEADER_OFFSET as usize + header::CRC_FIELD;
    let mut sha = Sha256::new();
    let mut crc = crc32::init();
    let mut head = [0u8; HEAD];
    let mut buf = [0u8; BLOCK as usize];
    let mut off = 0usize;
    while off < length as usize {
        let n = (length as usize - off).min(buf.len());
        io.read(start + off as u32, &mut buf[..n])?;
        sha.update(&buf[..n]);
        if off < HEAD {
            let k = (HEAD - off).min(n);
            head[off..off + k].copy_from_slice(&buf[..k]);
        }
        if off + n <= crc_at || off >= crc_at + 4 {
            crc = crc32::update(crc, &buf[..n]);
        } else {
            for (i, b) in buf[..n].iter().enumerate() {
                let v = if (crc_at..crc_at + 4).contains(&(off + i)) { 0 } else { *b };
                crc = crc32::update(crc, &[v]);
            }
        }
        off += n;
    }
    Ok((sha.finalize().into(), crc32::finish(crc), head))
}

/// The checks of `header::validate`, on the head and the streamed CRC.
fn check_head(head: &[u8; HEAD], crc: u32, o: &Offer, slot: Range, ram: Range) -> bool {
    let Some(h) = Header::parse(&head[g7250_layout::HEADER_OFFSET as usize..]) else { return false };
    let sp = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
    let pc = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
    h.magic == header::MAGIC
        && h.header_version == header::HEADER_VERSION
        && h.header_len as usize == header::HEADER_LEN
        && h.hw_id == header::HW_ID_G7250_MAIN
        && h.kind == header::Kind::Application as u8
        && h.load_addr == slot.start
        && h.length == o.length
        && h.crc == crc
        && h.fw_version == o.version
        && sp > ram.start
        && sp <= ram.end
        && pc & 1 == 1
        && slot.contains_span(pc & !1, 2)
}
