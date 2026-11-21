// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! What the previous stage left: captured at the boot stage's first instruction, before
//! anything is changed, and printed once the console runs. [HIS] HIS 90 (`d199d33`, rows
//! F1-Q3/F1-Q4): record PWR_CR3, RCC_CR/CFGR, VTOR, PRIMASK, CCR (caches), NVIC_ISER and the
//! IWDG state at the first instruction and report them on USART1. Reads only.
//!
//! VTOR is the one exception: the cortex-m-rt reset handler (feature `set-vtor`) points it at our
//! vector table before `main`, so that a fault before `main` cannot vector into the
//! bootloader's table. The value printed is therefore ours.

/// Register addresses (RM0468; ARMv7-M architecture reference manual).
const PWR_CR3: u32 = 0x5802_480C;
const RCC_CR: u32 = 0x5802_4400;
const RCC_CFGR: u32 = 0x5802_4410;
const RCC_CSR: u32 = 0x5802_4474; // LSION / LSIRDY: the IWDG runs the LSI when started
const IWDG1_PR: u32 = 0x5800_4804;
const IWDG1_RLR: u32 = 0x5800_4808;
const SCB_VTOR: u32 = 0xE000_ED08;
const SCB_CCR: u32 = 0xE000_ED14;
const NVIC_ISER: u32 = 0xE000_E100;

#[derive(Clone, Copy, Debug, Default)]
pub struct StartState {
    pub pwr_cr3: u32,
    pub rcc_cr: u32,
    pub rcc_cfgr: u32,
    pub rcc_csr: u32,
    pub iwdg_pr: u32,
    pub iwdg_rlr: u32,
    pub vtor: u32,
    pub primask: bool,
    pub ccr: u32,
    pub nvic_iser: [u32; 8],
}

fn rd(addr: u32) -> u32 {
    // SAFETY: fixed, always-mapped system and peripheral registers; reading has no side effect.
    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// Capture. Call first in the boot stage. RCC_CR and the PWR/IWDG registers are readable
/// without enabling any clock on the H7 (RCC, PWR and IWDG are always clocked).
#[inline(always)]
pub fn capture() -> StartState {
    let mut s = StartState {
        // cortex-m's `Primask::Active` means "interrupts enabled"; PRIMASK = 1 means masked.
        primask: cortex_m::register::primask::read().is_inactive(),
        pwr_cr3: rd(PWR_CR3),
        rcc_cr: rd(RCC_CR),
        rcc_cfgr: rd(RCC_CFGR),
        rcc_csr: rd(RCC_CSR),
        iwdg_pr: rd(IWDG1_PR),
        iwdg_rlr: rd(IWDG1_RLR),
        vtor: rd(SCB_VTOR),
        ccr: rd(SCB_CCR),
        nvic_iser: [0; 8],
    };
    for i in 0..8 {
        s.nvic_iser[i] = rd(NVIC_ISER + 4 * i as u32);
    }
    s
}

impl StartState {
    /// Print as `boot: start …` lines.
    pub fn print(&self) {
        crate::println!(
            "boot: start PWR_CR3={:#010x} RCC_CR={:#010x} RCC_CFGR={:#010x} RCC_CSR={:#010x}",
            self.pwr_cr3, self.rcc_cr, self.rcc_cfgr, self.rcc_csr
        );
        crate::println!(
            "boot: start VTOR={:#010x} (set by our reset handler, not the previous stage's) PRIMASK={} CCR={:#010x} (I-cache {}, D-cache {}) IWDG PR={:#x} RLR={:#x} LSI {}",
            self.vtor,
            self.primask as u8,
            self.ccr,
            if self.ccr & (1 << 17) != 0 { "on" } else { "off" },
            if self.ccr & (1 << 16) != 0 { "on" } else { "off" },
            self.iwdg_pr,
            self.iwdg_rlr,
            if self.rcc_csr & 1 != 0 { "on" } else { "off" }
        );
        let n = &self.nvic_iser;
        crate::println!(
            "boot: start NVIC_ISER={:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x} {:08x}",
            n[0], n[1], n[2], n[3], n[4], n[5], n[6], n[7]
        );
    }
}
