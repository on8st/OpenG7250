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
use g7250_safety::tx::{ArmContext, DropReason, KeyContext, TxCore};

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
    pub duc_iq_ignored: u64,
    pub dropped_record: u64,
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

impl P2 {
    pub fn new(tx: TxCore, mut regs: RegMap) -> Self {
        // Before any session: DDC0 9 (run bit clear), DUC0 14 (TX not calibrated), Table B8-3.
        regs.stream_status = g7250_radio::stream::register(9, g7250_radio::stream::duc0(tx.calibrated()));
        P2 {
            session: Session::new(),
            regs,
            tx,
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
            dropped_record: 0,
        }
    }

    /// Open the P2 sockets (Table B2-1). Port 1028 is never opened (0017).
    pub fn open(chip: &mut Chip) -> Result<(), ChError<embassy_stm32::spi::Error>> {
        for (s, port) in PORTS {
            chip.udp_open(s, port)?;
        }
        Ok(())
    }

    /// `LINK_STATUS` (Table B8-2): [15:0] speed, bit 16 full duplex, [31:24] refusal reason.
    pub fn set_link(&mut self, l: g7250_ch394::Link) {
        self.link_ok = l.up && l.mbit_100;
        let refusal = if l.up && !l.mbit_100 { 1u32 } else { 0 };
        self.regs.link_status = l.speed() as u32 | ((l.full_duplex as u32) << 16) | (refusal << 24);
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
            maintenance: false,
            mode_class_declared: self.regs.tx_mode_class != 0,
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
                    if !restart {
                        self.seq_status = 0;
                        self.seq_regs = 0;
                    }
                    self.regs.watchdog_written = self.session.general.map(|g| g.timer);
                    self.next_status_ms = now;
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

    /// Recompute `STREAM_STATUS` and push it to the owner on change (B8.2; Table B8-3).
    fn update_stream_status(&mut self, chip: &mut Chip) {
        let i = g7250_radio::stream::DdcInputs {
            link_ok: self.link_ok,
            running: self.session.running(),
            general: self.session.general,
            ddc: self.session.ddc,
            offered_384: false,
            // DDC0 needs the FPGA bus, closed until its parameters are cleared (fpga_gate;
            // coordinator 2026-11-21): reason 2 until then.
            ddc0_built: false,
        };
        let v = g7250_radio::stream::register(g7250_radio::stream::ddc0(&i), g7250_radio::stream::duc0(self.tx.calibrated()));
        if v != self.regs.stream_status {
            self.regs.stream_status = v;
            if let Some(dest) = self.session.register_destination() {
                self.send_regs(chip, dest, &[(regmap::addr::STREAM_STATUS, v)]);
            }
        }
    }

    /// A C&C packet on 1024 (General), 1025, 1026 or 1027.
    pub fn cnc(&mut self, chip: &mut Chip, from: Addr, kind: CnC, p: &[u8], now: u64) {
        let first_general = kind == CnC::General && self.session.owner() == Some(from);
        let ev = self.session.on_cnc(from, kind, p, now, self.link_ok);
        // The status of a new session is part of its start pushes; compute it first.
        let i = g7250_radio::stream::DdcInputs {
            link_ok: self.link_ok,
            running: self.session.running(),
            general: self.session.general,
            ddc: self.session.ddc,
            offered_384: false,
            ddc0_built: false,
        };
        self.regs.stream_status = g7250_radio::stream::register(g7250_radio::stream::ddc0(&i), g7250_radio::stream::duc0(self.tx.calibrated()));
        self.events(chip, &ev, now);
        self.update_stream_status(chip);
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
        let mut c = WriteCtx { from_owner: owner, busy: false, tx: &mut self.tx, rf: &mut rf, arm, key: KeyContext::default() };
        let ans = self.regs.handle(p, &mut c);
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
        if let Some(owner) = self.session.owner() {
            if now >= self.next_status_ms {
                self.next_status_ms = now + STATUS_MS;
                // Table B9-1: sequence number, everything else 0 in this build (no PTT/KEY in
                // v1.0, 0030; ADC words follow with the sensor driver, P-SEN-1).
                let mut b = [0u8; 60];
                b[0..4].copy_from_slice(&self.seq_status.to_be_bytes());
                self.seq_status = self.seq_status.wrapping_add(1);
                let _ = chip.udp_send(S1025, owner.0, owner.1, &b);
            }
        }
    }

    /// The radio's address changes (Set IP, B3.9; A1.6 C-1): a running session ends, the arm
    /// drops with SESSION_CHANGED (Table A3-1 event 9), nothing is carried over (B5.4 item 3).
    pub fn address_changed(&mut self, _chip: &mut Chip, _now: u64) {
        if let Some(owner) = self.session.owner() {
            let mut rf = crate::BoardRf;
            let _ = self.tx.drop(DropReason::SessionChanged, &mut rf);
            self.tx.session_end(&mut rf);
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
        }
        self.session.discovery_in_use(from)
    }
}
