// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Transfer of a new image into the other slot (A1.8, U-2 to U-5, U-8; Table M-1, M-4).
//!
//! The package carries one image: a sealed application image linked for the slot it is staged
//! in (the other slot; `IDENTIFY` tag 23 tells the host which slot runs). Table M-4 byte 52
//! names that slot; a package for the running slot gets 63 at `UPDATE_BEGIN` (0043). `UPDATE_VERIFY` checks, in
//! order (0067): the SHA-256 of the received bytes, the image check ([`crate::image`], over the
//! length the image record gives), and the record against the package header.
//!
//! Blocks arrive in order the first time (a gap gets 54). A block sent again as a new request is
//! written again, replacing what was received (0067): the same bytes need no write (except the
//! first block of a sector, which starts the sector anew); other bytes need an erase, and the other blocks of that 128 KiB sector then count as not received. A slot
//! sector is erased when the first block in it arrives, so a request does at most one erase
//! (0069). Every program and erase passes [`FlashGuard`] as `Writer::Application { running }`: the running slot, the boot
//! stage and sector 0 are out of reach.

use crate::image;
use g7250_layout::header;
use g7250_layout::{FLASH_WORD, FlashGuard, Op, Range, SECTOR_SIZE, SLOT_A, SLOT_B, Slot, Writer};
use g7250_store::{FlashIo, Store};
use sha2::{Digest, Sha256};

/// Block data size (Table M-1 `UPDATE_BLOCK`; `UPDATE_BEGIN` reply).
pub const BLOCK: u32 = 512;
/// Blocks in one 128 KiB flash sector, sectors and blocks in a slot.
const PER_SECTOR: u32 = SECTOR_SIZE / BLOCK;
const SECTORS: usize = (SLOT_A.len() / SECTOR_SIZE) as usize;
const BLOCKS: usize = (SLOT_A.len() / BLOCK) as usize;
const _: () = assert!(SLOT_A.len() == SLOT_B.len() && SLOT_A.len() % SECTOR_SIZE == 0);
/// Flash bytes of one staging mark record in the store.
const MARK: u32 = FLASH_WORD;
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
    /// Nothing taken: room made in the store (0067 addendum).
    RoomMade = 64,
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
    /// Aborted or abandoned: the staging area is discarded; the update state lasts until a
    /// restart, and UPDATE_BEGIN gets 58 (A1.8 "After UPDATE_ABORT").
    Discarded,
    /// A write of the staging slot failed: the transfer is discarded and the update state lasts
    /// until a restart, as after UPDATE_ABORT, but a new UPDATE_BEGIN is accepted (A1.8 "Write
    /// failure"; 0043).
    Failed,
    /// The boot record names the new image as the trial; a commanded reset follows.
    Activated,
}

pub struct Stager {
    phase: Phase,
    offer: Option<Offer>,
    /// The blocks received, one bit each.
    received: [u32; BLOCKS / 32],
    /// The block after the highest one received in this transfer: the next new one.
    next: u32,
    /// The slot sectors erased for this transfer.
    prepared: [bool; SECTORS],
    /// The staging mark of this transfer is set in the store (0067).
    marked: bool,
    /// The store was compacted for a mark, for the log.
    room_made: bool,
    count: u32,
    last_ms: u64,
    /// Why the last UPDATE_VERIFY failed, for the log (A1.7 "every update step"): "missing-blocks",
    /// "hash", a word of the image check, or "header" (0067).
    verify_failed: Option<&'static str>,
}

impl Default for Stager {
    fn default() -> Self {
        Self::new()
    }
}

impl Stager {
    pub const fn new() -> Self {
        Stager { phase: Phase::Idle, offer: None, received: [0; BLOCKS / 32], next: 0, prepared: [false; SECTORS], marked: false, room_made: false, count: 0, last_ms: 0, verify_failed: None }
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
            Phase::Idle | Phase::Discarded | Phase::Failed => 0,
        };
        (s, self.received(), self.count)
    }

    /// Blocks received.
    fn received(&self) -> u32 {
        self.received.iter().map(|w| w.count_ones()).sum()
    }

    fn has(&self, n: u32) -> bool {
        self.received[(n / 32) as usize] & (1 << (n % 32)) != 0
    }

    /// Forget what was received: a new transfer, or none.
    fn clear_blocks(&mut self) {
        self.received = [0; BLOCKS / 32];
        self.next = 0;
        self.prepared = [false; SECTORS];
    }

    /// No update command other than `UPDATE_STATUS` for 10 min while in the update state (also
    /// after `UPDATE_ABORT` and after `UPDATE_VERIFY` without `UPDATE_ACTIVATE`): the caller
    /// discards, logs and restarts into the running image (A1.8; T-A1-34, T-A1-46).
    pub fn abandoned(&self, now: u64) -> bool {
        self.abandon_due().is_some_and(|due| now >= due)
    }

    /// When the update is abandoned if no update command comes first, `None` without one in
    /// progress: the caller wakes then, so that the limit acts within 100 ms (0073 point 2).
    pub fn abandon_due(&self) -> Option<u64> {
        matches!(self.phase, Phase::Receiving | Phase::Verified | Phase::Discarded | Phase::Failed).then_some(self.last_ms + ABANDON_MS)
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
            self.clear_blocks();
            self.count = count;
        }
        self.phase = Phase::Receiving;
        Ok(count)
    }

    /// `UPDATE_BLOCK` with the staging mark: set in the store before the first block of a
    /// transfer (so before the first erase), and again before a block that cancels a good verify
    /// (0067). A mark that cannot be written is a write failure (0043). UPDATE_BEGIN makes room
    /// for many marks; should it run out, the block makes room again (its one erase, 0069), takes
    /// nothing and gets 64: the transfer goes on, and the block sent again is taken (0067
    /// addendum). A 64 restarts the 10-minute timer.
    pub fn block_marked<S: FlashIo, F: FlashIo>(
        &mut self,
        store: Option<&mut Store<S>>,
        io: &mut F,
        run: &Running,
        n: u32,
        crc_ok: bool,
        data: &[u8],
        now: u64,
    ) -> Result<u32, Refusal> {
        self.accepts(n, crc_ok, data.len() as u32)?;
        let set = store.map(|s| {
            // Room for the mark to set, if it is not, and for the verify to clear it.
            if s.free() < MARK + if self.marked { 0 } else { MARK } {
                return s.compact().map(|()| false);
            }
            if !self.marked {
                crate::boot::set_staging(s, Some(run.slot.other()))?;
            }
            Ok(true)
        });
        match set {
            Some(Ok(true)) => self.marked = true,
            Some(Ok(false)) => {
                self.room_made = true;
                self.last_ms = now;
                return Err(Refusal::RoomMade);
            }
            _ => {
                self.phase = Phase::Failed;
                return Err(Refusal::VerifyFailed);
            }
        }
        self.block(io, run, n, crc_ok, data, now)
    }

    /// `UPDATE_VERIFY` with the staging mark: a good verify clears it before it counts (0067),
    /// making room for that record if it must (its one erase; 0067 addendum). A record that
    /// cannot be written is a write failure: the transfer is discarded (0043).
    pub fn verify_marked<S: FlashIo, F: FlashIo>(&mut self, store: Option<&mut Store<S>>, io: &mut F, run: &Running, now: u64) -> Result<[u8; 32], Refusal> {
        let hash = self.verify(io, run, now)?;
        if self.marked {
            let mut made = false;
            let cleared = store.map(|s| {
                if s.free() < MARK {
                    s.compact()?;
                    made = true;
                }
                crate::boot::set_staging(s, None)
            });
            self.room_made |= made;
            match cleared {
                Some(Ok(())) => self.marked = false,
                _ => {
                    self.phase = Phase::Failed;
                    return Err(Refusal::VerifyFailed);
                }
            }
        }
        Ok(hash)
    }

    /// Whether an `UPDATE_BLOCK` is taken (its length if it is).
    fn accepts(&self, n: u32, crc_ok: bool, len: u32) -> Result<(), Refusal> {
        // No transfer in progress: 54, before the CRC (53), nothing changes (0063).
        if !matches!(self.phase, Phase::Receiving | Phase::Verified) {
            return Err(Refusal::OutOfOrder);
        }
        let o = self.offer.ok_or(Refusal::OutOfOrder)?;
        if !crc_ok {
            return Err(Refusal::BadCrc);
        }
        if n >= self.count || n > self.next {
            return Err(Refusal::OutOfOrder);
        }
        let expect = if n + 1 == self.count { o.length - n * BLOCK } else { BLOCK };
        if len != expect {
            return Err(Refusal::Malformed);
        }
        Ok(())
    }

    /// `UPDATE_BLOCK` `n` with its data; `crc_ok` from the request. Returns blocks received.
    pub(crate) fn block<F: FlashIo>(&mut self, io: &mut F, run: &Running, n: u32, crc_ok: bool, data: &[u8], now: u64) -> Result<u32, Refusal> {
        self.accepts(n, crc_ok, data.len() as u32)?;
        if self.put_block(io, run, n, data).is_err() {
            self.phase = Phase::Failed;
            return Err(Refusal::VerifyFailed);
        }
        // An accepted block cancels a good verify (0067).
        self.phase = Phase::Receiving;
        self.last_ms = now; // answered with 0
        Ok(self.received())
    }

    /// Block `n` into the slot, new or again (0067 "Blocks sent again").
    fn put_block<F: FlashIo>(&mut self, io: &mut F, run: &Running, n: u32, data: &[u8]) -> Result<(), ()> {
        let sector = (n / PER_SECTOR) as usize;
        if self.has(n) {
            let mut b = [0u8; BLOCK as usize];
            let b = &mut b[..data.len()];
            // The first block of a sector always starts the sector anew, so that every block
            // sent again in order (0067: what a host does after a failed verify) replaces all:
            // if it were skipped as unchanged, a later changed block would erase it.
            if n % PER_SECTOR != 0 && io.read(run.slot.other().range().start + n * BLOCK, b).is_ok() && b == data {
                return Ok(()); // the slot holds these bytes already
            }
            self.prepared[sector] = false;
        }
        if !self.prepared[sector] {
            let first = n / PER_SECTOR * PER_SECTOR;
            for k in first..first + PER_SECTOR {
                self.received[(k / 32) as usize] &= !(1 << (k % 32));
            }
            self.erase(io, run, sector)?;
            self.prepared[sector] = true;
        }
        self.write_block(io, run, n, data)?;
        self.received[(n / 32) as usize] |= 1 << (n % 32);
        self.next = self.next.max(n + 1);
        Ok(())
    }

    fn erase<F: FlashIo>(&mut self, io: &mut F, run: &Running, sector: usize) -> Result<(), ()> {
        let at = run.slot.other().range().start + sector as u32 * SECTOR_SIZE;
        FlashGuard::check(Writer::Application { running: run.slot }, Op::Erase, at, SECTOR_SIZE).map_err(|_| ())?;
        io.erase_sector(at).map_err(|_| ())
    }

    fn write_block<F: FlashIo>(&mut self, io: &mut F, run: &Running, n: u32, data: &[u8]) -> Result<(), ()> {
        let w = Writer::Application { running: run.slot };
        let at = run.slot.other().range().start + n * BLOCK;
        let mut buf = [0xFFu8; BLOCK as usize];
        buf[..data.len()].copy_from_slice(data);
        let len = (data.len() as u32).div_ceil(FLASH_WORD) * FLASH_WORD;
        FlashGuard::check(w, Op::Program, at, len).map_err(|_| ())?;
        io.program(at, &buf[..len as usize]).map_err(|_| ())
    }

    /// `UPDATE_VERIFY` (U-4; 0067): (1) the SHA-256 of the received bytes, (2) the image check of
    /// the staging slot, as at a start, (3) the image record against the package header; the
    /// first that fails is named for the log. Returns the computed hash.
    pub(crate) fn verify<F: FlashIo>(&mut self, io: &mut F, run: &Running, now: u64) -> Result<[u8; 32], Refusal> {
        // No transfer in progress: 55, nothing changes (0063).
        if !matches!(self.phase, Phase::Receiving | Phase::Verified) {
            self.verify_failed = None;
            return Err(Refusal::VerifyFailed);
        }
        match self.checks(io, run) {
            Ok(hash) => {
                self.verify_failed = None;
                self.phase = Phase::Verified;
                self.last_ms = now; // answered with 0
                Ok(hash)
            }
            Err(word) => {
                self.verify_failed = Some(word);
                self.phase = Phase::Receiving; // the transfer goes on; ACTIVATE needs a good verify
                Err(Refusal::VerifyFailed)
            }
        }
    }

    /// The three checks of a verify; the word of the first that fails.
    fn checks<F: FlashIo>(&self, io: &mut F, run: &Running) -> Result<[u8; 32], &'static str> {
        let o = match self.offer {
            Some(o) if self.received() == self.count => o,
            _ => return Err("missing-blocks"),
        };
        let other = run.slot.other();
        let hash = sha256(io, other.range().start, o.length).map_err(|_| image::Check::Unreadable.word())?;
        if hash != o.sha256 {
            return Err("hash");
        }
        let h = image::check(io, header::Kind::Application, other.range(), run.app_ram, &mut || {}).map_err(|c| c.word())?;
        let slot_ok = o.target_slot == 0 || o.target_slot == slot_code(other);
        if h.length != o.length || h.fw_version != o.version || !slot_ok {
            return Err("header");
        }
        Ok(hash)
    }

    /// A block or a verify made room in the store since the last call (for the log).
    pub fn take_room_made(&mut self) -> bool {
        core::mem::take(&mut self.room_made)
    }

    /// The staging mark is set: blocks were taken and no good verify ended them (0067).
    pub fn staging_marked(&self) -> bool {
        self.marked
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
            self.clear_blocks();
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

/// SHA-256 of the received bytes.
fn sha256<F: FlashIo>(io: &mut F, start: u32, length: u32) -> Result<[u8; 32], F::Error> {
    let mut sha = Sha256::new();
    let mut buf = [0u8; BLOCK as usize];
    let mut off = 0;
    while off < length {
        let n = (length - off).min(BLOCK) as usize;
        io.read(start + off, &mut buf[..n])?;
        sha.update(&buf[..n]);
        off += n as u32;
    }
    Ok(sha.finalize().into())
}
