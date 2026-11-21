// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The fault state of A3.5 (0007 Decision 5): `SAFETY_STATE`, `FAULT_CODE` (first cause),
//! `FAULT_FLAGS` (bit *c* for every code since the last clear), `FAULT_CLASS` (the highest class
//! among them), and the `FAULT_CLEAR` rules (B8.4; Table B8-3 "Fault code").
//!
//! Class 2 is latched until a reset and never cleared by `FAULT_CLEAR`. Class 1 is cleared by the
//! owner writing the current `FAULT_CODE` once every flagged code is class 1 and not escalated,
//! its condition is gone and [`HOLD_OFF_MS`] have passed since the latest entry. The third entry
//! of the same class 1 code since power-on is escalated: class 2 (counters.rs counts them).

/// Fault codes (Table B8-3).
pub mod code {
    pub const SWR_REPEATED: u8 = 2;
    pub const PA_OVERTEMP: u8 = 3;
    pub const SENSOR_IMPLAUSIBLE: u8 = 9;
    pub const RF_NOT_KEYED: u8 = 10;
    pub const TX_PATH_NOT_BLOCKED: u8 = 16;
    pub const WDT_RESET_AT_START: u8 = 17;
    pub const SELFTEST_FAILED: u8 = 18;
    pub const INTERNAL_ERROR: u8 = 19;
}

/// The class of a code (Table B8-3): 1 host-clearable, 2 latched until reset; 0 for a code
/// that is not a fault (reserved or none).
pub const fn class_of(c: u8) -> u8 {
    match c {
        code::SWR_REPEATED | code::PA_OVERTEMP | code::WDT_RESET_AT_START => 1,
        code::SENSOR_IMPLAUSIBLE | code::RF_NOT_KEYED | code::TX_PATH_NOT_BLOCKED | code::SELFTEST_FAILED | code::INTERNAL_ERROR => 2,
        _ => 0,
    }
}

/// Why a `FAULT_CLEAR` write is refused (B8.4 write reject reasons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClearReject {
    NotClearable,
    BadValue,
}

/// A clear needs this long since the latest fault entry (A3.5 "Clearing"; 0022 §27).
pub const HOLD_OFF_MS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    first: u8,
    flags: u32,
    /// Class 1 codes escalated to class 2 (bit per code).
    escalated: u32,
    /// Time of the latest entry, ms.
    last_entry_ms: u64,
}

impl Faults {
    pub const fn new() -> Self {
        Faults { first: 0, flags: 0, escalated: 0, last_entry_ms: 0 }
    }

    /// Enter the fault state with `c` at `now_ms` (A3.5 "Entry"). Returns true if `c` was not set
    /// yet (a new entry: log it and push the registers). For a class 1 code, use
    /// [`Self::raise_counted`].
    pub fn raise(&mut self, c: u8) -> bool {
        self.raise_at(c, self.last_entry_ms)
    }

    /// As [`Self::raise`], with the entry time.
    pub fn raise_at(&mut self, c: u8, now_ms: u64) -> bool {
        if class_of(c) == 0 {
            return false;
        }
        let bit = 1u32 << c;
        let new = self.flags & bit == 0;
        if self.first == 0 {
            self.first = c;
        }
        self.flags |= bit;
        self.last_entry_ms = self.last_entry_ms.max(now_ms);
        new
    }

    /// A class 1 code's entry, `count` being its entry count since power-on after this one
    /// (counters.rs): from the third it is escalated, class 2 (A3.5 "Escalation").
    pub fn raise_counted(&mut self, c: u8, now_ms: u64, count: u32) -> bool {
        if count >= crate::counters::ESCALATE_AT {
            self.escalated |= 1 << c;
        }
        self.raise_at(c, now_ms)
    }

    /// Whether a code is set and escalated.
    pub fn is_escalated(&self, c: u8) -> bool {
        self.escalated & self.flags & (1 << c) != 0
    }

    /// In the fault state: unkey, no arm (A3.5 "Effect").
    pub fn active(&self) -> bool {
        self.flags != 0
    }
    pub fn safety_state(&self) -> u32 {
        self.active() as u32
    }
    pub fn first_code(&self) -> u32 {
        self.first as u32
    }
    pub fn flags(&self) -> u32 {
        self.flags
    }
    /// The highest class among the codes set (escalated codes count as class 2).
    pub fn class(&self) -> u32 {
        let mut c = 0;
        for i in 1..32u8 {
            if self.flags & (1 << i) != 0 {
                c = c.max(if self.escalated & (1 << i) != 0 { 2 } else { class_of(i) });
            }
        }
        c as u32
    }

    /// `FAULT_CLEAR` with value `v` at `now_ms` (A3.5 "Clearing"; B8.4; Table B8-2 0x0215):
    /// - no fault: 0 is `NOT_CLEARABLE`, any other value `BAD_VALUE` (0039);
    /// - a value other than the current `FAULT_CODE`: `BAD_VALUE`;
    /// - the current code: all flags cleared if every flagged code is class 1 and not escalated,
    ///   its condition is gone (`gone`), and [`HOLD_OFF_MS`] have passed since the latest entry;
    ///   else `NOT_CLEARABLE` and nothing changes. A clear never re-arms (the caller only clears).
    pub fn clear(&mut self, v: u32, now_ms: u64, gone: impl Fn(u8) -> bool) -> Result<(), ClearReject> {
        if self.flags == 0 {
            return Err(if v == 0 { ClearReject::NotClearable } else { ClearReject::BadValue });
        }
        if v != self.first as u32 {
            return Err(ClearReject::BadValue);
        }
        let all_ok = (1..32u8).filter(|&i| self.flags & (1 << i) != 0).all(|i| class_of(i) == 1 && self.escalated & (1 << i) == 0 && gone(i));
        if !all_ok || now_ms.saturating_sub(self.last_entry_ms) < HOLD_OFF_MS {
            return Err(ClearReject::NotClearable);
        }
        // The escalation stays with the entry counters (power-on only); the flags go.
        *self = Faults { last_entry_ms: self.last_entry_ms, ..Faults::new() };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes_per_table_b8_3() {
        for c in [2, 3, 17] {
            assert_eq!(class_of(c), 1, "{c}");
        }
        for c in [9, 10, 16, 18, 19] {
            assert_eq!(class_of(c), 2, "{c}");
        }
        for c in [0, 1, 4, 5, 6, 7, 8, 11, 20, 31] {
            assert_eq!(class_of(c), 0, "{c}");
        }
    }

    #[test]
    fn first_cause_flags_and_class() {
        let mut f = Faults::new();
        assert_eq!((f.safety_state(), f.first_code(), f.flags(), f.class()), (0, 0, 0, 0));
        assert!(f.raise(code::INTERNAL_ERROR));
        assert!(!f.raise(code::INTERNAL_ERROR), "already set: no new entry");
        assert!(f.raise(code::TX_PATH_NOT_BLOCKED));
        assert_eq!(f.safety_state(), 1);
        assert_eq!(f.first_code(), 19, "the first cause stays");
        assert_eq!(f.flags(), (1 << 19) | (1 << 16));
        assert_eq!(f.class(), 2);
        assert!(f.active());
        assert!(!f.raise(4), "a reserved code is not a fault");
    }

    #[test]
    fn class_2_is_latched_until_reset() {
        let mut f = Faults::new();
        let gone = |_| true;
        assert_eq!(f.clear(0, 0, gone), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(7, 0, gone), Err(ClearReject::BadValue));
        f.raise_at(code::INTERNAL_ERROR, 0);
        assert_eq!(f.clear(19, 60_000, gone), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(0, 60_000, gone), Err(ClearReject::BadValue), "not the current code");
        assert_eq!((f.flags(), f.class()), (1 << 19, 2), "nothing cleared");
    }

    #[test]
    fn class_1_clear_after_the_hold_off_with_the_condition_gone() {
        // T-A3-23: under 10 s after the latest entry NOT_CLEARABLE even with the condition gone;
        // after 10 s it clears; the current code only; condition not gone NOT_CLEARABLE.
        let mut f = Faults::new();
        f.raise_counted(code::WDT_RESET_AT_START, 1_000, 1);
        assert_eq!(f.class(), 1);
        assert_eq!(f.clear(17, 10_999, |_| true), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(2, 11_000, |_| true), Err(ClearReject::BadValue));
        assert_eq!(f.clear(17, 11_000, |_| false), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(17, 11_000, |_| true), Ok(()));
        assert_eq!((f.safety_state(), f.first_code(), f.flags(), f.class()), (0, 0, 0, 0));
        // A newer entry restarts the hold-off for all.
        let mut f = Faults::new();
        f.raise_counted(code::WDT_RESET_AT_START, 0, 1);
        f.raise_counted(code::SWR_REPEATED, 8_000, 1);
        assert_eq!(f.clear(17, 12_000, |_| true), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(17, 18_000, |_| true), Ok(()));
        // A class 2 code with a class 1 one: not clearable.
        let mut f = Faults::new();
        f.raise_counted(code::WDT_RESET_AT_START, 0, 1);
        f.raise_at(code::INTERNAL_ERROR, 0);
        assert_eq!(f.clear(17, 60_000, |_| true), Err(ClearReject::NotClearable));
    }

    #[test]
    fn the_third_entry_of_a_class_1_code_is_class_2() {
        let mut f = Faults::new();
        f.raise_counted(code::SWR_REPEATED, 0, 2);
        assert_eq!(f.class(), 1);
        assert_eq!(f.clear(2, 20_000, |_| true), Ok(()));
        f.raise_counted(code::SWR_REPEATED, 30_000, 3);
        assert!(f.is_escalated(code::SWR_REPEATED));
        assert_eq!(f.class(), 2, "escalated: latched until reset");
        assert_eq!(f.clear(2, 90_000, |_| true), Err(ClearReject::NotClearable));
    }
}
