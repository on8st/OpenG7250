// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The memory of the log ring (A1.7; 0068; g7250_safety::logring), kept over a reset without
//! power loss: section `.logring` (NOLOAD), at the same address in the boot stage and in both
//! application slots, outside the RAM of all three, as the persist block (persist.rs): never the
//! Backup SRAM, read only when the reset flags show no power loss, written in whole aligned 64-bit
//! words. Whether the stage before the boot stage leaves it alone is P-SYS-9.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use g7250_safety::logring::{Ring, WORDS, Words};

#[unsafe(link_section = ".logring")]
static mut LOG: core::mem::MaybeUninit<[u64; WORDS]> = core::mem::MaybeUninit::uninit();
/// The region in the linker scripts.
const _: () = assert!(core::mem::size_of::<[u64; WORDS]>() <= 6272);

pub struct Mem(());

impl Words for Mem {
    fn get(&self, i: usize) -> u64 {
        assert!(i < WORDS);
        // SAFETY: inside the static above; volatile, as it outlives the program.
        unsafe { read_volatile((addr_of_mut!(LOG) as *const u64).add(i)) }
    }
    fn set(&mut self, i: usize, v: u64) {
        assert!(i < WORDS);
        // SAFETY: as `get`.
        unsafe { write_volatile((addr_of_mut!(LOG) as *mut u64).add(i), v) }
    }
}

/// The ring at a start: the boot stage with the reset flags' `power_lost`; the application, which
/// continues it, with false (the boot stage opened it), or true if no boot stage handed over.
pub fn open(power_lost: bool) -> (Ring<Mem>, bool) {
    Ring::open(Mem(()), power_lost)
}

/// Just before a commanded reset: whatever ring was in use is not used again in this run.
pub fn last(f: impl FnOnce(&mut Ring<Mem>)) {
    f(&mut open(false).0);
}
