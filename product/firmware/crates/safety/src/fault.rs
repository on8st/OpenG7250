// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The fault state of A3.5 (0007 Decision 5): `SAFETY_STATE`, `FAULT_CODE` (first cause),
//! `FAULT_FLAGS` (bit *c* for every code since the last clear), `FAULT_CLASS` (the highest class
//! among them), and the `FAULT_CLEAR` rules (B8.4; Table B8-3 "Fault code").
//!
//! This step holds the class 2 codes this firmware raises: `TX_PATH_NOT_BLOCKED` (the safe state
//! found not held) and `INTERNAL_ERROR` (a failed flash operation, B2.3 "flash work that fails";
//! 0065 point 2). Class 2 is latched until a reset and never cleared by `FAULT_CLEAR`. The class 1
//! rules (condition gone, 10 s hold-off, escalation, counters kept over a reset) come with
//! `WDT_RESET_AT_START`, the next step; until then a clear of a class 1 code is refused too.

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Faults {
    first: u8,
    flags: u32,
    escalated: bool,
}

impl Faults {
    pub const fn new() -> Self {
        Faults { first: 0, flags: 0, escalated: false }
    }

    /// Enter the fault state with `c` (A3.5 "Entry"). Returns true if `c` was not set yet (a new
    /// entry: log it and push the registers).
    pub fn raise(&mut self, c: u8) -> bool {
        if class_of(c) == 0 {
            return false;
        }
        let bit = 1u32 << c;
        let new = self.flags & bit == 0;
        if self.first == 0 {
            self.first = c;
        }
        self.flags |= bit;
        new
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
                c = c.max(class_of(i));
            }
        }
        if self.escalated && c == 1 {
            c = 2;
        }
        c as u32
    }

    /// `FAULT_CLEAR` with value `v` (A3.5 "Clearing"; B8.4): without a fault, 0 is
    /// `NOT_CLEARABLE` and any other value `BAD_VALUE` (0039); with a class 2 code set,
    /// `NOT_CLEARABLE`; a value other than the current fault code, `BAD_VALUE`. Class 1 clears
    /// come with their conditions in the next step: refused (`NOT_CLEARABLE`) until then.
    pub fn clear(&mut self, v: u32) -> Result<(), ClearReject> {
        if self.flags == 0 {
            return Err(if v == 0 { ClearReject::NotClearable } else { ClearReject::BadValue });
        }
        if self.class() == 2 {
            return Err(ClearReject::NotClearable);
        }
        if v != self.first as u32 {
            return Err(ClearReject::BadValue);
        }
        Err(ClearReject::NotClearable)
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
        assert_eq!(f.clear(0), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(7), Err(ClearReject::BadValue));
        f.raise(code::INTERNAL_ERROR);
        assert_eq!(f.clear(19), Err(ClearReject::NotClearable));
        assert_eq!(f.clear(0), Err(ClearReject::NotClearable));
        assert_eq!((f.flags(), f.class()), (1 << 19, 2), "nothing cleared");
    }
}
