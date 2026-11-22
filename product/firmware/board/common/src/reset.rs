// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The reset flags, read once per start by the boot stage and then cleared (A1.7; 0068). The
//! application gets them through the handover (persist.rs), never from the register.

use crate::pac;
pub use g7250_safety::reset::Flags;

pub fn read_and_clear() -> Flags {
    let r = pac::RCC.rsr().read();
    let f = Flags {
        power_on: r.porrstf(),
        brown_out: r.borrstf(),
        pin: r.pinrstf(),
        software: r.sftrstf(),
        independent_watchdog: r.iwdg1rstf(),
        window_watchdog: r.wwdg1rstf(),
        low_power: r.lpwrrstf(),
    };
    pac::RCC.rsr().modify(|w| w.set_rmvf(true));
    f
}
