// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Sequence-number gaps on ports 1027 and 1029 (B11.6; Table M-6 `gap_1027`, `gap_1029`; 0078).
//!
//! A gap is a packet whose sequence number is newer than the newest one received on that port
//! in the session by a difference, modulo 2^32, from 2 to 2^31 - 1; it counts once, whatever the
//! number of packets missing. 0 and 2^31 and above are not newer. On port 1027, where 0 is never
//! newer (B11.6a), a packet numbered 0 is left out and the step from 0xFFFFFFFF to 1 is a
//! difference of 1. The first packet of a session sets the start (on 1027 the first with a
//! non-zero number). The count runs from the start of the session and stays after its end.

#[derive(Clone, Copy, Debug)]
pub struct Gaps {
    newest: Option<u32>,
    /// Port 1027: 0 is not in the numbering.
    skip_zero: bool,
    pub count: u64,
}

impl Gaps {
    pub const fn new(skip_zero: bool) -> Self {
        Gaps { newest: None, skip_zero, count: 0 }
    }

    /// A session starts (also by a takeover): the count starts again from this session.
    pub fn session_start(&mut self) {
        self.newest = None;
        self.count = 0;
    }

    /// A packet of the session with sequence number `seq`.
    pub fn on(&mut self, seq: u32) {
        if self.skip_zero && seq == 0 {
            return;
        }
        let Some(n) = self.newest else {
            self.newest = Some(seq);
            return;
        };
        let mut d = seq.wrapping_sub(n);
        if d == 0 || d >= 1 << 31 {
            return; // not newer
        }
        if self.skip_zero && seq < n {
            d -= 1; // the step went past 0, which 1027 does not use
        }
        if d >= 2 {
            self.count += 1;
        }
        self.newest = Some(seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(skip_zero: bool, seqs: &[u32]) -> u64 {
        let mut g = Gaps::new(skip_zero);
        for &s in seqs {
            g.on(s);
        }
        g.count
    }

    #[test]
    fn the_cases_of_b11_6_t_b11_04() {
        // Port 1029: 10, 11, 14, 13, 15 give one gap; 0xFFFFFFFF then 0 none.
        assert_eq!(run(false, &[10, 11, 14, 13, 15]), 1);
        assert_eq!(run(false, &[0xFFFF_FFFF, 0]), 0);
        assert_eq!(run(false, &[0xFFFF_FFFE, 0]), 1);
        // Port 1027: 0xFFFFFFFF then 1 none; then 0 is left out; 0xFFFFFFFF then 2 one.
        assert_eq!(run(true, &[0xFFFF_FFFF, 1]), 0);
        assert_eq!(run(true, &[0xFFFF_FFFF, 0, 1, 2]), 0);
        assert_eq!(run(true, &[0xFFFF_FFFF, 2]), 1);
        assert_eq!(run(true, &[0, 0, 0]), 0, "a host that sends 0 in every packet");
        assert_eq!(run(true, &[0, 5, 6]), 0, "the start is the first non-zero number");
        // A gap counts once however many are missing; repeats and older numbers are no gap.
        assert_eq!(run(false, &[1, 1000, 1000, 999, 1001]), 1);
        assert_eq!(run(false, &[1, 1 + (1 << 31)]), 0, "2^31 is not newer");
        assert_eq!(run(false, &[1, (1 << 31)]), 1, "2^31 - 1 is newer");
        assert_eq!(run(false, &[1, 3, 5, 7]), 3);
    }

    #[test]
    fn a_session_start_starts_the_count_again() {
        let mut g = Gaps::new(false);
        for s in [1, 5, 9] {
            g.on(s);
        }
        assert_eq!(g.count, 2);
        g.session_start();
        assert_eq!(g.count, 0);
        g.on(100);
        g.on(101);
        assert_eq!(g.count, 0, "the first packet of the new session sets the start");
    }
}
