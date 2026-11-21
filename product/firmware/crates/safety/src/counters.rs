// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The counters of A3.5 "Counters" (0022 §27): `TRIP_COUNT`, the fault entry counter per
//! class 1 code (escalation: the third entry since power-on is class 2) and a reset counter per
//! reset cause. They count since power-on and survive `RESTART` and every reset without power
//! loss (as far as the hardware keeps the RAM: P-SYS-9), so the board keeps them in a no-init
//! RAM block (board persist.rs) in this encoding: one slot of [`SLOT_WORDS`] 64-bit words,
//! `[magic | sequence << 32, then the values in pairs, the last pair's high half the check]`.

/// The class 1 codes that are counted (Table B8-3): `SWR_REPEATED`, `PA_OVERTEMP`,
/// `WDT_RESET_AT_START`.
pub const COUNTED_CODES: [u8; 3] = [crate::fault::code::SWR_REPEATED, crate::fault::code::PA_OVERTEMP, crate::fault::code::WDT_RESET_AT_START];

/// Reset causes counted (RCC_RSR, RM0468): pin, software (`RESTART`, commanded resets), the
/// independent and the window watchdog, low-power. Power-on and brown-out clear the counters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetCause {
    Pin = 0,
    Software = 1,
    IndependentWatchdog = 2,
    WindowWatchdog = 3,
    LowPower = 4,
}

pub const RESET_CAUSE_NAMES: [&str; 5] = ["pin", "software", "independent_watchdog", "window_watchdog", "low_power"];

/// The third entry of the same class 1 code since power-on is handled as class 2 (A3.5
/// "Escalation").
pub const ESCALATE_AT: u32 = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub trip_count: u32,
    /// Per [`COUNTED_CODES`] entry.
    pub entries: [u32; 3],
    /// Per [`ResetCause`].
    pub resets: [u32; 5],
}

/// 64-bit words of one encoded slot: the header, then 9 values and the check in 5 words.
pub const SLOT_WORDS: usize = 6;
const MAGIC: u32 = 0x4737_4335; // "G7C5"

impl Counters {
    fn values(&self) -> [u32; 9] {
        let e = self.entries;
        let r = self.resets;
        [self.trip_count, e[0], e[1], e[2], r[0], r[1], r[2], r[3], r[4]]
    }

    fn check(seq: u32, v: &[u32; 9]) -> u32 {
        let mut c = MAGIC ^ seq.rotate_left(16);
        for (i, x) in v.iter().enumerate() {
            c = c.rotate_left(5) ^ x.wrapping_mul(0x9E37_79B9) ^ i as u32;
        }
        !c
    }

    /// One slot with sequence number `seq`.
    pub fn encode(&self, seq: u32) -> [u64; SLOT_WORDS] {
        let v = self.values();
        let c = Self::check(seq, &v);
        let mut w = [0u64; SLOT_WORDS];
        w[0] = MAGIC as u64 | (seq as u64) << 32;
        for k in 0..4 {
            w[1 + k] = v[2 * k] as u64 | (v[2 * k + 1] as u64) << 32;
        }
        w[5] = v[8] as u64 | (c as u64) << 32;
        w
    }

    /// A slot's sequence and counters, if it is whole (magic and check).
    pub fn decode(w: &[u64; SLOT_WORDS]) -> Option<(u32, Counters)> {
        if w[0] as u32 != MAGIC {
            return None;
        }
        let seq = (w[0] >> 32) as u32;
        let mut v = [0u32; 9];
        for k in 0..4 {
            v[2 * k] = w[1 + k] as u32;
            v[2 * k + 1] = (w[1 + k] >> 32) as u32;
        }
        v[8] = w[5] as u32;
        if (w[5] >> 32) as u32 != Self::check(seq, &v) {
            return None;
        }
        Some((seq, Counters { trip_count: v[0], entries: [v[1], v[2], v[3]], resets: [v[4], v[5], v[6], v[7], v[8]] }))
    }

    /// The newer of two slots that is whole (sequence compared with wrap-around).
    pub fn newest(a: &[u64; SLOT_WORDS], b: &[u64; SLOT_WORDS]) -> Option<(usize, u32, Counters)> {
        match (Self::decode(a), Self::decode(b)) {
            (Some((sa, ca)), Some((sb, cb))) => Some(if sb.wrapping_sub(sa) as i32 > 0 { (1, sb, cb) } else { (0, sa, ca) }),
            (Some((sa, ca)), None) => Some((0, sa, ca)),
            (None, Some((sb, cb))) => Some((1, sb, cb)),
            (None, None) => None,
        }
    }

    /// Count an entry of `code` (saturating); returns the count after it, or `None` for a code
    /// that is not counted.
    pub fn count_entry(&mut self, code: u8) -> Option<u32> {
        let i = COUNTED_CODES.iter().position(|&c| c == code)?;
        self.entries[i] = self.entries[i].saturating_add(1);
        Some(self.entries[i])
    }

    pub fn count_reset(&mut self, c: ResetCause) {
        let i = c as usize;
        self.resets[i] = self.resets[i].saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_torn_slots() {
        let c = Counters { trip_count: 7, entries: [1, 0, 2], resets: [3, 4, 5, 6, 0xFFFF_FFFF] };
        let w = c.encode(9);
        assert_eq!(Counters::decode(&w), Some((9, c)));
        // Any changed word makes the slot invalid (a reset in the middle of a write).
        for i in 0..SLOT_WORDS {
            let mut x = w;
            x[i] ^= 1 << 40;
            assert_eq!(Counters::decode(&x), None, "word {i}");
        }
        // The newer whole slot wins; a torn newer one leaves the older.
        let d = Counters { trip_count: 8, ..c };
        let w2 = d.encode(10);
        assert_eq!(Counters::newest(&w, &w2), Some((1, 10, d)));
        let mut torn = w2;
        torn[3] ^= 1;
        assert_eq!(Counters::newest(&w, &torn), Some((0, 9, c)));
        assert_eq!(Counters::newest(&torn, &torn), None);
        // Wrap-around of the sequence.
        assert_eq!(Counters::newest(&c.encode(u32::MAX), &d.encode(0)).map(|n| n.0), Some(1));
    }

    #[test]
    fn entries_count_and_saturate() {
        let mut c = Counters::default();
        assert_eq!(c.count_entry(crate::fault::code::WDT_RESET_AT_START), Some(1));
        assert_eq!(c.count_entry(crate::fault::code::WDT_RESET_AT_START), Some(2));
        assert_eq!(c.count_entry(crate::fault::code::INTERNAL_ERROR), None, "class 2 codes are not counted");
        c.entries[0] = u32::MAX;
        assert_eq!(c.count_entry(crate::fault::code::SWR_REPEATED), Some(u32::MAX));
        c.count_reset(ResetCause::Software);
        assert_eq!(c.resets, [0, 1, 0, 0, 0]);
    }
}
