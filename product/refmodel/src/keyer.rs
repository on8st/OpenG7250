//! The CWX keyer of B10.5a (0046): a pure state machine in logical time. Inputs are paddle
//! changes at the time the radio received them; outputs are key-on / key-off transitions at
//! the exact times the rules give, so the model's own scheduling jitter never shows.
//!
//! Item 4 as amended (0046, spec/draft 7da25a9, rulings on SF-19..SF-21): the keyer decides at
//! the end of the 1 T space after each element and at a paddle edge while idle. Mode A has no
//! memory: only the paddles held at the decision point count. Mode B also counts the opposite
//! paddle pressed or held at any time during the mark of the current element, even if
//! released; the mark is the memory window, not the space. Strict spacing: at the 3 T point an
//! element starts only if a paddle is held then.
//!
//! GPL-3.0-or-later, (c) 2026 ON8ST.

pub const CAP_US: u64 = 1_000_000; // a paddle bit held continuously for more than 1 s unkeys

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elem {
    Dot,
    Dash,
}

impl Elem {
    fn other(self) -> Elem {
        match self {
            Elem::Dot => Elem::Dash,
            Elem::Dash => Elem::Dot,
        }
    }
}

/// Keyer settings of DUC-specific byte 5 bits [3], [5], [6] and bytes 9, 10 (Table B10-1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub iambic: bool,
    pub mode_b: bool,
    pub strict: bool,
    pub speed: u8,
    pub weight: u8,
}

impl Settings {
    /// The speed in force: 0 keys nothing, 1-60 honoured, above 60 used as 60 (item 1).
    pub fn wpm(&self) -> u32 {
        (self.speed as u32).min(60)
    }
    /// Weight in force: 0 read as 50, clamped to 33-66 (item 3).
    pub fn weight_in_force(&self) -> i64 {
        match self.weight {
            0 => 50,
            w => (w as i64).clamp(33, 66),
        }
    }
    /// Unit T in microseconds: 1200 / S ms (item 1).
    pub fn unit_us(&self) -> u64 {
        1_200_000 / self.wpm().max(1) as u64
    }
    /// (mark, space after it) in microseconds for an element (items 2, 3).
    pub fn lengths(&self, e: Elem) -> (u64, u64) {
        let t = self.unit_us() as i64;
        let d = t * (self.weight_in_force() - 50) / 50;
        let mark = match e {
            Elem::Dot => t,
            Elem::Dash => 3 * t,
        };
        ((mark + d) as u64, (t - d) as u64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    Idle,
    /// strict spacing: an element is due at this time if a paddle is still held
    Wait { at: u64 },
    Mark { e: Elem, end: u64 },
    Space { e: Elem, end: u64 },
    /// straight key (iambic clear): key-down while a bit is held
    Straight,
    /// stopped by the 1 s cap or a failed key condition: needs both bits 0 and a new edge
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Off {
    /// the end of a mark, or the release of a straight key
    Element,
    /// the 1 s cap of B10.5a (TX_STATE unkey reason 9)
    Cap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Out {
    On(u64),
    Off(u64, Off),
    /// the keyer went idle at this time after the request was released (TX request released)
    Released(u64),
}

#[derive(Debug, Clone)]
pub struct Keyer {
    st: State,
    pub set: Settings,
    dot: bool,
    dash: bool,
    dot_since: Option<u64>,
    dash_since: Option<u64>,
    first: Option<Elem>,
    /// mode B: the opposite paddle was pressed or held during the current mark
    mem_opp: bool,
    last_mark_end: Option<u64>,
}

impl Keyer {
    pub fn new(set: Settings) -> Keyer {
        Keyer {
            st: State::Idle,
            set,
            dot: false,
            dash: false,
            dot_since: None,
            dash_since: None,
            first: None,
            mem_opp: false,
            last_mark_end: None,
        }
    }

    pub fn keyed(&self) -> bool {
        matches!(self.st, State::Mark { .. } | State::Straight)
    }

    pub fn active(&self) -> bool {
        !matches!(self.st, State::Idle | State::Stopped)
    }

    /// The next time at which `advance` has something to do.
    pub fn next_due(&self) -> Option<u64> {
        let st = match self.st {
            State::Wait { at } => Some(at),
            State::Mark { end, .. } | State::Space { end, .. } => Some(end),
            _ => None,
        };
        let cap = [self.dot_since, self.dash_since].iter().flatten().map(|t| t + CAP_US).min();
        match (st, cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Process every boundary up to and including `now`, in order.
    pub fn advance(&mut self, now: u64, out: &mut Vec<Out>) {
        while let Some(t) = self.next_due() {
            if t > now {
                break;
            }
            self.step(t, out);
        }
    }

    fn cap_at(&self) -> Option<u64> {
        [self.dot_since, self.dash_since].iter().flatten().map(|t| t + CAP_US).min()
    }

    fn step(&mut self, t: u64, out: &mut Vec<Out>) {
        if self.cap_at() == Some(t) {
            if self.keyed() {
                out.push(Out::Off(t, Off::Cap));
            }
            self.st = State::Stopped;
            self.dot_since = None;
            self.dash_since = None;
            return;
        }
        match self.st {
            State::Wait { at } if at == t => {
                if self.dot || self.dash {
                    let e = self.pick_first();
                    self.start(e, t, out);
                } else {
                    self.st = State::Idle;
                }
            }
            State::Mark { e, end } if end == t => {
                out.push(Out::Off(t, Off::Element));
                self.last_mark_end = Some(t);
                let (_, space) = self.set.lengths(e);
                self.st = State::Space { e, end: t + space };
            }
            State::Space { e, end } if end == t => self.decide(e, t, out),
            _ => {}
        }
    }

    fn pick_first(&self) -> Elem {
        match (self.dot, self.dash) {
            (true, true) => self.first.unwrap_or(Elem::Dot),
            (true, false) => Elem::Dot,
            _ => Elem::Dash,
        }
    }

    fn start(&mut self, e: Elem, t: u64, out: &mut Vec<Out>) {
        let (mark, _) = self.set.lengths(e);
        out.push(Out::On(t));
        self.mem_opp = self.held(e.other());
        self.st = State::Mark { e, end: t + mark };
    }

    fn held(&self, e: Elem) -> bool {
        match e {
            Elem::Dot => self.dot,
            Elem::Dash => self.dash,
        }
    }

    /// The decision point at the end of the space after element `last` (item 4).
    fn decide(&mut self, last: Elem, t: u64, out: &mut Vec<Out>) {
        let opp = last.other();
        let opp_req = self.held(opp) || (self.set.mode_b && self.mem_opp);
        let next = if opp_req {
            Some(opp) // alternate (both held), or the opposite alone
        } else if self.held(last) {
            Some(last) // one paddle held: repeat its element
        } else {
            None
        };
        match next {
            Some(e) => self.start(e, t, out),
            None => {
                self.mem_opp = false;
                self.st = State::Idle;
                out.push(Out::Released(t));
            }
        }
    }

    /// A paddle change received at `t` (after `advance(t)`). Settings apply from the next
    /// element on.
    pub fn input(&mut self, t: u64, dot: bool, dash: bool, out: &mut Vec<Out>) {
        self.advance(t, out);
        let (pd, pa) = (self.dot, self.dash);
        if dot && !pd {
            self.dot_since = Some(t);
        }
        if !dot {
            self.dot_since = None;
        }
        if dash && !pa {
            self.dash_since = Some(t);
        }
        if !dash {
            self.dash_since = None;
        }
        // which paddle came first, for a squeeze from idle
        if !pd && !pa {
            self.first = match (dot, dash) {
                (true, false) => Some(Elem::Dot),
                (false, true) => Some(Elem::Dash),
                _ => None,
            };
        }
        self.dot = dot;
        self.dash = dash;
        let pressed_dot = dot && !pd;
        let pressed_dash = dash && !pa;
        match self.st {
            State::Stopped => {
                if !dot && !dash {
                    self.st = State::Idle;
                }
            }
            State::Idle => {
                if !(pressed_dot || pressed_dash) {
                    return;
                }
                if !self.set.iambic {
                    out.push(Out::On(t));
                    self.st = State::Straight;
                    return;
                }
                let wait_until = match self.last_mark_end {
                    Some(end) if self.set.strict => end + 3 * self.set.unit_us(),
                    _ => t,
                };
                if wait_until > t {
                    self.st = State::Wait { at: wait_until };
                } else {
                    let e = self.pick_first();
                    self.start(e, t, out);
                }
            }
            State::Straight => {
                if !dot && !dash {
                    out.push(Out::Off(t, Off::Element));
                    out.push(Out::Released(t));
                    self.last_mark_end = Some(t);
                    self.st = State::Idle;
                }
            }
            State::Mark { e, .. } => {
                if self.held(e.other()) {
                    self.mem_opp = true;
                }
            }
            State::Wait { .. } | State::Space { .. } => {}
        }
    }

    /// A key condition of A3.1a failed: stop; keying needs both bits 0 and a new edge.
    pub fn stop(&mut self, t: u64, out: &mut Vec<Out>) {
        if self.keyed() {
            out.push(Out::Off(t, Off::Element));
        }
        self.st = if self.dot || self.dash { State::Stopped } else { State::Idle };
    }

    /// Session end, takeover or settings that forbid keying: back to rest, nothing held.
    pub fn reset(&mut self) {
        let set = self.set;
        *self = Keyer::new(set);
    }
}
