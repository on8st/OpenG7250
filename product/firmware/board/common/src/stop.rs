// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Unexpected stop of the firmware (A3.6; 0075, 0078), in the boot stage and the application:
//! every processor fault vector (HardFault, MemManage, BusFault, UsageFault) and the panic path
//! end here. In this order: a stack pointer of known value (the old one is not trusted), the RF
//! control pins at their safe levels by the same code as at reset (`safe::apply_at_reset`, which
//! writes every register it needs and reads none it relies on), the commanded-reset word cleared
//! by one write, the log line `restart: processor fault <kind>` or `restart: stop <code>` where it
//! can, then a software reset, without waiting for the watchdog. `board/check-fault-vectors.py`
//! checks in each ELF that every fault vector points here.
//!
//! Residuals (0075 point 5, 0078): a fault inside this path before the clear is a lockup, ended
//! by the processor watchdog; a fault while the log line is written likewise, after the pins.

use core::sync::atomic::{AtomicUsize, Ordering};

unsafe extern "C" {
    /// The top of the stack (cortex-m-rt), the value the core loads at reset.
    static _stack_start: u32;
}

/// The uptime for the log line, ms: the application registers its time; the boot stage uses
/// the cycle counter since its start.
static UPTIME: AtomicUsize = AtomicUsize::new(0);

pub fn set_uptime(f: fn() -> u32) {
    UPTIME.store(f as usize, Ordering::Relaxed);
}

fn uptime() -> u32 {
    match UPTIME.load(Ordering::Relaxed) {
        0 => crate::safe::us_since_boot_stage() / 1000,
        p => {
            // SAFETY: only `set_uptime` stores here, a `fn() -> u32`.
            let f = unsafe { core::mem::transmute::<usize, fn() -> u32>(p) };
            f()
        }
    }
}

macro_rules! fault_vector {
    ($name:ident, $vector:literal) => {
        /// A processor fault vector (A3.6 "Unexpected stop of the firmware"): a stack of known
        /// value first, then the pins, then [`fault_entry`] with the vector number.
        /// On a flash word of its own with its literal (`.fault_text` in the memory scripts).
        #[unsafe(no_mangle)]
        #[unsafe(naked)]
        #[unsafe(link_section = concat!(".text.g7250_fault_vector_", stringify!($vector)))]
        pub unsafe extern "C" fn $name() -> ! {
            core::arch::naked_asm!(
                "ldr r0, ={stack}",
                "msr msp, r0",
                "bl {pins}",
                "movs r0, #{vector}",
                "b.w {entry}",
                stack = sym _stack_start,
                pins = sym g7250_fault_pins,
                vector = const $vector,
                entry = sym fault_entry,
            )
        }
    };
}

fault_vector!(HardFault, 3);
fault_vector!(MemoryManagement, 4);
fault_vector!(BusFault, 5);
fault_vector!(UsageFault, 6);

/// The pins of A3.6 at their safe levels: the reset-time code, inlined here, straight-line and
/// returning, so that `board/check-safe-margin.py` bounds this path as it does the reset path.
/// From the start of a flash word (`.fault_text` in the memory scripts).
#[unsafe(no_mangle)]
#[inline(never)]
#[unsafe(link_section = ".text.g7250_fault_pins")]
pub extern "C" fn g7250_fault_pins() {
    crate::safe::apply_at_reset();
}

/// The kind of the log line (0075 point 2): the vector, or for a HardFault escalated from another
/// fault (HFSR FORCED) the fault status that is set (CFSR: MMFSR, BFSR, UFSR; ARMv7-M).
fn kind(vector: u32) -> &'static str {
    // SAFETY: reads of the SCB fault status registers.
    let (hfsr, cfsr) = unsafe { (core::ptr::read_volatile(0xE000_ED2C as *const u32), core::ptr::read_volatile(0xE000_ED28 as *const u32)) };
    match vector {
        4 => "memmanage",
        5 => "bus",
        6 => "usage",
        3 if hfsr & 1 << 30 != 0 && cfsr & 0xFF != 0 => "memmanage",
        3 if hfsr & 1 << 30 != 0 && cfsr & 0xFF00 != 0 => "bus",
        3 if hfsr & 1 << 30 != 0 && cfsr & 0xFFFF_0000 != 0 => "usage",
        3 if hfsr & (1 << 30 | 1 << 1) != 0 => "hard",
        _ => "other",
    }
}

/// After the pins ([`g7250_fault_pins`], called by the vector's entry).
extern "C" fn fault_entry(vector: u32) -> ! {
    crate::persist::clear_commanded();
    finish(format_args!("restart: processor fault {}", kind(vector)))
}

/// The firmware's own stop (a panic, a failed assertion): `code` lower-case letters, digits and
/// hyphens, at most 16 characters (0075 point 2).
pub fn stop(code: &'static str) -> ! {
    crate::safe::apply_at_reset();
    cortex_m::interrupt::disable();
    crate::persist::clear_commanded();
    finish(format_args!("restart: stop {}", code))
}

fn finish(line: core::fmt::Arguments<'_>) -> ! {
    let up = uptime();
    crate::logmem::last(|r| r.log(up, line));
    cortex_m::asm::dsb();
    cortex_m::peripheral::SCB::sys_reset()
}
