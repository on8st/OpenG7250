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

use core::ptr::{addr_of_mut, read_volatile, write_volatile};
use g7250_safety::counters::{Counters, SLOT_WORDS};

/// Two slots. The AXI SRAM has a 64-bit ECC word: it is written in whole, aligned 64-bit words
/// only ([HIS] HIS 10, MEDIUM), never in halves.
/// The whole block as one static with a fixed layout, so that the boot stage (which uses only
/// the handover) and the application place every part at the same address: the counter slots
/// at 0x2401FF80, the handover after them. (Two statics let the boot stage's linker drop the
/// unused slots and put the handover 96 bytes lower than the application looked for it.)
#[repr(C)]
struct Region {
    slots: [[u64; SLOT_WORDS]; 2],
    handover: [u64; 3],
}
#[unsafe(link_section = ".persist")]
static mut REGION: core::mem::MaybeUninit<Region> = core::mem::MaybeUninit::uninit();
const _: () = assert!(core::mem::size_of::<Region>() <= 128);

// The boot stage's TX path check, handed to the application (A3.6b; 0066 addendum): written by
// the boot stage just before the jump (whole 64-bit words, so read only what was written this
// start), taken and cleared by the application. `Region::handover`.
const HANDOVER_MAGIC: u32 = 0x4737_4842; // "G7HB"

/// What the boot stage's TX path check found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handover {
    /// Microseconds from the boot stage's start to its first check.
    pub first_check_us: u32,
    /// A fault it found: index into the Table A3-6 pins, bit 7 the configuration (else the level).
    pub fault: Option<u8>,
    /// A mismatch run still open at the jump: pin index and its start, ms since the boot stage
    /// start (the DWT cycle counter runs on into the application).
    pub open: Option<(u8, u32)>,
    /// The reset flags of this start, read once by the boot stage (0068).
    pub reset: crate::reset::Flags,
}

fn handover_ptr() -> *mut u64 {
    // SAFETY: a field of the module's static, raw pointer only.
    unsafe { core::ptr::addr_of_mut!((*addr_of_mut!(REGION).cast::<Region>()).handover) as *mut u64 }
}

/// The boot stage, just before the jump.
pub fn write_handover(h: &Handover) {
    let f = h.fault.unwrap_or(0xFF) as u64;
    let (oi, os) = h.open.map_or((0xFFu64, 0u64), |(i, s)| (i as u64, s as u64));
    // SAFETY: the `.persist` block, 8-aligned; volatile; the magic word last.
    unsafe {
        let r = h.reset.bits() as u32;
        write_volatile(handover_ptr().add(2), r as u64 | (!r as u64) << 32);
        write_volatile(handover_ptr().add(1), f | oi << 8 | os << 32);
        write_volatile(handover_ptr(), HANDOVER_MAGIC as u64 | (h.first_check_us as u64) << 32);
    }
}

/// The application, at start: the boot stage's handover of this start, then cleared.
pub fn take_handover() -> Option<Handover> {
    // SAFETY: as `write_handover`; the boot stage wrote the words before the jump.
    let (w0, w1, w2) = unsafe { (read_volatile(handover_ptr()), read_volatile(handover_ptr().add(1)), read_volatile(handover_ptr().add(2))) };
    unsafe {
        write_volatile(handover_ptr(), 0);
    }
    if w0 as u32 != HANDOVER_MAGIC || w2 as u32 != !((w2 >> 32) as u32) {
        return None;
    }
    let b = |v: u64| (v != 0xFF).then_some(v as u8);
    Some(Handover {
        first_check_us: (w0 >> 32) as u32,
        fault: b(w1 & 0xFF),
        open: b((w1 >> 8) & 0xFF).map(|i| (i, (w1 >> 32) as u32)),
        reset: crate::reset::Flags::from_bits(w2 as u8),
    })
}

fn slot(i: usize) -> *mut u64 {
    // SAFETY: a static of this module; only raw pointers, read and written volatile.
    unsafe { (core::ptr::addr_of_mut!((*addr_of_mut!(REGION).cast::<Region>()).slots) as *mut [u64; SLOT_WORDS]).add(i) as *mut u64 }
}

fn read(i: usize) -> [u64; SLOT_WORDS] {
    let mut w = [0u64; SLOT_WORDS];
    for (k, x) in w.iter_mut().enumerate() {
        // SAFETY: `slot` points into the `.persist` block (8-aligned); volatile, as it outlives
        // the program.
        *x = unsafe { read_volatile(slot(i).add(k)) };
    }
    w
}

fn write(i: usize, w: &[u64; SLOT_WORDS]) {
    // SAFETY: as `read`. Values and check first, the header (magic, sequence) last: a reset in
    // between leaves this slot invalid and the other one is used.
    unsafe {
        for k in (1..SLOT_WORDS).rev() {
            write_volatile(slot(i).add(k), w[k]);
        }
        write_volatile(slot(i), w[0]);
    }
}

fn newest() -> Option<(usize, u32, Counters)> {
    Counters::newest(&read(0), &read(1))
}

/// At start, before any other use: `power_lost` from the reset flags (power-on or brown-out).
/// Returns the counters kept over the reset, and whether they were found (false after a power
/// loss, or when a previous stage overwrote the block: then they start again at 0).
pub fn start(power_lost: bool) -> (Counters, bool) {
    if !power_lost {
        if let Some((_, _, c)) = newest() {
            return (c, true);
        }
    }
    let z = Counters::default();
    write(0, &z.encode(0));
    write(1, &z.encode(0));
    (z, false)
}

/// The counters kept (zero if none; after [`start`] there always are some).
pub fn get() -> Counters {
    newest().map_or(Counters::default(), |n| n.2)
}

/// Keep new counters (call on every change).
pub fn set(c: &Counters) {
    let (i, seq) = match newest() {
        Some((_, _, old)) if old == *c => return,
        Some((i, s, _)) => (1 - i, s.wrapping_add(1)),
        None => (0, 1),
    };
    write(i, &c.encode(seq));
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
