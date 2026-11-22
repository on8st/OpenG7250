// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The log ring of A1.7, kept over every reset without power loss (0068): [`ENTRIES`] entries in
//! 64-bit words, each with a check word, and a header in two copies. Boot stage and application
//! use the same ring: the boot stage opens it at every start and writes the start entry, the
//! application continues it.
//!
//! Layout, in 64-bit words: two header copies of [`HEADER`] words (`[magic | copy sequence << 32,
//! first | next << 32, check]`), then the entries of [`ENTRY`] words (`[seq | uptime << 32,
//! length, text, check]`). Entry `seq` lies at `seq % ENTRIES`. `first` is the oldest sequence
//! number kept, `next` the next one to write. The memory is written in whole aligned 64-bit
//! words (the AXI SRAM's ECC word, [HIS] HIS 10).
//!
//! A reset at any point of [`Ring::push`] loses at most the entry being written: the header first
//! gives up the entry about to be overwritten, then the entry is written, then the header takes
//! it in; an entry complete but not yet in the header is found by its sequence number.

/// Text bytes of an entry.
pub const TEXT: usize = 72;
pub const ENTRIES: u32 = 64;
const HEADER: usize = 3;
const ENTRY: usize = 2 + TEXT / 8 + 1;
/// Size of the ring in 64-bit words.
pub const WORDS: usize = 2 * HEADER + ENTRIES as usize * ENTRY;
const MAGIC: u32 = 0x4737_4C47; // "G7LG"

/// The memory of a ring.
pub trait Words {
    /// A word, or `None` if it cannot be read (never written since power-up, or an ECC error).
    fn get(&self, i: usize) -> Option<u64>;
    fn set(&mut self, i: usize, v: u64);
}

impl Words for [u64; WORDS] {
    fn get(&self, i: usize) -> Option<u64> {
        Some(self[i])
    }
    fn set(&mut self, i: usize, v: u64) {
        self[i] = v;
    }
}

/// One entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub seq: u32,
    pub uptime_ms: u32,
    pub len: u8,
    pub text: [u8; TEXT],
}

impl Entry {
    pub fn text(&self) -> &[u8] {
        &self.text[..self.len as usize]
    }
}

fn check(words: &[u64]) -> u64 {
    // FNV-1a over the words, so that a ring of zeros does not check.
    let mut h = 0xCBF2_9CE4_8422_2325u64 ^ MAGIC as u64;
    for w in words {
        h = (h ^ w).wrapping_mul(0x0000_0100_0000_01B3);
        h ^= h >> 29;
    }
    h
}

pub struct Ring<W> {
    w: W,
    first: u32,
    next: u32,
    /// Sequence of the newest header copy.
    copy: u32,
}

impl<W: Words> Ring<W> {
    /// The ring at a start. `power_lost`: the memory was never written since power-up and is not
    /// read. Returns the ring and whether it was kept; a ring that is not kept starts empty, with
    /// sequence numbers from 0.
    pub fn open(w: W, power_lost: bool) -> (Self, bool) {
        let mut r = Ring { w, first: 0, next: 0, copy: 0 };
        if !power_lost && r.load() {
            return (r, true);
        }
        for i in 0..WORDS {
            r.w.set(i, 0);
        }
        (r.first, r.next, r.copy) = (0, 0, 0);
        r.header();
        (r, false)
    }

    /// Take in what the memory holds; false if the ring fails as a whole.
    fn load(&mut self) -> bool {
        let mut best: Option<(u32, u32, u32)> = None;
        for k in 0..2 {
            let Some(h) = self.words::<HEADER>(k * HEADER) else { continue };
            let (first, next) = (h[1] as u32, (h[1] >> 32) as u32);
            if h[0] as u32 != MAGIC || h[2] != check(&h[..2]) || next.wrapping_sub(first) > ENTRIES {
                continue;
            }
            let c = (h[0] >> 32) as u32;
            if best.is_none_or(|b| c.wrapping_sub(b.0) as i32 > 0) {
                best = Some((c, first, next));
            }
        }
        let Some((copy, first, next)) = best else { return false };
        self.copy = copy;
        self.first = first;
        // The entries that check, in order from the oldest, and one more if it was written but
        // not yet in the header; the first that fails and every entry after it are dropped.
        let mut n = 0;
        while n < ENTRIES && self.read(first.wrapping_add(n)).is_some() {
            n += 1;
        }
        self.next = first.wrapping_add(n);
        if n == 0 && next != first {
            return false;
        }
        for i in 0..ENTRIES {
            if i >= n {
                self.wipe(first.wrapping_add(i));
            }
        }
        self.header();
        true
    }

    /// `N` words from `at`, if every one can be read.
    fn words<const N: usize>(&self, at: usize) -> Option<[u64; N]> {
        let mut a = [0u64; N];
        for (i, w) in a.iter_mut().enumerate() {
            *w = self.w.get(at + i)?;
        }
        Some(a)
    }

    /// Word index of entry `seq`.
    fn at(seq: u32) -> usize {
        2 * HEADER + (seq % ENTRIES) as usize * ENTRY
    }

    fn read(&self, seq: u32) -> Option<Entry> {
        let at = Self::at(seq);
        let e = self.words::<ENTRY>(at)?;
        let len = e[1] as usize;
        if e[0] as u32 != seq || e[ENTRY - 1] != check(&e[..ENTRY - 1]) || len > TEXT {
            return None;
        }
        let mut text = [0u8; TEXT];
        for (c, w) in text.chunks_exact_mut(8).zip(&e[2..]) {
            c.copy_from_slice(&w.to_le_bytes());
        }
        Some(Entry { seq, uptime_ms: (e[0] >> 32) as u32, len: len as u8, text })
    }

    fn wipe(&mut self, seq: u32) {
        let at = Self::at(seq);
        for i in 0..ENTRY {
            self.w.set(at + i, 0);
        }
    }

    /// Write the header into the copy that is not the newest; its check word last.
    fn header(&mut self) {
        self.copy = self.copy.wrapping_add(1);
        let k = (self.copy % 2) as usize * HEADER;
        let h = [MAGIC as u64 | (self.copy as u64) << 32, self.first as u64 | (self.next as u64) << 32];
        self.w.set(k + 2, 0);
        self.w.set(k, h[0]);
        self.w.set(k + 1, h[1]);
        self.w.set(k + 2, check(&h));
    }

    /// Append an entry; the oldest gives way when the ring is full.
    pub fn push(&mut self, uptime_ms: u32, text: &[u8]) {
        let n = text.len().min(TEXT);
        let mut t = [0u8; TEXT];
        t[..n].copy_from_slice(&text[..n]);
        let mut e = [0u64; ENTRY];
        e[0] = self.next as u64 | (uptime_ms as u64) << 32;
        e[1] = n as u64;
        for (w, c) in e[2..].iter_mut().zip(t.chunks_exact(8)) {
            *w = u64::from_le_bytes(c.try_into().unwrap_or([0; 8]));
        }
        e[ENTRY - 1] = check(&e[..ENTRY - 1]);
        if self.next.wrapping_sub(self.first) == ENTRIES {
            self.first = self.first.wrapping_add(1);
            self.header();
        }
        let at = Self::at(self.next);
        self.w.set(at + ENTRY - 1, 0);
        for (i, w) in e.iter().enumerate() {
            self.w.set(at + i, *w);
        }
        self.next = self.next.wrapping_add(1);
        self.header();
    }

    /// Append a formatted entry, cut at [`TEXT`] bytes.
    pub fn log(&mut self, uptime_ms: u32, args: core::fmt::Arguments<'_>) {
        struct Cut([u8; TEXT], usize);
        impl core::fmt::Write for Cut {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                let n = s.len().min(TEXT - self.1);
                self.0[self.1..self.1 + n].copy_from_slice(&s.as_bytes()[..n]);
                self.1 += n;
                Ok(())
            }
        }
        let mut c = Cut([0; TEXT], 0);
        let _ = core::fmt::write(&mut c, args);
        self.push(uptime_ms, &c.0[..c.1]);
    }

    /// The memory itself (host tests: a reset).
    pub fn into_memory(self) -> W {
        self.w
    }

    /// Drop every entry; sequence numbers go on.
    pub fn clear(&mut self) {
        self.first = self.next;
        self.header();
        for s in 0..ENTRIES {
            self.wipe(s);
        }
    }

    /// The oldest sequence number kept, and the next one.
    pub fn range(&self) -> (u32, u32) {
        (self.first, self.next)
    }

    pub fn entry(&self, seq: u32) -> Option<Entry> {
        (seq.wrapping_sub(self.first) < self.next.wrapping_sub(self.first)).then(|| self.read(seq)).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::boxed::Box;
    use std::string::String;
    use std::vec::Vec;

    impl Words for Box<[u64; WORDS]> {
        fn get(&self, i: usize) -> Option<u64> {
            Some(self[i])
        }
        fn set(&mut self, i: usize, v: u64) {
            self[i] = v;
        }
    }

    /// Memory that stops taking writes after `left` words (a reset in the middle of a write).
    #[derive(Clone)]
    struct Cut {
        m: Box<[u64; WORDS]>,
        left: usize,
    }
    impl Words for Cut {
        fn get(&self, i: usize) -> Option<u64> {
            Some(self.m[i])
        }
        fn set(&mut self, i: usize, v: u64) {
            if self.left > 0 {
                self.left -= 1;
                self.m[i] = v;
            }
        }
    }

    /// Memory with words that cannot be read until they are written.
    struct Faulty {
        m: Box<[u64; WORDS]>,
        bad: Vec<bool>,
    }
    impl Words for Faulty {
        fn get(&self, i: usize) -> Option<u64> {
            (!self.bad[i]).then(|| self.m[i])
        }
        fn set(&mut self, i: usize, v: u64) {
            self.bad[i] = false;
            self.m[i] = v;
        }
    }

    #[test]
    fn unreadable_words_fail_their_check_and_the_start_goes_on() {
        // Never written since power-up (the flags did not show the power loss): log lost, then
        // an ordinary ring.
        let (mut r, kept) = Ring::open(Faulty { m: fresh(), bad: vec![true; WORDS] }, false);
        assert!(!kept);
        r.log(1, format_args!("start after software: log lost"));
        assert!(r.w.bad.iter().all(|b| !b), "every word written");
        let (r, kept) = Ring::open(r.w, false);
        assert!(kept);
        assert_eq!(texts(&r).len(), 1);
        // One word of a kept ring, at every place.
        for i in 0..WORDS {
            let n = 100;
            let mut bad = vec![false; WORDS];
            bad[i] = true;
            let (r, kept) = Ring::open(Faulty { m: filled(n), bad }, false);
            let (a, b) = r.range();
            if i < 2 * HEADER {
                assert!(kept && (a, b) == (n - ENTRIES, n), "a header copy: the other one holds ({i})");
            } else {
                // The entry in that place, and those after it, are dropped.
                let slot = ((i - 2 * HEADER) / ENTRY) as u32;
                let seq = (n - ENTRIES..n).find(|s| s % ENTRIES == slot).unwrap();
                assert_eq!(kept, seq != n - ENTRIES, "{i}");
                if kept {
                    assert_eq!((a, b), (n - ENTRIES, seq));
                }
            }
            assert!(r.w.bad.iter().all(|b| !b), "nothing unreadable is left behind ({i})");
        }
    }

    fn fresh() -> Box<[u64; WORDS]> {
        Box::new([0xDEAD_BEEF_0123_4567; WORDS])
    }

    fn texts<W: Words>(r: &Ring<W>) -> Vec<(u32, u32, String)> {
        let (a, b) = r.range();
        (a..b).map(|s| r.entry(s).map(|e| (e.seq, e.uptime_ms, String::from_utf8(e.text().to_vec()).unwrap())).unwrap()).collect()
    }

    fn filled(n: u32) -> Box<[u64; WORDS]> {
        let (mut r, kept) = Ring::open(fresh(), true);
        assert!(!kept);
        for i in 0..n {
            r.log(1000 + i, format_args!("entry {i}"));
        }
        r.w
    }

    #[test]
    fn kept_over_a_reset_with_the_sequence_going_on() {
        for n in [0, 1, 5, 63, 64, 65, 200] {
            let (mut r, kept) = Ring::open(filled(n), false);
            assert!(kept, "{n}");
            let t = texts(&r);
            assert_eq!(t.len() as u32, n.min(ENTRIES));
            for (seq, up, s) in &t {
                assert_eq!((*up, s.as_str()), (1000 + seq, format!("entry {seq}").as_str()));
            }
            r.log(7, format_args!("start after software: log kept"));
            let t = texts(&r);
            assert_eq!(t.last().unwrap(), &(n, 7, "start after software: log kept".into()));
        }
    }

    #[test]
    fn power_loss_or_a_ring_that_fails_as_a_whole_starts_at_0() {
        let (r, kept) = Ring::open(filled(10), true);
        assert!(!kept);
        assert_eq!(r.range(), (0, 0));
        // Memory never written, all zeros, all ones, and a ring with both headers broken.
        let mut broken = filled(10);
        broken[0] ^= 1;
        broken[HEADER] ^= 1;
        for m in [fresh(), Box::new([0; WORDS]), Box::new([u64::MAX; WORDS]), broken] {
            let (mut r, kept) = Ring::open(m, false);
            assert!(!kept);
            r.log(1, format_args!("start after power-on: log lost"));
            assert_eq!(texts(&r), [(0, 1, String::from("start after power-on: log lost"))]);
        }
        // One header broken: the other one holds.
        let mut one = filled(10);
        one[2] ^= 1;
        assert!(Ring::open(one, false).1);
    }

    #[test]
    fn a_corrupted_entry_and_those_after_it_are_dropped_t_a1_91() {
        for n in [10u32, 64, 150] {
            let first = n.saturating_sub(ENTRIES);
            for bad in [first, first + 1, first + 4, n - 1] {
                for word in [0, 1, 3, ENTRY - 1] {
                    let mut m = filled(n);
                    m[Ring::<[u64; WORDS]>::at(bad) + word] ^= 1 << 17;
                    let (mut r, kept) = Ring::open(m, false);
                    // The first one broken: nothing is left, the ring starts again.
                    assert_eq!(kept, bad != first, "n {n} bad {bad} word {word}");
                    if !kept {
                        assert_eq!(r.range(), (0, 0));
                        continue;
                    }
                    assert_eq!(r.range(), (first, bad));
                    assert_eq!(texts(&r).len() as u32, bad - first);
                    // The entries after it stay dropped at the next start too.
                    r.log(5, format_args!("new"));
                    let (r, kept) = Ring::open(r.w, false);
                    assert!(kept);
                    assert_eq!(r.range(), (first, bad + 1));
                }
            }
        }
    }

    #[test]
    fn a_reset_at_any_word_of_a_push_loses_at_most_that_entry() {
        for n in [0u32, 3, 63, 64, 130] {
            let base = filled(n);
            let first = n.saturating_sub(ENTRIES);
            for left in 0..40 {
                let (mut r, _) = Ring::open(base.clone(), false);
                let m = r.w.clone();
                r = Ring::open(m, false).0;
                let mut c = Ring { w: Cut { m: r.w, left }, first: r.first, next: r.next, copy: r.copy };
                c.log(9, format_args!("the last one"));
                let (r, kept) = Ring::open(c.w.m, false);
                assert!(kept, "n {n} left {left}");
                let t = texts(&r);
                let (a, b) = r.range();
                assert!(b == n || b == n + 1, "n {n} left {left}: {a}..{b}");
                assert!(a == first || a == first + 1);
                // Every older entry is there, in sequence with no gap.
                assert!(t.iter().enumerate().all(|(i, e)| e.0 == a + i as u32));
                if b == n + 1 {
                    assert_eq!(t.last().unwrap().2, "the last one");
                }
                if left == 39 {
                    assert_eq!(b, n + 1);
                }
            }
        }
    }

    #[test]
    fn clear_and_long_texts() {
        let (mut r, _) = Ring::open(filled(100), false);
        r.clear();
        assert_eq!(r.range(), (100, 100));
        r.log(1, format_args!("{:200}", "x"));
        let (r, kept) = Ring::open(r.w, false);
        assert!(kept, "an empty ring is kept, and one entry after a clear");
        assert_eq!(r.entry(100).unwrap().text().len(), TEXT);
        assert_eq!((r.entry(99), r.entry(101)), (None, None));
        let (mut r, _) = Ring::open(filled(3), false);
        r.clear();
        let (r, kept) = Ring::open(r.w, false);
        assert!(kept);
        assert_eq!(r.range(), (3, 3));
    }
}
