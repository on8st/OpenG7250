// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! G7250-P2 on the board, step 3: sockets of Table B2-1, session (B5), register map (B8),
//! capability record (B4) and the High Priority status packet (B9.2). DDC0 I&Q (B6) comes in the
//! next step: `STREAM_STATUS` reports DDC0 as not running.

use g7250_board::println;
use g7250_ch394::{Ch394, Error as ChError};
use g7250_proto::ratelimit::RateLimit;
use g7250_radio::regmap::{self, Pair, RegMap, WriteCtx};
use g7250_radio::session::{Addr, CnC, EndReason, Event, Session};
use core::cell::RefCell;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use g7250_radio::duc_fifo::DucFifo;
use g7250_safety::keypath::{KeyPath, Requests};
use g7250_safety::tx::{ArmContext, DropReason, KeyContext, TxCore};

/// The DUC FIFO (B10.7), 20 packets; static, so its 29 KB stay out of the task's stack.
static DUC: Mutex<CriticalSectionRawMutex, RefCell<DucFifo<20>>> = Mutex::new(RefCell::new(DucFifo::new()));

fn duc<R>(f: impl FnOnce(&mut DucFifo<20>) -> R) -> R {
    DUC.lock(|d| f(&mut d.borrow_mut()))
}

/// Socket numbers (Table B2-1 rows 1-8).
pub const S1024: u8 = 0;
pub const S1025: u8 = 1;
pub const S1026: u8 = 2;
pub const S1027: u8 = 3;
pub const S1029: u8 = 4;
pub const S1035: u8 = 5;
pub const S7250: u8 = 6;
pub const S7251: u8 = 7;
pub const PORTS: [(u8, u16); 7] = [(S1024, 1024), (S1025, 1025), (S1026, 1026), (S1027, 1027), (S1029, 1029), (S1035, 1035), (S7250, 7250)];

/// High Priority status period while not transmitting (B9.4; record `status_rates`).
const STATUS_MS: u64 = 50;

pub struct P2 {
    pub session: Session,
    pub regs: RegMap,
    pub tx: TxCore,
    /// The network key path (B10.1, B10.5, B10.10; behind the calibration gate).
    pub key: KeyPath,
    seq_status: u32,
    seq_regs: u32,
    /// Last unasked record per destination (B4.4 item 2: at most once per 1 s).
    last_unasked: Option<(Addr, u64)>,
    /// The unasked record of this session has gone out (B4.4 item 2: once per session).
    unasked_sent: bool,
    /// `CAP_REQUEST` answers to non-owners: at most once per 1 s per source (B4.4 item 3) ...
    record_last: [Option<(Addr, u64)>; 8],
    /// ... and 10 per second in total.
    record_total: RateLimit<1>,
    /// Last NOT_OWNER answer per source (B8.5: one per 100 ms).
    not_owner_last: [Option<(Addr, u64)>; 8],
    refused_limit: RateLimit<8>,
    next_status_ms: u64,
    pub link_ok: bool,
    pub dropped_reg_from_7250: u64,
    pub dropped_not_owner: u64,
    /// DUC I&Q discarded whole before the FIFO (B10.7: not the owner's, no session, record
    /// `tx` bit 1 or 4 = 0, a DUC-specific refusal standing; `GET_STATS`).
    pub duc_iq_ignored: u64,
    /// The key path was keyed after the last call (to empty and arm the FIFO on a change).
    was_keyed: bool,
    pub dropped_record: u64,
    /// A firmware update is in progress (A1.8: STREAM_STATUS reason 10, register writes BUSY).
    pub update_in_progress: bool,
    /// The running image is an unconfirmed trial: arms get `MAINTENANCE` (A1.8; T-A1-24).
    pub trial: bool,
    /// Last reason-10 answer to a run request during an update (at most one per 100 ms).
    last_update_answer: Option<u64>,
}

/// At most one event per `min_ms` per source; a small table, the oldest entry reused.
fn spaced(t: &mut [Option<(Addr, u64)>; 8], from: Addr, now: u64, min_ms: u64) -> bool {
    if let Some(e) = t.iter_mut().flatten().find(|e| e.0 == from) {
        if now.saturating_sub(e.1) < min_ms {
            return false;
        }
        e.1 = now;
        return true;
    }
    let i = t.iter().position(|e| e.is_none()).unwrap_or_else(|| (0..t.len()).min_by_key(|&i| t[i].map_or(0, |e| e.1)).unwrap());
    t[i] = Some((from, now));
    true
}

pub type Chip = Ch394<crate::net::CsDevice<'static>>;

/// `LINK_STATUS` bit 17: a non-default P2 port was asked for and not honoured (Table B8-2).
const LINK_PORT_NOT_HONOURED: u32 = 1 << 17;

impl P2 {
    pub fn new(tx: TxCore, mut regs: RegMap) -> Self {
        // Before any session: DDC0 9 (run bit clear), DUC0 14 (TX not calibrated), Table B8-3.
        regs.stream_status = g7250_radio::stream::register(9, g7250_radio::stream::duc0(false, false, true, tx.calibrated()));
        P2 {
            session: Session::new(),
            regs,
            tx,
            key: KeyPath::new(),
            seq_status: 0,
            seq_regs: 0,
            last_unasked: None,
            unasked_sent: false,
            record_last: [None; 8],
            record_total: RateLimit::new(u16::MAX, 10),
            not_owner_last: [None; 8],
            refused_limit: RateLimit::new(10, 10),
            next_status_ms: 0,
            link_ok: true,
            dropped_reg_from_7250: 0,
            dropped_not_owner: 0,
            duc_iq_ignored: 0,
            was_keyed: false,
            dropped_record: 0,
            update_in_progress: false,
            trial: false,
            last_update_answer: None,
        }
    }

    /// Open the P2 sockets (Table B2-1). Port 1028 is never opened (0017).
    pub fn open(chip: &mut Chip) -> Result<(), ChError<embassy_stm32::spi::Error>> {
        for (s, port) in PORTS {
            chip.udp_open(s, port)?;
        }
        Ok(())
    }

    /// `LINK_STATUS` (Table B8-2): [15:0] speed, bit 16 full duplex, bit 17 kept (latched until
    /// the next session), [31:24] refusal reason.
    pub fn set_link(&mut self, l: g7250_ch394::Link) {
        self.link_ok = l.up && l.mbit_100;
        let refusal = if l.up && !l.mbit_100 { 1u32 } else { 0 };
        let kept = self.regs.link_status & LINK_PORT_NOT_HONOURED;
        self.regs.link_status = l.speed() as u32 | ((l.full_duplex as u32) << 16) | kept | (refusal << 24);
    }

    /// A General packet of the session with a non-default G7250 port (B2.3, Table B7-1; 0020
    /// §13): the default stays, the request is logged and `LINK_STATUS` bit 17 is set until the
    /// next session. Returns true if the bit was newly set.
    fn foreign_port(&mut self, g: Option<g7250_radio::session::General>) -> bool {
        if !g.is_some_and(|g| g.foreign_port) || self.regs.link_status & LINK_PORT_NOT_HONOURED != 0 {
            return false;
        }
        self.regs.link_status |= LINK_PORT_NOT_HONOURED;
        println!("p2: General packet asks for a non-default port: not honoured, default ports kept (LINK_STATUS bit 17)");
        true
    }

    fn send_regs(&mut self, chip: &mut Chip, to: Addr, pairs: &[Pair]) {
        let mut out = [0u8; regmap::MAX_LEN];
        for chunk in pairs.chunks(regmap::MAX_PAIRS) {
            let n = regmap::encode(self.seq_regs, chunk, &mut out);
            self.seq_regs = self.seq_regs.wrapping_add(1);
            let _ = chip.udp_send(S7250, to.0, to.1, &out[..n]);
        }
    }

    fn arm_ctx(&self) -> ArmContext {
        ArmContext {
            run: self.session.running(),
            watchdog_ok: self.regs.watchdog_effective(),
            in_fault: false,
            inhibit: false,
            ptt_bit: self.session.ptt,
            level1_active: true,
            maintenance: self.update_in_progress || self.trial,
            mode_class_declared: self.regs.tx_mode_class != 0,
        }
    }

    /// Facts at a TX request edge (A3.1a). The TX frequency and the RF path are not followed
    /// yet (DUC-specific and High Priority frequency words, A3.9), so no request counts as in
    /// range and no RF path as ready; with the lock closed the request is refused before that.
    fn key_ctx(&self) -> KeyContext {
        KeyContext {
            in_fault: false,
            freq_in_range: false,
            rf_path_ready: false,
            pa_enabled: self.session.general.is_some_and(|g| g.pa_enable),
            mode_class_declared: self.regs.tx_mode_class != 0,
            keyed_by_other: false,
        }
    }

    /// Bring `TX_STATE` up to date and push it to the owner on change (Table B8-2: on every
    /// change). An arm that dropped under a key (Table A3-1) is reported as reason 5.
    fn sync_tx_state(&mut self, chip: &mut Chip, now: u64) {
        if self.key.keyed().is_some() && self.tx.keyed().is_none() {
            self.key.arm_dropped();
        }
        // The FIFO follows the key: armed for the underflow at a key, emptied at every unkey
        // (B10.5, B10.7); a transmission whose RF the keyer forms takes no I&Q (B10.5b).
        let keyed = self.key.keyed().is_some();
        if keyed != self.was_keyed {
            self.was_keyed = keyed;
            let iq = self.key.duc_iq_in_use();
            duc(|f| if keyed { f.key(now, iq) } else { f.unkey() });
        }
        let v = self.key.tx_state();
        if v != self.regs.tx_state {
            self.regs.tx_state = v;
            if let Some(dest) = self.session.register_destination() {
                self.send_regs(chip, dest, &[(regmap::addr::TX_STATE, v)]);
            }
        }
    }

    /// Apply session events: drops (A3.2), session values (A3.6), pushes and counters (B2.7).
    fn events(&mut self, chip: &mut Chip, ev: &[Event], now: u64) {
        for e in ev {
            match *e {
                Event::Ended { owner, reason } => {
                    let r = match reason {
                        EndReason::Stop => DropReason::RunCleared,
                        EndReason::Watchdog => DropReason::Watchdog,
                        EndReason::Takeover => DropReason::Takeover,
                        EndReason::LinkDown => DropReason::LinkDown,
                    };
                    let mut rf = crate::BoardRf;
                    let was_armed = self.tx.drop(r, &mut rf);
                    self.tx.session_end(&mut rf);
                    self.key.arm_dropped();
                    self.key.session_end();
                    self.was_keyed = false;
                    duc(|f| f.unkey());
                    self.regs.tx_state = self.key.tx_state();
                    self.regs.session_end();
                    self.regs.armed = false;
                    if reason == EndReason::Takeover && was_armed {
                        let p = [(regmap::addr::TX_ARM_STATE, self.tx.arm_state_reg())];
                        self.send_regs(chip, owner, &p);
                    }
                    if matches!(reason, EndReason::Stop | EndReason::Takeover) {
                        self.seq_status = 0;
                        self.seq_regs = 0;
                    }
                    println!("p2: session of {}:{} ended ({:?})", crate::net::ip_str(owner.0).as_str(), owner.1, reason);
                }
                Event::Started { owner, restart, takeover } => {
                    self.unasked_sent = false;
                    // Bit 17 is latched until the next session (Table B8-2); the start pushes
                    // below carry the value for this session's General packet.
                    self.regs.link_status &= !LINK_PORT_NOT_HONOURED;
                    self.foreign_port(self.session.general);
                    if !restart {
                        self.seq_status = 0;
                        self.seq_regs = 0;
                    }
                    self.regs.watchdog_written = self.session.general.map(|g| g.timer);
                    self.next_status_ms = now;
                    // The new owner's set-up packets apply now (B5.1a item 2): PA enable from its
                    // General packet, if any (after a takeover by a High Priority packet alone it
                    // stays cleared, A3.6a), and its DUC-specific settings.
                    let mut rf = crate::BoardRf;
                    let pa = self.session.general.is_some_and(|g| g.pa_enable);
                    self.key.pa_enable(pa, &mut self.tx, &mut rf);
                    if let Some((d, clamped)) = self.session.duc_setup.take() {
                        self.key.set_duc(d, clamped);
                        if clamped {
                            println!("p2: DUC-specific hang delay above 2000 ms clamped to 2000 ms (Table B10-1)");
                        }
                    }
                    self.regs.tx_state = self.key.tx_state();
                    println!(
                        "p2: session of {}:{} started{}{}",
                        crate::net::ip_str(owner.0).as_str(),
                        owner.1,
                        if restart { " (restart)" } else { "" },
                        if takeover { " (takeover)" } else { "" }
                    );
                    let Some(dest) = self.session.register_destination() else { continue };
                    let pairs = self.regs.session_start_pairs(&self.tx);
                    self.send_regs(chip, dest, &pairs);
                    // The unasked record, once per session, if the set-up had a General packet
                    // (B4.4 item 2), at most once per 1 s per destination.
                    if self.session.general.is_some() {
                        self.send_unasked_record(chip, dest, now);
                    }
                }
                Event::Refused { from } => {
                    if self.refused_limit.allow(from.0, from.1, now) {
                        let p = [(regmap::addr::LINK_STATUS, self.regs.link_status), (regmap::addr::STREAM_STATUS, 0x08)];
                        self.send_regs(chip, from, &p);
                    }
                }
            }
        }
    }

    fn send_unasked_record(&mut self, chip: &mut Chip, dest: Addr, now: u64) {
        if self.unasked_sent || self.last_unasked.is_some_and(|(a, t)| a == dest && now - t < 1000) {
            return;
        }
        self.unasked_sent = true;
        self.last_unasked = Some((dest, now));
        let mut pairs: heapless::Vec<Pair, 240> = heapless::Vec::new();
        for (i, v) in self.regs.record.iter().enumerate() {
            let _ = pairs.push((i as u16, *v));
        }
        self.send_regs(chip, dest, &pairs);
    }

    /// `STREAM_STATUS` now (Table B8-3, with its reason order).
    fn stream_status(&self) -> u32 {
        use g7250_radio::stream;
        let i = stream::DdcInputs {
            link_ok: self.link_ok,
            running: self.session.running(),
            general: self.session.general,
            ddc: self.session.ddc,
            offered_384: false,
            // DDC0 needs the FPGA bus, closed until its parameters are cleared (fpga_gate;
            // P-RX-2, 0042): reason 2 until then.
            ddc0_built: g7250_safety::fpga_gate::permit().is_some(),
            update: self.update_in_progress,
        };
        let duc = stream::duc0(self.update_in_progress, self.session.running(), self.link_ok, self.tx.calibrated());
        stream::register(stream::ddc0(&i), duc)
    }

    /// Recompute `STREAM_STATUS` and push it to the owner on change (B8.2; Table B8-3).
    fn update_stream_status(&mut self, chip: &mut Chip) {
        let v = self.stream_status();
        if v != self.regs.stream_status {
            self.regs.stream_status = v;
            if let Some(dest) = self.session.register_destination() {
                self.send_regs(chip, dest, &[(regmap::addr::STREAM_STATUS, v)]);
            }
        }
    }

    /// `UPDATE_BEGIN` accepted (A1.8; 0022 §10): the session ends (an arm drops with
    /// `MAINTENANCE`), streams stop, and from now on until a restart no session starts.
    pub fn enter_update(&mut self, now: u64) {
        let mut rf = crate::BoardRf;
        let _ = self.tx.drop(DropReason::Maintenance, &mut rf);
        self.key.arm_dropped();
        self.key.session_end();
        self.was_keyed = false;
        duc(|f| f.unkey());
        self.regs.tx_state = self.key.tx_state();
        if let Some(owner) = self.session.owner() {
            self.tx.session_end(&mut rf);
            self.regs.session_end();
            self.regs.armed = false;
            println!("p2: session of {}:{} ended (firmware update)", crate::net::ip_str(owner.0).as_str(), owner.1);
        }
        self.session = Session::new();
        self.update_in_progress = true;
        self.regs.stream_status = self.stream_status();
        let _ = now;
    }

    /// A C&C packet on 1024 (General), 1025, 1026 or 1027.
    pub fn cnc(&mut self, chip: &mut Chip, from: Addr, kind: CnC, p: &[u8], now: u64) {
        if self.update_in_progress {
            // No session during an update; a run request is answered with STREAM_STATUS
            // reason 10, at most once per 100 ms (A1.8; T-A1-23).
            if kind == CnC::HighPriority && p.len() > 4 && p[4] & 0x01 != 0 && self.last_update_answer.is_none_or(|t| now.saturating_sub(t) >= 100) {
                self.last_update_answer = Some(now);
                let pairs = [(regmap::addr::STREAM_STATUS, self.stream_status())];
                self.send_regs(chip, from, &pairs);
            }
            return;
        }
        let first_general = kind == CnC::General && self.session.owner() == Some(from);
        let ev = self.session.on_cnc(from, kind, p, now, self.link_ok);
        // The status of a new session is part of its start pushes; compute it first.
        self.regs.stream_status = self.stream_status();
        self.events(chip, &ev, now);
        self.update_stream_status(chip);
        if self.session.owner() == Some(from) {
            let mut rf = crate::BoardRf;
            match kind {
                CnC::HighPriority if self.session.running() => {
                    let ctx = self.key_ctx();
                    self.key.on_high_priority(Requests::parse(p), now, &mut self.tx, &ctx, &mut rf);
                }
                CnC::DucSpecific => {
                    if self.key.on_duc_specific(p) {
                        println!("p2: DUC-specific hang delay above 2000 ms clamped to 2000 ms (Table B10-1)");
                    }
                }
                CnC::General => {
                    let on = self.session.general.is_some_and(|g| g.pa_enable);
                    self.key.pa_enable(on, &mut self.tx, &mut rf);
                }
                _ => {}
            }
            self.sync_tx_state(chip, now);
        }
        if kind == CnC::General && self.session.owner() == Some(from) && self.foreign_port(self.session.general) {
            if let Some(dest) = self.session.register_destination() {
                let p = [(regmap::addr::LINK_STATUS, self.regs.link_status)];
                self.send_regs(chip, dest, &p);
            }
        }
        if first_general {
            if let Some(g) = self.session.general {
                self.regs.watchdog_written = Some(g.timer);
                // The unasked record at the owner's first General packet of the session
                // (B4.4 item 2), if not sent at the start.
                if let Some(dest) = self.session.register_destination() {
                    self.send_unasked_record(chip, dest, now);
                }
            }
        }
    }

    /// A datagram on 7250 (B8).
    pub fn register_packet(&mut self, chip: &mut Chip, from: Addr, p: &[u8], now: u64) {
        let owner = self.session.owner() == Some(from);
        if p.len() < 10 {
            return; // Table B11-1
        }
        if !owner && from.1 == regmap::PORT {
            self.dropped_reg_from_7250 += 1; // another G7250 (B8.5)
            return;
        }
        let mut rf = crate::BoardRf;
        let arm = self.arm_ctx();
        let class = self.regs.tx_mode_class;
        let mut c = WriteCtx { from_owner: owner, busy: self.update_in_progress, tx: &mut self.tx, rf: &mut rf, arm, key: KeyContext::default() };
        let ans = self.regs.handle(p, &mut c);
        // A mode class change while keyed unkeys first (A3.9c); a disarm under a key is reason 5.
        if self.regs.tx_mode_class != class {
            self.key.mode_class_changed(&mut self.tx, &mut rf);
        }
        self.sync_tx_state(chip, now);
        if ans.pairs.is_empty() {
            return;
        }
        let dest = if owner { self.session.register_destination().unwrap_or(from) } else { from };
        if !owner {
            if ans.record_sent {
                // B4.4 item 3: once per 1 s per source (a real interval, not a fixed window), 10
                // per second in total; requests above it are dropped and counted.
                if !spaced(&mut self.record_last, from, now, 1000) || !self.record_total.allow([0; 4], 0, now) {
                    self.dropped_record += 1;
                    return;
                }
            }
            if ans.not_owner {
                let i = self.not_owner_last.iter().position(|e| e.is_some_and(|(a, _)| a == from));
                if let Some(i) = i {
                    if now - self.not_owner_last[i].unwrap().1 < 100 {
                        self.dropped_not_owner += 1;
                        return;
                    }
                }
                let slot = i.or_else(|| self.not_owner_last.iter().position(|e| e.is_none())).unwrap_or(0);
                self.not_owner_last[slot] = Some((from, now));
            }
        }
        self.send_regs(chip, dest, &ans.pairs);
    }

    /// Call every few ms: watchdog and link (B5.6, B5.8), High Priority status (B9.2).
    pub fn tick(&mut self, chip: &mut Chip, now: u64) {
        if let Some(e) = self.session.tick(now, self.regs.watchdog_effective(), self.link_ok) {
            self.events(chip, &[e], now);
            self.update_stream_status(chip);
        }
        // Keyed timeout, CWX cap, maximum TX time, hang delay (B10.5, B10.5a, B10.5b, Table A3-2).
        let mut rf = crate::BoardRf;
        let mut changed = self.key.tick(now, self.regs.keyed_timeout_ms, self.regs.tx_time_in_force(), &mut self.tx, &mut rf);
        // The DUC takes its samples while keyed; unkeyed, old I&Q ages out (B10.7). The underrun
        // check needs the underrun time of the calibration set (P-TIM-2), none in this build.
        let keyed = self.was_keyed;
        let since = duc(|f| {
            if keyed {
                f.consume(now);
            } else {
                f.idle(now);
            }
            f.underflow_since()
        });
        if let Some(u) = self.tx.underrun_ms() {
            changed |= self.key.underrun(now, since, u, self.regs.keyed_timeout_ms, &mut self.tx, &mut rf);
        }
        if changed {
            self.sync_tx_state(chip, now);
        }
        if let Some(owner) = self.session.owner() {
            if now >= self.next_status_ms {
                self.next_status_ms = now + STATUS_MS;
                // Table B9-1: sequence number, everything else 0 in this build (no PTT/KEY in
                // v1.0, 0030; ADC words follow with the sensor driver, P-SEN-1).
                let mut b = [0u8; 60];
                b[0..4].copy_from_slice(&self.seq_status.to_be_bytes());
                // Byte 30 bit [2]: DUC FIFO underflow from the firmware's own buffer (Table B9-1).
                if duc(|f| f.underflow_since().is_some()) {
                    b[30] |= 0x04;
                }
                self.seq_status = self.seq_status.wrapping_add(1);
                let _ = chip.udp_send(S1025, owner.0, owner.1, &b);
            }
        }
    }

    /// A DUC I&Q packet on port 1029 (B10.7). It enters the FIFO only from the owner during its
    /// session, with record `tx` bits 1 and 4 set and no DUC-specific refusal standing; anything
    /// else is discarded whole and counted, with no other effect.
    pub fn duc_iq(&mut self, from: Addr, p: &[u8], now: u64) {
        let ok = self.session.owner() == Some(from)
            && !self.update_in_progress
            && self.tx.record_tx_bit1()
            && self.tx.record_tx_bit4()
            && !self.key.duc().refused();
        if !ok {
            self.duc_iq_ignored += 1;
            return;
        }
        // A full FIFO while keyed, or I&Q while the keyer forms the RF: counted in the FIFO.
        let _ = duc(|f| f.push(p, now));
    }

    /// The radio's address changes (Set IP, B3.9; A1.6 C-1): a running session ends, the arm
    /// drops with SESSION_CHANGED (Table A3-1 event 9), nothing is carried over (B5.4 item 3).
    pub fn address_changed(&mut self, _chip: &mut Chip, _now: u64) {
        if let Some(owner) = self.session.owner() {
            let mut rf = crate::BoardRf;
            let _ = self.tx.drop(DropReason::SessionChanged, &mut rf);
            self.tx.session_end(&mut rf);
            self.key.arm_dropped();
            self.key.session_end();
            self.was_keyed = false;
            duc(|f| f.unkey());
            self.regs.tx_state = self.key.tx_state();
            self.regs.session_end();
            self.regs.armed = false;
            println!("p2: session of {}:{} ended (address change)", crate::net::ip_str(owner.0).as_str(), owner.1);
        }
        self.session = Session::new();
        self.seq_status = 0;
        self.seq_regs = 0;
    }

    /// Discovery byte 4 (B3.4) and the REDISCOVERY drop (Table A3-1 event 12).
    pub fn discovery_from(&mut self, from: Addr) -> bool {
        if self.session.owner() == Some(from) {
            let mut rf = crate::BoardRf;
            let _ = self.tx.drop(DropReason::Rediscovery, &mut rf);
            self.key.arm_dropped();
        }
        self.session.discovery_in_use(from)
    }
}
