// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The update commands of the management channel (A1.8; Table M-1) against an [`Updater`], which
//! the board implements with `g7250-update` and its flash (host tests use a model).

use g7250_proto::mgmt::Status;
use g7250_proto::mgmt::payload::Package;

/// The staging and boot-record side of an update. Every call that can fail returns the status
/// of Table A1-3 for the reply.
pub trait Updater {
    /// `UPDATE_BEGIN`: the checks of A1.8 before anything is written; returns the block count.
    fn begin(&mut self, foreign: bool, downgrade: bool, pkg: &Package, now: u64) -> Result<u32, Status>;
    /// `UPDATE_BLOCK`; returns the blocks received so far. A write failure discards the
    /// transfer and returns 55 (0043).
    fn block(&mut self, number: u32, crc_ok: bool, data: &[u8], now: u64) -> Result<u32, Status>;
    /// `UPDATE_VERIFY`: the computed SHA-256.
    fn verify(&mut self, now: u64) -> Result<[u8; 32], Status>;
    /// For the log after UPDATE_VERIFY (A1.7 "every update step"): the staging slot (1 A, 2 B)
    /// and the staged version after a good verify; after a failed one, the failed check
    /// ("missing-blocks", "hash", a word of the image check, or "header"; 0067).
    fn verify_note(&self) -> (u8, u32, Option<&'static str>) {
        (0, 0, None)
    }
    /// The last `UPDATE_BLOCK` or `UPDATE_VERIFY` made room in the store (for the log, once).
    fn room_made(&mut self) -> bool {
        false
    }
    /// `UPDATE_ACTIVATE`: the boot record names the staged image as the trial (U-5).
    fn activate(&mut self, now: u64) -> Result<(), Status>;
    /// `UPDATE_ABORT`; false if it had no effect (a second abort: 0063 addendum).
    fn abort(&mut self, now: u64) -> bool;
    /// `UPDATE_STATUS`: state, blocks received, block count.
    fn status(&self) -> (u8, u32, u32);
    /// `ROLLBACK` (A1.8; 0064 point 3): with a confirmed running image (else 58) and a good
    /// image in the other slot (else 55), the boot record names the other slot as a trial.
    fn rollback(&mut self, now: u64) -> Result<Rollback, Status>;
    /// The update state of A1.8 holds (from `UPDATE_BEGIN` until a restart), or the running image
    /// is an unconfirmed trial.
    fn in_progress(&self) -> bool;
    /// An `UPDATE_BEGIN` was accepted since the start: there is an update state to leave
    /// (A1.8; 0074). False in a trial image that has not begun one.
    fn begun(&self) -> bool;
}

/// What an accepted `ROLLBACK` goes from and to, for its log line: slot (1 A, 2 B) and the
/// image version ([31:24] major, [23:16] minor, [15:0] build).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rollback {
    pub from: (u8, u32),
    pub to: (u8, u32),
}

/// No update path (host tests of other commands).
pub struct NoUpdate;

impl Updater for NoUpdate {
    fn begin(&mut self, _: bool, _: bool, _: &Package, _: u64) -> Result<u32, Status> {
        Err(Status::NotSupported)
    }
    fn block(&mut self, _: u32, _: bool, _: &[u8], _: u64) -> Result<u32, Status> {
        Err(Status::NotSupported)
    }
    fn verify(&mut self, _: u64) -> Result<[u8; 32], Status> {
        Err(Status::NotSupported)
    }
    fn activate(&mut self, _: u64) -> Result<(), Status> {
        Err(Status::NotSupported)
    }
    fn abort(&mut self, _: u64) -> bool {
        false
    }
    fn status(&self) -> (u8, u32, u32) {
        (0, 0, 0)
    }
    fn rollback(&mut self, _: u64) -> Result<Rollback, Status> {
        Err(Status::NotSupported)
    }
    fn in_progress(&self) -> bool {
        false
    }
    fn begun(&self) -> bool {
        false
    }
}
