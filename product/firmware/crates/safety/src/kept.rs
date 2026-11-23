// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The block of RAM kept over a reset that holds the counters and the boot stage's handover
//! (the board's persist.rs, which places it and reads it with bus faults ignored): what is taken
//! from it and what is written to it, for host tests with words that cannot be read.
//!
//! [`WORDS`] 64-bit words: two counter slots of [`SLOT_WORDS`], then three of the handover.

use crate::counters::{Counters, SLOT_WORDS};
use crate::logring::Words;
use crate::reset::Flags;

/// The first word of the handover.
pub const HANDOVER: usize = 2 * SLOT_WORDS;
pub const WORDS: usize = HANDOVER + 3;

const HANDOVER_MAGIC: u32 = 0x4737_4842; // "G7HB"

/// What the boot stage's TX path check found (A3.6b; 0066 addendum), and the reset flags of this
/// start: written by the boot stage just before the jump, taken and cleared by the application.
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
    pub reset: Flags,
}

/// The boot stage, just before the jump; the magic word last.
pub fn write_handover(m: &mut impl Words, h: &Handover) {
    let f = h.fault.unwrap_or(0xFF) as u64;
    let (oi, os) = h.open.map_or((0xFFu64, 0u64), |(i, s)| (i as u64, s as u64));
    let r = h.reset.bits() as u32;
    m.set(HANDOVER + 2, r as u64 | (!r as u64) << 32);
    m.set(HANDOVER + 1, f | oi << 8 | os << 32);
    m.set(HANDOVER, HANDOVER_MAGIC as u64 | (h.first_check_us as u64) << 32);
}

/// The application, at start: the boot stage's handover of this start, then cleared. `None` if
/// there is none or any word of it cannot be read.
pub fn take_handover(m: &mut impl Words) -> Option<Handover> {
    let words = (m.get(HANDOVER), m.get(HANDOVER + 1), m.get(HANDOVER + 2));
    m.set(HANDOVER, 0);
    let (Some(w0), Some(w1), Some(w2)) = words else { return None };
    if w0 as u32 != HANDOVER_MAGIC || w2 as u32 != !((w2 >> 32) as u32) {
        return None;
    }
    let b = |v: u64| (v != 0xFF).then_some(v as u8);
    Some(Handover {
        first_check_us: (w0 >> 32) as u32,
        fault: b(w1 & 0xFF),
        open: b((w1 >> 8) & 0xFF).map(|i| (i, (w1 >> 32) as u32)),
        reset: Flags::from_bits(w2 as u8),
    })
}

fn read(m: &impl Words, slot: usize) -> [u64; SLOT_WORDS] {
    let mut w = [0u64; SLOT_WORDS];
    for (k, x) in w.iter_mut().enumerate() {
        // A word that cannot be read makes the slot fail: zeros do not check.
        match m.get(slot * SLOT_WORDS + k) {
            Some(v) => *x = v,
            None => return [0; SLOT_WORDS],
        }
    }
    w
}

/// Values and check first, the header (magic, sequence) last: a reset in between leaves this slot
/// invalid and the other one is used.
fn write(m: &mut impl Words, slot: usize, w: &[u64; SLOT_WORDS]) {
    for k in (0..SLOT_WORDS).rev() {
        m.set(slot * SLOT_WORDS + k, w[k]);
    }
}

fn newest(m: &impl Words) -> Option<(usize, u32, Counters)> {
    Counters::newest(&read(m, 0), &read(m, 1))
}

/// At start, before any other use: `power_lost` from the reset flags (power-on or brown-out; the
/// block is then written and not read). Returns the counters kept over the reset, and whether they
/// were found (false: they start again at 0).
pub fn start(m: &mut impl Words, power_lost: bool) -> (Counters, bool) {
    if !power_lost {
        if let Some((_, _, c)) = newest(m) {
            return (c, true);
        }
    }
    let z = Counters::default();
    write(m, 0, &z.encode(0));
    write(m, 1, &z.encode(0));
    (z, false)
}

/// The counters kept (zero if none; after [`start`] there always are some).
pub fn get(m: &impl Words) -> Counters {
    newest(m).map_or(Counters::default(), |n| n.2)
}

/// Keep new counters, in the slot that is not the newest valid one.
pub fn set(m: &mut impl Words, c: &Counters) {
    let (i, seq) = match newest(m) {
        Some((_, _, old)) if old == *c => return,
        Some((i, s, _)) => (1 - i, s.wrapping_add(1)),
        None => (0, 1),
    };
    write(m, i, &c.encode(seq));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// The block with words that cannot be read (a whole write makes a word good again, as the
    /// ECC is written with it); every read is counted, the words written are kept in order.
    #[derive(Clone)]
    struct Ram {
        w: [u64; WORDS],
        bad: [bool; WORDS],
        reads: Cell<u32>,
        writes: Vec<usize>,
    }

    impl Words for Ram {
        fn get(&self, i: usize) -> Option<u64> {
            self.reads.set(self.reads.get() + 1);
            (!self.bad[i]).then_some(self.w[i])
        }
        fn set(&mut self, i: usize, v: u64) {
            self.w[i] = v;
            self.bad[i] = false;
            self.writes.push(i);
        }
    }

    const NEW: Counters = Counters { trip_count: 5, entries: [1, 2, 0], resets: [0, 3, 1, 0, 0] };
    const OLD: Counters = Counters { trip_count: 4, ..NEW };

    /// Slot `newer` holds NEW, the other OLD; a handover is there.
    fn ram(newer: usize) -> Ram {
        let mut m = Ram { w: [0; WORDS], bad: [false; WORDS], reads: Cell::new(0), writes: vec![] };
        write(&mut m, 1 - newer, &OLD.encode(8));
        write(&mut m, newer, &NEW.encode(9));
        write_handover(&mut m, &handover());
        m.writes.clear();
        m
    }

    fn handover() -> Handover {
        Handover { first_check_us: 17, fault: Some(0x83), open: Some((2, 41)), reset: Flags { software: true, pin: true, ..Flags::default() } }
    }

    #[test]
    fn an_unreadable_word_costs_its_slot_only() {
        for newer in 0..2 {
            for k in 0..SLOT_WORDS {
                // In the older slot: nothing lost, and the next write repairs it.
                let mut m = ram(newer);
                m.bad[(1 - newer) * SLOT_WORDS + k] = true;
                assert_eq!(start(&mut m, false), (NEW, true), "older slot, word {k}");
                assert!(m.writes.is_empty(), "nothing is written when counters were found");
                let next = Counters { trip_count: 6, ..NEW };
                set(&mut m, &next);
                assert_eq!(get(&m), next);
                assert_eq!(m.bad, [false; WORDS], "the slot was written in whole");
                // In the newer slot: the older one, as it was; a new value goes over the bad slot.
                let mut m = ram(newer);
                m.bad[newer * SLOT_WORDS + k] = true;
                assert_eq!(start(&mut m, false), (OLD, true), "newer slot, word {k}");
                assert_eq!(get(&m), OLD);
                set(&mut m, &next);
                assert_eq!(get(&m), next);
                assert!(m.writes.iter().all(|&i| i / SLOT_WORDS == newer), "the only good slot is never written");
                assert_eq!(m.bad, [false; WORDS]);
            }
        }
    }

    #[test]
    fn unreadable_words_in_both_slots_start_again_at_zero() {
        for a in 0..SLOT_WORDS {
            for b in 0..SLOT_WORDS {
                let mut m = ram(1);
                m.bad[a] = true;
                m.bad[SLOT_WORDS + b] = true;
                assert_eq!(get(&m), Counters::default());
                assert_eq!(start(&mut m, false), (Counters::default(), false), "words {a}, {b}");
                assert_eq!(m.bad, [false; WORDS], "both slots written in whole");
                assert_eq!(start(&mut m, false), (Counters::default(), true), "readable from now on");
            }
        }
        // Every word of the block, as RAM never written since power-up: nothing but the defaults.
        let mut m = ram(0);
        m.bad = [true; WORDS];
        assert_eq!(take_handover(&mut m), None);
        assert_eq!(start(&mut m, false), (Counters::default(), false));
        assert_eq!(get(&m), Counters::default());
        set(&mut m, &NEW);
        assert_eq!(get(&m), NEW);
    }

    #[test]
    fn a_set_with_no_readable_slot_writes_one() {
        let mut m = ram(0);
        m.bad[0] = true;
        m.bad[SLOT_WORDS] = true;
        set(&mut m, &NEW);
        assert_eq!(get(&m), NEW);
    }

    #[test]
    fn after_a_power_loss_the_counters_are_written_and_not_read() {
        let mut m = ram(0);
        m.bad = [true; WORDS];
        assert_eq!(start(&mut m, true), (Counters::default(), false));
        assert_eq!(m.reads.get(), 0);
        assert!((0..HANDOVER).all(|i| !m.bad[i]), "every counter word written");
        let mut m = ram(0);
        assert_eq!(start(&mut m, true), (Counters::default(), false), "also when the words are whole");
        assert_eq!(get(&m), Counters::default());
    }

    #[test]
    fn the_header_is_written_last_and_a_cut_write_leaves_the_other_slot() {
        let mut m = ram(0);
        let next = Counters { trip_count: 6, ..NEW };
        set(&mut m, &next);
        assert_eq!(m.writes, [11, 10, 9, 8, 7, 6], "slot 1, header last");
        for cut in 0..SLOT_WORDS {
            let mut x = ram(0);
            for &i in &m.writes[..cut] {
                x.set(i, m.w[i]);
            }
            // The word being written at the reset may be left unreadable.
            assert_eq!(get(&x), NEW, "cut after {cut} words");
            x.bad[m.writes[cut]] = true;
            assert_eq!(start(&mut x, false), (NEW, true), "cut in word {cut}");
        }
    }

    #[test]
    fn the_handover_is_taken_once_and_not_with_an_unreadable_word() {
        let mut m = ram(0);
        assert_eq!(take_handover(&mut m), Some(handover()));
        assert_eq!(take_handover(&mut m), None, "taken once");
        for k in 0..3 {
            let mut m = ram(0);
            m.bad[HANDOVER + k] = true;
            assert_eq!(take_handover(&mut m), None, "word {k}");
            assert_eq!(m.writes, [HANDOVER], "cleared all the same");
            assert_eq!(take_handover(&mut m), None);
            assert_eq!(start(&mut m, false), (NEW, true), "the counters are not touched");
        }
        // Not whole: no magic, or the flags do not match their complement.
        for k in [0, 2] {
            let mut m = ram(0);
            m.w[HANDOVER + k] ^= 1;
            assert_eq!(take_handover(&mut m), None, "word {k} changed");
        }
        let none = Handover { fault: None, open: None, ..handover() };
        write_handover(&mut m, &none);
        assert_eq!(m.writes.last(), Some(&HANDOVER), "the magic word last");
        assert_eq!(take_handover(&mut m), Some(none));
    }
}
