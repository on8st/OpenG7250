// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Values kept over a reset without power loss: `TRIP_COUNT` (Table B8-2: "survives `RESTART`
//! and resets without power loss", A3.5).
//!
//! Where: the top 32 bytes of the AXI SRAM that every option setting has (0x2401FFE0, section
//! `.persist`, NOLOAD), outside the RAM of both the boot stage and the application (their
//! `memory.x` end 32 bytes lower), so neither image's data, bss or stack ever reaches it. Never
//! the Backup SRAM or the backup domain ([HIS] HIS 15 rule 4; the vendored HAL keeps them off).
//! A previous stage that writes there (the vendor bootloader, HIS 10) is caught by the checks
//! below and only costs the count.
//!
//! When: the block is read only when the reset flags show no power-on or brown-out reset since
//! the application last cleared them (the boot stage does not clear them, `reset::read`). After a
//! power loss it is written first: SRAM never written since power-up may hold words with a bad
//! ECC, and a read of such a word is a bus error (RM0468, RAMECC). So every read is of a block
//! this application wrote since power-up.
//!
//! How: two slots, each `[magic, sequence, value, check]`; a write goes to the slot that is not
//! the newest valid one, so a reset in the middle of a write leaves the other slot whole.

use core::ptr::{addr_of_mut, read_volatile, write_volatile};

const MAGIC: u32 = 0x4737_5043; // "G7PC"

#[unsafe(link_section = ".persist")]
static mut BLOCK: core::mem::MaybeUninit<[[u32; 4]; 2]> = core::mem::MaybeUninit::uninit();

fn check(seq: u32, v: u32) -> u32 {
    !(v ^ seq.rotate_left(16)) ^ MAGIC
}

fn slot(i: usize) -> *mut [u32; 4] {
    // SAFETY: a static of this module; only raw pointers, read and written volatile.
    unsafe { (addr_of_mut!(BLOCK) as *mut [u32; 4]).add(i) }
}

fn read(i: usize) -> Option<(u32, u32)> {
    // SAFETY: `slot` points into the `.persist` block; volatile, as it outlives the program.
    let s = unsafe { read_volatile(slot(i)) };
    (s[0] == MAGIC && s[3] == check(s[1], s[2])).then_some((s[1], s[2]))
}

fn write(i: usize, seq: u32, v: u32) {
    // SAFETY: as `read`; each slot is written whole, magic last.
    unsafe {
        let p = slot(i) as *mut u32;
        write_volatile(p, 0);
        write_volatile(p.add(1), seq);
        write_volatile(p.add(2), v);
        write_volatile(p.add(3), check(seq, v));
        write_volatile(p, MAGIC);
    }
}

/// The newest valid slot: index, sequence, value.
fn newest() -> Option<(usize, u32, u32)> {
    match (read(0), read(1)) {
        (Some((a, va)), Some((b, vb))) => Some(if b.wrapping_sub(a) as i32 > 0 { (1, b, vb) } else { (0, a, va) }),
        (Some((a, va)), None) => Some((0, a, va)),
        (None, Some((b, vb))) => Some((1, b, vb)),
        (None, None) => None,
    }
}

/// At start, before any other use: `power_lost` from the reset flags (power-on or brown-out).
/// Returns the trip count kept over the reset, and whether it was found (false after a power
/// loss, or when a previous stage overwrote the block: then the count starts again at 0).
pub fn start(power_lost: bool) -> (u32, bool) {
    if !power_lost {
        if let Some((_, _, v)) = newest() {
            return (v, true);
        }
    }
    write(0, 0, 0);
    write(1, 0, 0);
    (0, false)
}

/// The trip count kept (0 if none; after [`start`] there always is one).
pub fn trip_count() -> u32 {
    newest().map_or(0, |n| n.2)
}

/// Keep a new trip count (call on every change).
pub fn set_trip_count(v: u32) {
    let (i, seq) = match newest() {
        Some((_, _, old)) if old == v => return,
        Some((i, s, _)) => (1 - i, s.wrapping_add(1)),
        None => (0, 1),
    };
    write(i, seq, v);
}
