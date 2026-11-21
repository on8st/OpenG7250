// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Transmit gate of the safety core: arm handshake (spec B10.2), drop events (A3.2, Table
//! A3-1), the key rule's arm and edge conditions (A3.1a), `ATU_TUNE` (Table B8-2; 0032), and the
//! calibration gate (ON8ST, 2026-11-20: TX is part of r1.0, but can only be armed once the
//! measured values are in the spec; until then every TX request is rejected with
//! `NOT_CALIBRATED`).
//!
//! The gate is in the types. Keying needs a [`KeyPermit`]; only [`TxCore::tx_request`] makes
//! one, only while armed, and only from a [`Calibration`]. A `Calibration` is made only by
//! [`Calibration::from_measured`] from a complete [`Measured`] set. This build's set,
//! [`MEASURED`], is empty, so no `Calibration`, no arm and no key can exist at run time.
//!
//! Wire codes are those of Table B8-3 (spec at 0e075f2): arm reject 11 `TX_NOT_IMPLEMENTED`
//! (record `tx` bit 1 clear) comes before 12 `NOT_CALIBRATED` (record `tx` bit 4 clear, Annex P
//! parameters not set) in the order of B10.2 item 3. This is r1.0 firmware, so
//! [`TX_IMPLEMENTED`] is true (0036): an arm that meets every other condition gets 12 until the
//! calibration set is complete; 11 is reachable only in a host-test core with bit 1 clear.

/// `ARM_MAGIC` = 0x47375458, ASCII "G7TX" (Table B8-2 note; 0020 §7).
pub const ARM_MAGIC: u32 = 0x4737_5458;

/// Arm reject reasons, `TX_ARM_STATE` [15:8] (Table B8-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ArmReject {
    NotRunning = 2,
    WatchdogDisabled = 3,
    InFault = 4,
    Inhibited = 5,
    PttActive = 6,
    LevelNotActive = 7,
    Maintenance = 8,
    NoModeClass = 9,
    BadToken = 10,
    TxNotImplemented = 11,
    NotCalibrated = 12,
}

/// Record `tx` bit 1: TX implemented in this firmware (B4.8). r1.0 implements transmit (0036);
/// keying stays behind the calibration gate ([`Calibration`]).
pub const TX_IMPLEMENTED: bool = true;

/// Drop reasons, `TX_ARM_STATE` [23:16] (Table B8-3; Table A3-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DropReason {
    HostDisarm = 1,
    RunCleared = 2,
    Watchdog = 3,
    Takeover = 4,
    Reset = 5,
    Fault = 6,
    LinkDown = 8,
    SessionChanged = 9,
    LevelChange = 10,
    Maintenance = 11,
    Rediscovery = 12,
}

/// Write reject reasons, `WRITE_STATUS` [7:0] (Table B8-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WriteReject {
    UnknownAddress = 1,
    ReadOnly = 2,
    NotOwner = 3,
    BadValue = 4,
    LooserLimit = 5,
    Busy = 6,
    NotClearable = 7,
    Keyed = 8,
    NotArmed = 9,
    NotKeyable = 10,
    NotCalibrated = 11,
}

/// TX sources, `TX_STATE` [11:8] (Table B8-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Ptt,
    Cwx,
    Key,
    Atu,
}

/// What is missing before TX may be armed. Each item is a value the spec must hold, from
/// measurement on the radio ([HIS] HIS 85: bias, detector laws and conversion constants "must
/// be established by measurement"; HIS-08, HIS-11, HIS-12; 0032 item 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Measured {
    /// PA bias DAC words for channels 2, 7, 8 while transmitting (HIS 85, SPI2; HIS-12).
    pub bias_words: Option<[u16; 3]>,
    /// Drive DAC (channel 1) mapping from the P2 drive level (B10, 0007 rule 4; HIS-12).
    pub drive_map: Option<()>,
    /// Forward and reflected detector constants (HIS 85 ADC1 PF11, PA6; HIS-11; 0006).
    pub power_detector: Option<()>,
    /// SWR thresholds S1, S2 and the minimum forward power (Table A3-2; HIS-11, HIS-12).
    pub swr_limits: Option<()>,
    /// PA temperature thresholds T1-T4 and the NTC law (Table A3-2; HIS 85 PB1; HIS-11, HIS-12).
    pub temperature: Option<()>,
    /// Supply voltage and PA current scaling (HIS 85 PA1_C, PC4; HIS-11).
    pub supply: Option<()>,
    /// "RF while not keyed" threshold `P_rf_off` (Table A3-2).
    pub rf_off: Option<()>,
    /// T/R timing and relay settle times (A3.9a; HIS-07).
    pub tr_timing: Option<()>,
    /// ATU latch meanings, safe values, tune power and timing (0032 item 2; HIS-08).
    pub atu: Option<()>,
}

impl Measured {
    pub const NONE: Measured = Measured {
        bias_words: None,
        drive_map: None,
        power_detector: None,
        swr_limits: None,
        temperature: None,
        supply: None,
        rf_off: None,
        tr_timing: None,
        atu: None,
    };

    /// Everything needed to arm TX (the ATU parameters are needed for `ATU_TUNE` only).
    pub fn complete_for_tx(&self) -> bool {
        self.bias_words.is_some()
            && self.drive_map.is_some()
            && self.power_detector.is_some()
            && self.swr_limits.is_some()
            && self.temperature.is_some()
            && self.supply.is_some()
            && self.rf_off.is_some()
            && self.tr_timing.is_some()
    }
}

/// The measured values of this build. Empty until the spec holds them; then filled from the
/// spec, never from the stock firmware or the EEPROM (clean room; [HIS] HIS 15 rule 3).
pub const MEASURED: Measured = Measured::NONE;

/// Proof that the measured values are complete. No public constructor besides
/// [`Calibration::from_measured`].
#[derive(Debug)]
pub struct Calibration {
    atu: bool,
    _sealed: (),
}

impl Calibration {
    pub fn from_measured(m: &Measured) -> Option<Calibration> {
        if m.complete_for_tx() { Some(Calibration { atu: m.atu.is_some(), _sealed: () }) } else { None }
    }
}

/// Permission to key once, for one source. Made only by [`TxCore::tx_request`].
#[derive(Debug)]
pub struct KeyPermit<'a> {
    pub source: Source,
    _cal: &'a Calibration,
}

/// Why a write to `TX_ARM` or a TX request was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Arm(ArmReject),
    /// The measured values are not in the spec yet. On the wire: see the module doc.
    NotCalibrated,
}

impl Refusal {
    /// The arm reject reason sent in `TX_ARM_STATE` [15:8] (Table B8-3).
    pub fn wire(self) -> u8 {
        match self {
            Refusal::Arm(r) => r as u8,
            Refusal::NotCalibrated => ArmReject::NotCalibrated as u8,
        }
    }
}

/// Facts about the session at the moment of an arm write (B10.2 item 3).
#[derive(Clone, Copy, Debug, Default)]
pub struct ArmContext {
    pub run: bool,
    pub watchdog_ok: bool,
    pub in_fault: bool,
    pub inhibit: bool,
    pub ptt_bit: bool,
    pub level1_active: bool,
    pub maintenance: bool,
    pub mode_class_declared: bool,
}

/// Facts at the moment of a TX request edge (A3.1a), besides the arm.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyContext {
    pub in_fault: bool,
    pub freq_in_range: bool,
    pub rf_path_ready: bool,
    pub pa_enabled: bool,
    pub mode_class_declared: bool,
    pub keyed_by_other: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmState {
    Disarmed = 0,
    Armed = 1,
    Inhibited = 2,
}

/// The radio side of an unkey: called before every disarm (A3.2: "unkey first").
pub trait RfActuator {
    /// End RF now and return to the safe state ([HIS] HIS 85).
    fn unkey(&mut self);
}

/// Token source (B10.2 item 5): never 0, 0xFFFFFFFF or `ARM_MAGIC`, always different from the
/// previous token, and different across boots through the boot counter.
#[derive(Clone, Copy, Debug)]
pub struct Tokens {
    boot: u32,
    seq: u32,
    last: u32,
}

impl Tokens {
    pub const fn new(boot_counter: u32) -> Self {
        Self { boot: boot_counter, seq: 0, last: 0 }
    }

    pub fn next(&mut self) -> u32 {
        loop {
            self.seq = self.seq.wrapping_add(1);
            // High half from the boot counter, low half from the sequence, mixed so that
            // neighbouring tokens differ in many bits; injective per (boot, seq) for seq < 2^16.
            let raw = (self.boot.wrapping_mul(0x9E37_79B9) & 0xFFFF_0000) | (self.seq & 0xFFFF);
            let t = raw ^ 0x5A5A_0000;
            if t != 0 && t != 0xFFFF_FFFF && t != ARM_MAGIC && t != self.last {
                self.last = t;
                return t;
            }
        }
    }
}

pub struct TxCore {
    state: ArmState,
    token: u32,
    generation: u8,
    last_reject: u8,
    last_drop: u8,
    keyed: Option<Source>,
    /// The arm was confirmed; TX requests key only on an edge after it (B10.2 item 4).
    confirmed: bool,
    tokens: Tokens,
    cal: Option<Calibration>,
    implemented: bool,
}

/// Outcome of a write to `TX_ARM`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmOutcome {
    /// Armed with a new generation: push `TX_ARM_STATE` (the confirmation).
    Armed,
    /// Already armed with this token: answered, no new generation (0020 §7).
    NoOp,
    Disarmed,
    Rejected(Refusal),
}

impl TxCore {
    /// State after any reset: disarmed, drop reason `RESET`, a new token (Table A3-1 event 5).
    pub fn new(boot_counter: u32, measured: &Measured) -> Self {
        let mut tokens = Tokens::new(boot_counter);
        let token = tokens.next();
        Self {
            state: ArmState::Disarmed,
            token,
            generation: 0,
            last_reject: 0,
            last_drop: DropReason::Reset as u8,
            keyed: None,
            confirmed: false,
            tokens,
            cal: Calibration::from_measured(measured),
            implemented: TX_IMPLEMENTED,
        }
    }

    /// For host tests of the B10.2 order: a core as in a build before M4, record `tx` bit 1 = 0.
    #[cfg(test)]
    fn not_implemented(mut self) -> Self {
        self.implemented = false;
        self
    }

    pub fn calibrated(&self) -> bool {
        self.cal.is_some()
    }
    pub fn token(&self) -> u32 {
        self.token
    }
    pub fn state(&self) -> ArmState {
        self.state
    }
    pub fn keyed(&self) -> Option<Source> {
        self.keyed
    }

    /// `TX_ARM_STATE` (Table B8-2): [3:0] state, [15:8] last reject, [23:16] last drop,
    /// [31:24] generation.
    pub fn arm_state_reg(&self) -> u32 {
        (self.state as u32) | ((self.last_reject as u32) << 8) | ((self.last_drop as u32) << 16) | ((self.generation as u32) << 24)
    }

    /// Record `tx` bit 1, "TX implemented in this firmware" (B4.8).
    pub fn record_tx_bit1(&self) -> bool {
        self.implemented
    }

    /// Record `tx` bit 4, the transmit parameters of Annex P set (B4.8; 0035, 0036).
    pub fn record_tx_bit4(&self) -> bool {
        self.implemented && self.calibrated()
    }

    /// A write to `TX_ARM` from the owner (a non-owner gets `WRITE_STATUS` `NOT_OWNER` before
    /// this is called, B10.2 item 3).
    pub fn write_tx_arm(&mut self, value: u32, ctx: &ArmContext, rf: &mut impl RfActuator) -> ArmOutcome {
        if value == 0 {
            return if self.state == ArmState::Disarmed {
                ArmOutcome::Disarmed
            } else {
                self.drop(DropReason::HostDisarm, rf);
                ArmOutcome::Disarmed
            };
        }
        if value != self.token ^ ARM_MAGIC {
            return self.reject(Refusal::Arm(ArmReject::BadToken));
        }
        if self.state != ArmState::Disarmed {
            return ArmOutcome::NoOp;
        }
        let checks = [
            (!ctx.run, ArmReject::NotRunning),
            (!ctx.watchdog_ok, ArmReject::WatchdogDisabled),
            (ctx.in_fault, ArmReject::InFault),
            (ctx.inhibit, ArmReject::Inhibited),
            (ctx.ptt_bit, ArmReject::PttActive),
            (!ctx.level1_active, ArmReject::LevelNotActive),
            (ctx.maintenance, ArmReject::Maintenance),
            (!ctx.mode_class_declared, ArmReject::NoModeClass),
        ];
        for (fails, why) in checks {
            if fails {
                return self.reject(Refusal::Arm(why));
            }
        }
        if !self.implemented {
            return self.reject(Refusal::Arm(ArmReject::TxNotImplemented));
        }
        if self.cal.is_none() {
            return self.reject(Refusal::NotCalibrated);
        }
        self.state = ArmState::Armed;
        self.generation = self.generation.wrapping_add(1);
        self.last_reject = 0;
        self.confirmed = true;
        ArmOutcome::Armed
    }

    fn reject(&mut self, r: Refusal) -> ArmOutcome {
        self.last_reject = r.wire();
        ArmOutcome::Rejected(r)
    }

    /// A 0→1 edge of a TX request (A3.1a). A refused request is not latched: the caller needs a
    /// new edge (A3.1a, last paragraph).
    pub fn tx_request(&mut self, source: Source, ctx: &KeyContext) -> Result<KeyPermit<'_>, Refusal> {
        let Some(cal) = self.cal.as_ref() else { return Err(Refusal::NotCalibrated) };
        if self.state != ArmState::Armed || !self.confirmed {
            return Err(Refusal::Arm(ArmReject::NotRunning));
        }
        if ctx.in_fault {
            return Err(Refusal::Arm(ArmReject::InFault));
        }
        if !ctx.freq_in_range || !ctx.rf_path_ready || !ctx.pa_enabled || ctx.keyed_by_other {
            return Err(Refusal::Arm(ArmReject::Inhibited));
        }
        if !ctx.mode_class_declared {
            return Err(Refusal::Arm(ArmReject::NoModeClass));
        }
        if source == Source::Atu && !cal.atu {
            return Err(Refusal::NotCalibrated);
        }
        self.keyed = Some(source);
        Ok(KeyPermit { source, _cal: cal })
    }

    /// End of the current transmission (UNKEY-level actions keep the arm, Table A3-2).
    pub fn unkeyed(&mut self) {
        self.keyed = None;
    }

    /// A write to `ATU_TUNE` (Table B8-2). 1 before the ATU parameters are set is
    /// `NOT_CALIBRATED` (0032 item 3); this check comes first because nothing else can make the
    /// write succeed while it fails (spec question SQ-F3-2 on the order).
    pub fn write_atu_tune(&mut self, value: u32, ctx: &KeyContext) -> Result<Option<KeyPermit<'_>>, WriteReject> {
        match value {
            0 => Ok(None),
            1 => {
                if !self.cal.as_ref().is_some_and(|c| c.atu) {
                    return Err(WriteReject::NotCalibrated);
                }
                if self.state != ArmState::Armed {
                    return Err(WriteReject::NotArmed);
                }
                if self.keyed.is_some() || ctx.keyed_by_other {
                    return Err(WriteReject::Keyed);
                }
                self.tx_request(Source::Atu, ctx).map(Some).map_err(|_| WriteReject::NotKeyable)
            }
            _ => Err(WriteReject::BadValue),
        }
    }

    /// A drop event of Table A3-1. While armed: unkey first, then disarm, then a new token and
    /// the reason (A3.2). While not armed: nothing changes (A3.2, "Drop events while not
    /// armed"). Returns true if `TX_ARM_STATE` and `TX_ARM_TOKEN` must be pushed.
    pub fn drop(&mut self, reason: DropReason, rf: &mut impl RfActuator) -> bool {
        if self.state == ArmState::Disarmed {
            if self.keyed.is_some() {
                rf.unkey();
                self.keyed = None;
            }
            return false;
        }
        rf.unkey();
        self.keyed = None;
        self.state = ArmState::Disarmed;
        self.confirmed = false;
        self.token = self.tokens.next();
        self.last_drop = reason as u8;
        true
    }

    /// Session end while not armed: a new token, pushed at the next session start (A3.2,
    /// exception for the session-ending events; T-A3-47).
    pub fn session_end(&mut self, rf: &mut impl RfActuator) {
        if self.state != ArmState::Disarmed {
            self.drop(DropReason::RunCleared, rf);
        } else {
            self.token = self.tokens.next();
        }
    }
}

/// `KEYED_TIMEOUT` write (B10.5): 100-500 ms accepted, others `BAD_VALUE`.
pub fn check_keyed_timeout(ms: u32) -> Result<u32, WriteReject> {
    if (100..=500).contains(&ms) { Ok(ms) } else { Err(WriteReject::BadValue) }
}

/// `TX_MODE_CLASS` write (B10.3): 1 SSB, 2 CW, 3 AM, 4 FM; 0 is not writable.
pub fn check_mode_class(v: u32) -> Result<u8, WriteReject> {
    if (1..=4).contains(&v) { Ok(v as u8) } else { Err(WriteReject::BadValue) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rf {
        log: std::vec::Vec<&'static str>,
    }
    impl RfActuator for Rf {
        fn unkey(&mut self) {
            self.log.push("unkey");
        }
    }

    fn ok_ctx() -> ArmContext {
        ArmContext { run: true, watchdog_ok: true, level1_active: true, mode_class_declared: true, ..Default::default() }
    }
    fn key_ctx() -> KeyContext {
        KeyContext { freq_in_range: true, rf_path_ready: true, pa_enabled: true, mode_class_declared: true, ..Default::default() }
    }
    fn test_measured(atu: bool) -> Measured {
        Measured {
            bias_words: Some([0, 0, 0]),
            drive_map: Some(()),
            power_detector: Some(()),
            swr_limits: Some(()),
            temperature: Some(()),
            supply: Some(()),
            rf_off: Some(()),
            tr_timing: Some(()),
            atu: if atu { Some(()) } else { None },
        }
    }

    #[test]
    fn this_build_is_not_calibrated() {
        assert!(!MEASURED.complete_for_tx());
        assert!(Calibration::from_measured(&MEASURED).is_none());
        let c = TxCore::new(1, &MEASURED);
        assert!(!c.calibrated());
        assert!(c.record_tx_bit1()); // r1.0 (0036)
        assert!(!c.record_tx_bit4());
    }

    #[test]
    fn every_tx_request_rejected_not_calibrated() {
        let mut c = TxCore::new(7, &MEASURED);
        let mut rf = Rf::default();
        let v = c.token() ^ ARM_MAGIC;
        // All preconditions hold, the token is right: this build refuses with 12 (T-B10-03).
        assert_eq!(c.write_tx_arm(v, &ok_ctx(), &mut rf), ArmOutcome::Rejected(Refusal::NotCalibrated));
        assert_eq!(c.state(), ArmState::Disarmed);
        assert_eq!((c.arm_state_reg() >> 8) & 0xFF, 12);
        // A precondition of B10.2 item 3 that fails comes before 12.
        let mut x = ok_ctx();
        x.mode_class_declared = false;
        assert_eq!(c.write_tx_arm(v, &x, &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::NoModeClass)));
        // With bit 1 clear (a build before M4), 11 comes before 12, and bit 4 is 0.
        let mut b = TxCore::new(7, &test_measured(false)).not_implemented();
        let vb = b.token() ^ ARM_MAGIC;
        assert_eq!(b.write_tx_arm(vb, &ok_ctx(), &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::TxNotImplemented)));
        assert_eq!((b.arm_state_reg() >> 8) & 0xFF, 11);
        assert!(!b.record_tx_bit4());
        for s in [Source::Ptt, Source::Cwx, Source::Key, Source::Atu] {
            assert_eq!(c.tx_request(s, &key_ctx()).unwrap_err(), Refusal::NotCalibrated);
        }
        assert_eq!(c.write_atu_tune(1, &key_ctx()).unwrap_err(), WriteReject::NotCalibrated);
        assert!(c.keyed().is_none());
        assert!(rf.log.is_empty());
    }

    #[test]
    fn reset_state() {
        let c = TxCore::new(3, &MEASURED);
        let r = c.arm_state_reg();
        assert_eq!(r & 0xF, 0);
        assert_eq!((r >> 16) & 0xFF, DropReason::Reset as u32);
        assert_eq!(r >> 24, 0);
    }

    #[test]
    fn preconditions_in_order() {
        let mut c = TxCore::new(1, &test_measured(false));
        let mut rf = Rf::default();
        let v = c.token() ^ ARM_MAGIC;
        let mut x = ok_ctx();
        x.run = false;
        assert_eq!(c.write_tx_arm(v, &x, &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::NotRunning)));
        let mut x = ok_ctx();
        x.watchdog_ok = false;
        assert_eq!(c.write_tx_arm(v, &x, &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::WatchdogDisabled)));
        let mut x = ok_ctx();
        x.ptt_bit = true;
        assert_eq!(c.write_tx_arm(v, &x, &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::PttActive)));
        let mut x = ok_ctx();
        x.mode_class_declared = false;
        assert_eq!(c.write_tx_arm(v, &x, &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::NoModeClass)));
        assert_eq!(c.write_tx_arm(v ^ 1, &ok_ctx(), &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::BadToken)));
    }

    #[test]
    fn calibrated_arm_key_and_drop_order() {
        let mut c = TxCore::new(1, &test_measured(false));
        let mut rf = Rf::default();
        let t0 = c.token();
        assert_eq!(c.write_tx_arm(t0 ^ ARM_MAGIC, &ok_ctx(), &mut rf), ArmOutcome::Armed);
        assert_eq!(c.arm_state_reg() >> 24, 1);
        assert_eq!(c.write_tx_arm(t0 ^ ARM_MAGIC, &ok_ctx(), &mut rf), ArmOutcome::NoOp);
        assert_eq!(c.arm_state_reg() >> 24, 1);
        let p = c.tx_request(Source::Ptt, &key_ctx()).unwrap();
        assert_eq!(p.source, Source::Ptt);
        assert_eq!(c.keyed(), Some(Source::Ptt));
        // ATU parameters missing: ATU_TUNE still NOT_CALIBRATED even when armed.
        assert_eq!(c.write_atu_tune(1, &key_ctx()).unwrap_err(), WriteReject::NotCalibrated);
        assert!(c.drop(DropReason::Takeover, &mut rf));
        assert_eq!(rf.log, ["unkey"]);
        assert_eq!(c.state(), ArmState::Disarmed);
        assert_ne!(c.token(), t0);
        assert_eq!((c.arm_state_reg() >> 16) & 0xFF, DropReason::Takeover as u32);
        // The old arm value no longer arms.
        assert_eq!(c.write_tx_arm(t0 ^ ARM_MAGIC, &ok_ctx(), &mut rf), ArmOutcome::Rejected(Refusal::Arm(ArmReject::BadToken)));
    }

    #[test]
    fn drop_while_not_armed_changes_nothing() {
        let mut c = TxCore::new(1, &test_measured(false));
        let mut rf = Rf::default();
        let before = (c.token(), c.arm_state_reg());
        assert!(!c.drop(DropReason::Watchdog, &mut rf));
        assert_eq!((c.token(), c.arm_state_reg()), before);
        assert!(rf.log.is_empty());
    }

    #[test]
    fn tokens_never_forbidden_and_always_new() {
        for boot in [0u32, 1, 2, 0xFFFF_FFFF, 0x1234_5678] {
            let mut t = Tokens::new(boot);
            let mut prev = 0;
            for _ in 0..70_000 {
                let x = t.next();
                assert!(x != 0 && x != 0xFFFF_FFFF && x != ARM_MAGIC);
                assert_ne!(x, prev);
                prev = x;
            }
        }
        // The first token of one boot differs from the tokens of the boot before.
        let mut a = Tokens::new(41);
        let last_a = (0..100).map(|_| a.next()).last().unwrap();
        let mut b = Tokens::new(42);
        assert_ne!(b.next(), last_a);
    }

    #[test]
    fn register_checks() {
        assert_eq!(check_keyed_timeout(99), Err(WriteReject::BadValue));
        assert_eq!(check_keyed_timeout(100), Ok(100));
        assert_eq!(check_keyed_timeout(500), Ok(500));
        assert_eq!(check_keyed_timeout(501), Err(WriteReject::BadValue));
        assert_eq!(check_mode_class(0), Err(WriteReject::BadValue));
        assert_eq!(check_mode_class(3), Ok(3));
        assert_eq!(check_mode_class(5), Err(WriteReject::BadValue));
    }
}
