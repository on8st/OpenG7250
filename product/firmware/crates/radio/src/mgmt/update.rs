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
    /// `UPDATE_ACTIVATE`: the boot record names the staged image as the trial (U-5).
    fn activate(&mut self, now: u64) -> Result<(), Status>;
    /// `UPDATE_ABORT`; false if it had no effect (a second abort: 0063 addendum).
    fn abort(&mut self, now: u64) -> bool;
    /// `UPDATE_STATUS`: state, blocks received, block count.
    fn status(&self) -> (u8, u32, u32);
    /// `ROLLBACK`: the other slot (valid, confirmed before) becomes the active image.
    fn rollback(&mut self, now: u64) -> Result<(), Status>;
    /// The update state of A1.8 holds (from `UPDATE_BEGIN` until a restart), or the running image
    /// is an unconfirmed trial.
    fn in_progress(&self) -> bool;
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
    fn rollback(&mut self, _: u64) -> Result<(), Status> {
        Err(Status::NotSupported)
    }
    fn in_progress(&self) -> bool {
        false
    }
}
