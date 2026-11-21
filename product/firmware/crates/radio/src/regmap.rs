// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! G7250 register map on UDP 7250 (spec B8; Table B8-2; B11.5): register state, the write rules
//! of B8.4 and B8.5, `CAP_REQUEST` (B4.4) and packet encoding (B8.1).

use crate::record;
use g7250_safety::keypath::{AtuWrite, KeyPath, Keyer};
use g7250_safety::tx::{self, ArmContext, KeyContext, TxCore, WriteReject};

pub const PORT: u16 = 7250;
/// Largest packet: 4 + 6 × 240 = 1444 bytes (B8.1).
pub const MAX_PAIRS: usize = 240;
pub const MAX_LEN: usize = 4 + 6 * MAX_PAIRS;

pub mod addr {
    pub const CAP_REQUEST: u16 = 0x0100;
    pub const STREAM_STATUS: u16 = 0x0101;
    pub const LINK_STATUS: u16 = 0x0102;
    pub const WRITE_STATUS: u16 = 0x0103;
    pub const TX_ARM_TOKEN: u16 = 0x0200;
    pub const TX_ARM: u16 = 0x0201;
    pub const TX_ARM_STATE: u16 = 0x0202;
    pub const TX_STATE: u16 = 0x0203;
    pub const TX_MODE_CLASS: u16 = 0x0204;
    pub const KEYED_TIMEOUT: u16 = 0x0205;
    pub const WATCHDOG_STATE: u16 = 0x0206;
    pub const SAFETY_STATE: u16 = 0x0210;
    pub const FAULT_CODE: u16 = 0x0211;
    pub const FAULT_FLAGS: u16 = 0x0212;
    pub const FAULT_CLASS: u16 = 0x0213;
    pub const TRIP_COUNT: u16 = 0x0214;
    pub const FAULT_CLEAR: u16 = 0x0215;
    pub const LIMIT_STATUS: u16 = 0x0220;
    pub const LIMIT_INHIBIT: u16 = 0x0221;
    pub const LIMIT_TX_TIME: u16 = 0x0230;
    pub const LIMIT_FWD_POWER: u16 = 0x0231;
    pub const LIMIT_SWR: u16 = 0x0232;
    pub const LIMIT_PA_TEMP: u16 = 0x0233;
    pub const TLM_PA_TEMP0: u16 = 0x0300;
    pub const TLM_PA_TEMP3: u16 = 0x0303;
    pub const TLM_PA_CURRENT: u16 = 0x0304;
    pub const TLM_SUPPLY_CURRENT: u16 = 0x0305;
    pub const TLM_SWR: u16 = 0x0306;
    pub const SENSOR_VALID: u16 = 0x0307;
    pub const ATU_TUNE: u16 = 0x0400;
    pub const ATU_STATE: u16 = 0x0401;
    pub const ANT_STATE: u16 = 0x0500;
}
use addr::*;

/// Built-in maximum continuous TX time, ms (0007 Resolved 3; Table B8-2 `LIMIT_TX_TIME`).
pub const BUILTIN_TX_TIME_MS: u32 = 300_000;
/// Built-in forward power bound, mW PEP: 100 W ([G7250-MAN]; Table A3-2).
pub const BUILTIN_FWD_POWER_MW: u32 = 100_000;
/// `ATU_STATE` 5: unknown, until P-ATU-1 is set (B8.3 group 4; 0032).
pub const ATU_STATE_UNKNOWN: u32 = 5;

/// A parsed register pair.
pub type Pair = (u16, u32);

/// Iterate the pairs of a host packet: bytes 4.., trailing incomplete pair ignored, padding
/// pairs (address 0xFFFF, or 0x0000 with 0x00000000) skipped (B8.1).
pub fn pairs(p: &[u8]) -> impl Iterator<Item = Pair> + '_ {
    p.get(4..).unwrap_or(&[]).chunks_exact(6).filter_map(|c| {
        let a = u16::from_be_bytes([c[0], c[1]]);
        let v = u32::from_be_bytes([c[2], c[3], c[4], c[5]]);
        if a == 0xFFFF || (a == 0 && v == 0) { None } else { Some((a, v)) }
    })
}

/// Encode a register packet: sequence number, then the pairs (B8.1). Returns the length.
pub fn encode(seq: u32, pairs: &[Pair], out: &mut [u8; MAX_LEN]) -> usize {
    let n = pairs.len().min(MAX_PAIRS);
    out[0..4].copy_from_slice(&seq.to_be_bytes());
    for (i, (a, v)) in pairs[..n].iter().enumerate() {
        out[4 + 6 * i..6 + 6 * i].copy_from_slice(&a.to_be_bytes());
        out[6 + 6 * i..10 + 6 * i].copy_from_slice(&v.to_be_bytes());
    }
    4 + 6 * n
}

/// Answer to a host packet: pairs to send back, at most 240 (one packet).
pub struct Answer {
    pub pairs: heapless::Vec<Pair, MAX_PAIRS>,
    /// The packet held a `CAP_REQUEST` with the right value: the record is in `pairs`.
    pub record_sent: bool,
    /// At least one write was rejected with `NOT_OWNER` (caller applies the 1/100 ms limit).
    pub not_owner: bool,
    /// The arm state changed: push `TX_ARM_TOKEN`/`TX_ARM_STATE` to the owner.
    pub arm_changed: bool,
    /// An `ATU_TUNE` no-op: its answer also carries `ATU_STATE` (A3.9b item 1).
    pub atu_noop: bool,
}

/// Who wrote, and what the session looks like at that moment.
pub struct WriteCtx<'a, R: Keyer> {
    pub from_owner: bool,
    /// A firmware update is in progress (`BUSY` first, B8.4).
    pub busy: bool,
    pub tx: &'a mut TxCore,
    pub rf: &'a mut R,
    pub arm: ArmContext,
    pub key: KeyContext,
    /// The key path, for `ATU_TUNE` (A3.9b), and the time of the packet.
    pub path: &'a mut KeyPath,
    pub now: u64,
}

/// Sequence numbers of local port 7250, per destination (B2.7, "Port 7250 counters per
/// destination"; 0051): the session's counter for the owner's register destination, reset and
/// continued with the session (and continued towards an old owner by the takeover notices,
/// A2.4); and one counter for every answer to a host that is not the owner, which no session
/// event resets, only a reset of the radio.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Port7250Seq {
    pub owner: u32,
    pub other: u32,
}

impl Port7250Seq {
    /// The next number for the owner's register destination.
    pub fn next_owner(&mut self) -> u32 {
        let n = self.owner;
        self.owner = n.wrapping_add(1);
        n
    }
    /// The next number for an answer to a host that is not the owner.
    pub fn next_other(&mut self) -> u32 {
        let n = self.other;
        self.other = n.wrapping_add(1);
        n
    }
    /// A stop, a takeover, a session start by another host, an address change (B2.7): the
    /// session's counter restarts; the non-owner counter does not.
    pub fn session_reset(&mut self) {
        self.owner = 0;
    }
}

/// Register state of the radio.
pub struct RegMap {
    pub record: [u32; record::LEN],
    pub stream_status: u32,
    pub link_status: u32,
    pub write_status: u32,
    reject_counter: u8,
    pub tx_mode_class: u8,
    pub keyed_timeout_ms: u32,
    /// General byte 38 bit [0] as last written; `None` before any General packet (B5.6).
    pub watchdog_written: Option<bool>,
    pub armed: bool,
    pub fault_flags: u32,
    pub trip_count: u32,
    /// `LIMIT_STATUS`, from the key path.
    pub limit_status: u32,
    /// Session values of the `LIMIT_<name>` registers; 0 = back to the persistent value.
    pub limit_tx_time: u32,
    pub limit_fwd_power: u32,
    pub tx_state: u32,
    /// `ATU_STATE` and `ATU_TUNE` in force, from the key path (A3.9b; 5 until P-ATU-1).
    pub atu_state: u32,
    pub atu_tune: u32,
}

impl RegMap {
    pub fn new(record: [u32; record::LEN]) -> Self {
        RegMap {
            record,
            stream_status: 0x01, // DDC0 not running, reason 1 "not enabled by the host"
            link_status: 0,
            write_status: 0,
            reject_counter: 0,
            tx_mode_class: 0,
            keyed_timeout_ms: 500,
            watchdog_written: None,
            armed: false,
            fault_flags: 0,
            trip_count: 0,
            limit_status: 0,
            limit_tx_time: 0,
            limit_fwd_power: 0,
            tx_state: 0,
            atu_state: ATU_STATE_UNKNOWN,
            atu_tune: 0,
        }
    }

    /// Session end: the session values return (A3.6; B10.3, B10.5; Table B8-2).
    pub fn session_end(&mut self) {
        self.tx_mode_class = 0;
        self.keyed_timeout_ms = 500;
        self.limit_tx_time = 0;
        self.limit_fwd_power = 0;
        self.watchdog_written = None;
    }

    /// `WATCHDOG_STATE` (Table B8-2): bit 0 written, bit 1 effective, bit 2 no General yet.
    pub fn watchdog_state(&self) -> u32 {
        match self.watchdog_written {
            None => 0b110,
            Some(w) => (w as u32) | (((w || self.armed) as u32) << 1),
        }
    }

    /// Effective hardware timer (B5.6): written OR armed; enabled before any General packet.
    pub fn watchdog_effective(&self) -> bool {
        self.watchdog_written.is_none_or(|w| w || self.armed)
    }

    pub fn tx_time_in_force(&self) -> u32 {
        if self.limit_tx_time == 0 { BUILTIN_TX_TIME_MS } else { self.limit_tx_time }
    }
    fn fwd_power_in_force(&self) -> u32 {
        if self.limit_fwd_power == 0 { BUILTIN_FWD_POWER_MW } else { self.limit_fwd_power }
    }

    /// Current value of a readable address, `None` if the address is not in the map.
    pub fn read(&self, a: u16, tx: &TxCore) -> Option<u32> {
        Some(match a {
            0x0000..=0x00CE => self.record[a as usize],
            CAP_REQUEST => 0,
            STREAM_STATUS => self.stream_status,
            LINK_STATUS => self.link_status,
            WRITE_STATUS => self.write_status,
            TX_ARM_TOKEN => tx.token(),
            TX_ARM => 0,
            TX_ARM_STATE => tx.arm_state_reg(),
            TX_STATE => self.tx_state,
            TX_MODE_CLASS => self.tx_mode_class as u32,
            KEYED_TIMEOUT => self.keyed_timeout_ms,
            WATCHDOG_STATE => self.watchdog_state(),
            SAFETY_STATE | FAULT_CODE | FAULT_CLASS | LIMIT_INHIBIT => 0,
            LIMIT_STATUS => self.limit_status,
            FAULT_FLAGS => self.fault_flags,
            TRIP_COUNT => self.trip_count,
            FAULT_CLEAR => 0,
            LIMIT_TX_TIME => self.tx_time_in_force(),
            LIMIT_FWD_POWER => self.fwd_power_in_force(),
            // S2 and T3 need the calibration set (P-PWR-2): no value in force yet.
            LIMIT_SWR | LIMIT_PA_TEMP => 0,
            TLM_PA_TEMP0..=TLM_PA_TEMP3 | TLM_PA_CURRENT | TLM_SUPPLY_CURRENT | TLM_SWR => 0,
            SENSOR_VALID => 0, // no sensor valid until calibrated (P-SEN-1)
            ATU_TUNE => self.atu_tune,
            ATU_STATE => self.atu_state,
            ANT_STATE => 0, // unknown until P-ANT-1
            _ => return None,
        })
    }

    fn reject(&mut self, a: u16, why: WriteReject) {
        self.reject_counter = self.reject_counter.wrapping_add(1);
        self.write_status = ((a as u32) << 16) | ((self.reject_counter as u32) << 8) | why as u32;
    }

    /// Handle one host packet (B8.4: pairs in order; answers carry the value in force).
    pub fn handle<R: Keyer>(&mut self, packet: &[u8], c: &mut WriteCtx<'_, R>) -> Answer {
        let mut ans = Answer { pairs: heapless::Vec::new(), record_sent: false, not_owner: false, arm_changed: false, atu_noop: false };
        for (a, v) in pairs(packet) {
            let push = |ans: &mut Answer, p: Pair| {
                let _ = ans.pairs.push(p);
            };
            // CAP_REQUEST is accepted from any host (B4.4, B8.5).
            if a == CAP_REQUEST {
                if v == 1 {
                    push(&mut ans, (a, v));
                    for (i, r) in self.record.iter().enumerate() {
                        push(&mut ans, (i as u16, *r));
                    }
                    ans.record_sent = true;
                } else {
                    self.reject(a, WriteReject::BadValue);
                    push(&mut ans, (a, 0));
                    push(&mut ans, (WRITE_STATUS, self.write_status));
                }
                continue;
            }
            let in_force = self.read(a, c.tx);
            // Order (B8.4): BUSY, then NOT_OWNER, then the address and value checks.
            let why = if in_force.is_none() {
                Some(WriteReject::UnknownAddress)
            } else if c.busy {
                Some(WriteReject::Busy)
            } else if !c.from_owner {
                ans.not_owner = true;
                Some(WriteReject::NotOwner)
            } else {
                self.write(a, v, c, &mut ans)
            };
            let value = self.read(a, c.tx).unwrap_or(0);
            match why {
                None => {
                    // For TX_ARM the answer is TX_ARM_STATE (B8.4).
                    if a == TX_ARM {
                        push(&mut ans, (TX_ARM_STATE, c.tx.arm_state_reg()));
                    } else {
                        push(&mut ans, (a, value));
                    }
                    if a == ATU_TUNE && ans.atu_noop {
                        ans.atu_noop = false;
                        push(&mut ans, (ATU_STATE, self.atu_state));
                    }
                }
                Some(r) => {
                    self.reject(a, r);
                    push(&mut ans, (if a == TX_ARM { TX_ARM_STATE } else { a }, if a == TX_ARM { c.tx.arm_state_reg() } else { value }));
                    push(&mut ans, (WRITE_STATUS, self.write_status));
                }
            }
        }
        ans
    }

    /// An owner's write to a known address. `None` = accepted.
    fn write<R: Keyer>(&mut self, a: u16, v: u32, c: &mut WriteCtx<'_, R>, ans: &mut Answer) -> Option<WriteReject> {
        match a {
            TX_ARM => {
                let before = c.tx.arm_state_reg();
                let mut arm = c.arm;
                arm.mode_class_declared = self.tx_mode_class != 0;
                arm.watchdog_ok = self.watchdog_written == Some(true) && self.watchdog_effective();
                let _ = c.tx.write_tx_arm(v, &arm, c.rf);
                self.armed = c.tx.state() != tx::ArmState::Disarmed;
                // A rejected arm precondition is reported in TX_ARM_STATE only (B8.4).
                ans.arm_changed = c.tx.arm_state_reg() != before;
                None
            }
            TX_MODE_CLASS => match tx::check_mode_class(v) {
                Ok(m) => {
                    self.tx_mode_class = m;
                    None
                }
                Err(e) => Some(e),
            },
            KEYED_TIMEOUT => match tx::check_keyed_timeout(v) {
                Ok(t) => {
                    self.keyed_timeout_ms = t;
                    None
                }
                Err(e) => Some(e),
            },
            FAULT_CLEAR => {
                // No fault can be set in this build; with FAULT_FLAGS = 0 a write of 0 is
                // NOT_CLEARABLE, any other value BAD_VALUE (B8.4; 0039).
                if self.fault_flags == 0 {
                    Some(if v == 0 { WriteReject::NotClearable } else { WriteReject::BadValue })
                } else {
                    Some(WriteReject::NotClearable)
                }
            }
            LIMIT_TX_TIME => {
                if v == 0 || v <= BUILTIN_TX_TIME_MS {
                    self.limit_tx_time = v;
                    None
                } else {
                    Some(WriteReject::LooserLimit)
                }
            }
            LIMIT_FWD_POWER => {
                if v == 0 || v <= BUILTIN_FWD_POWER_MW {
                    self.limit_fwd_power = v;
                    None
                } else {
                    Some(WriteReject::LooserLimit)
                }
            }
            // Checked against S2 / T3, which need the calibration set (P-PWR-2; A1.12; 0039).
            LIMIT_SWR | LIMIT_PA_TEMP => Some(WriteReject::NotCalibrated),
            ATU_TUNE => {
                let r = c.path.atu_tune(v, c.now, c.tx, &c.key, c.rf);
                self.atu_state = c.path.atu_state(c.tx);
                self.atu_tune = c.path.atu_tune_in_force();
                match r {
                    Ok(AtuWrite::NoOp) => {
                        ans.atu_noop = true;
                        None
                    }
                    Ok(_) => None,
                    Err(e) => Some(e),
                }
            }
            _ => Some(WriteReject::ReadOnly),
        }
    }

    /// The pairs pushed to the owner at a session start (B10.2 item 1; B8.2): the arm, stream
    /// and link state and every status register a host reads without writing.
    pub fn session_start_pairs(&self, tx: &TxCore) -> [Pair; 10] {
        [
            (TX_ARM_TOKEN, tx.token()),
            (TX_ARM_STATE, tx.arm_state_reg()),
            (TX_STATE, self.tx_state),
            (WATCHDOG_STATE, self.watchdog_state()),
            (STREAM_STATUS, self.stream_status),
            (LINK_STATUS, self.link_status),
            (SAFETY_STATE, 0),
            (SENSOR_VALID, 0),
            (ATU_STATE, self.atu_state),
            (ANT_STATE, 0),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use g7250_safety::tx::MEASURED;

    struct Rf;
    impl g7250_safety::tx::RfActuator for Rf {
        fn unkey(&mut self) {}
    }
    impl Keyer for Rf {
        fn key(&mut self, _p: &g7250_safety::tx::KeyPermit<'_>) -> bool {
            false
        }
        fn cw_element(&mut self, _on: bool) {}
        fn atu_step(&mut self, _now: u64) -> Option<bool> {
            None
        }
    }

    fn map() -> RegMap {
        RegMap::new(record::build(&record::Params { fw_version: 0x0001_0001, tx_implemented: false, tx_calibrated: false, rate_384: false, rx_rates: None }))
    }

    fn packet(pairs: &[Pair]) -> std::vec::Vec<u8> {
        let mut v = std::vec![0, 0, 0, 0];
        for (a, d) in pairs {
            v.extend_from_slice(&a.to_be_bytes());
            v.extend_from_slice(&d.to_be_bytes());
        }
        v
    }

    fn run(m: &mut RegMap, tx: &mut TxCore, owner: bool, p: &[Pair]) -> Answer {
        let mut rf = Rf;
        let mut path = KeyPath::new();
        let mut c = WriteCtx { from_owner: owner, busy: false, tx, rf: &mut rf, arm: ArmContext { run: true, level1_active: true, ..Default::default() }, key: KeyContext::default(), path: &mut path, now: 0 };
        m.handle(&packet(p), &mut c)
    }

    #[test]
    fn cap_request_from_anyone_gives_echo_and_whole_record_in_one_packet() {
        let mut m = map();
        let mut tx = TxCore::new(1, &MEASURED);
        let a = run(&mut m, &mut tx, false, &[(CAP_REQUEST, 1)]);
        assert!(a.record_sent && !a.not_owner);
        assert_eq!(a.pairs.len(), 1 + record::LEN);
        assert_eq!(a.pairs[0], (CAP_REQUEST, 1));
        assert_eq!(a.pairs[1], (0, 0x4737_3235));
        let mut out = [0u8; MAX_LEN];
        assert_eq!(encode(0, &a.pairs, &mut out), 4 + 6 * 208);
        // Another value: BAD_VALUE, no record.
        let a = run(&mut m, &mut tx, false, &[(CAP_REQUEST, 2)]);
        assert!(!a.record_sent);
        assert_eq!(a.pairs[1].0, WRITE_STATUS);
        assert_eq!(a.pairs[1].1 & 0xFF, WriteReject::BadValue as u32);
    }

    #[test]
    fn non_owner_unknown_read_only_padding() {
        let mut m = map();
        let mut tx = TxCore::new(1, &MEASURED);
        let a = run(&mut m, &mut tx, false, &[(TX_MODE_CLASS, 1)]);
        assert!(a.not_owner);
        assert_eq!(a.pairs[0], (TX_MODE_CLASS, 0));
        assert_eq!(a.pairs[1].1, ((TX_MODE_CLASS as u32) << 16) | (1 << 8) | 3);
        assert_eq!(m.tx_mode_class, 0);
        // Read-only, the record included, answered with the current value.
        let a = run(&mut m, &mut tx, true, &[(0x0002, 5), (ATU_STATE, 1)]);
        assert_eq!(a.pairs[0], (0x0002, 0xCF));
        assert_eq!(a.pairs[1].1 & 0xFF, 2);
        assert_eq!(a.pairs[2], (ATU_STATE, 5));
        // Unknown and absent-group addresses: value 0 and UNKNOWN_ADDRESS.
        let a = run(&mut m, &mut tx, true, &[(0x0600, 1), (0x00D0, 1)]);
        assert_eq!(a.pairs[0], (0x0600, 0));
        assert_eq!(a.pairs[1].1 & 0xFF, 1);
        // Padding pairs are not answered; a trailing incomplete pair is ignored.
        let mut p = packet(&[(0xFFFF, 1), (0, 0), (KEYED_TIMEOUT, 300)]);
        p.extend_from_slice(&[0x02, 0x05, 0]);
        let mut rf = Rf;
        let mut path = KeyPath::new();
        let mut c = WriteCtx { from_owner: true, busy: false, tx: &mut tx, rf: &mut rf, arm: ArmContext::default(), key: KeyContext::default(), path: &mut path, now: 0 };
        let a = m.handle(&p, &mut c);
        assert_eq!(a.pairs.as_slice(), &[(KEYED_TIMEOUT, 300)]);
    }

    #[test]
    fn owner_writes_tx_registers() {
        let mut m = map();
        let mut tx = TxCore::new(1, &MEASURED);
        let a = run(&mut m, &mut tx, true, &[(TX_MODE_CLASS, 3), (KEYED_TIMEOUT, 99), (LIMIT_TX_TIME, 400_000), (LIMIT_TX_TIME, 60_000)]);
        assert_eq!(a.pairs[0], (TX_MODE_CLASS, 3));
        assert_eq!(a.pairs[1], (KEYED_TIMEOUT, 500));
        assert_eq!(a.pairs[2].1 & 0xFF, WriteReject::BadValue as u32);
        assert_eq!(a.pairs[3], (LIMIT_TX_TIME, 300_000));
        assert_eq!(a.pairs[4].1 & 0xFF, WriteReject::LooserLimit as u32);
        assert_eq!(a.pairs[5], (LIMIT_TX_TIME, 60_000));
        // ATU tune and S2/T3: NOT_CALIBRATED.
        let a = run(&mut m, &mut tx, true, &[(ATU_TUNE, 1), (LIMIT_SWR, 2000)]);
        assert_eq!(a.pairs[1].1 & 0xFF, WriteReject::NotCalibrated as u32);
        assert_eq!(a.pairs[3].1 & 0xFF, WriteReject::NotCalibrated as u32);
        // FAULT_CLEAR without a fault.
        let a = run(&mut m, &mut tx, true, &[(FAULT_CLEAR, 0), (FAULT_CLEAR, 7)]);
        assert_eq!(a.pairs[1].1 & 0xFF, WriteReject::NotClearable as u32);
        assert_eq!(a.pairs[3].1 & 0xFF, WriteReject::BadValue as u32);
        // Session end resets the session values.
        m.session_end();
        assert_eq!((m.tx_mode_class, m.read(LIMIT_TX_TIME, &tx)), (0, Some(300_000)));
        // T-A3-06: after the end, with no session, a write (B8.4a: a read) gets the values of
        // A3.6a with NOT_OWNER, all pairs in one answer.
        let a = run(&mut m, &mut tx, false, &[(TX_MODE_CLASS, 3), (KEYED_TIMEOUT, 300), (TX_STATE, 1)]);
        assert!(a.not_owner);
        assert_eq!(a.pairs[0], (TX_MODE_CLASS, 0));
        assert_eq!(a.pairs[2], (KEYED_TIMEOUT, 500));
        assert_eq!(a.pairs[4], (TX_STATE, 0));
        assert_eq!(a.pairs.len(), 6);
    }

    #[test]
    fn tx_arm_is_answered_with_tx_arm_state_and_refused() {
        let mut m = map();
        let mut tx = TxCore::new(1, &MEASURED);
        m.watchdog_written = Some(true);
        m.tx_mode_class = 1;
        let v = tx.token() ^ tx::ARM_MAGIC;
        let a = run(&mut m, &mut tx, true, &[(TX_ARM, v)]);
        assert_eq!(a.pairs[0].0, TX_ARM_STATE);
        // This build (r1.0, uncalibrated): NOT_CALIBRATED, reason 12 in [15:8], disarmed (T-B10-03).
        assert_eq!(a.pairs[0].1 & 0xF, 0);
        assert_eq!((a.pairs[0].1 >> 8) & 0xFF, 12);
        assert!(a.arm_changed);
    }

    #[test]
    fn t_b8_21_answers_to_non_owners_have_their_own_counter() {
        let mut q = Port7250Seq::default();
        // A's stream and C's answers interleaved: neither shows a gap.
        assert_eq!((q.next_owner(), q.next_other(), q.next_owner(), q.next_other(), q.next_other(), q.next_owner()), (0, 0, 1, 1, 2, 2));
        // A's session ends or B takes over: the session's counter restarts, C's does not.
        q.session_reset();
        assert_eq!((q.next_owner(), q.next_other()), (0, 3));
        // Only a reset of the radio (a new value) starts it again.
        assert_eq!(Port7250Seq::default().next_other(), 0);
    }

    #[test]
    fn watchdog_state_bits() {
        let mut m = map();
        assert_eq!(m.watchdog_state(), 0b110);
        assert!(m.watchdog_effective());
        m.watchdog_written = Some(false);
        assert_eq!(m.watchdog_state(), 0b000);
        assert!(!m.watchdog_effective());
        m.armed = true;
        assert_eq!(m.watchdog_state(), 0b010);
    }
}
