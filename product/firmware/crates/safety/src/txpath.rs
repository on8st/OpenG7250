// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The TX path check of A3.6b (0066 as corrected in c245f5e): while TX is not armed and not
//! keyed, the TX-side outputs of A3.6 are checked at least every [`PERIOD_MS`]; a mismatch seen
//! by every check over at least [`HOLD_MS`] is drop event 13, fault 16 `TX_PATH_NOT_BLOCKED`
//! (class 2). So a mismatch shorter than 10 ms never counts, and one of 20 ms or more always
//! does, within 20 ms of its start (first seen within 5 ms, then 10 ms of checks).
//!
//! The pins ([HIS] HIS 85): chain A output enable PA10 (TX low-pass bank and T/R; TX path
//! disable), the T/R lines PF4, PF6, PC7, PH11, and the PA DAC RESET PB14 (the bias channels:
//! the firmware never writes the DAC, so RESET held low keeps every output at VrefL; the DAC
//! value itself has no read-back). HIS 85 names no SEND or TUNER trigger pin.

use crate::safe_state::Port;

/// Check period, at most (A3.6b).
pub const PERIOD_MS: u64 = 5;
/// A mismatch seen by every check over this long is the fault (A3.6b).
pub const HOLD_MS: u64 = 10;

/// The TX-side outputs of A3.6 the check covers ([HIS] HIS 85), as (port, pin, safe level);
/// named in the safe state, the one place that names RF pins.
pub const PINS: [(Port, u8, bool); 6] = crate::safe_state::TX_PATH_PINS;

/// Which comparison failed (A3.6b items 1 and 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    /// Item 1: not an output of the kind A3.6 sets, or the output register not at the safe level.
    Configuration,
    /// Item 2: the input register does not read the safe level (held from outside).
    PinLevel,
    /// Item 1 for a PA bias channel (2, 7 or 8): the value last written is not 0 (the DAC cannot
    /// be read back; Table A3-6). Reported with the DAC's RESET pin.
    DacValue(u8),
}

/// The PA bias channels of the SPI2 DAC that must be 0 whenever not transmitting ([HIS] HIS 85).
pub const BIAS_CHANNELS: [u8; 3] = [2, 7, 8];

/// Item 1 for the bias channels: the first channel whose last written value (`last[i]` for
/// `BIAS_CHANNELS[i]`) is not 0.
pub fn bias_mismatch(last: [u16; 3]) -> Option<u8> {
    BIAS_CHANNELS.iter().zip(last).find(|(_, v)| *v != 0).map(|(c, _)| *c)
}

/// A mismatch on one pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mismatch {
    pub port: Port,
    pub pin: u8,
    pub what: Comparison,
}

/// Samples of the Table A3-6 pins taken in a flash operation's wait loop, where the watchdog is
/// fed (A3.6b, 0066 addendum (a): the 5 ms period does not apply during an erase or program; a
/// sample counts as a check). Times are core cycles (DWT); the arithmetic only, so that it can
/// be inlined into the RAM routine (no tables, no calls).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sampler {
    /// Start of the current run of samples with a mismatch, and its pin index.
    run: Option<(u32, u8)>,
    /// A run that lasted [`HOLD_MS`] (in cycles, `hold`): its pin index.
    pub held: Option<u8>,
}

impl Sampler {
    /// One sample at cycle `t`; `bad` is the index into [`PINS`] of a mismatch, or `None`;
    /// `hold` is [`HOLD_MS`] in cycles.
    #[inline(always)]
    pub fn sample(&mut self, t: u32, bad: Option<u8>, hold: u32) {
        match (bad, self.run) {
            (None, _) => self.run = None,
            (Some(i), None) => self.run = Some((t, i)),
            (Some(_), Some((s, i))) => {
                if self.held.is_none() && t.wrapping_sub(s) >= hold {
                    self.held = Some(i);
                }
            }
        }
    }

    /// The run still open when the operation ended at cycle `end`: cycles since its start, and
    /// its pin index.
    pub fn open_at(&self, end: u32) -> Option<(u32, u8)> {
        self.run.map(|(s, i)| (end.wrapping_sub(s), i))
    }
}

/// The time filter of A3.6b.
#[derive(Clone, Copy, Debug, Default)]
pub struct Check {
    /// Start of the current run of checks that all saw a mismatch.
    since: Option<u64>,
}

impl Check {
    pub const fn new() -> Self {
        Check { since: None }
    }

    /// After a flash operation (0066 addendum (a)), before the check that runs when it ends: a
    /// mismatch the wait-loop samples saw for [`HOLD_MS`] is the fault at once (`Some(true)`);
    /// one still present at the end counts from its first sample (`open_ms` before `now_ms`), so
    /// that the checks after the end give the fault no later than 20 ms after it.
    pub fn after_flash(&mut self, now_ms: u64, held: bool, open_ms: Option<u64>) -> bool {
        if held {
            return true;
        }
        if let Some(ms) = open_ms {
            let start = now_ms.saturating_sub(ms);
            self.since = Some(self.since.map_or(start, |s| s.min(start)));
        }
        false
    }

    /// One check at `now_ms` with its result. `tx_open`: armed or keyed (no check; the run
    /// ends). Returns the mismatch when it has lasted [`HOLD_MS`] over every check.
    pub fn observe(&mut self, now_ms: u64, tx_open: bool, seen: Option<Mismatch>) -> Option<Mismatch> {
        if tx_open {
            self.since = None;
            return None;
        }
        match seen {
            None => {
                self.since = None;
                None
            }
            Some(m) => {
                let s = *self.since.get_or_insert(now_ms);
                (now_ms.saturating_sub(s) >= HOLD_MS).then_some(m)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: Mismatch = Mismatch { port: PINS[1].0, pin: PINS[1].1, what: Comparison::PinLevel };

    /// Checks every `period` ms from 0 to `end`, the mismatch from `from` for `len` ms; the time
    /// of the fault, if any.
    fn run(period: u64, from: u64, len: u64, end: u64, open: impl Fn(u64) -> bool) -> Option<u64> {
        let mut c = Check::new();
        let mut t = 0;
        while t <= end {
            let seen = (t >= from && t < from + len).then_some(M);
            if c.observe(t, open(t), seen).is_some() {
                return Some(t);
            }
            t += period;
        }
        None
    }

    #[test]
    fn shorter_than_10_ms_never_20_ms_always_within_20_ms() {
        for phase in 0..PERIOD_MS {
            // T-A3-61: a 5 ms pulse gives no fault, nor does anything under 10 ms.
            for len in [1, 5, 9] {
                assert_eq!(run(PERIOD_MS, 100 + phase, len, 400, |_| false), None, "len {len} phase {phase}");
            }
            // 20 ms (and longer) always, within 20 ms of the start.
            for len in [20, 50, 1000] {
                let start = 100 + phase;
                let t = run(PERIOD_MS, start, len, 2000, |_| false).expect("fault");
                assert!(t - start <= 20, "len {len} phase {phase}: at +{}", t - start);
            }
        }
    }

    #[test]
    fn samples_during_flash_work_t_b5_22() {
        const HOLD: u32 = 10 * 64_000; // 10 ms at 64 MHz
        // Forced and released within the erase, seen for under 10 ms: no fault.
        let mut s = Sampler::default();
        for t in (0..2_000_000u32).step_by(1000) {
            let bad = (t >= 100_000 && t < 100_000 + HOLD - 64_000).then_some(1);
            s.sample(t, bad, HOLD);
        }
        assert_eq!((s.held, s.open_at(2_000_000)), (None, None));
        let mut c = Check::new();
        assert!(!c.after_flash(5_000, false, None));
        assert_eq!(c.observe(5_000, false, None), None);
        // Seen for 10 ms in the loop: the fault at once after the operation.
        let mut s = Sampler::default();
        for t in (0..2_000_000u32).step_by(1000) {
            s.sample(t, (t >= 100_000 && t < 100_000 + HOLD + 1000).then_some(2), HOLD);
        }
        assert_eq!(s.held, Some(2));
        assert!(Check::new().after_flash(5_000, true, None));
        // Forced just before the end of a long erase and kept: counted from its first sample, so
        // the checks after the end give it within 20 ms of the end.
        for before_end_ms in [0u64, 1, 5, 9, 15, 300] {
            let end = 4_000_000u32; // 62.5 ms
            let mut s = Sampler::default();
            for t in (0..=end).step_by(1000) {
                s.sample(t, (t as u64 >= (end as u64).saturating_sub(before_end_ms * 64_000)).then_some(0), HOLD);
            }
            let open = s.open_at(end).map(|(cyc, _)| cyc as u64 / 64_000);
            let held = s.held.is_some();
            let mut c = Check::new();
            let end_ms = 1_000u64;
            let mut fault_at = c.after_flash(end_ms, held, open).then_some(end_ms);
            let mut t = end_ms;
            while fault_at.is_none() && t < end_ms + 100 {
                if c.observe(t, false, Some(Mismatch { port: PINS[0].0, pin: PINS[0].1, what: Comparison::PinLevel })).is_some() {
                    fault_at = Some(t);
                }
                t += PERIOD_MS;
            }
            let at = fault_at.expect("fault");
            assert!(at - end_ms <= 20, "forced {before_end_ms} ms before the end: fault {} ms after it", at - end_ms);
        }
    }

    #[test]
    fn bias_channels_by_the_value_last_written() {
        assert_eq!(bias_mismatch([0, 0, 0]), None);
        assert_eq!(bias_mismatch([0, 5, 0]), Some(7));
        assert_eq!(bias_mismatch([1, 0, 9]), Some(2));
    }

    #[test]
    fn no_check_while_armed_or_keyed() {
        // The path open while armed: no fault, however long; the run starts again after it.
        assert_eq!(run(PERIOD_MS, 0, 1000, 1000, |_| true), None);
        assert_eq!(run(PERIOD_MS, 0, 1000, 1000, |t| t % 50 < 45), None, "an arm every 50 ms keeps resetting the run");
    }

    #[test]
    fn the_pins_are_the_tx_side_of_his_85() {
        // PA10 high (outputs disabled), PF4 high, PF6 PC7 PH11 low (RX levels), PB14 low (DAC
        // RESET): the same levels as the safe state.
        for (port, pin, level) in PINS {
            let r = crate::safe_state::rule(port, pin).expect("in the safe state");
            let want = if level { crate::safe_state::Action::DriveHigh } else { crate::safe_state::Action::DriveLow };
            assert_eq!(r.action, want, "{port:?}{pin}");
        }
    }
}
