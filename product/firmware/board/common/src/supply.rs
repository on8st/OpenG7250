// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! PWR_CR3 handled once and explicitly (rule in `g7250_safety::supply`; [HIS] HIS 10).

use g7250_safety::supply::{self, Outcome, PwrRegs};

const PWR_CSR1: u32 = 0x5802_4804;
const PWR_CR3: u32 = 0x5802_480C;

pub struct Regs;

impl PwrRegs for Regs {
    fn read_cr3(&mut self) -> u32 {
        // SAFETY: always-mapped PWR register; reading has no side effect.
        unsafe { core::ptr::read_volatile(PWR_CR3 as *const u32) }
    }
    fn write_cr3(&mut self, v: u32) {
        // SAFETY: called at most once, by `supply::configure`, only from the power-on value and
        // only with the HIS 10 configuration.
        unsafe { core::ptr::write_volatile(PWR_CR3 as *mut u32, v) }
    }
    fn read_csr1(&mut self) -> u32 {
        // SAFETY: always-mapped PWR register; reading has no side effect.
        unsafe { core::ptr::read_volatile(PWR_CSR1 as *const u32) }
    }
}

/// Boot stage: apply the rule. The caller holds the safe state if the outcome is not ok.
pub fn configure() -> Outcome {
    supply::configure(&mut Regs, supply::READY_POLLS, crate::wdg::refresh)
}

/// Application: is the supply set as HIS 10 says, and ready? Reads only.
pub fn ready_for_init() -> (bool, u32) {
    let mut r = Regs;
    let cr3 = r.read_cr3();
    (supply::ready_for_init(cr3, r.read_csr1()), cr3)
}
