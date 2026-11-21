// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Slot choice of the boot stage (0028 Decision 3; spec A1.8 U-5, U-6).
//!
//! The boot record lives in the configuration sector as an append-only sequence of 32-byte flash
//! words; the last valid entry counts. Format (little-endian):
//!
//! | Byte | Field |
//! |---|---|
//! | 0-3 | magic "G7BR" |
//! | 4 | active slot (0 = A, 1 = B) |
//! | 5 | trial slot (0 = A, 1 = B, 0xFF = none) |
//! | 6 | trial attempts left |
//! | 7 | confirmed (1 = the trial image confirmed itself) |
//! | 8-11 | sequence number |
//! | 12 | flags: bit 0 the last trial was rolled back (the application logs it, U-6) |
//! | 13-27 | reserved, 0xFF |
//! | 28-31 | CRC-32 of bytes 0-27 |
//!
//! Since the update path (A1.8) the record is a record of the store (`g7250-store`, kind
//! `BootRecord`): the boot stage writes it for a trial attempt and a rollback, the application
//! for an activation, a confirmation and a `ROLLBACK`. A missing record means slot A.

use crate::Slot;
use crate::crc32::crc32;

pub const RECORD_MAGIC: u32 = u32::from_le_bytes(*b"G7BR");
pub const RECORD_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BootRecord {
    pub active: Slot,
    pub trial: Option<Slot>,
    pub attempts_left: u8,
    pub confirmed: bool,
    pub seq: u32,
    /// The last trial did not confirm itself and was rolled back (U-6).
    pub rolled_back: bool,
}

fn slot_from(b: u8) -> Option<Slot> {
    match b {
        0 => Some(Slot::A),
        1 => Some(Slot::B),
        _ => None,
    }
}

fn slot_byte(s: Slot) -> u8 {
    match s {
        Slot::A => 0,
        Slot::B => 1,
    }
}

impl BootRecord {
    pub fn parse(b: &[u8; RECORD_LEN]) -> Option<Self> {
        if u32::from_le_bytes([b[0], b[1], b[2], b[3]]) != RECORD_MAGIC {
            return None;
        }
        if crc32(&b[..28]) != u32::from_le_bytes([b[28], b[29], b[30], b[31]]) {
            return None;
        }
        let active = slot_from(b[4])?;
        let trial = match b[5] {
            0xFF => None,
            x => Some(slot_from(x)?),
        };
        Some(Self {
            active,
            trial,
            attempts_left: b[6],
            confirmed: b[7] == 1,
            seq: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            rolled_back: b[12] != 0xFF && b[12] & 1 != 0,
        })
    }

    pub fn encode(&self) -> [u8; RECORD_LEN] {
        let mut b = [0xFFu8; RECORD_LEN];
        b[0..4].copy_from_slice(&RECORD_MAGIC.to_le_bytes());
        b[4] = slot_byte(self.active);
        b[5] = self.trial.map(slot_byte).unwrap_or(0xFF);
        b[6] = self.attempts_left;
        b[7] = self.confirmed as u8;
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b[12] = self.rolled_back as u8;
        let c = crc32(&b[..28]);
        b[28..32].copy_from_slice(&c.to_le_bytes());
        b
    }
}

/// The last valid record in an append-only area (erased flash reads 0xFF).
pub fn last_record(area: &[u8]) -> Option<BootRecord> {
    let mut best: Option<BootRecord> = None;
    for chunk in area.chunks_exact(RECORD_LEN) {
        let arr: &[u8; RECORD_LEN] = chunk.try_into().ok()?;
        if arr.iter().all(|&x| x == 0xFF) {
            break; // end of the written part
        }
        if let Some(r) = BootRecord::parse(arr) {
            if best.is_none_or(|b| r.seq > b.seq) {
                best = Some(r);
            }
        }
    }
    best
}

/// The boot stage's record update for a choice: a trial uses up one attempt before it starts; a
/// trial that is out of attempts and unconfirmed is rolled back (U-6). `None`: nothing to write.
pub fn record_for(record: Option<BootRecord>, choice: Choice) -> Option<BootRecord> {
    let r = record?;
    match (r.trial, choice) {
        (Some(t), Choice::Trial(s)) if t == s => Some(BootRecord { attempts_left: r.attempts_left.saturating_sub(1), seq: r.seq + 1, ..r }),
        (Some(_), _) if !r.confirmed => Some(BootRecord { trial: None, attempts_left: 0, rolled_back: true, seq: r.seq + 1, ..r }),
        _ => None,
    }
}

/// What the boot stage does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Start this slot as a confirmed image.
    Boot(Slot),
    /// Start this slot as a trial; one attempt is used up (the boot stage records it first).
    Trial(Slot),
    /// No bootable image: stay in the boot stage, safe state held, watchdog refreshed.
    NoImage,
}

/// Choose the slot. `valid(slot)` tells whether the slot holds a sealed, valid image.
pub fn choose(record: Option<BootRecord>, valid: impl Fn(Slot) -> bool) -> Choice {
    let fallback = |preferred: Slot| {
        if valid(preferred) {
            Choice::Boot(preferred)
        } else if valid(preferred.other()) {
            Choice::Boot(preferred.other())
        } else {
            Choice::NoImage
        }
    };
    match record {
        None => fallback(Slot::A),
        Some(r) => match r.trial {
            Some(t) if !r.confirmed && r.attempts_left > 0 && valid(t) => Choice::Trial(t),
            // Trial used up, invalid or confirmed elsewhere: roll back to the active slot (U-6).
            _ => fallback(r.active),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_record_prefers_a() {
        assert_eq!(choose(None, |_| true), Choice::Boot(Slot::A));
        assert_eq!(choose(None, |s| s == Slot::B), Choice::Boot(Slot::B));
        assert_eq!(choose(None, |_| false), Choice::NoImage);
    }

    #[test]
    fn trial_then_rollback() {
        let mut r = BootRecord { active: Slot::A, trial: Some(Slot::B), attempts_left: 1, confirmed: false, seq: 3, rolled_back: false };
        assert_eq!(choose(Some(r), |_| true), Choice::Trial(Slot::B));
        r.attempts_left = 0;
        assert_eq!(choose(Some(r), |_| true), Choice::Boot(Slot::A));
        // A trial image that is not valid is never started.
        r.attempts_left = 1;
        assert_eq!(choose(Some(r), |s| s == Slot::A), Choice::Boot(Slot::A));
    }

    #[test]
    fn trial_attempt_then_rollback_records() {
        let r = BootRecord { active: Slot::A, trial: Some(Slot::B), attempts_left: 1, confirmed: false, seq: 3, rolled_back: false };
        // First start: one attempt used up, then the trial runs.
        let c = choose(Some(r), |_| true);
        assert_eq!(c, Choice::Trial(Slot::B));
        let r2 = record_for(Some(r), c).unwrap();
        assert_eq!((r2.attempts_left, r2.trial, r2.seq), (0, Some(Slot::B), 4));
        // Next start without confirmation: rollback to A, flagged.
        let c = choose(Some(r2), |_| true);
        assert_eq!(c, Choice::Boot(Slot::A));
        let r3 = record_for(Some(r2), c).unwrap();
        assert_eq!((r3.trial, r3.active, r3.rolled_back), (None, Slot::A, true));
        // A confirmed record needs no write.
        let ok = BootRecord { active: Slot::B, trial: None, attempts_left: 0, confirmed: true, seq: 9, rolled_back: false };
        assert_eq!(record_for(Some(ok), choose(Some(ok), |_| true)), None);
        assert_eq!(record_for(None, Choice::Boot(Slot::A)), None);
    }

    #[test]
    fn record_roundtrip_and_last_wins() {
        let r1 = BootRecord { active: Slot::A, trial: None, attempts_left: 0, confirmed: true, seq: 1, rolled_back: false };
        let r2 = BootRecord { active: Slot::B, trial: None, attempts_left: 0, confirmed: true, seq: 2, rolled_back: true };
        let mut area = [0xFFu8; 128];
        area[..32].copy_from_slice(&r1.encode());
        area[32..64].copy_from_slice(&r2.encode());
        assert_eq!(last_record(&area), Some(r2));
        // A torn (corrupt) write is skipped.
        area[64..96].copy_from_slice(&[0u8; 32]);
        assert_eq!(last_record(&area), Some(r2));
        assert_eq!(last_record(&[0xFFu8; 64]), None);
    }
}
