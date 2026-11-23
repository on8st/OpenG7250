// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Values kept over a reset without power loss (A3.5 "Counters"): `TRIP_COUNT`, the fault entry
//! counter per class 1 code and the reset counter per reset cause (g7250_safety::counters).
//!
//! Where: the top 128 bytes of the AXI SRAM that every option setting has (0x2401FF80, section
//! `.persist`, NOLOAD), outside the RAM of both the boot stage and the application (their
//! `memory.x` end 128 bytes lower), so neither image's data, bss or stack ever reaches it. Never
//! the Backup SRAM or the backup domain ([HIS] HIS 15 rule 4; the vendored HAL keeps them off).
//! A previous stage that writes there (the vendor bootloader, HIS 10) is caught by the checks
//! and only costs the counts (P-SYS-9).
//!
//! When: the block is read only when the reset flags of this start (read and cleared by the boot
//! stage, handed over) show no power-on or brown-out reset, and at least one reset. After a
//! power loss it is written first: SRAM never written since power-up may hold words with a bad
//! ECC, and a read of such a word is a bus error (RM0468, RAMECC). So every read is of a block
//! this application wrote since power-up.
//!
//! How: two slots of [`SLOT_WORDS`] 64-bit words (counters.rs encoding: header, values, check);
//! a write goes to the slot that is not the newest valid one, header last, so a reset in the
//! middle of a write leaves the other slot whole.

//!
//! What is taken from the words and written to them is g7250_safety::kept (host tests, also with
//! words that cannot be read); here only the place and the accesses.

use core::ptr::{addr_of, addr_of_mut, write_volatile};
use g7250_safety::counters::Counters;
pub use g7250_safety::kept::Handover;
use g7250_safety::kept::{self, WORDS};
use g7250_safety::logring::Words;

/// Two counter slots, then the handover. The AXI SRAM has a 64-bit ECC word: it is written in
/// whole, aligned 64-bit words only ([HIS] HIS 10, MEDIUM), never in halves.
/// The whole block as one static, so that the boot stage (which uses only the handover) and the
/// application place every part at the same address: the counter slots at 0x2401FF80, the
/// handover after them. (Two statics let the boot stage's linker drop the unused slots and put
/// the handover 96 bytes lower than the application looked for it.)
#[unsafe(link_section = ".persist")]
static mut REGION: core::mem::MaybeUninit<[u64; WORDS]> = core::mem::MaybeUninit::uninit();
const _: () = assert!(core::mem::size_of::<[u64; WORDS]>() <= 128);

struct Mem;

impl Words for Mem {
    fn get(&self, i: usize) -> Option<u64> {
        assert!(i < WORDS);
        // SAFETY: inside the static above, only raw pointers; 8-aligned.
        unsafe { crate::kept::read_tolerant((addr_of!(REGION) as *const u64).add(i)) }
    }
    fn set(&mut self, i: usize, v: u64) {
        assert!(i < WORDS);
        // SAFETY: inside the static above; volatile, as it outlives the program.
        unsafe { write_volatile((addr_of_mut!(REGION) as *mut u64).add(i), v) }
    }
}

/// The boot stage, just before the jump.
pub fn write_handover(h: &Handover) {
    kept::write_handover(&mut Mem, h)
}

/// The application, at start: the boot stage's handover of this start, then cleared.
pub fn take_handover() -> Option<Handover> {
    kept::take_handover(&mut Mem)
}

/// At start, before any other use: `power_lost` from the reset flags (power-on or brown-out).
/// Returns the counters kept over the reset, and whether they were found (false after a power
/// loss, or when a previous stage overwrote the block: then they start again at 0).
pub fn start(power_lost: bool) -> (Counters, bool) {
    kept::start(&mut Mem, power_lost)
}

/// The counters kept (zero if none; after [`start`] there always are some).
pub fn get() -> Counters {
    kept::get(&Mem)
}

/// Keep new counters (call on every change).
pub fn set(c: &Counters) {
    kept::set(&mut Mem, c)
}

/// The trip count kept.
pub fn trip_count() -> u32 {
    get().trip_count
}

/// Keep a new trip count (call on every change).
pub fn set_trip_count(v: u32) {
    let mut c = get();
    c.trip_count = v;
    set(&c);
}
