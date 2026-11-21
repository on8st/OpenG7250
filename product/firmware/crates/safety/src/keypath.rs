// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Network key path of level 1 (spec B10.1, B10.5, B10.5a, B10.5b, B10.6, B10.10, A3.1a,
//! A3.9c): TX request edges from the owner's High Priority packets, CWX through the keyer with
//! break-in set (own transmission, hang delay) or clear (elements inside the owner's PTT
//! transmission), the DUC-specific settings (deferred while keyed), the keyed timeout, the CWX
//! key-down cap, the maximum continuous TX time, the unkeys for PA enable cleared and a mode
//! class change, and the `TX_STATE` register (Table B8-2).
//!
//! The lock stays in the types: a key needs a [`KeyPermit`] from [`TxCore::tx_request`], which
//! needs a [`crate::tx::Calibration`], which this build cannot make. Every request edge is
//! therefore refused with `NOT_CALIBRATED` and not latched, nothing keys, `TX_STATE` bit 0 stays
//! 0, and neither [`Keyer::key`] nor [`Keyer::cw_element`] is ever called (B10 intro; 0035,
//! 0036).
//!
//! Not in this module yet: the DUC FIFO and its underrun check (B10.5; with CW selected and
//! break-in clear it is waived and DUC I&Q dropped, B10.5b item 2.3), the TX frequency and
//! RF-path checks of A3.9, the ATU tune cycle (A3.9b). KEY keys nothing in v1.0 (0030; [`crate::cw`]).

use crate::cw::{CwKeyer, DucSettings};
use crate::tx::{ArmReject, KeyContext, KeyPermit, Refusal, RfActuator, Source, TxCore};

/// `TX_STATE` [23:16], last unkey reason (Table B8-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UnkeyReason {
    Released = 1,
    KeyedTimeout = 2,
    Underrun = 3,
    Limit = 4,
    ArmDropped = 5,
    Retune = 6,
    PaDisabled = 7,
    CwxCap = 9,
    ModeClass = 10,
}

/// Limit id `LIMIT_TX_TIME` (Table B8-3), `TX_STATE` [31:24] with reason 4.
pub const LIMIT_TX_TIME_ID: u8 = 7;

/// CWX paddle bit held longer than this unkeys (B10.5a): not before 1000 ms.
pub const CWX_CAP_MS: u64 = 1000;

/// The radio side of a key. Only a [`KeyPermit`] can call [`Keyer::key`]. `key` returns true
/// if the PA is now actually keyed (`TX_STATE` bit 0 means "PA actually keyed", Table B8-2).
/// `cw_element` switches the keyer's CW waveform on or off inside a keyed transmission; it is
/// called only between a successful `key` and the next `unkey`.
pub trait Keyer: RfActuator {
    fn key(&mut self, permit: &KeyPermit<'_>) -> bool;
    fn cw_element(&mut self, on: bool);
}

/// The TX request bits of one High Priority packet from the owner ([P2] bytes 4, 5; B10.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requests {
    /// Byte 4 bit [1].
    pub ptt: bool,
    /// Byte 5 bit [0]: host CW selected.
    pub cwx: bool,
    /// Byte 5 bits [1], [2]: the paddles; ignored while bit [0] is clear (B10.5a).
    pub dot: bool,
    pub dash: bool,
}

impl Requests {
    pub fn parse(p: &[u8]) -> Self {
        let (b4, b5) = (p.get(4).copied().unwrap_or(0), p.get(5).copied().unwrap_or(0));
        let cwx = b5 & 0x01 != 0;
        Requests { ptt: b4 & 0x02 != 0, cwx, dot: cwx && b5 & 0x02 != 0, dash: cwx && b5 & 0x04 != 0 }
    }
    /// The CWX TX request: bit [0] and at least one paddle (B10.5a).
    pub fn cwx_request(&self) -> bool {
        self.cwx && (self.dot || self.dash)
    }
}

#[derive(Debug)]
pub struct KeyPath {
    prev: Requests,
    req: Requests,
    source: Option<Source>,
    keyed_at: u64,
    last_hp_ms: u64,
    dot_since: Option<u64>,
    dash_since: Option<u64>,
    /// After a refused, stopped or capped CWX request: keying needs both paddles at 0 first.
    cwx_blocked: bool,
    /// Break-in clear: the CWX request has been 0 since the PTT key (B10.5b item 2.2).
    cwx_ready: bool,
    last_unkey: u8,
    limit_id: u8,
    pa_disabled: bool,
    outside: bool,
    /// DUC-specific settings in force, and the latest one received while keyed (B10.5b item 3).
    duc: DucSettings,
    pending: Option<DucSettings>,
    cw: CwKeyer,
    element_on: bool,
    /// Hang delays above 2000 ms clamped (Table B10-1; `GET_STATS`).
    pub hang_clamped: u32,
}

impl Default for KeyPath {
    fn default() -> Self {
        KeyPath::new()
    }
}

impl KeyPath {
    pub fn new() -> Self {
        KeyPath {
            prev: Requests::default(),
            req: Requests::default(),
            source: None,
            keyed_at: 0,
            last_hp_ms: 0,
            dot_since: None,
            dash_since: None,
            cwx_blocked: false,
            cwx_ready: false,
            last_unkey: 0,
            limit_id: 0,
            pa_disabled: true,
            outside: false,
            duc: DucSettings::SESSION_START,
            pending: None,
            cw: CwKeyer::default(),
            element_on: false,
            hang_clamped: 0,
        }
    }

    pub fn keyed(&self) -> Option<Source> {
        self.source
    }

    pub fn duc(&self) -> &DucSettings {
        &self.duc
    }

    /// `TX_STATE` (Table B8-2): bit 0 keyed, bit 3 last request outside the TX ranges, bit 4 PA
    /// disabled by the host, [11:8] source, [23:16] last unkey reason, [31:24] limit id.
    pub fn tx_state(&self) -> u32 {
        let src = match self.source {
            None => 0,
            Some(Source::Ptt) => 1,
            Some(Source::Cwx) => 2,
            Some(Source::Key) => 3,
            Some(Source::Atu) => 4,
        };
        (self.source.is_some() as u32)
            | ((self.outside as u32) << 3)
            | ((self.pa_disabled as u32) << 4)
            | (src << 8)
            | ((self.last_unkey as u32) << 16)
            | ((self.limit_id as u32) << 24)
    }

    /// The keyer feeds the transmission: a CWX transmission (break-in set), or the owner's PTT
    /// transmission with break-in clear and CW selected, once the request was 0 after the key.
    fn keyer_feeds(&self) -> bool {
        match self.source {
            Some(Source::Cwx) => true,
            Some(Source::Ptt) => !self.duc.break_in && self.duc.cw_select && self.cwx_ready,
            _ => false,
        }
    }

    /// The CWX cap ends this transmission (B10.5a; B10.5b item 2.4).
    fn cap_applies(&self) -> bool {
        match self.source {
            Some(Source::Cwx) => true,
            Some(Source::Ptt) => !self.duc.break_in && self.duc.cw_select,
            _ => false,
        }
    }

    fn end(&mut self) {
        self.cw.stop();
        self.element_on = false;
        // Settings received while keyed apply right after the unkey (B10.5b item 3; B10.6).
        if let Some(p) = self.pending.take() {
            self.duc = p;
        }
    }

    fn unkey(&mut self, why: UnkeyReason, tx: &mut TxCore, rf: &mut impl RfActuator) {
        if self.source.take().is_some() {
            rf.unkey();
            tx.unkeyed();
            self.last_unkey = why as u8;
            self.limit_id = if why == UnkeyReason::Limit { LIMIT_TX_TIME_ID } else { 0 };
            self.end();
        }
    }

    fn try_key(&mut self, s: Source, now: u64, tx: &mut TxCore, ctx: &KeyContext, rf: &mut impl Keyer) -> bool {
        let keyed = {
            // Bit 3 reports a request refused for its frequency; a request refused earlier
            // (not calibrated, not armed) says nothing about the frequency.
            let permit = match tx.tx_request(s, ctx) {
                Ok(p) => p,
                Err(e) => {
                    if e == Refusal::Arm(ArmReject::Inhibited) {
                        self.outside = !ctx.freq_in_range;
                    }
                    return false;
                }
            };
            rf.key(&permit)
        };
        self.outside = false;
        if keyed {
            self.source = Some(s);
            self.keyed_at = now;
            self.last_unkey = 0;
            self.limit_id = 0;
        } else {
            tx.unkeyed();
        }
        keyed
    }

    fn run_keyer(&mut self, now: u64, rf: &mut impl Keyer) {
        let on = if self.source.is_none() {
            false
        } else {
            let (d, a) = if self.keyer_feeds() && !self.cwx_blocked { (self.req.dot, self.req.dash) } else { (false, false) };
            self.cw.step(now, d, a, &self.duc)
        };
        if on != self.element_on {
            self.element_on = on;
            rf.cw_element(on);
        }
    }

    /// A DUC-specific packet from the owner (B10.6). While keyed, the hang delay included, it is
    /// deferred to the unkey (B10.5b item 3). Returns true if the hang delay was clamped.
    pub fn on_duc_specific(&mut self, p: &[u8]) -> bool {
        let (s, clamped) = DucSettings::parse(p);
        self.set_duc(s, clamped);
        clamped
    }

    /// Settings from a DUC-specific packet already parsed (a set-up packet taken at the session
    /// start, B5.1a item 2), with the same deferral while keyed.
    pub fn set_duc(&mut self, s: DucSettings, clamped: bool) {
        self.hang_clamped += clamped as u32;
        if self.source.is_some() {
            self.pending = Some(s);
        } else {
            self.duc = s;
        }
    }

    /// A High Priority packet from the owner (B10.1, B10.5b). Returns true if `TX_STATE` changed.
    pub fn on_high_priority(&mut self, r: Requests, now: u64, tx: &mut TxCore, ctx: &KeyContext, rf: &mut impl Keyer) -> bool {
        let before = self.tx_state();
        self.last_hp_ms = now;
        self.dot_since = if r.dot { self.dot_since.or(Some(now)) } else { None };
        self.dash_since = if r.dash { self.dash_since.or(Some(now)) } else { None };
        if !r.dot && !r.dash {
            self.cwx_blocked = false;
        }
        self.req = r;
        if self.source == Some(Source::Ptt) && !r.ptt {
            self.unkey(UnkeyReason::Released, tx, rf);
        }
        if self.source == Some(Source::Ptt) && !r.cwx_request() {
            self.cwx_ready = true;
        }
        let ptt_edge = r.ptt && !self.prev.ptt;
        let cwx_edge = r.cwx_request() && !self.prev.cwx_request();
        match self.source {
            None if ptt_edge => {
                // A DUC-specific refusal (EER, format) refuses PTT, CW selected or not (B10.5b
                // item 2.6); not latched.
                if !self.duc.refused() && self.try_key(Source::Ptt, now, tx, ctx, rf) {
                    self.cwx_ready = !r.cwx_request();
                }
            }
            None if cwx_edge && !self.cwx_blocked => {
                // CWX keys by itself only with break-in set and a keyer speed (B10.5b item 1,
                // Table B10-1); otherwise it forms no RF and is not latched (item 2.5).
                let ok = self.duc.break_in && self.duc.speed_wpm != 0 && self.try_key(Source::Cwx, now, tx, ctx, rf);
                if !ok {
                    self.cwx_blocked = true;
                }
            }
            // Break-in set: a CWX paddle edge during a PTT transmission is refused for that
            // element and the keyer stops (B10.5b item 1). Break-in clear without CW selected:
            // refused, no RF (item 2.3). A PTT edge during a CW transmission, its hang delay
            // included, is refused and not latched (item 1): nothing to do.
            Some(Source::Ptt) if cwx_edge && (self.duc.break_in || !self.duc.cw_select) => {
                self.cwx_blocked = true;
            }
            _ => {}
        }
        self.prev = r;
        self.run_keyer(now, rf);
        self.tx_state() != before
    }

    /// The keyer, the hang delay and the time supervision while keyed (B10.5, B10.5a, B10.5b,
    /// Table A3-2). Call at least every millisecond for the element timing; the unkey bounds
    /// allow 10 ms. Returns true if `TX_STATE` changed.
    pub fn tick(&mut self, now: u64, keyed_timeout_ms: u32, max_tx_ms: u32, tx: &mut TxCore, rf: &mut impl Keyer) -> bool {
        let Some(s) = self.source else { return false };
        self.run_keyer(now, rf);
        let held_over = |since: Option<u64>| since.is_some_and(|t| now.saturating_sub(t) > CWX_CAP_MS);
        if now.saturating_sub(self.last_hp_ms) > keyed_timeout_ms as u64 {
            // Also during the hang delay; it ends the hang delay (B10.5).
            self.unkey(UnkeyReason::KeyedTimeout, tx, rf);
        } else if self.cap_applies() && (held_over(self.dot_since) || held_over(self.dash_since)) {
            self.unkey(UnkeyReason::CwxCap, tx, rf);
            self.cwx_blocked = true;
        } else if now.saturating_sub(self.keyed_at) > max_tx_ms as u64 {
            self.unkey(UnkeyReason::Limit, tx, rf);
        } else if s == Source::Cwx
            && self.cw.idle()
            && !self.req.dot
            && !self.req.dash
            && self.cw.last_end().is_none_or(|e| now >= e + self.duc.hang_ms as u64)
        {
            // End of the hang delay after the last element: the CW transmission ends (A3.4d).
            self.unkey(UnkeyReason::Released, tx, rf);
        } else {
            return false;
        }
        true
    }

    /// General byte 58 bit [0] (B10.10): clearing it while keyed unkeys; setting it never keys.
    pub fn pa_enable(&mut self, enabled: bool, tx: &mut TxCore, rf: &mut impl RfActuator) -> bool {
        let before = self.tx_state();
        self.pa_disabled = !enabled;
        if !enabled {
            self.unkey(UnkeyReason::PaDisabled, tx, rf);
        }
        self.tx_state() != before
    }

    /// `TX_MODE_CLASS` changed (A3.9c): unkey first, then the new class applies.
    pub fn mode_class_changed(&mut self, tx: &mut TxCore, rf: &mut impl RfActuator) -> bool {
        let k = self.source.is_some();
        self.unkey(UnkeyReason::ModeClass, tx, rf);
        k
    }

    /// The arm was dropped (Table A3-1); [`TxCore::drop`] has already unkeyed.
    pub fn arm_dropped(&mut self) -> bool {
        if self.source.take().is_some() {
            self.last_unkey = UnkeyReason::ArmDropped as u8;
            self.limit_id = 0;
            self.end();
            return true;
        }
        false
    }

    /// Session end or takeover: request levels forgotten, PA enable cleared, DUC-specific
    /// settings back to their power-on values (A3.6a, Table A3-4).
    pub fn session_end(&mut self) {
        let keep = (self.last_unkey, self.limit_id, self.hang_clamped);
        *self = KeyPath::new();
        (self.last_unkey, self.limit_id, self.hang_clamped) = keep;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::{ARM_MAGIC, ArmContext, ArmOutcome, MEASURED, Measured};

    #[derive(Default)]
    struct Rf {
        log: std::vec::Vec<&'static str>,
    }
    impl RfActuator for Rf {
        fn unkey(&mut self) {
            self.log.push("unkey");
        }
    }
    impl Keyer for Rf {
        fn key(&mut self, _p: &KeyPermit<'_>) -> bool {
            self.log.push("key");
            true
        }
        fn cw_element(&mut self, on: bool) {
            self.log.push(if on { "on" } else { "off" });
        }
    }

    fn measured() -> Measured {
        Measured {
            bias_words: Some([0, 0, 0]),
            drive_map: Some(()),
            power_detector: Some(()),
            swr_limits: Some(()),
            temperature: Some(()),
            supply: Some(()),
            rf_off: Some(()),
            tr_timing: Some(()),
            atu: None,
        }
    }
    fn ctx() -> KeyContext {
        KeyContext { freq_in_range: true, rf_path_ready: true, pa_enabled: true, mode_class_declared: true, ..Default::default() }
    }
    fn armed() -> TxCore {
        let mut c = TxCore::new(1, &measured());
        let v = c.token() ^ ARM_MAGIC;
        let a = ArmContext { run: true, watchdog_ok: true, level1_active: true, mode_class_declared: true, ..Default::default() };
        assert_eq!(c.write_tx_arm(v, &a, &mut Rf::default()), ArmOutcome::Armed);
        c
    }
    fn ptt(on: bool) -> Requests {
        Requests { ptt: on, ..Default::default() }
    }
    fn cwx(dot: bool, dash: bool) -> Requests {
        Requests { cwx: true, dot, dash, ..Default::default() }
    }
    /// A DUC-specific packet: byte 5 flags, speed, hang delay.
    fn duc(b5: u8, wpm: u8, hang: u16) -> [u8; 60] {
        let mut p = [0u8; 60];
        p[5] = b5;
        p[9] = wpm;
        p[11..13].copy_from_slice(&hang.to_be_bytes());
        p
    }
    const BREAK_IN: u8 = 0x80;
    const CW: u8 = 0x02;
    const IAMBIC: u8 = 0x08;

    fn path() -> (TxCore, KeyPath, Rf) {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        k.pa_enable(true, &mut tx, &mut rf);
        (tx, k, rf)
    }

    /// High Priority every 10 ms and a tick every ms, from `from` to `to`.
    fn drive(k: &mut KeyPath, tx: &mut TxCore, rf: &mut Rf, from: u64, to: u64, r: impl Fn(u64) -> Requests) {
        for t in from..to {
            if t % 10 == 0 {
                k.on_high_priority(r(t), t, tx, &ctx(), rf);
            }
            k.tick(t, 500, 300_000, tx, rf);
        }
    }

    #[test]
    fn this_build_keys_nothing_and_never_calls_the_keyer() {
        // The lock is closed: every edge, every source, every DUC setting refused; the keyer is
        // never called.
        let mut tx = TxCore::new(1, &MEASURED);
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        for i in 0..40u64 {
            if i % 7 == 0 {
                k.on_duc_specific(&duc(if i % 2 == 0 { BREAK_IN | CW | IAMBIC } else { CW }, 20, 100));
            }
            let r = Requests { ptt: i % 2 == 0, cwx: true, dot: i % 3 == 0, dash: i % 4 == 0 };
            k.on_high_priority(r, i * 10, &mut tx, &ctx(), &mut rf);
            k.tick(i * 10 + 5, 500, 300_000, &mut tx, &mut rf);
        }
        assert!(rf.log.is_empty());
        assert!(k.keyed().is_none() && tx.keyed().is_none());
        assert_eq!(k.tx_state() & 0x0000_0F01, 0);
    }

    #[test]
    fn ptt_edge_keys_release_unkeys_and_a_held_bit_never_keys() {
        let (mut tx, mut k, mut rf) = path();
        assert!(k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), &mut rf));
        assert_eq!(k.tx_state() & 0xF01, 0x101);
        assert!(k.on_high_priority(ptt(false), 50, &mut tx, &ctx(), &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::Released as u32);
        assert_eq!(rf.log, ["key", "unkey"]);
        // A refused edge is not latched: PTT stays 1 while the frequency comes into range.
        let mut bad = ctx();
        bad.freq_in_range = false;
        k.on_high_priority(ptt(true), 100, &mut tx, &bad, &mut rf);
        assert_eq!(k.tx_state() & 0b1001, 0b1000); // bit 3: last request outside the ranges
        k.on_high_priority(ptt(true), 150, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        assert_eq!(rf.log.len(), 2);
    }

    #[test]
    fn keyed_timeout_not_before_and_by_plus_10_ms() {
        let (mut tx, mut k, mut rf) = path();
        k.on_high_priority(ptt(true), 1000, &mut tx, &ctx(), &mut rf);
        assert!(!k.tick(1300, 300, 300_000, &mut tx, &mut rf));
        assert!(k.tick(1301, 300, 300_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::KeyedTimeout as u32);
        // The arm is kept; a new edge is needed.
        assert!(tx.state() == crate::tx::ArmState::Armed);
        k.on_high_priority(ptt(true), 1400, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
    }

    #[test]
    fn max_tx_time_unkeys_with_limit_id_7() {
        let (mut tx, mut k, mut rf) = path();
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), &mut rf);
        let mut t = 0;
        while t < 60_000 {
            t += 50;
            k.on_high_priority(ptt(true), t, &mut tx, &ctx(), &mut rf);
            k.tick(t, 500, 60_000, &mut tx, &mut rf);
        }
        assert!(k.keyed().is_some());
        k.on_high_priority(ptt(true), 60_001, &mut tx, &ctx(), &mut rf);
        assert!(k.tick(60_001, 500, 60_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16, (LIMIT_TX_TIME_ID as u32) << 8 | UnkeyReason::Limit as u32);
    }

    #[test]
    fn break_in_cwx_keys_by_itself_holds_the_hang_delay_and_then_unkeys() {
        let (mut tx, mut k, mut rf) = path();
        // Break-in clear (session start): a CWX edge alone keys nothing.
        k.on_high_priority(cwx(true, false), 0, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        k.on_high_priority(cwx(false, false), 10, &mut tx, &ctx(), &mut rf);
        k.on_duc_specific(&duc(BREAK_IN | IAMBIC, 20, 200));
        // One dot (60 ms at 20 wpm), then 200 ms hang after its space.
        drive(&mut k, &mut tx, &mut rf, 20, 600, |t| cwx(t < 40, false));
        assert_eq!(rf.log, ["key", "on", "off", "unkey"]);
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::Released as u32);
        // A second dot inside the hang delay continues the same transmission.
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(BREAK_IN | IAMBIC, 20, 200));
        drive(&mut k, &mut tx, &mut rf, 0, 800, |t| cwx(t < 20 || (200..220).contains(&t), false));
        assert_eq!(rf.log, ["key", "on", "off", "on", "off", "unkey"]);
    }

    #[test]
    fn break_in_ptt_and_cw_never_share_a_transmission() {
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(BREAK_IN | IAMBIC, 20, 300));
        k.on_high_priority(cwx(true, false), 0, &mut tx, &ctx(), &mut rf);
        assert_eq!(k.keyed(), Some(Source::Cwx));
        // PTT edge during the CW transmission: refused, not latched, the CW transmission goes on.
        k.on_high_priority(Requests { ptt: true, ..cwx(false, false) }, 100, &mut tx, &ctx(), &mut rf);
        assert_eq!(k.keyed(), Some(Source::Cwx));
        drive(&mut k, &mut tx, &mut rf, 100, 1000, |_| ptt(true));
        assert!(k.keyed().is_none(), "the held PTT bit never keys");
        // CWX paddle edge during a PTT transmission: refused, the keyer forms nothing.
        k.on_high_priority(ptt(false), 1000, &mut tx, &ctx(), &mut rf);
        k.on_high_priority(ptt(true), 1010, &mut tx, &ctx(), &mut rf);
        assert_eq!(k.keyed(), Some(Source::Ptt));
        let n = rf.log.len();
        drive(&mut k, &mut tx, &mut rf, 1020, 1300, |_| Requests { ptt: true, ..cwx(true, false) });
        assert_eq!(rf.log.len(), n);
    }

    #[test]
    fn break_in_clear_elements_only_inside_ptt_after_a_new_edge() {
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(CW | IAMBIC, 20, 0));
        // Without PTT: nothing, not latched.
        k.on_high_priority(cwx(true, false), 0, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        // PTT with the paddle already held: the transmission keys, no element until the paddle
        // is released and pressed again.
        drive(&mut k, &mut tx, &mut rf, 10, 300, |_| Requests { ptt: true, ..cwx(true, false) });
        assert_eq!(rf.log, ["key"]);
        assert_eq!(k.tx_state() >> 8 & 0xF, 1, "TX source PTT");
        drive(&mut k, &mut tx, &mut rf, 300, 320, |_| ptt(true));
        drive(&mut k, &mut tx, &mut rf, 320, 400, |_| Requests { ptt: true, ..cwx(true, false) });
        assert_eq!(rf.log, ["key", "on", "off"]);
        // PTT release ends the transmission and the keyer.
        k.on_high_priority(Requests { ptt: false, ..cwx(true, false) }, 400, &mut tx, &ctx(), &mut rf);
        assert_eq!(rf.log.last(), Some(&"unkey"));
        // CW not selected: a paddle edge in the PTT transmission forms no element.
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(IAMBIC, 20, 0));
        drive(&mut k, &mut tx, &mut rf, 0, 50, |_| ptt(true));
        drive(&mut k, &mut tx, &mut rf, 50, 300, |_| Requests { ptt: true, ..cwx(true, false) });
        assert_eq!(rf.log, ["key"]);
    }

    #[test]
    fn cwx_cap_ends_the_cw_or_the_ptt_transmission_and_blocks() {
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(BREAK_IN | IAMBIC, 20, 100));
        k.on_high_priority(cwx(true, false), 0, &mut tx, &ctx(), &mut rf);
        let mut t = 0;
        while t < 1000 {
            t += 10;
            k.on_high_priority(cwx(true, false), t, &mut tx, &ctx(), &mut rf);
            assert!(!k.tick(t, 500, 300_000, &mut tx, &mut rf));
        }
        assert!(k.tick(1001, 500, 300_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::CwxCap as u32);
        // Dash pressed while dot still held: no key until both are 0.
        k.on_high_priority(cwx(true, true), 1010, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        k.on_high_priority(cwx(false, false), 1020, &mut tx, &ctx(), &mut rf);
        k.on_high_priority(cwx(true, false), 1030, &mut tx, &ctx(), &mut rf);
        assert_eq!(k.keyed(), Some(Source::Cwx));
        // Break-in clear with CW selected: the cap ends the PTT transmission (B10.5b item 2.4).
        let (mut tx, mut k, mut rf) = path();
        k.on_duc_specific(&duc(CW | IAMBIC, 20, 0));
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), &mut rf);
        drive(&mut k, &mut tx, &mut rf, 10, 1100, |_| Requests { ptt: true, ..cwx(true, false) });
        assert!(k.keyed().is_none());
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::CwxCap as u32);
    }

    #[test]
    fn duc_settings_while_keyed_wait_for_the_unkey_and_a_refusal_blocks_ptt() {
        let (mut tx, mut k, mut rf) = path();
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), &mut rf);
        k.on_duc_specific(&duc(BREAK_IN | CW, 20, 100));
        assert!(!k.duc().break_in, "deferred while keyed");
        k.on_high_priority(ptt(false), 10, &mut tx, &ctx(), &mut rf);
        assert!(k.duc().break_in && k.duc().cw_select);
        // EER set: the next PTT edge is refused and not latched.
        k.on_duc_specific(&duc(0x01, 20, 0));
        k.on_high_priority(ptt(true), 20, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        k.on_duc_specific(&duc(0, 20, 0));
        k.on_high_priority(ptt(true), 30, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none(), "held PTT does not key after the refusal clears");
        // Speed 0: CWX does not key even with break-in.
        k.on_duc_specific(&duc(BREAK_IN, 0, 0));
        k.on_high_priority(cwx(true, false), 40, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        // Session end: back to the power-on settings; the clamp counter stays.
        k.on_duc_specific(&duc(BREAK_IN, 20, 5000));
        k.session_end();
        assert_eq!(*k.duc(), DucSettings::SESSION_START);
        assert_eq!(k.hang_clamped, 1);
    }

    #[test]
    fn pa_enable_cleared_and_mode_class_change_unkey_and_never_rekey() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        assert_eq!(k.tx_state() & 0x10, 0x10); // PA disabled until the host sets it
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), &mut rf);
        assert!(k.pa_enable(false, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::PaDisabled as u32);
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 10, &mut tx, &ctx(), &mut rf);
        assert!(k.keyed().is_none());
        k.on_high_priority(ptt(false), 20, &mut tx, &ctx(), &mut rf);
        k.on_high_priority(ptt(true), 30, &mut tx, &ctx(), &mut rf);
        assert!(k.mode_class_changed(&mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::ModeClass as u32);
        assert_eq!(rf.log, ["key", "unkey", "key", "unkey"]);
    }

    #[test]
    fn requests_parse() {
        let mut p = [0u8; 1444];
        p[4] = 0x03;
        p[5] = 0x06; // paddles without bit 0: ignored
        assert_eq!(Requests::parse(&p), Requests { ptt: true, cwx: false, dot: false, dash: false });
        p[5] = 0x07;
        assert!(Requests::parse(&p).cwx_request());
    }
}
