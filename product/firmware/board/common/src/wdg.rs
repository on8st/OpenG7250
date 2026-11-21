// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Independent watchdog IWDG1, raw. Whether it starts from reset is open ([HIS] HIS 10); we
//! start it ourselves in the boot stage, so that a hung boot stage, a hung trial image or a hung
//! application resets the MCU, and the reset brings the safe state back.

use crate::pac;
use crate::pac::iwdg::vals::{Key, Pr};

/// LSI 32 kHz / 64 = 500 Hz; 1000 ticks = 2 s.
pub const TIMEOUT_MS: u32 = 2000;
const RELOAD: u16 = (TIMEOUT_MS / 2) as u16 - 1;

/// Reload the counter. Harmless while the watchdog is not running.
#[inline(always)]
pub fn refresh() {
    pac::IWDG1.kr().write(|w| w.set_key(Key::RESET));
}

/// Start (or re-program) the watchdog with [`TIMEOUT_MS`]. Once started it cannot be stopped.
pub fn start() {
    pac::IWDG1.kr().write(|w| w.set_key(Key::START));
    pac::IWDG1.kr().write(|w| w.set_key(Key::ENABLE));
    pac::IWDG1.pr().write(|w| w.set_pr(Pr::DIVIDE_BY64));
    pac::IWDG1.rlr().write(|w| w.set_rl(RELOAD));
    // Wait for the update to reach the LSI domain; refresh meanwhile in case it was running
    // with a short timeout.
    while {
        let sr = pac::IWDG1.sr().read();
        sr.pvu() || sr.rvu()
    } {
        refresh();
    }
    refresh();
}
