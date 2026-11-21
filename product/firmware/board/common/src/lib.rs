// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Board support shared by the boot stage and the application.
//!
//! Everything here uses raw register access (`stm32-metapac`), so that it runs before any HAL
//! is initialised: the safe state comes first ([HIS] HIS 85), before clocks, before the console.
#![no_std]

pub use stm32_metapac as pac;

pub mod console;
pub mod reset;
pub mod safe;
pub mod wdg;

/// Firmware version of this build: [31:24] major, [23:16] minor, [15:0] build (B4.8 `fw_version`).
pub const FW_MAJOR: u8 = 0;
pub const FW_MINOR: u8 = 1;
pub const FW_BUILD: u16 = 1;
pub const FW_VERSION: u32 = g7250_layout::header::version(FW_MAJOR, FW_MINOR, FW_BUILD);
/// Pre-release builds are beta builds (discovery byte 23 non-zero, B3.5).
pub const FW_BETA: bool = true;

/// First thing at every start, boot stage and application alike.
///
/// 1. Mask interrupts, then apply the safe state ([HIS] HIS 85, with PH9 high per 0029).
/// 2. Clear whatever the previous stage left enabled or pending (the vendor bootloader, or our
///    boot stage), so that no foreign interrupt reaches our vector table once unmasked.
/// 3. Refresh the independent watchdog, in case the option bytes start it from reset
///    ([HIS] HIS 10, "IWDG (open)"). Harmless if it is not running.
///
/// Leaves interrupts masked (PRIMASK set); the caller unmasks when it is ready.
pub fn early_init() {
    cortex_m::interrupt::disable();
    safe::apply();
    // SAFETY: we own the core at this point; nothing else runs.
    unsafe {
        let p = cortex_m::Peripherals::steal();
        let mut syst = p.SYST;
        syst.disable_interrupt();
        syst.disable_counter();
        let nvic = &*cortex_m::peripheral::NVIC::PTR;
        for i in 0..8 {
            nvic.icer[i].write(0xFFFF_FFFF);
            nvic.icpr[i].write(0xFFFF_FFFF);
        }
        // Clear a pending SysTick and PendSV left by a previous stage.
        let scb = &*cortex_m::peripheral::SCB::PTR;
        scb.icsr.write((1 << 25) | (1 << 27));
    }
    wdg::refresh();
}
