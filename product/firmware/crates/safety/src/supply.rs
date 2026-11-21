// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Core supply configuration (PWR_CR3) of the Main, handled once and explicitly.
//!
//! [HIS] HIS 10 (`d199d33`, MEDIUM): the board is wired for the RM0468 configuration "SMPS
//! supplies the LDO, SMPS output 2.5 V also available to external circuits"; new firmware selects
//! this configuration and no other. PWR_CR3's supply bits can be written once per power-on, a
//! configuration that does not match the wiring can stop or harm the part, and whether an
//! earlier stage (the vendor bootloader) already wrote it is not known.
//!
//! Rule (coordinator, 2026-11-21):
//! - already the HIS 10 configuration: keep it, do not write;
//! - still the power-on value: write exactly the HIS 10 configuration once, read it back, and
//!   wait for ACTVOSRDY with a bound;
//! - anything else, a back-read that differs after the write (an earlier stage locked the
//!   power-on value), or no ACTVOSRDY within the bound: do not write (again), stay in the safe
//!   state, report it; no transmit and no further init that depends on the supply.
//!
//! Register facts: RM0468, PWR_CR3 (offset 0x0C): BYPASS bit 0, LDOEN bit 1, SDEN bit 2, SDEXTHP
//! bit 3, SDLEVEL bits 5:4 (01 1.8 V, 10 and 11 2.5 V); reset value 0x00000006 (SDEN, LDOEN).
//! PWR_CSR1 (offset 0x04) ACTVOSRDY bit 13.

/// The supply-configuration bits of PWR_CR3.
pub const SUPPLY_MASK: u32 = 0x3F;
/// Power-on value of the supply bits (RM0468: PWR_CR3 reset value 0x00000006).
pub const POWER_ON: u32 = 0x06;
/// HIS 10: LDOEN | SDEN | SDEXTHP | SDLEVEL = 2.5 V, BYPASS = 0.
pub const HIS10: u32 = 0x02 | 0x04 | 0x08 | (0b10 << 4);
/// SDLEVEL = 11 is also 2.5 V (RM0468); accepted as the HIS 10 configuration when found.
const HIS10_ALT: u32 = 0x02 | 0x04 | 0x08 | (0b11 << 4);
/// PWR_CSR1 ACTVOSRDY.
pub const ACTVOSRDY: u32 = 1 << 13;

/// What to do with the value found in PWR_CR3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Already the HIS 10 configuration: do not write.
    Keep,
    /// Still the power-on value: write this full register value (other bits kept), once.
    Write(u32),
    /// Another configuration: do not write; safe state; report.
    Mismatch,
}

pub fn is_his10(cr3: u32) -> bool {
    let s = cr3 & SUPPLY_MASK;
    s == HIS10 || s == HIS10_ALT
}

pub fn plan(cr3: u32) -> Plan {
    match cr3 & SUPPLY_MASK {
        _ if is_his10(cr3) => Plan::Keep,
        POWER_ON => Plan::Write((cr3 & !SUPPLY_MASK) | HIS10),
        _ => Plan::Mismatch,
    }
}

/// Result of the whole procedure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Found as HIS 10, not written.
    Kept,
    /// Written once from the power-on value.
    Written,
    /// Not the HIS 10 configuration (found, or after the write: locked by an earlier stage).
    Mismatch { found: u32 },
    /// ACTVOSRDY did not come within the bound.
    NotReady { cr3: u32 },
}

impl Outcome {
    pub fn ok(&self) -> bool {
        matches!(self, Outcome::Kept | Outcome::Written)
    }
}

/// Register access for [`configure`]; the board implements it on PWR, tests on a model.
pub trait PwrRegs {
    fn read_cr3(&mut self) -> u32;
    fn write_cr3(&mut self, v: u32);
    fn read_csr1(&mut self) -> u32;
}

/// Number of polls of ACTVOSRDY before giving up. At 64 MHz one poll takes well under 1 µs;
/// 2 000 000 polls are more than a second, far above the regulator settling time, and below the
/// 2 s watchdog.
pub const READY_POLLS: u32 = 2_000_000;

/// Run the rule above. Writes PWR_CR3 at most once. `refresh` is called while waiting (the
/// independent watchdog).
pub fn configure(r: &mut impl PwrRegs, polls: u32, mut refresh: impl FnMut()) -> Outcome {
    let found = r.read_cr3();
    let outcome = match plan(found) {
        Plan::Mismatch => return Outcome::Mismatch { found },
        Plan::Keep => Outcome::Kept,
        Plan::Write(v) => {
            r.write_cr3(v);
            let back = r.read_cr3();
            if !is_his10(back) {
                // An earlier stage locked the power-on value: the write was ignored.
                return Outcome::Mismatch { found: back };
            }
            Outcome::Written
        }
    };
    for i in 0..polls {
        if r.read_csr1() & ACTVOSRDY != 0 {
            return outcome;
        }
        if i % 1024 == 0 {
            refresh();
        }
    }
    Outcome::NotReady { cr3: r.read_cr3() }
}

/// For the application: the supply is usable for the HAL init only if it holds the HIS 10
/// configuration and ACTVOSRDY is set. Reads only.
pub fn ready_for_init(cr3: u32, csr1: u32) -> bool {
    is_his10(cr3) && csr1 & ACTVOSRDY != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PWR model: the supply bits are write-once per power-on; ACTVOSRDY comes after `ready_after`
    /// polls once the configuration is valid.
    struct Pwr {
        cr3: u32,
        locked: bool,
        writes: u32,
        polls: u32,
        ready_after: Option<u32>,
    }

    impl Pwr {
        fn power_on() -> Self {
            Pwr { cr3: POWER_ON, locked: false, writes: 0, polls: 0, ready_after: Some(10) }
        }
        fn written_by_earlier_stage(v: u32) -> Self {
            Pwr { cr3: v, locked: true, writes: 0, polls: 0, ready_after: Some(0) }
        }
    }

    impl PwrRegs for Pwr {
        fn read_cr3(&mut self) -> u32 {
            self.cr3
        }
        fn write_cr3(&mut self, v: u32) {
            self.writes += 1;
            if !self.locked {
                self.cr3 = (self.cr3 & !SUPPLY_MASK) | (v & SUPPLY_MASK);
                self.locked = true;
            }
        }
        fn read_csr1(&mut self) -> u32 {
            self.polls += 1;
            match self.ready_after {
                Some(n) if self.polls > n => ACTVOSRDY,
                _ => 0,
            }
        }
    }

    #[test]
    fn values() {
        assert_eq!(HIS10, 0x2E);
        assert_eq!(plan(0x06), Plan::Write(0x2E));
        // Other bits of the register are kept (for example VBE, bit 8).
        assert_eq!(plan(0x106), Plan::Write(0x12E));
    }

    #[test]
    fn case1_already_his10_is_not_written() {
        for v in [HIS10, HIS10_ALT, 0x0100_002E] {
            let mut p = Pwr::written_by_earlier_stage(v);
            assert_eq!(configure(&mut p, 100, || {}), Outcome::Kept);
            assert_eq!(p.writes, 0);
        }
    }

    #[test]
    fn case2_power_on_value_is_written_once_exactly() {
        let mut p = Pwr::power_on();
        assert_eq!(configure(&mut p, 100, || {}), Outcome::Written);
        assert_eq!(p.writes, 1);
        assert_eq!(p.cr3 & SUPPLY_MASK, 0x2E);
    }

    #[test]
    fn case3_other_configuration_is_not_written() {
        // LDO only, direct SMPS, SMPS 1.8 V feeding the LDO, bypass, external supply.
        for v in [0x02, 0x04, 0x16, 0x01, 0x00, 0x1E, 0x2F] {
            let mut p = Pwr::written_by_earlier_stage(v);
            assert_eq!(configure(&mut p, 100, || {}), Outcome::Mismatch { found: v }, "{v:#x}");
            assert_eq!(p.writes, 0, "{v:#x}");
        }
    }

    #[test]
    fn power_on_value_locked_by_earlier_stage_is_a_mismatch() {
        // The earlier stage wrote the power-on value itself: our write is ignored.
        let mut p = Pwr::written_by_earlier_stage(POWER_ON);
        assert_eq!(configure(&mut p, 100, || {}), Outcome::Mismatch { found: POWER_ON });
        assert_eq!(p.writes, 1);
    }

    #[test]
    fn ready_wait_is_bounded() {
        let mut p = Pwr::power_on();
        p.ready_after = None;
        let mut refreshed = 0;
        assert_eq!(configure(&mut p, 5000, || refreshed += 1), Outcome::NotReady { cr3: 0x2E });
        assert_eq!(p.polls, 5000);
        assert!(refreshed >= 4);
        let mut p = Pwr::written_by_earlier_stage(HIS10);
        p.ready_after = None;
        assert_eq!(configure(&mut p, 10, || {}), Outcome::NotReady { cr3: HIS10 });
        assert_eq!(p.writes, 0);
    }

    #[test]
    fn app_gate() {
        assert!(ready_for_init(0x2E, ACTVOSRDY));
        assert!(!ready_for_init(0x2E, 0));
        assert!(!ready_for_init(0x06, ACTVOSRDY));
        assert!(!ready_for_init(0x02, ACTVOSRDY));
    }
}
