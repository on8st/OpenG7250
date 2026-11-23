// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Reads of RAM kept over a reset (logmem.rs, persist.rs). The reset flags say whether it was
//! written since power-up, but they alone must not keep a start alive (P-SYS-2 is open: the stage
//! before the boot stage may have cleared them): a word never written, or hit by an ECC error, can
//! answer a read with a bus error (RM0468, RAMECC). So the read is made with bus faults ignored,
//! as the flash reads are (flash.rs), and such a word is simply not there.

use g7250_safety::tolerant::{CFSR_BFSR, ram_unreadable};

const SCB_CCR: u32 = 0xE000_ED14;
const CCR_BFHFNMIGN: u32 = 1 << 8;
const SCB_CFSR: u32 = 0xE000_ED28;

/// The 64-bit word at `p`, `None` if the read faulted. One instance (never inlined):
/// `board/check-faultmask.py` checks that nothing branches out while FAULTMASK is set.
///
/// # Safety
/// `p` is an aligned address inside the RAM kept over a reset.
#[inline(never)]
pub unsafe fn read_tolerant(p: *const u64) -> Option<u64> {
    use core::ptr::{read_volatile as rd, write_volatile as wr};
    // SAFETY: the SCB registers are the core's; `p` per the contract. FAULTMASK is set only around
    // the read and restored.
    unsafe {
        wr(SCB_CFSR as *mut u32, CFSR_BFSR);
        core::arch::asm!("cpsid f", options(nostack, preserves_flags));
        let ccr = rd(SCB_CCR as *const u32);
        wr(SCB_CCR as *mut u32, ccr | CCR_BFHFNMIGN);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        let v = rd(p);
        cortex_m::asm::dsb();
        let cfsr = rd(SCB_CFSR as *const u32);
        wr(SCB_CFSR as *mut u32, CFSR_BFSR);
        wr(SCB_CCR as *mut u32, ccr);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        core::arch::asm!("cpsie f", options(nostack, preserves_flags));
        if ram_unreadable(cfsr) { None } else { Some(v) }
    }
}
