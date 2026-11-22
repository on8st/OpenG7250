// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Time over a flash operation. The time driver (vendored embassy-stm32, `time_driver/gp16.rs`)
//! counts half periods of a 16-bit counter at 1 MHz (32 768 us) in its interrupt, from one flag
//! per half. An erase runs with interrupts masked for seconds: at most two half periods can be
//! pending, the others were lost (uptime ran late by nearly every erase), and an odd number lost
//! left the period's parity wrong, with which the driver corrects a read that races its interrupt
//! (the time could step back by 65.5 ms).
//!
//! So the half periods are counted here, at the end and still masked: the length comes from the
//! counter itself, exact modulo 65 536 us, and from the core cycle counter, which runs on while
//! interrupts are masked, for the multiple of it. The result is exact as long as the two agree
//! within 32 768 us; the cycle counter wraps after 67 s, the longest operation is 8 s
//! (flash.rs `LIMIT_MS`). Both run from the same 64 MHz clock.

/// Ticks (us) of one half period.
pub const HALF: u32 = 32_768;
/// Core cycles per tick (the application asserts it against the core clock).
pub const CYCLES_PER_US: u32 = 64;

/// What the time driver and the core offer; the board implements it on the registers.
pub trait Clock {
    /// The 16-bit counter.
    fn cnt(&mut self) -> u16;
    /// Half periods flagged and not yet counted: the overflow and the halfway flag.
    fn flags(&mut self) -> (bool, bool);
    /// Clear those two flags and no other.
    fn clear_flags(&mut self);
    fn period(&mut self) -> u32;
    /// Set the period; alarms that are due then wake their tasks.
    fn set_period(&mut self, p: u32);
    /// The core cycle counter.
    fn cycles(&mut self) -> u32;
}

/// The time when interrupts were masked.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    period: u32,
    cnt: u16,
    cycles: u32,
}

/// Interrupts are masked: note the time, with a half period flagged and not yet counted.
pub fn begin(c: &mut impl Clock) -> Mark {
    loop {
        let cnt = c.cnt();
        let cycles = c.cycles();
        let (over, half) = c.flags();
        let period = c.period() + over as u32 + half as u32;
        // No boundary between the reads: the flags belong to this half.
        if (c.cnt() ^ cnt) & 0x8000 == 0 {
            return Mark { period, cnt, cycles };
        }
    }
}

/// Ticks from the mark to a counter value read now, `cycles` the cycle counter read with it.
fn ticks(m: &Mark, cnt: u16, cycles: u32) -> u32 {
    let low = cnt.wrapping_sub(m.cnt) as u32;
    let about = cycles.wrapping_sub(m.cycles) / CYCLES_PER_US;
    // The value nearest to `about` that the counter allows (about + HALF cannot overflow).
    if about + HALF <= low { low } else { low + (about + HALF - low) / 65_536 * 65_536 }
}

/// Before interrupts are unmasked: count the half periods since the mark. A span under one half
/// period is left to the interrupt, which has at most one to count. Returns whether it counted.
pub fn end(c: &mut impl Clock, m: &Mark) -> bool {
    loop {
        let cnt = c.cnt();
        let cycles = c.cycles();
        let d = ticks(m, cnt, cycles);
        if d < HALF {
            return false;
        }
        c.clear_flags();
        // A boundary up to here may have its flag cleared or not: once more. A later one is
        // flagged, not counted here, and the interrupt counts it.
        if (c.cnt() ^ cnt) & 0x8000 == 0 {
            let boundaries = ((m.cnt as u32 % HALF) + d) / HALF;
            c.set_period(m.period + boundaries);
            return true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counter and its flags; `t` in core cycles. Every access takes `step` cycles.
    struct Sim {
        t: u64,
        step: u64,
        over: bool,
        half: bool,
        period: u32,
        /// Error of the cycle counter, in parts per 100 000.
        skew: i64,
    }

    const HC: u64 = (HALF * CYCLES_PER_US) as u64;

    impl Sim {
        fn at(t: u64, step: u64) -> Sim {
            Sim { t, step, over: false, half: false, period: (t / HC) as u32, skew: 0 }
        }
        fn advance(&mut self, dt: u64) {
            let (a, b) = (self.t / HC, (self.t + dt) / HC);
            let n = b - a;
            if n >= 2 {
                self.over = true;
                self.half = true;
            } else if n == 1 {
                if b % 2 == 0 { self.over = true } else { self.half = true }
            }
            self.t += dt;
        }
        fn isr(&mut self) {
            self.period += self.over as u32 + self.half as u32;
            self.over = false;
            self.half = false;
        }
        /// The driver's `now()`, without its interrupt having run.
        fn now(&self) -> u64 {
            let cnt = (self.t / CYCLES_PER_US as u64) as u16;
            ((self.period as u64) << 15) + (cnt as u32 ^ ((self.period & 1) << 15)) as u64
        }
        fn truth(&self) -> u64 {
            self.t / CYCLES_PER_US as u64
        }
    }

    impl Clock for Sim {
        fn cnt(&mut self) -> u16 {
            let v = (self.t / CYCLES_PER_US as u64) as u16;
            self.advance(self.step);
            v
        }
        fn flags(&mut self) -> (bool, bool) {
            let v = (self.over, self.half);
            self.advance(self.step);
            v
        }
        fn clear_flags(&mut self) {
            self.over = false;
            self.half = false;
            self.advance(self.step);
        }
        fn period(&mut self) -> u32 {
            self.advance(self.step);
            self.period
        }
        fn set_period(&mut self, p: u32) {
            self.period = p;
            self.advance(self.step);
        }
        fn cycles(&mut self) -> u32 {
            let v = (self.t as i64 + self.t as i64 / 100_000 * self.skew) as u32;
            self.advance(self.step);
            v
        }
    }

    /// One masked operation of `len` cycles from `t0`: the time is exact, before the interrupt
    /// has run (a read racing it), after it, and over the next boundary; and it never decreases.
    fn one(t0: u64, len: u64, step: u64, skew: i64) {
        let mut s = Sim::at(t0, step);
        s.skew = skew;
        let mut last = s.now();
        let m = begin(&mut s);
        s.advance(len);
        end(&mut s, &m);
        let mut check = |s: &Sim, what: &str| {
            assert_eq!(s.now(), s.truth(), "{what}: start {t0} length {len} step {step}");
            assert!(s.now() >= last);
            last = s.now();
        };
        check(&s, "before the interrupt");
        s.isr();
        check(&s, "after it");
        let to_next = HC - s.t % HC;
        s.advance(to_next - 1);
        check(&s, "before the next boundary");
        s.advance(1);
        check(&s, "a read racing the next boundary");
        s.isr();
        check(&s, "after the next boundary");
    }

    const US: u64 = CYCLES_PER_US as u64;
    const MAX: u64 = 8_000_000 * US;

    #[test]
    fn every_start_phase_with_lengths_up_to_8_s() {
        let lens = [0, 580, 30_000, 32_700, 32_768, 33_000, 65_000, 65_536, 66_000, 98_304, 1_100_000, 1_120_000, 2_200_000, 3_999_999, 8_000_000];
        for phase in 0..65_536u64 {
            for l in lens {
                one(40 * HC + phase * US, l * US, 8, 0);
            }
        }
    }

    #[test]
    fn every_length_up_to_8_s_at_chosen_phases() {
        for phase in [0, 1, 777, 16_384, 32_767, 32_768, 32_769, 50_000, 65_535u64] {
            for l in (0..=8_000_000u64).step_by(7) {
                one(6 * HC + phase * US + l % US, l * US, 8, 0);
            }
        }
    }

    #[test]
    fn a_boundary_at_every_point_of_the_hand_over() {
        // Every access in turn meets the boundary, at start and at end, with slow accesses too.
        for step in [1, 8, 64, 640, 6_400] {
            for off in 0..40 * step.min(64) {
                let off = off * step / step.min(64);
                let d = 30 * step;
                one(4 * HC + HC - off.min(HC - 1), 5 * HC + d, step, 0);
                for n in [1, 2, 33, 34, 244] {
                    one(4 * HC + 100, n * HC - 100 - off, step, 0);
                }
            }
        }
    }

    #[test]
    fn the_cycle_counter_wraps_and_may_differ() {
        // A wrap inside the operation.
        let wrap = 1u64 << 32;
        for l in [1_100_000u64, 4_000_000, 8_000_000] {
            for before in [1, l / 3, l - 1] {
                one(wrap - before * US, l * US, 8, 0);
            }
        }
        const { assert!((1u64 << 32) / 64 > 8 * 8_000_000) };
        // 0.1 % apart, far more than two dividers of one clock can be: still exact.
        for skew in [-100, 100] {
            for l in (0..=8_000_000u64).step_by(9_973) {
                one(0, l * US, 8, skew);
            }
        }
        assert!(MAX < wrap);
    }

    #[test]
    fn a_half_period_pending_at_the_start_is_counted_once() {
        for len in [10, 580, 32_000, 33_000, 1_100_000, 4_000_000u64] {
            for phase in (0..32_000u64).step_by(101) {
                // Interrupts were masked across a boundary before the operation.
                let mut s = Sim::at(7 * HC - 5 * US, 8);
                s.advance(5 * US + phase * US);
                let m = begin(&mut s);
                s.advance(len * US);
                end(&mut s, &m);
                s.isr();
                assert_eq!(s.now(), s.truth(), "phase {phase} length {len}");
                // A period one short is hidden by the driver's race correction up to here.
                s.advance(HC - s.t % HC);
                assert_eq!(s.now(), s.truth(), "racing the next boundary: phase {phase} length {len}");
            }
        }
    }

    #[test]
    fn many_operations_in_a_row_lose_nothing() {
        // A compaction: an erase, then words, the interrupt sometimes late.
        let mut s = Sim::at(123_456, 8);
        let mut last = 0;
        for k in 0..20_000u64 {
            let len = if k % 500 == 0 { 1_100_000 + k * 37 } else { 100 + k % 480 };
            let m = begin(&mut s);
            s.advance(len * US);
            end(&mut s, &m);
            assert!(s.now() >= last);
            last = s.now();
            if k % 3 != 0 || len > 1_000 {
                s.isr();
            }
            assert_eq!(s.now(), s.truth());
            s.advance((k % 90) * US);
            s.isr();
        }
    }

    #[test]
    fn without_it_time_is_lost_and_can_step_back() {
        // What fw-01a108d does: the interrupt alone after the erase.
        let mut back = 0;
        for phase in (0..65_536u64).step_by(64) {
            let mut s = Sim::at(40 * HC + phase * US, 8);
            s.advance(1_100_000 * US);
            s.isr();
            assert!(s.truth() - s.now() > 900_000, "lost");
            s.advance(HC - s.t % HC - 1);
            let before = s.now();
            s.advance(1);
            back += (s.now() < before) as u32;
        }
        assert!(back > 0);
    }
}
