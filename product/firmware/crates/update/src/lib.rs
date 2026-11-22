// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Firmware update of the G7250 main firmware (spec A1.8, U-1 to U-10; 0028 Decision 3).
//!
//! - [`boot`]: the boot record as a record of the store, the boot stage's choice with the record
//!   it writes before a start (trial attempt, rollback), and the application's records
//!   (activation, confirmation, `ROLLBACK`).
//! - [`image`]: the image check (0067), the same for `UPDATE_VERIFY` and for a start.
//! - [`stage`]: the transfer into the other slot (the staging area): checks of `UPDATE_BEGIN`,
//!   numbered blocks with a CRC each, the SHA-256 and image check of `UPDATE_VERIFY`, and the
//!   activation.
//!
//! Power-loss safety (U-5, U-6). The running image is never written: the stager writes only the
//! other slot, through [`FlashGuard`](g7250_layout::FlashGuard) as `Writer::Application`. The
//! switch is one store record (the store is power-safe, `g7250-store`): before it is complete the
//! old record holds and the old image starts; after it the new image starts as a trial. A trial
//! uses up its one attempt in the boot stage before the jump, so a trial that never confirms
//! itself (crash, hang, power loss) is rolled back at the next start. The tests below cut power at
//! every flash word and erase of an update, a trial start, a confirmation and a rollback.
#![cfg_attr(not(test), no_std)]

pub mod boot;
pub mod image;
pub mod stage;

#[cfg(test)]
mod sim;
#[cfg(test)]
mod tests;
