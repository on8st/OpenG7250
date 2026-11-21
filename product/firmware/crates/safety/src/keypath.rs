// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Network key path of level 1 (spec B10.1, B10.5, B10.5a, B10.10, A3.1a, A3.9c): TX request
//! edges from the owner's High Priority packets, the keyed timeout, the CWX key-down cap, the
//! maximum continuous TX time, the unkeys for PA enable cleared and a mode class change, and
//! the `TX_STATE` register (Table B8-2).
//!
//! The lock stays in the types: a key needs a [`KeyPermit`] from [`TxCore::tx_request`], which
//! needs a [`crate::tx::Calibration`], which this build cannot make. Every request edge is
//! therefore refused with `NOT_CALIBRATED` and not latched, nothing keys, `TX_STATE` bit 0 stays
//! 0, and [`Keyer::key`] is never called (B10 intro; 0035, 0036).
//!
//! Not in this module yet: break-in, the keyer element timing for CWX and the KEY input
//! (B10.5a, B10.5b, B10.9), the CW hang delay (A3.4d), the DUC FIFO underrun check (B10.5),
//! the RF-path checks of A3.9 and the ATU tune cycle (A3.9b). With break-in not parsed, CWX is
//! taken as break-in clear: it forms RF only inside a PTT transmission, which needs the keyer,
//! so a CWX edge alone never keys here.

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

/// The radio side of a key. Only a [`KeyPermit`] can call it. Returns true if the PA is now
/// actually keyed (`TX_STATE` bit 0 means "PA actually keyed", Table B8-2).
pub trait Keyer: RfActuator {
    fn key(&mut self, permit: &KeyPermit<'_>) -> bool;
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

#[derive(Debug, Default)]
pub struct KeyPath {
    prev: Requests,
    source: Option<Source>,
    keyed_at: u64,
    last_hp_ms: u64,
    dot_since: Option<u64>,
    dash_since: Option<u64>,
    /// After a refused or capped CWX request: keying needs both paddles at 0 first (A3.1a).
    cwx_blocked: bool,
    last_unkey: u8,
    limit_id: u8,
    pa_disabled: bool,
    outside: bool,
}

impl KeyPath {
    pub fn new() -> Self {
        KeyPath { pa_disabled: true, ..Default::default() }
    }

    pub fn keyed(&self) -> Option<Source> {
        self.source
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

    fn unkey(&mut self, why: UnkeyReason, tx: &mut TxCore, rf: &mut impl RfActuator) {
        if self.source.take().is_some() {
            rf.unkey();
            tx.unkeyed();
            self.last_unkey = why as u8;
            self.limit_id = if why == UnkeyReason::Limit { LIMIT_TX_TIME_ID } else { 0 };
        }
    }

    fn try_key(&mut self, s: Source, now: u64, tx: &mut TxCore, ctx: &KeyContext, rf: &mut impl Keyer) -> Result<(), Refusal> {
        let keyed = {
            // Bit 3 reports a request refused for its frequency; a request refused earlier
            // (not calibrated, not armed) says nothing about the frequency.
            let permit = match tx.tx_request(s, ctx) {
                Ok(p) => p,
                Err(e) => {
                    if e == Refusal::Arm(ArmReject::Inhibited) {
                        self.outside = !ctx.freq_in_range;
                    }
                    return Err(e);
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
        Ok(())
    }

    /// A High Priority packet from the owner (B10.1, B10.5). Returns true if `TX_STATE` changed.
    pub fn on_high_priority(&mut self, r: Requests, now: u64, tx: &mut TxCore, ctx: &KeyContext, break_in: bool, rf: &mut impl Keyer) -> bool {
        let before = self.tx_state();
        self.last_hp_ms = now;
        self.dot_since = if r.dot { self.dot_since.or(Some(now)) } else { None };
        self.dash_since = if r.dash { self.dash_since.or(Some(now)) } else { None };
        if !r.dot && !r.dash {
            self.cwx_blocked = false;
        }
        // Release of the request that keyed (B10.1; A3.3).
        match self.source {
            Some(Source::Ptt) if !r.ptt => self.unkey(UnkeyReason::Released, tx, rf),
            Some(Source::Cwx) if !r.cwx_request() => self.unkey(UnkeyReason::Released, tx, rf),
            _ => {}
        }
        if self.source.is_none() {
            // A 0->1 edge at a moment when every key condition holds; a refused request is not
            // latched (A3.1a). PTT first: a PTT edge and a CWX edge in one packet key as PTT.
            if r.ptt && !self.prev.ptt {
                let _ = self.try_key(Source::Ptt, now, tx, ctx, rf);
            } else if r.cwx_request() && !self.prev.cwx_request() && !self.cwx_blocked {
                // With break-in clear CWX forms RF only inside the owner's PTT transmission
                // (B10.5b), which needs the keyer; until then only break-in may key it.
                if !break_in || self.try_key(Source::Cwx, now, tx, ctx, rf).is_err() {
                    self.cwx_blocked = break_in;
                }
            }
        }
        self.prev = r;
        self.tx_state() != before
    }

    /// Time supervision while keyed (B10.5, B10.5a, Table A3-2). Call at least every 10 ms
    /// (tolerance +10 ms). Returns true if `TX_STATE` changed.
    pub fn tick(&mut self, now: u64, keyed_timeout_ms: u32, max_tx_ms: u32, tx: &mut TxCore, rf: &mut impl RfActuator) -> bool {
        let Some(s) = self.source else { return false };
        let held_over = |since: Option<u64>| since.is_some_and(|t| now.saturating_sub(t) > CWX_CAP_MS);
        if now.saturating_sub(self.last_hp_ms) > keyed_timeout_ms as u64 {
            self.unkey(UnkeyReason::KeyedTimeout, tx, rf);
        } else if s == Source::Cwx && (held_over(self.dot_since) || held_over(self.dash_since)) {
            self.unkey(UnkeyReason::CwxCap, tx, rf);
            self.cwx_blocked = true;
        } else if now.saturating_sub(self.keyed_at) > max_tx_ms as u64 {
            self.unkey(UnkeyReason::Limit, tx, rf);
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
            return true;
        }
        false
    }

    /// Session end or takeover: request levels forgotten, PA enable cleared (A3.6a).
    pub fn session_end(&mut self) {
        let last = (self.last_unkey, self.limit_id);
        *self = KeyPath::new();
        (self.last_unkey, self.limit_id) = last;
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

    #[test]
    fn this_build_keys_nothing_and_never_calls_the_keyer() {
        // The lock is closed: every edge, every source, refused; the keyer is never called.
        let mut tx = TxCore::new(1, &MEASURED);
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        for i in 0..20u64 {
            let r = Requests { ptt: i % 2 == 0, cwx: true, dot: i % 3 == 0, dash: i % 4 == 0 };
            k.on_high_priority(r, i * 10, &mut tx, &ctx(), i % 5 == 0, &mut rf);
            k.tick(i * 10 + 5, 500, 300_000, &mut tx, &mut rf);
        }
        assert!(rf.log.is_empty());
        assert!(k.keyed().is_none() && tx.keyed().is_none());
        assert_eq!(k.tx_state() & 0x0000_0F01, 0);
    }

    #[test]
    fn ptt_edge_keys_release_unkeys_and_a_held_bit_never_keys() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        k.pa_enable(true, &mut tx, &mut rf);
        assert!(k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), false, &mut rf));
        assert_eq!(k.tx_state() & 0xF01, 0x101);
        assert!(k.on_high_priority(ptt(false), 50, &mut tx, &ctx(), false, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::Released as u32);
        assert_eq!(rf.log, ["key", "unkey"]);
        // A refused edge is not latched: PTT stays 1 while the frequency comes into range.
        let mut bad = ctx();
        bad.freq_in_range = false;
        k.on_high_priority(ptt(true), 100, &mut tx, &bad, false, &mut rf);
        assert_eq!(k.tx_state() & 0b1001, 0b1000); // bit 3: last request outside the ranges
        k.on_high_priority(ptt(true), 150, &mut tx, &ctx(), false, &mut rf);
        assert!(k.keyed().is_none());
        assert_eq!(rf.log.len(), 2);
    }

    #[test]
    fn keyed_timeout_not_before_and_by_plus_10_ms() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 1000, &mut tx, &ctx(), false, &mut rf);
        assert!(!k.tick(1300, 300, 300_000, &mut tx, &mut rf));
        assert!(k.tick(1301, 300, 300_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::KeyedTimeout as u32);
        // The arm is kept; a new edge is needed.
        assert!(tx.state() == crate::tx::ArmState::Armed);
        k.on_high_priority(ptt(true), 1400, &mut tx, &ctx(), false, &mut rf);
        assert!(k.keyed().is_none());
    }

    #[test]
    fn max_tx_time_unkeys_with_limit_id_7() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), false, &mut rf);
        let mut t = 0;
        while t < 60_000 {
            t += 50;
            k.on_high_priority(ptt(true), t, &mut tx, &ctx(), false, &mut rf);
            k.tick(t, 500, 60_000, &mut tx, &mut rf);
        }
        assert!(k.keyed().is_some());
        k.on_high_priority(ptt(true), 60_001, &mut tx, &ctx(), false, &mut rf);
        assert!(k.tick(60_001, 500, 60_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16, (LIMIT_TX_TIME_ID as u32) << 8 | UnkeyReason::Limit as u32);
    }

    #[test]
    fn cwx_with_break_in_keys_and_a_paddle_held_over_1_s_unkeys_and_blocks() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        k.pa_enable(true, &mut tx, &mut rf);
        let dot = Requests { cwx: true, dot: true, ..Default::default() };
        // Break-in clear: a CWX edge alone does not key.
        k.on_high_priority(dot, 0, &mut tx, &ctx(), false, &mut rf);
        assert!(k.keyed().is_none());
        k.on_high_priority(Requests { cwx: true, ..Default::default() }, 10, &mut tx, &ctx(), true, &mut rf);
        k.on_high_priority(dot, 20, &mut tx, &ctx(), true, &mut rf);
        assert_eq!(k.keyed(), Some(Source::Cwx));
        let mut t = 20;
        while t < 1020 {
            t += 10;
            k.on_high_priority(dot, t, &mut tx, &ctx(), true, &mut rf);
            assert!(!k.tick(t, 500, 300_000, &mut tx, &mut rf));
        }
        assert!(k.tick(1021, 500, 300_000, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::CwxCap as u32);
        // Dash pressed while dot still held: no key until both are 0.
        let both = Requests { cwx: true, dot: true, dash: true, ..Default::default() };
        k.on_high_priority(both, 1030, &mut tx, &ctx(), true, &mut rf);
        k.on_high_priority(Requests { cwx: true, ..Default::default() }, 1040, &mut tx, &ctx(), true, &mut rf);
        k.on_high_priority(dot, 1050, &mut tx, &ctx(), true, &mut rf);
        assert_eq!(k.keyed(), Some(Source::Cwx));
    }

    #[test]
    fn pa_enable_cleared_and_mode_class_change_unkey_and_never_rekey() {
        let mut tx = armed();
        let mut k = KeyPath::new();
        let mut rf = Rf::default();
        assert_eq!(k.tx_state() & 0x10, 0x10); // PA disabled until the host sets it
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 0, &mut tx, &ctx(), false, &mut rf);
        assert!(k.pa_enable(false, &mut tx, &mut rf));
        assert_eq!(k.tx_state() >> 16 & 0xFF, UnkeyReason::PaDisabled as u32);
        k.pa_enable(true, &mut tx, &mut rf);
        k.on_high_priority(ptt(true), 10, &mut tx, &ctx(), false, &mut rf);
        assert!(k.keyed().is_none());
        k.on_high_priority(ptt(false), 20, &mut tx, &ctx(), false, &mut rf);
        k.on_high_priority(ptt(true), 30, &mut tx, &ctx(), false, &mut rf);
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
