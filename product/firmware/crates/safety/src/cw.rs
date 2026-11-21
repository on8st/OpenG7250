// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The DUC-specific settings that the key path uses (spec B10.6, Table B10-1) and the CW keyer
//! for CWX (B10.5a "Keyer timing", decision 0046; B10.5b): unit, elements and spaces, weight,
//! iambic mode A and B (Curtis) with one element of memory per paddle, strict character
//! spacing, and the straight-key reading with iambic clear.
//!
//! The keyer only decides when an element is on. The element's RF is the DUC CW waveform with
//! the ramp of byte 17 (B10.5b item 2.3), which needs the DUC on the FPGA bus; that bus stays
//! closed (fpga_gate), so the board's element output writes nothing yet. The hardware keyer from
//! KEY is not used in v1.0 (Table B10-1, 0030): KEY keys nothing in this firmware.
//!
//! Element boundaries are kept in microseconds, so marks such as 79.2 ms (T-B10-44) are not
//! rounded per element; the keyer is advanced in milliseconds, which puts each key-on and
//! key-off event within 1 ms after its nominal time (tolerance ±1 ms or ±2 %, B10.5a item 6).

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
    /// Byte 9, words per minute, used up to [`SPEED_MAX_WPM`]; 0: the keyer forms no element
    /// and CWX does not key (B10.5a item 1).
    pub speed_wpm: u8,
    /// Byte 10 as received; the keyer reads it under B10.5a item 3.
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
/// Highest keyer speed honoured; above it 60 is used, logged and counted (B10.5a item 1; 0046).
pub const SPEED_MAX_WPM: u8 = 60;

/// Values of a DUC-specific packet used clamped, each logged (A1.7) and counted (`GET_STATS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Clamped {
    /// Hang delay above 2000 ms (Table B10-1).
    pub hang: bool,
    /// Keyer speed above 60 wpm (B10.5a item 1).
    pub speed: bool,
}

impl Clamped {
    pub fn any(&self) -> bool {
        self.hang || self.speed
    }
}

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

    /// A DUC-specific packet (at least 60 bytes, B11), and what was clamped.
    pub fn parse(p: &[u8]) -> (DucSettings, Clamped) {
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
            speed_wpm: b(9).min(SPEED_MAX_WPM),
            weight: b(10),
            hang_ms: hang.min(HANG_MAX_MS),
            rf_delay_ms: b(13),
            format_ok: matches!(rate, 0 | 192) && matches!(b(16), 0 | 24),
            ramp_ms: if (5..=10).contains(&ramp) { ramp } else { 8 },
        };
        (s, Clamped { hang: hang > HANG_MAX_MS, speed: b(9) > SPEED_MAX_WPM })
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
    /// Mark until the time given, µs (straight key: `u64::MAX`, ends on release).
    On(El, u64),
    /// The space after an element, until the time given, µs.
    Space(El, u64),
}

/// The unit *T* and the weight shift Δ, µs (B10.5a items 1, 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timing {
    t: u64,
    delta: i64,
}

impl Timing {
    fn of(s: &DucSettings) -> Option<Timing> {
        let speed = s.speed_wpm.min(SPEED_MAX_WPM);
        if speed == 0 {
            return None;
        }
        let t = 1_200_000 / speed as u64;
        let w = if s.weight == 0 { 50 } else { s.weight.clamp(33, 66) } as i64;
        Some(Timing { t, delta: t as i64 * (w - 50) / 50 })
    }
    /// Dot 1 T, dash 3 T, each lengthened by Δ (items 2, 3).
    fn mark(&self, el: El) -> u64 {
        let units = match el {
            El::Dot => 1,
            El::Dash => 3,
        };
        (units * self.t as i64 + self.delta) as u64
    }
    /// The 1 T space after a mark, shortened by Δ (items 2, 3).
    fn space(&self) -> u64 {
        (self.t as i64 - self.delta) as u64
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Paddles {
    dot: bool,
    dash: bool,
}

impl Paddles {
    fn get(&self, e: El) -> bool {
        match e {
            El::Dot => self.dot,
            El::Dash => self.dash,
        }
    }
    fn set(&mut self, e: El) {
        match e {
            El::Dot => self.dot = true,
            El::Dash => self.dash = true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CwKeyer {
    phase: Phase,
    /// One element of memory per paddle: pressed during the current element (item 4).
    mem: Paddles,
    /// The paddle of the current element was released during it (to see a new press).
    released: bool,
    /// Both paddles held at some time during the current element (mode B, item 4).
    squeeze: bool,
    /// The paddles at the last step (which one was pressed first, item 4).
    prev: Paddles,
    /// End of the last mark and of its space, µs (strict spacing, item 5; hang delay).
    last_mark_end: Option<u64>,
    last_end: Option<u64>,
}

impl Default for CwKeyer {
    fn default() -> Self {
        CwKeyer { phase: Phase::Idle, mem: Paddles::default(), released: false, squeeze: false, prev: Paddles::default(), last_mark_end: None, last_end: None }
    }
}

impl CwKeyer {
    /// No element in progress or due.
    pub fn idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// End of the last element, its space included, ms (rounded up).
    pub fn last_end(&self) -> Option<u64> {
        self.last_end.map(|us| us.div_ceil(1000))
    }

    /// The keyer stops (end of the transmission, a refused element, B10.5a, B10.5b).
    pub fn stop(&mut self) {
        *self = CwKeyer::default();
    }

    /// Advance to `now_ms` with the paddles as they are; returns whether a mark is on.
    pub fn step(&mut self, now_ms: u64, dot: bool, dash: bool, s: &DucSettings) -> bool {
        let now = now_ms * 1000;
        let p = Paddles { dot, dash };
        let on = self.advance(now, p, s);
        self.prev = p;
        on
    }

    fn advance(&mut self, now: u64, p: Paddles, s: &DucSettings) -> bool {
        let Some(t) = Timing::of(s) else {
            self.phase = Phase::Idle;
            return false;
        };
        if !s.iambic {
            // Straight key: key-down while a paddle bit is held, no element timing (B10.5a).
            let on = p.dot || p.dash;
            if on {
                self.phase = Phase::On(El::Dot, u64::MAX);
            } else if self.phase != Phase::Idle {
                self.phase = Phase::Idle;
                self.last_mark_end = Some(now);
                self.last_end = Some(now);
            }
            return on;
        }
        loop {
            match self.phase {
                Phase::Idle => {
                    if !p.dot && !p.dash {
                        return false;
                    }
                    // Strict spacing: a new character waits 3 T after the end of the last mark
                    // (item 5); clear: at once.
                    if s.strict && self.last_mark_end.is_some_and(|e| now < e + 3 * t.t) {
                        return false;
                    }
                    // Both held: the element of the paddle pressed first (item 4); dot if both
                    // came in the same step.
                    let el = if p.dot && p.dash && self.prev.dash && !self.prev.dot {
                        El::Dash
                    } else if p.dot {
                        El::Dot
                    } else {
                        El::Dash
                    };
                    self.start(el, now, &t);
                }
                Phase::On(el, until) => {
                    if p.dot && p.dash {
                        self.squeeze = true;
                    }
                    // Memory: the other paddle pressed during this element; one held from
                    // before is not a new press (alternation while both are held is below).
                    if p.get(el.other()) && !self.prev.get(el.other()) {
                        self.mem.set(el.other());
                    }
                    if !p.get(el) {
                        self.released = true;
                    } else if self.released {
                        self.mem.set(el);
                    }
                    if now < until {
                        return true;
                    }
                    self.last_mark_end = Some(until);
                    self.phase = Phase::Space(el, until + t.space());
                }
                Phase::Space(el, until) => {
                    if now < until {
                        return false;
                    }
                    let o = el.other();
                    let next = if !p.dot && !p.dash {
                        // Both released. Mode A stops: its memory acts only while a paddle is
                        // still held at the end of the space (SF-19, interim reading). Mode B:
                        // after a squeeze the opposite element, else the memory of a paddle
                        // pressed during the mark (item 4; SF-20: "during an element" = the mark).
                        if !s.mode_b {
                            None
                        } else if self.squeeze {
                            Some(o)
                        } else if self.mem.get(o) {
                            Some(o)
                        } else if self.mem.get(el) {
                            Some(el)
                        } else {
                            None
                        }
                    } else if self.mem.get(o) || (p.dot && p.dash) {
                        Some(o)
                    } else if p.get(el) || self.mem.get(el) {
                        Some(el)
                    } else {
                        Some(o)
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
        self.mem = Paddles::default();
        self.released = false;
        self.squeeze = false;
        self.phase = Phase::On(el, at + t.mark(el));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iambic(wpm: u8, mode_b: bool) -> DucSettings {
        DucSettings { iambic: true, mode_b, speed_wpm: wpm, ..DucSettings::SESSION_START }
    }

    /// Run the keyer 1 ms at a time and return the key-on and key-off events as intervals.
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

    /// Marks and the spaces between them, ms.
    fn marks_spaces(e: &[(u64, u64)]) -> (Vec<u64>, Vec<u64>) {
        (e.iter().map(|x| x.1 - x.0).collect(), e.windows(2).map(|w| w[1].0 - w[0].1).collect())
    }

    /// Within ±1 ms or ±2 % of the nominal µs (B10.5a item 6).
    fn within(got_ms: u64, nominal_us: u64) -> bool {
        let tol = (nominal_us / 50).max(1000);
        (got_ms * 1000).abs_diff(nominal_us) <= tol
    }

    #[test]
    fn t_b10_43_unit_marks_spaces_and_the_speed_clamp() {
        let s = iambic(20, false); // T = 60 ms
        let mut k = CwKeyer::default();
        let (m, sp) = marks_spaces(&run(&mut k, &s, 0, 1000, |_| (true, false)));
        assert!(m.iter().all(|&x| x == 60) && sp.iter().all(|&x| x == 60), "{m:?} {sp:?}");
        let mut k = CwKeyer::default();
        let (m, sp) = marks_spaces(&run(&mut k, &s, 0, 1500, |_| (false, true)));
        assert!(m.iter().all(|&x| x == 180) && sp.iter().all(|&x| x == 60), "{m:?} {sp:?}");
        let mut k = CwKeyer::default();
        let (m, _) = marks_spaces(&run(&mut k, &iambic(60, false), 0, 300, |_| (true, false)));
        assert!(m.iter().all(|&x| x == 20));
        // 70 wpm is used as 60, and reported as clamped.
        let mut p = [0u8; 60];
        p[9] = 70;
        let (d, c) = DucSettings::parse(&p);
        assert_eq!((d.speed_wpm, c), (60, Clamped { hang: false, speed: true }));
    }

    #[test]
    fn t_b10_44_weight_shifts_mark_and_space() {
        // (weight, nominal dot mark µs, nominal space µs) at 20 wpm; 80 is used as 66, 0 as 50.
        for (w, mark, space) in [(66, 79_200, 40_800), (33, 39_600, 80_400), (80, 79_200, 40_800), (0, 60_000, 60_000)] {
            let s = DucSettings { weight: w, ..iambic(20, false) };
            let mut k = CwKeyer::default();
            let e = run(&mut k, &s, 0, 1200, |_| (true, false));
            let (m, sp) = marks_spaces(&e);
            assert!(m.iter().all(|&x| within(x, mark)), "w {w}: {m:?}");
            assert!(sp.iter().all(|&x| within(x, space)), "w {w}: {sp:?}");
            // Dot plus space stays 2 T: the key-on events are 120 ms apart.
            assert!(e.windows(2).all(|x| x[1].0 - x[0].0 == 120), "w {w}: {e:?}");
        }
        // Dash plus space stays 4 T.
        let s = DucSettings { weight: 66, ..iambic(20, false) };
        let mut k = CwKeyer::default();
        let e = run(&mut k, &s, 0, 1500, |_| (false, true));
        assert!(e.windows(2).all(|x| x[1].0 - x[0].0 == 240));
        assert!(e.iter().all(|x| within(x.1 - x.0, 199_200)));
    }

    #[test]
    fn t_b10_45_strict_spacing_counts_3_t_from_the_end_of_the_mark() {
        // A dot (mark 0-60), then the paddle pressed 1.5 T (90 ms) after the end of the mark.
        let p = |t: u64| (t < 10 || (150..400).contains(&t), false);
        let s = DucSettings { strict: true, ..iambic(20, false) };
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &s, 0, 500, p)[1].0, 60 + 180);
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, false), 0, 500, p)[1].0, 150, "clear: at once");
        // SF-21 (interim): a paddle pressed and released before the 3 T starts nothing.
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &s, 0, 500, |t| (t < 10 || (150..200).contains(&t), false)), [(0, 60)]);
    }

    #[test]
    fn sf_19_mode_a_memory_acts_only_while_a_paddle_is_held_at_the_end_of_the_space() {
        // Dash held 0-150 (released during the mark), dot tapped 40-60 during the mark: nothing
        // held at the end of the space (240).
        let p = |t: u64| ((40..60).contains(&t), t < 150);
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, false), 0, 600, p), [(0, 180)], "mode A stops");
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, true), 0, 600, p), [(0, 180), (240, 300)], "mode B sends the dot");
        // Dash still held at 240: the remembered dot follows in mode A too.
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, false), 0, 600, |t| ((40..60).contains(&t), t < 250))[..2], [(0, 180), (240, 300)]);
    }

    #[test]
    fn t_b10_46_iambic_a_and_b_and_memory() {
        // Squeeze during a dash (dash 0-180), both released before its end.
        let p = |t: u64| (40 <= t && t < 100, t < 100);
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, false), 0, 600, p), [(0, 180)], "mode A ends after the dash");
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &iambic(20, true), 0, 600, p), [(0, 180), (240, 300)], "mode B: one dot");
        // A dot paddle pressed during a dash with the dash paddle held: a dot after the dash, in
        // both modes.
        let p = |t: u64| ((40..60).contains(&t), t < 250);
        for b in [false, true] {
            let mut k = CwKeyer::default();
            assert_eq!(run(&mut k, &iambic(20, b), 0, 600, p)[..2], [(0, 180), (240, 300)], "mode B {b}");
        }
        // Both held: alternate, starting with the paddle pressed first.
        let mut k = CwKeyer::default();
        let e = run(&mut k, &iambic(20, false), 0, 700, |t| (t >= 5, true));
        assert_eq!(&e[..3], &[(0, 180), (240, 300), (360, 540)]);
    }

    #[test]
    fn straight_key_and_speed_0() {
        let st = DucSettings { speed_wpm: 20, ..DucSettings::SESSION_START };
        let mut k = CwKeyer::default();
        assert_eq!(run(&mut k, &st, 0, 1000, |t| (false, (100..700).contains(&t))), [(100, 700)]);
        let mut k = CwKeyer::default();
        assert!(run(&mut k, &DucSettings::SESSION_START, 0, 500, |_| (true, true)).is_empty());
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
        let (s, c) = DucSettings::parse(&p);
        assert!(c.hang && !c.speed && s.hang_ms == 2000 && s.break_in && s.strict && s.mode_b && s.iambic && s.cw_select);
        assert!(!s.refused() && s.ramp_ms == 8);
        p[16] = 16;
        assert!(DucSettings::parse(&p).0.refused());
        p[16] = 0;
        p[5] = 1;
        assert!(DucSettings::parse(&p).0.refused());
        assert!(!DucSettings::SESSION_START.refused());
    }
}
