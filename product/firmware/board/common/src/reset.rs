// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Reset cause. A watchdog reset seen at start-up enters the fault state (0007 sub-decision 5,
//! "Entry"); it is reported on the console and, from F2 on, in the log.

use crate::pac;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cause {
    pub power_on: bool,
    pub brown_out: bool,
    pub pin: bool,
    pub software: bool,
    pub independent_watchdog: bool,
    pub window_watchdog: bool,
    pub low_power: bool,
}

impl Cause {
    pub fn name(&self) -> &'static str {
        if self.independent_watchdog {
            "watchdog"
        } else if self.window_watchdog {
            "window-watchdog"
        } else if self.power_on {
            "power-on"
        } else if self.brown_out {
            "brown-out"
        } else if self.software {
            "software"
        } else if self.low_power {
            "low-power"
        } else if self.pin {
            "pin"
        } else {
            "unknown"
        }
    }
    pub fn is_watchdog(&self) -> bool {
        self.independent_watchdog || self.window_watchdog
    }
}

/// Read the reset flags. `clear` removes them so the next start sees only its own cause; the
/// boot stage reads without clearing and hands the cause on, the application clears.
pub fn read(clear: bool) -> Cause {
    let r = pac::RCC.rsr().read();
    let c = Cause {
        power_on: r.porrstf(),
        brown_out: r.borrstf(),
        pin: r.pinrstf(),
        software: r.sftrstf(),
        independent_watchdog: r.iwdg1rstf(),
        window_watchdog: r.wwdg1rstf(),
        low_power: r.lpwrrstf(),
    };
    if clear {
        pac::RCC.rsr().modify(|w| w.set_rmvf(true));
    }
    c
}
