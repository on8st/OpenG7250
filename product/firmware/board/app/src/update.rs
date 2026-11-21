// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Board side of the firmware update (A1.8): the stager of `g7250-update` on the raw flash, the
//! boot record in the store, the trial boot's self-confirmation (U-6).
//!
//! Self-confirmation (A1.8; 0044): a trial image confirms itself once it has run at least
//! [`CONFIRM_AFTER_MS`] with the network up (CH394 initialised, management socket open: the main
//! loop only runs then), the main loop running (the check is in it), the watchdog serviced (the
//! watchdog task refreshed it within the last second) and no class 2 fault ([`class2_fault`]). No
//! host is needed. If it has not confirmed [`TRIAL_LIMIT_MS`] (60 s) after its start, the main
//! task (not the network loop, so that a network that never comes up cannot stop it) makes a
//! commanded reset; the boot stage finds the attempt used and starts the previous image. A reset
//! for any other reason before the confirmation (crash, watchdog, power loss) does the same: the
//! attempt was recorded before the jump. Until confirmed, arms are refused with `MAINTENANCE`.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static CLASS2: AtomicBool = AtomicBool::new(false);
static TRIAL_PENDING: AtomicBool = AtomicBool::new(false);
static WDG_PET_MS: AtomicU32 = AtomicU32::new(0);

/// A class 2 fault was seen in this run (0007 sub-decision 5): the safe state found not held, a
/// watchdog reset seen at start-up. A trial image with one never confirms itself.
pub fn set_class2_fault() {
    CLASS2.store(true, Ordering::Relaxed);
}
pub fn class2_fault() -> bool {
    CLASS2.load(Ordering::Relaxed)
}
/// The watchdog task refreshed the IWDG at `now_ms`.
pub fn watchdog_petted(now_ms: u64) {
    WDG_PET_MS.store(now_ms as u32, Ordering::Relaxed);
}
fn watchdog_serviced(now_ms: u64) -> bool {
    (now_ms as u32).wrapping_sub(WDG_PET_MS.load(Ordering::Relaxed)) < 1000
}
/// The running image is a trial that has not confirmed itself.
pub fn trial_pending() -> bool {
    TRIAL_PENDING.load(Ordering::Relaxed)
}
/// The criterion of 0044 at `now_ms`, checked from the main loop, which runs from
/// `network_up_ms` on (CH394 initialised, management socket open).
pub fn may_confirm(now_ms: u64, network_up_ms: u64) -> bool {
    now_ms.saturating_sub(network_up_ms) >= CONFIRM_AFTER_MS
        && watchdog_serviced(now_ms)
        && !class2_fault()
        && g7250_board::safe::holds()
}

use crate::mgmt_init::NvStore;
use g7250_board::flash::RawFlash;
use g7250_layout::header::Kind as ImageKind;
use g7250_layout::{Range, Slot, Writer};
use g7250_proto::mgmt::Status;
use g7250_proto::mgmt::payload::Package;
use g7250_radio::mgmt::Updater;
use g7250_update::boot::{self, UpdateState};
use g7250_update::stage::{Offer, Refusal, Running, Stager};

/// Run time after which a trial image confirms itself.
pub const CONFIRM_AFTER_MS: u64 = 10_000;
/// A trial must confirm itself within 60 s of its start (U-6).
pub const TRIAL_LIMIT_MS: u64 = 60_000;

const APP_RAM: Range = Range::new(0x2400_0000, 0x2402_0000);

pub struct BoardUpdate {
    pub stager: Stager,
    pub run: Running,
    /// Update state at start (IDENTIFY tag 10).
    pub state: UpdateState,
}

impl BoardUpdate {
    /// From the boot record at start.
    pub fn new(store: Option<&mut NvStore>, slot: Slot) -> Self {
        let rec = store.and_then(|s| boot::load(s).ok().flatten());
        let state = boot::state(rec, slot);
        let run = Running { slot, version: g7250_board::FW_VERSION, hw_revision: 0, trial: state == UpdateState::Trial, app_ram: APP_RAM };
        TRIAL_PENDING.store(run.trial, Ordering::Relaxed);
        BoardUpdate { stager: Stager::new(), run, state }
    }

    /// The trial's self-confirmation (U-6). Returns true when the record was written.
    pub fn confirm(&mut self, store: Option<&mut NvStore>) -> bool {
        let Some(s) = store else { return false };
        match boot::confirm(s, self.run.slot) {
            Ok(_) => {
                TRIAL_PENDING.store(false, Ordering::Relaxed);
                self.run.trial = false;
                self.state = UpdateState::Normal;
                true
            }
            Err(_) => false,
        }
    }
}

fn status(r: Refusal) -> Status {
    Status::from_u16(r as u16).unwrap_or(Status::VerificationFailed)
}

fn other_slot_valid(slot: Slot) -> bool {
    let r = slot.other().range();
    // SAFETY: on-chip flash, always mapped and readable.
    let image = unsafe { core::slice::from_raw_parts(r.start as *const u8, r.len() as usize) };
    g7250_layout::header::validate(image, ImageKind::Application, r, APP_RAM).is_ok()
}

/// The [`Updater`] of one management request: the stager and the store.
pub struct Port<'a> {
    pub u: &'a mut BoardUpdate,
    pub store: Option<&'a mut NvStore>,
}

impl Updater for Port<'_> {
    fn begin(&mut self, foreign: bool, downgrade: bool, p: &Package, now: u64) -> Result<u32, Status> {
        let o = Offer {
            foreign,
            downgrade,
            header_version: p.header_version,
            component: p.component,
            hardware_id: p.hardware_id,
            min_hw_revision: p.min_hw_revision,
            version: g7250_layout::header::version(p.version.major, p.version.minor, p.version.build),
            length: p.length,
            sha256: p.sha256,
            signature_len: p.signature_len,
            target_slot: p.target_slot,
        };
        self.u.stager.begin(&o, &self.u.run, now).map_err(status)
    }

    fn block(&mut self, n: u32, crc_ok: bool, data: &[u8], now: u64) -> Result<u32, Status> {
        let mut io = RawFlash::new(Writer::Application { running: self.u.run.slot });
        self.u.stager.block(&mut io, &self.u.run, n, crc_ok, data, now).map_err(status)
    }

    fn verify(&mut self, now: u64) -> Result<[u8; 32], Status> {
        let mut io = RawFlash::new(Writer::Application { running: self.u.run.slot });
        self.u.stager.verify(&mut io, &self.u.run, now).map_err(status)
    }

    fn activate(&mut self, now: u64) -> Result<(), Status> {
        let Some(s) = self.store.as_deref_mut() else { return Err(Status::VerificationFailed) };
        self.u.stager.activate(s, &self.u.run, now).map_err(status)
    }

    fn abort(&mut self, now: u64) {
        self.u.stager.abort(now);
    }

    fn status(&self) -> (u8, u32, u32) {
        self.u.stager.status()
    }

    fn rollback(&mut self, _now: u64) -> Result<(), Status> {
        // Not during an update (staging may have overwritten the other slot), nor without a
        // valid image there.
        if self.u.stager.in_progress() || !other_slot_valid(self.u.run.slot) {
            return Err(Status::VerificationFailed);
        }
        let Some(s) = self.store.as_deref_mut() else { return Err(Status::VerificationFailed) };
        boot::rollback(s, self.u.run.slot).map_err(|_| Status::VerificationFailed)
    }

    fn in_progress(&self) -> bool {
        self.u.stager.in_progress() || self.u.run.trial
    }
}
