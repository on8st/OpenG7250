// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The reset flags of the MCU (RCC_RSR, RM0468), read once per start by the boot stage, then
//! cleared, and handed to the application (A1.7 "The start entry"; 0068 point 4): the start entry,
//! `IDENTIFY` tag 17, the reset counters of A3.5 and `WDT_RESET_AT_START` use this one reading.
//! Whether the flags are trustworthy after the stage before the boot stage is P-SYS-2.

use crate::counters::ResetCause;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub power_on: bool,
    pub brown_out: bool,
    pub pin: bool,
    pub software: bool,
    pub independent_watchdog: bool,
    pub window_watchdog: bool,
    pub low_power: bool,
}

/// The cause named for a start; the value is `IDENTIFY` tag 17 (Table M-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cause {
    Unknown = 0,
    PowerOn = 1,
    Software = 2,
    Watchdog = 3,
    BrownOut = 4,
    Pin = 5,
}

impl Cause {
    pub const fn name(self) -> &'static str {
        match self {
            Cause::Unknown => "unknown",
            Cause::PowerOn => "power-on",
            Cause::Software => "software",
            Cause::Watchdog => "watchdog",
            Cause::BrownOut => "brown-out",
            Cause::Pin => "pin",
        }
    }
}

impl Flags {
    pub const fn watchdog(&self) -> bool {
        self.independent_watchdog || self.window_watchdog
    }

    /// The first of watchdog (either), software, power-on, brown-out, pin; unknown with none.
    pub const fn cause(&self) -> Cause {
        if self.watchdog() {
            Cause::Watchdog
        } else if self.software {
            Cause::Software
        } else if self.power_on {
            Cause::PowerOn
        } else if self.brown_out {
            Cause::BrownOut
        } else if self.pin {
            Cause::Pin
        } else {
            Cause::Unknown
        }
    }

    /// The RAM kept over a reset may never have been written since power-up: a power-on or
    /// brown-out flag, whatever the cause named, or no flag at all (not known).
    pub const fn power_lost(&self) -> bool {
        self.power_on || self.brown_out || self.bits() == 0
    }

    /// The reset counter of A3.5 that this start adds to.
    pub const fn counted(&self) -> Option<ResetCause> {
        match self.cause() {
            Cause::Watchdog if self.independent_watchdog => Some(ResetCause::IndependentWatchdog),
            Cause::Watchdog => Some(ResetCause::WindowWatchdog),
            Cause::Software => Some(ResetCause::Software),
            Cause::Pin => Some(ResetCause::Pin),
            Cause::Unknown if self.low_power => Some(ResetCause::LowPower),
            _ => None,
        }
    }

    pub const fn bits(&self) -> u8 {
        self.power_on as u8
            | (self.brown_out as u8) << 1
            | (self.pin as u8) << 2
            | (self.software as u8) << 3
            | (self.independent_watchdog as u8) << 4
            | (self.window_watchdog as u8) << 5
            | (self.low_power as u8) << 6
    }

    pub const fn from_bits(b: u8) -> Self {
        Flags {
            power_on: b & 1 != 0,
            brown_out: b & 2 != 0,
            pin: b & 4 != 0,
            software: b & 8 != 0,
            independent_watchdog: b & 16 != 0,
            window_watchdog: b & 32 != 0,
            low_power: b & 64 != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_cause_in_the_order_of_0068() {
        for b in 0..128u8 {
            let f = Flags::from_bits(b);
            assert_eq!(f.bits(), b);
            let want = if b & 0b011_0000 != 0 {
                Cause::Watchdog
            } else if b & 8 != 0 {
                Cause::Software
            } else if b & 1 != 0 {
                Cause::PowerOn
            } else if b & 2 != 0 {
                Cause::BrownOut
            } else if b & 4 != 0 {
                Cause::Pin
            } else {
                Cause::Unknown
            };
            assert_eq!(f.cause(), want, "{b:#b}");
            assert_eq!(f.power_lost(), b & 3 != 0 || b == 0);
        }
        // A power-on sets brown-out and pin too; a software reset the pin flag.
        assert_eq!(Flags::from_bits(0b111).cause().name(), "power-on");
        assert_eq!(Flags::from_bits(0b1100).cause().name(), "software");
        assert_eq!([Cause::Unknown as u8, Cause::PowerOn as u8, Cause::Software as u8, Cause::Watchdog as u8, Cause::BrownOut as u8, Cause::Pin as u8], [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn counted_under_the_named_cause() {
        assert_eq!(Flags::from_bits(0b001_0100).counted(), Some(ResetCause::IndependentWatchdog));
        assert_eq!(Flags::from_bits(0b010_1100).counted(), Some(ResetCause::WindowWatchdog));
        assert_eq!(Flags::from_bits(0b000_1100).counted(), Some(ResetCause::Software));
        assert_eq!(Flags::from_bits(0b000_0100).counted(), Some(ResetCause::Pin));
        assert_eq!(Flags::from_bits(0b100_0000).counted(), Some(ResetCause::LowPower));
        assert_eq!(Flags::from_bits(0b000_0111).counted(), None);
        assert_eq!(Flags::default().counted(), None);
    }
}
