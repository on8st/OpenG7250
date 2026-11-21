// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The boot record in the store (kind `BootRecord`, payload `select::BootRecord::encode`).
//!
//! Who writes what:
//! - the boot stage ([`boot_stage`]): one trial attempt used up before a trial start; a rollback
//!   when a trial is out of attempts and unconfirmed (U-6);
//! - the application: the activation ([`activate`], U-5), the confirmation ([`confirm`], U-6)
//!   and `ROLLBACK` ([`rollback`]).

use g7250_layout::Slot;
use g7250_layout::select::{self, BootRecord, Choice, RECORD_LEN};
use g7250_store::{Error, FlashIo, Kind, Store};

/// Format version of the record payload in the store.
pub const RECORD_VERSION: u8 = 1;

/// The newest boot record, if any (none: slot A, `select::choose`).
pub fn load<F: FlashIo>(s: &mut Store<F>) -> Result<Option<BootRecord>, Error<F::Error>> {
    let mut b = [0u8; RECORD_LEN];
    Ok(match s.get(Kind::BootRecord, &mut b)? {
        Some((_, n)) if n == RECORD_LEN => BootRecord::parse(&b),
        _ => None,
    })
}

pub fn save<F: FlashIo>(s: &mut Store<F>, r: &BootRecord) -> Result<(), Error<F::Error>> {
    s.put(Kind::BootRecord, RECORD_VERSION, &r.encode())
}

fn next_seq(r: Option<BootRecord>) -> u32 {
    r.map_or(1, |r| r.seq.wrapping_add(1))
}

/// The boot stage's decision: choose a slot and write the record that goes with the choice
/// before the start. A trial is started only once its attempt is recorded; if that write fails,
/// the active slot starts instead (a trial that cannot be counted could restart for ever).
pub fn boot_stage<F: FlashIo>(s: &mut Store<F>, valid: impl Fn(Slot) -> bool) -> Choice {
    let rec = load(s).ok().flatten();
    let choice = select::choose(rec, &valid);
    if let Some(new) = select::record_for(rec, choice) {
        if save(s, &new).is_err() {
            if let (Choice::Trial(_), Some(r)) = (choice, rec) {
                return select::choose(Some(BootRecord { trial: None, ..r }), &valid);
            }
        }
    }
    choice
}

/// Update state for `IDENTIFY` tag 10 (I-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UpdateState {
    Normal = 0,
    /// The running image is an unconfirmed trial: RX works, an arm gets `MAINTENANCE` (A1.8).
    Trial = 1,
    /// The last trial was rolled back (logged at start, U-6).
    RolledBack = 2,
}

pub fn state(rec: Option<BootRecord>, running: Slot) -> UpdateState {
    match rec {
        Some(r) if r.trial == Some(running) && !r.confirmed => UpdateState::Trial,
        Some(r) if r.rolled_back => UpdateState::RolledBack,
        _ => UpdateState::Normal,
    }
}

/// `UPDATE_ACTIVATE` (U-5): the other slot becomes the trial, the running slot stays the one to
/// return to. One record: atomic.
pub fn activate<F: FlashIo>(s: &mut Store<F>, running: Slot) -> Result<(), Error<F::Error>> {
    let rec = load(s)?;
    let r = BootRecord { active: running, trial: Some(running.other()), attempts_left: 1, confirmed: false, seq: next_seq(rec), rolled_back: false };
    save(s, &r)
}

/// Self-confirmation of a trial image (U-6). Returns true if a record was written; false if the
/// running image is not an unconfirmed trial (nothing to do).
pub fn confirm<F: FlashIo>(s: &mut Store<F>, running: Slot) -> Result<bool, Error<F::Error>> {
    let rec = load(s)?;
    match rec {
        Some(r) if r.trial == Some(running) && !r.confirmed => {
            let n = BootRecord { active: running, trial: None, attempts_left: 0, confirmed: true, seq: next_seq(rec), rolled_back: false };
            save(s, &n)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

/// `ROLLBACK` (A1.8 "ROLLBACK"; 0064 point 3): the other slot starts as a trial (U-6), the
/// running slot stays the one to return to, so a rollback target that fails (no confirmation
/// within 60 s, a reset before it) returns to the image that sent ROLLBACK. The caller checks
/// that the running image is confirmed and the other slot holds a good image, then resets
/// (commanded reset, A1.9).
pub fn rollback<F: FlashIo>(s: &mut Store<F>, running: Slot) -> Result<(), Error<F::Error>> {
    let rec = load(s)?;
    let r = BootRecord { active: running, trial: Some(running.other()), attempts_left: 1, confirmed: false, seq: next_seq(rec), rolled_back: false };
    save(s, &r)
}
