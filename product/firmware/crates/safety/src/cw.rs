// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The DUC-specific settings that the key path uses (spec B10.6, Table B10-1) and the CW keyer
//! for CWX (B10.5a, B10.5b): element timing at the keyer speed and weight, iambic mode A and B,
//! straight-key reading with iambic clear, strict character spacing.
//!
//! The keyer only decides when an element is on. The element's RF is the DUC CW waveform with
//! the ramp of byte 17 (B10.5b item 2.3), which needs the DUC on the FPGA bus; that bus stays
//! closed (fpga_gate), so the board's element output writes nothing yet. The hardware keyer from
//! KEY is not used in v1.0 (Table B10-1, 0030): KEY keys nothing in this firmware.
//!
//! Element timing (PARIS, 1200 / wpm ms per dot) and the weight reading are this firmware's: the
//! spec says "keyer speed and weight" ([P2] bytes 9-10) without a formula. Weight 50 is the
//! standard 1:1 dot and space; the weight moves on-time to off-time within one dot period, so
//! the speed stays; values outside 25-75 are clamped, and 0 reads as 50.

/// The DUC-specific fields the key path uses (Table B10-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DucSettings {
    /// Byte 5 bit [0] EER: refused (not offered), transmit refused while set.
    pub eer: bool,
    /// Byte 5 bit [1]: CW selected.
    pub cw_select: bool,
    /// Byte 5 bit [3]: iambic; clear = straight key for CWX (B10.5a).
    pub iambic: bool,
    /// Byte 5 bit [5]: mode B (mode A if clear).
    pub mode_b: bool,
    /// Byte 5 bit [6]: strict character spacing.
    pub strict: bool,
    /// Byte 5 bit [7]: break-in (B10.5b).
    pub break_in: bool,
    /// Byte 9, words per minute; 0: the keyer forms no element and CWX does not key.
    pub speed_wpm: u8,
    /// Byte 10.
    pub weight: u8,
    /// Bytes 11-12, ms, clamped to [`HANG_MAX_MS`].
    pub hang_ms: u16,
    /// Byte 13, ms: SEND before RF (A3.9a item 4; used by the key sequence).
    pub rf_delay_ms: u8,
    /// Bytes 14-15 are 0 or 192 and byte 16 is 0 or 24.
    pub format_ok: bool,
    /// Byte 17, ms: 5-10, else 8 (Table B10-1).
    pub ramp_ms: u8,
}

/// Longest hang delay (Table B10-1).
pub const HANG_MAX_MS: u16 = 2000;

impl DucSettings {
    /// The values at power-on and after every session end and takeover (A3.6a, Table A3-4).
    pub const SESSION_START: DucSettings = DucSettings {
        eer: false,
        cw_select: false,
        iambic: false,
        mode_b: false,
        strict: false,
        break_in: false,
        speed_wpm: 0,
        weight: 0,
        hang_ms: 0,
        rf_delay_ms: 0,
        format_ok: true,
        ramp_ms: 8,
    };

    /// A DUC-specific packet (at least 60 bytes, B11). Returns the settings and whether the hang
    /// delay was clamped (logged and counted, Table B10-1).
    pub fn parse(p: &[u8]) -> (DucSettings, bool) {
        let b = |i: usize| p.get(i).copied().unwrap_or(0);
        let b5 = b(5);
        let hang = u16::from_be_bytes([b(11), b(12)]);
        let rate = u16::from_be_bytes([b(14), b(15)]);
        let ramp = b(17);
        let s = DucSettings {
            eer: b5 & 0x01 != 0,
            cw_select: b5 & 0x02 != 0,
            iambic: b5 & 0x08 != 0,
            mode_b: b5 & 0x20 != 0,
            strict: b5 & 0x40 != 0,
            break_in: b5 & 0x80 != 0,
            speed_wpm: b(9),
            weight: b(10),
            hang_ms: hang.min(HANG_MAX_MS),
            rf_delay_ms: b(13),
            format_ok: matches!(rate, 0 | 192) && matches!(b(16), 0 | 24),
            ramp_ms: if (5..=10).contains(&ramp) { ramp } else { 8 },
        };
        (s, hang > HANG_MAX_MS)
    }

    /// A refusal of B10.6 stands: no PTT transmission keys (A3.1a).
    pub fn refused(&self) -> bool {
        self.eer || !self.format_ok
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum El {
    Dot,
    Dash,
}

impl El {
    fn other(self) -> El {
        match self {
            El::Dot => El::Dash,
            El::Dash => El::Dot,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    /// Element on until the time given (straight key: `u64::MAX`, ends on release).
    On(El, u64),
    /// The space after an element, until the time given.
    Space(El, u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    dot: u64,
    dot_on: u64,
    dash_on: u64,
    space: u64,
}

fn timing(s: &DucSettings) -> Option<Timing> {
    if s.speed_wpm == 0 {
        return None;
    }
    let dot = 1200 / s.speed_wpm as u64;
    let w = if s.weight == 0 { 50 } else { s.weight.clamp(25, 75) } as u64;
    let dot_on = dot * w / 50;
    Some(Timing { dot, dot_on, dash_on: 2 * dot + dot_on, space: 2 * dot - dot_on })
}

#[derive(Clone, Copy, Debug)]
pub struct CwKeyer {
    phase: Phase,
    /// The other paddle was pressed during the element or its space (mode B memory).
    mem: bool,
    /// End of the last element (its space included), for the hang delay and strict spacing.
    last_end: Option<u64>,
}

impl Default for CwKeyer {
    fn default() -> Self {
        CwKeyer { phase: Phase::Idle, mem: false, last_end: None }
    }
}

impl CwKeyer {
    /// No element in progress or due.
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// End of the last element, its space included.
    pub fn last_end(&self) -> Option<u64> {
        self.last_end
    }

    /// The keyer stops (end of the transmission, a refused element, B10.5a, B10.5b).
    pub fn stop(&mut self) {
        *self = CwKeyer::default();
    }

    /// Advance to `now` with the paddles as they are; returns whether an element is on.
    pub fn step(&mut self, now: u64, dot: bool, dash: bool, s: &DucSettings) -> bool {
        let Some(t) = timing(s) else {
            self.phase = Phase::Idle;
            return false;
        };
        let held = |e: El| match e {
            El::Dot => dot,
            El::Dash => dash,
        };
        if !s.iambic {
            // Straight key: key-down while a paddle bit is held, no element timing (B10.5a).
            let on = dot || dash;
            if on {
                self.phase = Phase::On(El::Dot, u64::MAX);
            } else if self.phase != Phase::Idle {
                self.phase = Phase::Idle;
                self.last_end = Some(now);
            }
            return on;
        }
        loop {
            match self.phase {
                Phase::Idle => {
                    if !dot && !dash {
                        return false;
                    }
                    // Strict spacing: a new character starts at least a letter space (3 dots)
                    // after the last element; its own space gave 1 of them.
                    if s.strict && self.last_end.is_some_and(|e| now < e + 2 * t.dot) {
                        return false;
                    }
                    let el = if dot { El::Dot } else { El::Dash };
                    self.start(el, now, &t);
                }
                Phase::On(el, until) => {
                    if held(el.other()) {
                        self.mem = true;
                    }
                    if now < until {
                        return true;
                    }
                    self.phase = Phase::Space(el, until + t.space);
                }
                Phase::Space(el, until) => {
                    if held(el.other()) {
                        self.mem = true;
                    }
                    if now < until {
                        return false;
                    }
                    let o = el.other();
                    let next = if (dot && dash) || (s.mode_b && self.mem) || held(o) {
                        Some(o)
                    } else if held(el) {
                        Some(el)
                    } else {
                        None
                    };
                    match next {
                        Some(e) => self.start(e, until, &t),
                        None => {
                            self.phase = Phase::Idle;
                            self.last_end = Some(until);
                            return false;
                        }
                    }
                }
            }
        }
    }

    fn start(&mut self, el: El, at: u64, t: &Timing) {
        self.mem = false;
        let on = match el {
            El::Dot => t.dot_on,
            El::Dash => t.dash_on,
        };
        self.phase = Phase::On(el, at + on);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iambic(wpm: u8, mode_b: bool) -> DucSettings {
        DucSettings { iambic: true, mode_b, speed_wpm: wpm, ..DucSettings::SESSION_START }
    }

    /// Run the keyer 1 ms at a time and return the on-intervals.
    fn run(k: &mut CwKeyer, s: &DucSettings, from: u64, to: u64, paddles: impl Fn(u64) -> (bool, bool)) -> Vec<(u64, u64)> {
        let mut out = vec![];
        let mut start = None;
        for t in from..to {
            let (d, a) = paddles(t);
            let on = k.step(t, d, a, s);
            match (on, start) {
                (true, None) => start = Some(t),
                (false, Some(b)) => {
                    out.push((b, t));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    #[test]
    fn dots_and_dashes_at_20_wpm() {
        let s = iambic(20, false); // dot 60 ms
        let mut k = CwKeyer::default();
        let dots = run(&mut k, &s, 0, 400, |_| (true, false));
        assert_eq!(&dots[..3], &[(0, 60), (120, 180), (240, 300)]);
        let mut k = CwKeyer::default();
        let dashes = run(&mut k, &s, 0, 500, |_| (false, true));
        assert_eq!(&dashes[..2], &[(0, 180), (240, 420)]);
    }

    #[test]
    fn squeeze_alternates_and_mode_b_adds_one_element() {
        let mut k = CwKeyer::default();
        let s = iambic(20, false);
        let e = run(&mut k, &s, 0, 600, |_| (true, true));
        assert_eq!(&e[..3], &[(0, 60), (120, 300), (360, 420)]); // dot dash dot
        // Squeeze released during the first dot: mode A stops, mode B sends the dash.
        let p = |t: u64| (t < 30, t < 30);
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, false), 0, 600, p), [(0, 60)]);
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, true), 0, 600, p), [(0, 60), (120, 300)]);
    }

    #[test]
    fn weight_straight_key_speed_0_and_strict_spacing() {
        let mut s = iambic(20, false);
        s.weight = 75; // dot on 90, space 30
        let mut k = CwKeyer::default();
        assert_eq!(&run(&mut k, &s, 0, 300, |_| (true, false))[..2], &[(0, 90), (120, 210)]);
        // Straight key: on while held.
        let st = DucSettings { speed_wpm: 20, ..DucSettings::SESSION_START };
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &st, 0, 1000, |t| (false, (100..700).contains(&t))), [(100, 700)]);
        // Speed 0: nothing.
        let mut k = CwKeyer::default();
        assert!(run(&mut k, &DucSettings::SESSION_START, 0, 500, |_| (true, true)).is_empty());
        // Strict spacing: a dot right after the last element waits for the letter space.
        let mut s = iambic(20, false);
        s.strict = true;
        let mut k = CwKeyer::default();
        let e = run(&mut k, &s, 0, 600, |t| (t < 10 || (130..260).contains(&t), false));
        assert_eq!(e, [(0, 60), (240, 300)]);
    }

    #[test]
    fn parse_and_session_start() {
        let mut p = [0u8; 60];
        p[5] = 0x80 | 0x40 | 0x20 | 0x08 | 0x02;
        p[9] = 25;
        p[10] = 50;
        p[11..13].copy_from_slice(&3000u16.to_be_bytes());
        p[14..16].copy_from_slice(&192u16.to_be_bytes());
        p[16] = 24;
        p[17] = 3;
        let (s, clamped) = DucSettings::parse(&p);
        assert!(clamped && s.hang_ms == 2000 && s.break_in && s.strict && s.mode_b && s.iambic && s.cw_select);
        assert!(!s.refused() && s.ramp_ms == 8);
        p[16] = 16;
        assert!(DucSettings::parse(&p).0.refused());
        p[16] = 0;
        p[5] = 1;
        assert!(DucSettings::parse(&p).0.refused());
        assert!(!DucSettings::SESSION_START.refused());
    }
}
