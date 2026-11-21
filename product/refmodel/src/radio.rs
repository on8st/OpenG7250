//! The core of the reference model: a deterministic state machine of the G7250 main unit as
//! seen on the network (spec parts A and B). No I/O: `on_datagram` and `tick` take the time in
//! microseconds and return the datagrams to send. The UDP shell is `main.rs`.
//!
//! Modelled in T1: B2 ports, B3 discovery and Set IP, B4 record, B5 session / takeover /
//! watchdog / set-up phase / restart, B6 DDC0 stream and refusals, B7 field handling, B8 register
//! map with every write rejection, B9 status packet, B10 arm handshake (locked and unlocked) with
//! a minimal keying model (PTT edge, keyed timeout, PA enable, TX ranges, mode class), the CWX
//! keyer of B10.5a with its timing (keyer.rs; 0046) and the key.log fixture, B11 minimum
//! lengths, A1 management channel (mgmt.rs). Not modelled: relays and settle times, sensors and
//! limit actions, faults, the hang delay, the hardware keyer from KEY, ATU tune cycle (always
//! NOT_CALIBRATED), DUC
//! FIFO, wideband, level 2, firmware update transfer.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};

use crate::cap;
use crate::codes::*;
use crate::keyer::{self, Keyer};
use crate::mgmt::Mgmt;
use crate::util::*;

pub const MS: u64 = 1_000;
/// The firmware in the slots (A1.8): survives resets; the boot stage picks the running slot.
#[derive(Debug, Clone)]
pub struct Image {
    pub version: (u8, u8, u16),
    pub len: u32,
    pub sha: [u8; 32],
    /// confirmed (A1.8 self-confirmation) or the factory image
    pub good: bool,
}

#[derive(Debug, Clone)]
pub struct Firmware {
    /// 1 slot A, 2 slot B (IDENTIFY tag 23)
    pub running: u8,
    pub slots: [Option<Image>; 2],
    /// start of an unconfirmed trial boot
    pub trial: Option<u64>,
    pub rolled_back: bool,
}

/// Flash layout of the model (IDENTIFY tag 25, 0062): in 128 KiB sectors, slot A, slot B, store.
pub const LAYOUT: [(u8, u8); 3] = [(2, 2), (4, 2), (6, 2)];
pub const SECTOR_BYTES: u32 = 128 * 1024;

impl Firmware {
    pub fn slot_size(slot: u8) -> u32 {
        LAYOUT[(slot - 1) as usize].1 as u32 * SECTOR_BYTES
    }
    pub fn other(&self) -> u8 {
        3 - self.running
    }
    pub fn running_image(&self) -> Option<&Image> {
        self.slots[(self.running - 1) as usize].as_ref()
    }
    pub fn other_image(&self) -> Option<&Image> {
        self.slots[(self.other() - 1) as usize].as_ref()
    }
}

/// TX_STATE [11:8] source CWX, ATU (Table B8-2)
pub const SRC_CWX: u8 = 2;
pub const SRC_ATU: u8 = 4;

/// The ATU tune cycle of A3.9b. The model has no ATU: how a cycle ends is set by the atu.sim
/// fixture (model only), and so is the ATU tune time, which is pending hardware (P-ATU-1).
#[derive(Debug, Clone)]
pub struct Atu {
    /// ATU_STATE once P-ATU-1 is set (0 off or bypassed, 2 tuning, 3 tuned, 4 failed)
    pub state: u32,
    /// the running cycle: (start, end by itself at, ends tuned)
    pub cycle: Option<(u64, Option<u64>, bool)>,
    pub last_end: Option<u64>,
    /// atu.sim: the next cycle's result (Some(true) tuned, Some(false) failed, None never by
    /// itself) and after how long
    pub sim_result: Option<bool>,
    pub sim_after_us: u64,
    pub tune_time_us: u64,
}

impl Default for Atu {
    fn default() -> Atu {
        // tune time: the manual's "initial tuning <= 10 s" plus a 1 s margin (P-ATU-1 pending)
        Atu { state: 0, cycle: None, last_end: None, sim_result: Some(true), sim_after_us: 300 * MS, tune_time_us: 11 * S }
    }
}
pub const S: u64 = 1_000_000;

#[derive(Debug, Clone)]
pub struct Config {
    pub ip: Ipv4Addr,
    pub netmask: Ipv4Addr,
    pub mac: [u8; 6],
    pub fw: (u8, u8, u16),
    pub beta: bool,
    /// Start with a complete simulated calibration set (Annex P, A1.12): record `tx` bit 4 = 1.
    /// The default is the r1.0 state before calibration: `tx` bit 4 = 0, TX frozen (0035, 0036).
    pub calibrated: bool,
    /// Start with a set that holds P-RX-2 only (DDC0 offered, B6.1), when not `calibrated`.
    pub rx_params: bool,
    /// Offered DDC0 rates: 48, 96, 192 ksps until P-NET-1 shows 50 MHz SPI (Annex P).
    pub ddc_rates: u32,
    /// Persistent limits (C-3). Built-in values per Table B8-2. SWR S2 and PA T3 are pending
    /// hardware (P-PWR-2): SIMULATED values, used only while a calibration set holds them.
    pub builtin_tx_time_ms: u32,
    pub builtin_fwd_power_mw: u32,
    pub builtin_swr_s2: u32,
    pub builtin_pa_temp_t3_mc: u32,
    pub boot_counter: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            ip: Ipv4Addr::new(127, 0, 0, 1),
            netmask: Ipv4Addr::new(255, 0, 0, 0),
            mac: [0x02, 0x00, 0x00, 0x00, 0x72, 0x50],
            fw: (0, 1, 1),
            beta: true,
            calibrated: false,
            rx_params: true,
            ddc_rates: 0x07,
            builtin_tx_time_ms: 300_000,
            builtin_fwd_power_mw: 100_000,
            builtin_swr_s2: 3_000,
            builtin_pa_temp_t3_mc: 70_000,
            boot_counter: 1,
        }
    }
}

/// A received datagram. `to_ip` is the destination address when the shell knows it (for the
/// broadcast rules); `None` means the radio's own address.
#[derive(Debug, Clone)]
pub struct Rx<'a> {
    pub src: Host,
    pub local_port: u16,
    pub to_ip: Option<Ipv4Addr>,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tx {
    pub src_port: u16,
    pub dst: Host,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct General {
    pub raw: Vec<u8>,
}
impl General {
    fn b(&self, i: usize) -> u8 {
        self.raw[i]
    }
    pub fn wd_bit(&self) -> bool {
        self.b(38) & 1 != 0
    }
    pub fn pa_enable(&self) -> bool {
        self.b(58) & 1 != 0
    }
    pub fn reg_dest(&self) -> u16 {
        be16(&self.raw, 31)
    }
}

#[derive(Debug, Clone, Default)]
pub struct HpState {
    pub ptt: bool,
    pub ddc0_phase: u32,
    pub duc0_phase: u32,
    pub drive: u8,
    pub last_ptt1_at: Option<u64>,
    /// the CWX request (byte 5 bit [0] and a paddle bit) in the last packet
    pub cwx_req: bool,
    pub last_cwx1_at: Option<u64>,
    /// a 0->1 CWX edge failed the 10 ms test (B11.6a): the paddles count as released until
    /// the request is 0 again
    pub cwx_ignored: bool,
}

/// RX-side and TX-side settings of a session (B5.1a, A3.6a).
#[derive(Debug, Clone, Default)]
pub struct Settings {
    pub general: Option<General>,
    /// A General packet with byte 38 bit [0] set was acted on in this session or its set-up.
    pub wd_general_seen: bool,
    pub ddc: Option<Vec<u8>>,
    pub duc: Option<Vec<u8>>,
    pub hp: HpState,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub owner: Host,
    pub last_cc: u64,
    pub s: Settings,
    pub next_status: u64,
    pub next_ddc: u64,
    pub record_sent: bool,
    pub port_not_honoured: bool,
    pub arm_confirmed_at: Option<u64>,
    pub keyed_at: Option<u64>,
    pub last_hp: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ArmState {
    pub state: u8,
    pub reject: u8,
    pub drop: u8,
    pub gen: u8,
}
impl ArmState {
    pub fn word(&self) -> u32 {
        (self.gen as u32) << 24 | (self.drop as u32) << 16 | (self.reject as u32) << 8 | self.state as u32
    }
}

#[derive(Debug, Clone, Default)]
pub struct TxState {
    pub keyed: bool,
    pub source: u8,
    pub unkey: u8,
    /// limit id with unkey reason 4 (TX_STATE [31:24])
    pub limit: u8,
    pub outside: bool,
}

/// Notional picture of the HIS 85 safe-state items, for the `hw.observe` fixture. The model
/// holds them in the safe state whenever it is not keyed; it never "keys" while TX is locked.
#[derive(Debug, Clone, Default)]
pub struct Hw {
    pub tx_levels: bool,
    pub chain_a_bit7_ever: bool,
    pub bias_nonzero_ever: bool,
    pub atu_latch_writes: u32,
}

pub struct Radio {
    pub cfg: Config,
    pub now: u64,
    pub booted_at: u64,
    pub record: Vec<u32>,
    pub session: Option<Session>,
    pub setup: Lru<Host, Settings>,
    pub last_owner: Option<Host>,
    /// RX-side settings carried for a restart by the same host (B5.4 item 1).
    pub carried: Option<Settings>,
    pub counters_continue: bool,
    pub seq: HashMap<u16, u32>,
    /// port 7250 counter for every answer to a host that is not the owner (0051); reset only
    /// by a reset of the radio
    pub nonowner_seq: u32,
    /// port 7250 counter of the old session towards the old owner, continued by the repeated
    /// TAKEOVER notice (0051)
    pub old_seq: u32,
    pub token: u32,
    pub token_n: u32,
    pub arm: ArmState,
    pub tx: TxState,
    pub mode_class: u8,
    pub keyed_timeout_ms: u32,
    pub session_limits: [u32; 4],
    pub persistent_limits: [u32; 4],
    pub write_rejects: u8,
    pub update_in_progress: bool,
    pub link_mbit: u16,
    pub stats: HashMap<String, u64>,
    pub log: Vec<(u32, u64, String)>,
    pub log_seq: u32,
    pub hw: Hw,
    pub scheduled: Vec<(u64, Tx)>,
    pub old_owner_notice: Option<(Host, u16, u64, u64, Vec<u8>)>, // host, dest port, until, last, packet
    lim_disc: Limiter<Host>,
    lim_cap: Limiter<Host>,
    lim_notowner: Limiter<Host>,
    lim_link: Limiter<Host>,
    lim_update: Limiter<Host>,
    last_unasked: HashMap<Host, u64>,
    pub mgmt: Mgmt,
    pub resets: u64,
    /// The stored calibration set (A1.12); kept by FACTORY_RESET, lost only with the model.
    pub cal: Option<Calibration>,
    /// The CWX keyer of B10.5a (0046).
    pub keyer: Keyer,
    /// The ATU tune cycle of A3.9b (0047).
    pub atu: Atu,
    /// The images in the two slots and the boot choice (A1.8).
    pub fw: Firmware,
    /// The key.log fixture (format v1.5): key changes and High Priority arrivals since start.
    pub keylog: Option<KeyLog>,
}

/// key.log (format v1.5): changes of the keyed state and High Priority packets as received.
#[derive(Debug, Clone, Default)]
pub struct KeyLog {
    pub key: Vec<(u64, bool)>,
    pub hp: Vec<(u64, u32, u8, u8)>,
}

/// Calibration set (A1.12, Annex M Table M-5).
#[derive(Debug, Clone)]
pub struct Calibration {
    pub id: [u8; 32],
    pub date: u32,
    pub source: String,
    pub entries: Vec<(String, Vec<u8>)>,
}

/// Parameters the freeze rule of Annex P names: groups P-RF, P-PWR, P-SEN, P-TIM, and P-TX-1,
/// P-ATU-1, P-ANT-1 (Table P-1).
pub const FREEZE_SET: [&str; 19] = [
    "P-RF-1", "P-RF-2", "P-RF-3", "P-RF-4", "P-RF-5", "P-PWR-1", "P-PWR-2", "P-PWR-3", "P-PWR-4",
    "P-PWR-5", "P-SEN-1", "P-SEN-2", "P-SEN-3", "P-SEN-4", "P-SEN-5", "P-TIM-1", "P-TIM-2",
    "P-TX-1", "P-ATU-1",
];
/// Annex P freeze rule as amended by 0042 (P-RX-2) and 0050 (P-SYS-9).
pub const FREEZE_EXTRA: [&str; 3] = ["P-ANT-1", "P-RX-2", "P-SYS-9"];
/// Every parameter id of Table P-1.
pub const ALL_PARAMS: [&str; 37] = [
    "P-RF-1", "P-RF-2", "P-RF-3", "P-RF-4", "P-RF-5", "P-ANT-1", "P-ATU-1", "P-PWR-1",
    "P-PWR-2", "P-PWR-3", "P-PWR-4", "P-PWR-5", "P-SEN-1", "P-SEN-2", "P-SEN-3", "P-SEN-4",
    "P-SEN-5", "P-TIM-1", "P-TIM-2", "P-NET-1", "P-NET-2", "P-NET-3", "P-SYS-1", "P-SYS-2",
    "P-SYS-3", "P-SYS-4", "P-SYS-5", "P-SYS-6", "P-SYS-7", "P-SYS-8", "P-SYS-9", "P-RX-1", "P-RX-2",
    "P-TX-1", "P-L2-1", "P-L2-2", "P-L2-3",
];

impl Calibration {
    pub fn has(&self, id: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == id)
    }
    /// Complete for the freeze rule (Annex P): every FREEZE_SET parameter and P-ANT-1.
    pub fn complete(&self) -> bool {
        FREEZE_SET.iter().all(|p| self.has(p)) && FREEZE_EXTRA.iter().all(|p| self.has(p))
    }
    /// A simulated complete set, for `--tx unlocked` (Annex P "Tests": simulated values).
    pub fn simulated() -> Self {
        let mut entries: Vec<(String, Vec<u8>)> = FREEZE_SET.iter().map(|p| (p.to_string(), vec![1])).collect();
        entries.extend(FREEZE_EXTRA.iter().map(|p| (p.to_string(), vec![1])));
        Calibration { id: [0x5A; 32], date: 0, source: "refmodel simulated".into(), entries }
    }
    /// A simulated set that holds only P-RX-2: DDC0 offered, TX still frozen.
    pub fn simulated_rx() -> Self {
        Calibration { id: [0x52; 32], date: 0, source: "refmodel simulated (P-RX-2 only)".into(),
                      entries: vec![("P-RX-2".into(), vec![1])] }
    }
}

pub fn phase_to_hz(p: u32) -> f64 {
    p as f64 * DSP_CLOCK_HZ as f64 / 4_294_967_296.0
}

impl Radio {
    pub fn new(cfg: Config, now: u64) -> Self {
        let cal = if cfg.calibrated {
            Some(Calibration::simulated())
        } else if cfg.rx_params {
            Some(Calibration::simulated_rx())
        } else {
            None
        };
        let record = cap::build(&cfg, cal.as_ref().map_or(false, |c| c.complete()), cal.as_ref().map_or(false, |c| c.has("P-RX-2")));
        let persistent = [cfg.builtin_tx_time_ms, cfg.builtin_fwd_power_mw, cfg.builtin_swr_s2,
                          cfg.builtin_pa_temp_t3_mc];
        let mut r = Radio {
            cfg,
            now,
            booted_at: now,
            record,
            session: None,
            setup: Lru::new(4),
            last_owner: None,
            carried: None,
            counters_continue: false,
            seq: HashMap::new(),
            token: 0,
            token_n: 0,
            arm: ArmState { drop: drop::RESET, ..Default::default() },
            tx: TxState::default(),
            mode_class: 0,
            keyed_timeout_ms: 500,
            session_limits: [0; 4],
            persistent_limits: persistent,
            write_rejects: 0,
            update_in_progress: false,
            link_mbit: 100,
            stats: HashMap::new(),
            log: Vec::new(),
            log_seq: 0,
            hw: Hw::default(),
            scheduled: Vec::new(),
            old_owner_notice: None,
            lim_disc: Limiter::new(S, 10, 50),
            lim_cap: Limiter::new(S, 1, 10),
            lim_notowner: Limiter::new(100 * MS, 1, usize::MAX),
            lim_link: Limiter::new(100 * MS, 1, 10),
            lim_update: Limiter::new(100 * MS, 1, usize::MAX),
            last_unasked: HashMap::new(),
            mgmt: Mgmt::new(now),
            resets: 0,
            cal,
            keyer: Keyer::new(keyer::Settings { iambic: false, mode_b: false, strict: false, speed: 0, weight: 50 }),
            keylog: None,
            atu: Atu::default(),
            fw: Firmware { running: 1, slots: [None, None], trial: None, rolled_back: false },
            nonowner_seq: 0,
            old_seq: 0,
        };
        r.new_token();
        // the factory image in slot A, confirmed
        r.fw.slots[0] = Some(Image { version: r.cfg.fw, len: 0, sha: [0; 32], good: true });
        r.logf("power-on reset");
        r
    }

    /// Stop everything for an update (A1.8; 0022 §10): the P2 session ends, the arm drops with
    /// MAINTENANCE, the streams stop; the state lasts until a restart.
    pub fn begin_update(&mut self, out: &mut Vec<Tx>) {
        self.update_in_progress = true;
        if self.session.is_some() {
            self.end_session(drop::MAINTENANCE, false, out);
        }
        self.logf("update started: session ended, streams stopped");
    }

    /// The boot stage after a reset: a trial image that has not confirmed itself is rolled back
    /// (A1.8 U-6; 0044).
    fn boot_choice(&mut self, cause: &str) {
        if self.fw.trial.is_some() {
            self.fw.trial = None;
            self.fw.slots[(self.fw.running - 1) as usize] = None;
            self.fw.running = self.fw.other();
            self.fw.rolled_back = true;
            self.logf(&format!("trial image not confirmed before reset ({cause}): rolled back to slot {}", self.fw.running));
        }
    }

    /// Self-confirmation of a trial image (0044): 10 s running with the network up (always up in
    /// the model) and no class 2 fault (not modelled).
    fn confirm_trial(&mut self) {
        if let Some(t) = self.fw.trial {
            if self.now >= t + 10 * S {
                self.fw.trial = None;
                if let Some(img) = self.fw.slots[(self.fw.running - 1) as usize].as_mut() {
                    img.good = true;
                }
                self.logf("trial image confirmed");
            }
        }
    }

    // ---------- helpers ----------
    /// Record `tx` bit 4 as published in the record (constant within a session, B4.7).
    pub fn tx_calibrated(&self) -> bool {
        self.record[0x20] & (1 << 4) != 0
    }
    /// Values that need the calibration set (A1.12 item 4): S2 and T3 are part of P-PWR-2.
    pub fn has_param(&self, id: &str) -> bool {
        self.cal.as_ref().map_or(false, |c| c.has(id))
    }
    /// Rebuild the record after a calibration change; only while no session runs (A1.12 item 1).
    pub fn rebuild_record(&mut self) {
        let complete = self.cal.as_ref().map_or(false, |c| c.complete());
        self.record = cap::build(&self.cfg, complete, self.has_param("P-RX-2"));
    }
    pub fn count(&mut self, what: &str) {
        *self.stats.entry(what.to_string()).or_insert(0) += 1;
    }
    pub fn logf(&mut self, msg: &str) {
        self.log_seq += 1;
        let up = (self.now - self.booted_at) / MS;
        self.log.push((self.log_seq, up, msg.to_string()));
        if self.log.len() > 512 {
            self.log.remove(0);
        }
    }
    pub fn own_subnet(&self, ip: Ipv4Addr) -> bool {
        in_subnet(ip, self.cfg.ip, self.cfg.netmask)
    }
    fn next_seq(&mut self, port: u16) -> u32 {
        let c = self.seq.entry(port).or_insert(0);
        let v = *c;
        *c = c.wrapping_add(1);
        v
    }
    fn new_token(&mut self) {
        // B10.2 item 5: boot counter in the upper half, a per-boot counter in the lower half.
        loop {
            self.token_n = self.token_n.wrapping_add(1);
            let t = (self.cfg.boot_counter & 0xFFFF) << 16 | (self.token_n & 0xFFFF);
            if t != 0 && t != 0xFFFF_FFFF && t != ARM_MAGIC && t != self.token {
                self.token = t;
                return;
            }
        }
    }
    pub fn armed(&self) -> bool {
        self.arm.state != 0
    }
    pub fn tx_state_word(&self) -> u32 {
        let pa_disabled = !self.session.as_ref().and_then(|s| s.s.general.as_ref()).map_or(false, |g| g.pa_enable());
        (self.tx.keyed as u32)
            | (self.tx.outside as u32) << 3
            | (pa_disabled as u32) << 4
            | (self.tx.source as u32) << 8
            | (self.tx.unkey as u32) << 16
            | (self.tx.limit as u32) << 24
    }
    pub fn watchdog_state(&self) -> u32 {
        let g = self.session.as_ref().and_then(|s| s.s.general.as_ref());
        // 0039: bit 0 reads 0 and bit 1 reads 1 while bit 2 (no General packet yet) is set.
        let none = g.is_none();
        let written = g.map_or(false, |g| g.wd_bit());
        let effective = none || written || self.armed();
        (written as u32) | (effective as u32) << 1 | (none as u32) << 2
    }
    fn reg_dest(&self) -> Option<Host> {
        let s = self.session.as_ref()?;
        let p = s.s.general.as_ref().map_or(0, |g| g.reg_dest());
        Some(SocketAddrV4::new(*s.owner.ip(), if p != 0 { p } else { s.owner.port() }))
    }
    fn reg_packet(&mut self, pairs: &[(u16, u32)]) -> Vec<u8> {
        let seq = self.next_seq(port::REGMAP);
        Self::reg_packet_seq(pairs, seq)
    }
    /// An answer to a non-owner, numbered by the non-owner counter (0051).
    fn reg_packet_nonowner(&mut self, pairs: &[(u16, u32)]) -> Vec<u8> {
        let seq = self.nonowner_seq;
        self.nonowner_seq = seq.wrapping_add(1);
        Self::reg_packet_seq(pairs, seq)
    }
    fn reg_packet_seq(pairs: &[(u16, u32)], seq: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity(4 + 6 * pairs.len());
        v.extend_from_slice(&seq.to_be_bytes());
        for (a, d) in pairs {
            v.extend_from_slice(&a.to_be_bytes());
            v.extend_from_slice(&d.to_be_bytes());
        }
        v
    }
    /// Register push to the owner's register destination (B8.1, B8.2).
    pub fn push(&mut self, pairs: &[(u16, u32)], out: &mut Vec<Tx>) {
        if let Some(dst) = self.reg_dest() {
            let data = self.reg_packet(pairs);
            out.push(Tx { src_port: port::REGMAP, dst, data });
        }
    }
    pub fn stream_status(&self) -> u32 {
        let ddc = match self.ddc0_reason() {
            0 => 0x80,
            r => r as u32,
        };
        let duc = match self.duc0_reason() {
            0 => if self.tx.keyed { 0x80 } else { 0 },
            r => r as u32,
        };
        let mic = sr::NOT_OFFERED as u32;
        let wb = sr::NOT_OFFERED as u32;
        ddc | duc << 8 | mic << 16 | wb << 24
    }
    pub fn link_status(&self) -> u32 {
        let refuse = if self.link_mbit < 100 { 1u32 } else { 0 };
        let p = self.session.as_ref().map_or(false, |s| s.port_not_honoured);
        self.link_mbit as u32 | 1 << 16 | (p as u32) << 17 | refuse << 24
    }
    /// The reason DDC0 does not run (0 = running), Table B8-3.
    /// Stream reason order of Table B8-3 (0042): 10, 9, 8 (radio-wide); 1 (not enabled by the
    /// host); 13, 6, 11, 12, 5 (General-packet refusals); 2, 14 (not offered, not calibrated);
    /// 3, 4, 7 (stream parameters).
    pub fn ddc0_reason(&self) -> u8 {
        if self.update_in_progress {
            return sr::UPDATE;
        }
        let Some(s) = self.session.as_ref() else { return sr::RUN_CLEAR };
        if self.link_mbit < 100 {
            return sr::LINK;
        }
        let enabled = s.s.ddc.as_ref().map_or(false, |d| d[7] & 1 != 0);
        if !enabled {
            return sr::NOT_ENABLED;
        }
        if let Some(g) = &s.s.general {
            let b37 = g.b(37);
            if b37 & 0b1000 == 0 {
                return sr::FREQ_ENCODING;
            }
            if b37 & 0b11 != 0 {
                return sr::TIMESTAMP;
            }
            if b37 & 0b100 != 0 {
                return sr::VNA;
            }
            if g.b(39) & !0b101 != 0 {
                return sr::FORMAT;
            }
        }
        // B6.1 (0042): DDC0 is not offered until the FPGA configuration frame is set (P-RX-2)
        if !self.has_param("P-RX-2") {
            return sr::NOT_OFFERED;
        }
        let d = s.s.ddc.as_ref().unwrap();
        if !self.rate_offered(be16(d, 18)) {
            return sr::RATE;
        }
        if !(d[22] == 0 || d[22] == 24) {
            return sr::SIZE;
        }
        0
    }

    /// DUC0 byte of STREAM_STATUS, same order: radio-wide, then EER (12), then 14.
    pub fn duc0_reason(&self) -> u8 {
        if self.update_in_progress {
            return sr::UPDATE;
        }
        let Some(s) = self.session.as_ref() else { return sr::RUN_CLEAR };
        if self.link_mbit < 100 {
            return sr::LINK;
        }
        if s.s.duc.as_ref().map_or(false, |d| d[5] & 1 != 0) {
            return sr::EER;
        }
        if !self.tx_calibrated() {
            return sr::TX_NOT_CALIBRATED;
        }
        0
    }
    fn rate_offered(&self, ksps: u16) -> bool {
        let bit = match ksps {
            48 => 0,
            96 => 1,
            192 => 2,
            384 => 3,
            _ => return false,
        };
        self.cfg.ddc_rates & (1 << bit) != 0
    }

    // ---------- arm / unkey / session end ----------
    fn unkey(&mut self, reason: u8, out: &mut Vec<Tx>) {
        if self.tx.keyed && self.tx.source == SRC_CWX {
            let now = self.now;
            self.log_key(now, false);
            if reason != unkey::CW_CAP {
                self.keyer.stop(now, &mut Vec::new());
            }
        }
        if self.tx.keyed && self.tx.source == SRC_ATU {
            // A3.9b items 6, 7 (0047): every end but a tuned one leaves ATU_STATE 4
            let tuned = reason == unkey::ATU_ENDED && self.atu.cycle.map_or(false, |c| c.2);
            self.atu.cycle = None;
            self.atu.last_end = Some(self.now);
            self.atu.state = if tuned { 3 } else { 4 };
            let st = self.atu_state();
            self.push(&[(reg::ATU_STATE, st)], out);
        }
        if self.tx.keyed {
            if reason != unkey::LIMIT {
                self.tx.limit = 0;
            }
            self.tx.keyed = false;
            self.tx.source = 0;
            self.tx.unkey = reason;
            self.hw.tx_levels = false;
            if let Some(s) = self.session.as_mut() {
                s.keyed_at = None;
            }
            let w = self.tx_state_word();
            self.push(&[(reg::TX_STATE, w)], out);
        }
    }
    /// A3.2: unkey, disarm, new token, push. While not armed nothing changes (exceptions of A3.2).
    pub fn drop_arm(&mut self, reason: u8, out: &mut Vec<Tx>) {
        if !self.armed() {
            return;
        }
        self.unkey(unkey::ARM_DROPPED, out);
        self.arm.state = 0;
        self.arm.drop = reason;
        self.new_token();
        let (w, t) = (self.arm.word(), self.token);
        self.push(&[(reg::TX_ARM_STATE, w), (reg::TX_ARM_TOKEN, t)], out);
        self.logf(&format!("arm dropped, reason {reason}"));
    }
    fn reset_tx_settings(&mut self) {
        self.mode_class = 0;
        self.keyed_timeout_ms = 500;
        self.session_limits = [0; 4];
        if let Some(s) = self.session.as_mut() {
            s.s.duc = None;
            s.s.wd_general_seen = false;
            if let Some(g) = s.s.general.as_mut() {
                g.raw[58] &= !1;
            }
        }
    }
    /// End of a session by stop, watchdog or link loss (RX settings carried for the same host).
    fn end_session(&mut self, reason: u8, keep_counters: bool, out: &mut Vec<Tx>) {
        let was_armed = self.armed();
        self.drop_arm(reason, out);
        if !was_armed {
            self.unkey(unkey::ARM_DROPPED, out);
        }
        if let Some(s) = self.session.take() {
            self.last_owner = Some(s.owner);
            let mut carry = s.s.clone();
            carry.duc = None;
            carry.wd_general_seen = false;
            if let Some(g) = carry.general.as_mut() {
                g.raw[38] &= !1; // byte 38 bit 0 not carried (B5.4 item 2)
                g.raw[58] &= !1;
            }
            self.carried = Some(carry);
            if !was_armed {
                self.new_token(); // A3.2 exception 3: a new token at every session end
            }
        }
        self.mode_class = 0;
        self.keyed_timeout_ms = 500;
        self.session_limits = [0; 4];
        self.keyer.reset();
        self.counters_continue = keep_counters;
        if !keep_counters {
            self.seq.clear();
        }
        self.setup.clear();
    }

    // ---------- inputs ----------
    pub fn on_datagram(&mut self, now: u64, rx: &Rx) -> Vec<Tx> {
        self.now = now;
        let mut out = Vec::new();
        self.run_keyer(now, &mut out);
        let ip = *rx.src.ip();
        match rx.local_port {
            port::MGMT => {
                let mut m = std::mem::replace(&mut self.mgmt, Mgmt::new(now));
                m.on_request(self, rx, &mut out);
                self.mgmt = m;
                return out;
            }
            port::DISCOVERY => self.on_1024(rx, &mut out),
            _ => {
                if rx.local_port == port::HIGH_PRIORITY {
                    // 0058: every High Priority packet read from port 1027, from any source,
                    // before the subnet and owner checks
                    self.count("hp_rx");
                }
                if !self.own_subnet(ip) {
                    self.count("drop.outside_subnet");
                    return out;
                }
                match rx.local_port {
                    port::DDC_SPECIFIC => self.on_cc(rx, CcKind::Ddc, &mut out),
                    port::DUC_SPECIFIC => self.on_cc(rx, CcKind::Duc, &mut out),
                    port::HIGH_PRIORITY => self.on_cc(rx, CcKind::Hp, &mut out),
                    port::DUC_IQ => self.on_duc_iq(rx, &mut out),
                    port::REGMAP => self.on_regmap(rx, &mut out),
                    _ => self.count("drop.closed_port"),
                }
            }
        }
        out
    }

    fn on_1024(&mut self, rx: &Rx, out: &mut Vec<Tx>) {
        let d = rx.data;
        if d.len() < 5 {
            self.count("drop.1024.short");
            return;
        }
        match d[4] {
            0x02 => {
                if rx.src.port() == port::DISCOVERY {
                    self.count("drop.1024.from_1024");
                    return;
                }
                if !self.lim_disc.allow(&rx.src, self.now) {
                    self.count("drop.discovery.rate");
                    return;
                }
                if let Some(s) = &self.session {
                    if s.owner == rx.src && self.armed() {
                        self.drop_arm(drop::REDISCOVERY, out);
                    }
                }
                let reply = self.discovery_reply(rx.src);
                out.push(Tx { src_port: port::DISCOVERY, dst: rx.src, data: reply });
            }
            0x03 => self.on_set_ip(rx, out),
            0x00 => {
                if !self.own_subnet(*rx.src.ip()) {
                    self.count("drop.outside_subnet");
                    return;
                }
                self.on_cc(rx, CcKind::General, out)
            }
            0x04 | 0x05 => self.count("drop.1024.erase_program"),
            _ => self.count("drop.1024.unknown"),
        }
    }

    pub fn discovery_reply(&self, from: Host) -> Vec<u8> {
        let mut r = vec![0u8; 60];
        let running_other = self.session.as_ref().map_or(false, |s| s.owner != from);
        r[4] = if running_other { 0x03 } else { 0x02 };
        r[5..11].copy_from_slice(&self.cfg.mac);
        r[11] = BOARD_TYPE;
        r[12] = P2_VERSION;
        let (maj, min, _) = self.cfg.fw;
        r[13] = (maj as u32 * 10 + min as u32).min(255) as u8;
        r[20] = 1;
        r[21] = 1;
        r[22] = 0;
        r[23] = self.cfg.beta as u8;
        r
    }

    fn on_set_ip(&mut self, rx: &Rx, out: &mut Vec<Tx>) {
        let d = rx.data;
        if d.len() < 15 {
            self.count("drop.setip.short");
            return;
        }
        if d[5..11] != self.cfg.mac {
            self.count("drop.setip.other_mac");
            return;
        }
        let limited = rx.to_ip == Some(Ipv4Addr::BROADCAST);
        if !limited && !self.own_subnet(*rx.src.ip()) {
            self.count("drop.setip.outside_subnet");
            return;
        }
        let new_ip = Ipv4Addr::new(d[11], d[12], d[13], d[14]);
        self.logf(&format!("Set IP {new_ip} from {}", rx.src));
        // The model answers with a discovery reply and records the address; applying it to the
        // sockets is the shell's job (it restarts on the new address when asked).
        let reply = self.discovery_reply(rx.src);
        out.push(Tx { src_port: port::DISCOVERY, dst: rx.src, data: reply });
        self.stats.insert("setip.pending".into(), u32::from(new_ip) as u64);
    }

    fn on_cc(&mut self, rx: &Rx, kind: CcKind, out: &mut Vec<Tx>) {
        let d = rx.data;
        let min = match kind {
            CcKind::General => 60,
            CcKind::Ddc => 1444,
            CcKind::Duc => 60,
            CcKind::Hp => 1444,
        };
        if d.len() < min {
            self.count(&format!("drop.short.{}", kind.name()));
            return;
        }
        let d = &d[..min];
        if self.link_mbit < 100 && kind == CcKind::Hp && d[4] & 1 != 0 {
            if self.lim_link.allow(&rx.src, self.now) {
                let data = self.reg_packet(&[(reg::LINK_STATUS, self.link_status()), (reg::STREAM_STATUS, self.stream_status())]);
                out.push(Tx { src_port: port::REGMAP, dst: rx.src, data });
            }
            return;
        }
        if self.update_in_progress {
            // A1.8 (0022 §10): no session during an update; a run request is answered with
            // STREAM_STATUS reason 10, at most once per 100 ms (an answer to a non-owner, 0051)
            if kind == CcKind::Hp && d[4] & 1 != 0 && self.lim_update.allow(&rx.src, self.now) {
                let data = self.reg_packet_nonowner(&[(reg::STREAM_STATUS, self.stream_status())]);
                out.push(Tx { src_port: port::REGMAP, dst: rx.src, data });
            }
            return;
        }
        let owner = self.session.as_ref().map(|s| s.owner);
        match owner {
            None => {
                if kind == CcKind::Hp && d[4] & 1 != 0 {
                    self.start_session(rx.src, out);
                    self.apply_cc(kind, d, out);
                } else if kind != CcKind::Hp {
                    let mut st = self.setup.get(&rx.src).cloned().unwrap_or_default();
                    st.apply_kept(kind, d);
                    self.setup.put(rx.src, st);
                }
            }
            Some(o) if o == rx.src => {
                self.session.as_mut().unwrap().last_cc = self.now;
                self.apply_cc(kind, d, out);
            }
            Some(_) => {
                self.takeover(rx.src, out);
                self.apply_cc(kind, d, out);
            }
        }
    }

    fn start_session(&mut self, host: Host, out: &mut Vec<Tx>) {
        let same = self.last_owner == Some(host);
        let mut s = if same { self.carried.take().unwrap_or_default() } else { Settings::default() };
        let kept: Vec<(Host, Settings)> = self.setup.take_all();
        for (h, st) in kept {
            if h == host {
                s.merge(st);
            }
        }
        if !(same && self.counters_continue) {
            self.seq.clear();
        }
        self.carried = None;
        let had_general = s.general.is_some();
        self.session = Some(Session {
            owner: host,
            last_cc: self.now,
            s,
            next_status: self.now,
            next_ddc: self.now,
            record_sent: false,
            port_not_honoured: false,
            arm_confirmed_at: None,
            keyed_at: None,
            last_hp: self.now,
        });
        self.last_owner = Some(host);
        self.check_ports();
        let t = self.token;
        self.push(&[(reg::TX_ARM_TOKEN, t)], out);
        if had_general {
            self.send_unasked_record(out);
        }
        self.logf(&format!("session start {host}"));
    }

    fn takeover(&mut self, newhost: Host, out: &mut Vec<Tx>) {
        let old = self.session.as_ref().unwrap().owner;
        let old_dest = self.reg_dest().unwrap();
        self.unkey(unkey::ARM_DROPPED, out);
        // A2.4 step 1 applies in full, armed or not (0022 §79).
        self.arm.state = 0;
        self.arm.drop = drop::TAKEOVER;
        self.new_token();
        let pairs = [(reg::TX_ARM_STATE, self.arm.word()), (reg::TX_ARM_TOKEN, self.token)];
        for i in 0..3u64 {
            let data = self.reg_packet(&pairs);
            let t = Tx { src_port: port::REGMAP, dst: old_dest, data };
            if i == 0 {
                out.push(t);
            } else {
                self.scheduled.push((self.now + i * 10 * MS, t));
            }
        }
        // the repeated notice continues the old session's counter towards the old owner (0051)
        self.old_seq = *self.seq.get(&port::REGMAP).unwrap_or(&0);
        let notice = self.reg_packet_noseq(&pairs);
        self.old_owner_notice = Some((old, old_dest.port(), self.now + 5 * S, 0, notice));
        self.count("takeover");
        self.logf(&format!("takeover {old} -> {newhost}"));
        // TX settings back (A3.6a); RX settings kept (A2.4 step 4)
        self.reset_tx_settings();
        self.keyer.reset();
        let s = self.session.as_mut().unwrap();
        s.owner = newhost;
        s.last_cc = self.now;
        s.record_sent = false;
        s.arm_confirmed_at = None;
        s.keyed_at = None;
        s.s.hp.ptt = false;
        s.s.hp.last_ptt1_at = None;
        s.s.hp.cwx_req = false;
        s.s.hp.last_cwx1_at = None;
        s.s.hp.cwx_ignored = false;
        if let Some(g) = s.s.general.as_mut() {
            g.raw[31] = 0;
            g.raw[32] = 0;
        }
        self.seq.clear();
        self.last_owner = Some(newhost);
    }

    fn reg_packet_noseq(&self, pairs: &[(u16, u32)]) -> Vec<u8> {
        let mut v = vec![0u8; 4];
        for (a, d) in pairs {
            v.extend_from_slice(&a.to_be_bytes());
            v.extend_from_slice(&d.to_be_bytes());
        }
        v
    }

    fn check_ports(&mut self) {
        // B2.3: bytes 5–12, 15–22, 29–30: zero or default honoured; else not honoured, bit 17.
        let defaults: [(usize, u16); 9] = [(5, 1025), (7, 1026), (9, 1027), (11, 1025), (15, 1029),
                                           (17, 1035), (19, 1026), (21, 1027), (29, 7250)];
        let Some(s) = self.session.as_ref() else { return };
        let Some(g) = s.s.general.as_ref() else { return };
        let mut bad = Vec::new();
        for (i, def) in defaults {
            let v = be16(&g.raw, i);
            if v != 0 && v != def {
                bad.push((i, v));
            }
        }
        if !bad.is_empty() {
            self.session.as_mut().unwrap().port_not_honoured = true;
            for (i, v) in bad {
                let what = if v == port::MGMT { " (management port: conflict)" } else { "" };
                self.logf(&format!("General bytes {i}-{}: port {v} not honoured{what}", i + 1));
            }
        }
    }

    fn send_unasked_record(&mut self, out: &mut Vec<Tx>) {
        let Some(dst) = self.reg_dest() else { return };
        if let Some(&t) = self.last_unasked.get(&dst) {
            if self.now.saturating_sub(t) < S {
                self.count("record.unasked.suppressed");
                return;
            }
        }
        self.last_unasked.insert(dst, self.now);
        let mut pairs = Vec::with_capacity(cap::LEN);
        for (i, v) in self.record.iter().enumerate() {
            pairs.push((i as u16, *v));
        }
        let data = self.reg_packet(&pairs);
        out.push(Tx { src_port: port::REGMAP, dst, data });
        if let Some(s) = self.session.as_mut() {
            s.record_sent = true;
        }
    }

    fn apply_cc(&mut self, kind: CcKind, d: &[u8], out: &mut Vec<Tx>) {
        let before_ddc = self.ddc0_reason();
        match kind {
            CcKind::General => {
                let first = !self.session.as_ref().unwrap().record_sent;
                let s = self.session.as_mut().unwrap();
                let g = General { raw: d.to_vec() };
                if g.wd_bit() {
                    s.s.wd_general_seen = true;
                }
                let pa_was = s.s.general.as_ref().map_or(false, |g| g.pa_enable());
                s.s.general = Some(g);
                self.check_ports();
                if first {
                    self.send_unasked_record(out);
                }
                let pa_now = self.session.as_ref().unwrap().s.general.as_ref().unwrap().pa_enable();
                if pa_was && !pa_now {
                    self.unkey(unkey::PA_DISABLED, out);
                }
                let w = self.watchdog_state();
                self.push(&[(reg::WATCHDOG_STATE, w)], out);
            }
            CcKind::Ddc => self.session.as_mut().unwrap().s.ddc = Some(d.to_vec()),
            CcKind::Duc => {
                self.session.as_mut().unwrap().s.duc = Some(d.to_vec());
                // B10.5a item 1 (0046): once per DUC-specific packet that carries S > 60
                if d.len() > 9 && d[9] > 60 {
                    self.count("cw_speed_clamped");
                    self.logf(&format!("CW speed {} used as 60", d[9]));
                }
                let (set, _) = self.keyer_settings();
                self.keyer.set = set;
            }
            CcKind::Hp => self.apply_hp(d, out),
        }
        if self.ddc0_reason() != before_ddc {
            let w = self.stream_status();
            self.push(&[(reg::STREAM_STATUS, w)], out);
        }
    }

    fn apply_hp(&mut self, d: &[u8], out: &mut Vec<Tx>) {
        let run = d[4] & 1 != 0;
        if !run {
            self.end_session(drop::RUN_CLEARED, false, out);
            return;
        }
        let ptt = d[4] & 0b10 != 0;
        let now = self.now;
        if let Some(l) = self.keylog.as_mut() {
            l.hp.push((now, be32(d, 0), d[4], d[5]));
        }
        let s = self.session.as_mut().unwrap();
        s.last_hp = now;
        let was = s.s.hp.ptt;
        s.s.hp.ptt = ptt;
        s.s.hp.ddc0_phase = be32(d, 9);
        s.s.hp.duc0_phase = be32(d, 329);
        s.s.hp.drive = d[345];
        let t_ref = s.s.hp.last_ptt1_at.or(s.arm_confirmed_at);
        if ptt {
            s.s.hp.last_ptt1_at = Some(now);
        }
        if ptt && !was {
            // B11.6a unnumbered rule: t_edge - t_ref >= 10 ms (exact on the model)
            let ok_10ms = t_ref.map_or(true, |t| now.saturating_sub(t) >= 10 * MS);
            if ok_10ms {
                self.try_key(out);
            }
        } else if !ptt && was && self.tx.keyed && self.tx.source == 1 {
            self.unkey(unkey::RELEASED, out);
        }
        self.cwx_input(d[5], out);
    }

    // ---------- CWX keyer (B10.5a; 0046) ----------
    fn log_key(&mut self, t: u64, on: bool) {
        if let Some(l) = self.keylog.as_mut() {
            l.key.push((t, on));
        }
    }

    /// Keyer settings from the owner's DUC-specific packet (byte 5 bits [3], [5], [6]; bytes
    /// 9, 10). Break-in is byte 5 bit [7].
    fn keyer_settings(&self) -> (keyer::Settings, bool) {
        let d = self.session.as_ref().and_then(|s| s.s.duc.as_ref());
        let b = |i: usize| d.map_or(0, |d| d.get(i).copied().unwrap_or(0));
        let b5 = b(5);
        (keyer::Settings { iambic: b5 & 0x08 != 0, mode_b: b5 & 0x20 != 0, strict: b5 & 0x40 != 0,
                           speed: b(9), weight: b(10) }, b5 & 0x80 != 0)
    }

    fn cwx_input(&mut self, b5: u8, out: &mut Vec<Tx>) {
        let (set, _) = self.keyer_settings();
        self.keyer.set = set;
        let cwx = b5 & 1 != 0;
        // bit [0] clear: bits [1], [2] ignored; speed 0: CWX keys nothing (Table B10-1)
        let on = cwx && set.speed != 0;
        let (mut dot, mut dash) = (on && b5 & 2 != 0, on && b5 & 4 != 0);
        let now = self.now;
        // B11.6a unnumbered rule for the CWX request: a 0->1 edge counts only if t_edge - t_ref
        // >= 10 ms (t_ref: the latest earlier packet with the request at 1, else the arm
        // confirmation); a failed edge is ignored, the request counts as 1 for the next edge
        let req = b5 & 1 != 0 && b5 & 6 != 0;
        if let Some(s) = self.session.as_mut() {
            let hp = &mut s.s.hp;
            let t_ref = hp.last_cwx1_at.or(s.arm_confirmed_at);
            if req && !hp.cwx_req {
                hp.cwx_ignored = !t_ref.map_or(true, |t| now.saturating_sub(t) >= 10 * MS);
            }
            if !req {
                hp.cwx_ignored = false;
            }
            if req {
                hp.last_cwx1_at = Some(now);
            }
            hp.cwx_req = req;
            if hp.cwx_ignored {
                dot = false;
                dash = false;
            }
        }
        let mut ko = Vec::new();
        self.keyer.input(now, dot, dash, &mut ko);
        self.apply_keyer(ko, out);
    }

    fn run_keyer(&mut self, now: u64, out: &mut Vec<Tx>) {
        if self.keyer.next_due().map_or(true, |t| t > now) {
            return;
        }
        let mut ko = Vec::new();
        self.keyer.advance(now, &mut ko);
        self.apply_keyer(ko, out);
    }

    /// The conditions of A3.1a other than the arm, for an ATU tune start (NOT_KEYABLE).
    fn atu_keyable(&self) -> bool {
        let Some(s) = self.session.as_ref() else { return false };
        let freq = phase_to_hz(s.s.hp.duc0_phase).round() as u32;
        let inside = cap::TX_BANDS.iter().any(|(lo, hi)| freq >= *lo && freq <= *hi);
        let pa = s.s.general.as_ref().map_or(false, |g| g.pa_enable());
        inside && pa && self.mode_class != 0 && self.tx_calibrated() && !self.update_in_progress
    }

    /// The conditions of A3.1a for a keyer element, as far as modelled (as try_key, with
    /// break-in set: CWX keys by itself, B10.5b).
    fn cwx_may_key(&self) -> bool {
        let Some(s) = self.session.as_ref() else { return false };
        let freq = phase_to_hz(s.s.hp.duc0_phase).round() as u32;
        let inside = cap::TX_BANDS.iter().any(|(lo, hi)| freq >= *lo && freq <= *hi);
        let pa = s.s.general.as_ref().map_or(false, |g| g.pa_enable());
        let enc_ok = s.s.general.as_ref().map_or(true, |g| g.b(37) & 0b1000 != 0);
        let eer = s.s.duc.as_ref().map_or(false, |d| d[5] & 1 != 0);
        let (_, break_in) = self.keyer_settings();
        self.tx_calibrated() && self.arm.state == 1 && s.arm_confirmed_at.is_some() && inside && pa
            && enc_ok && !eer && self.mode_class != 0 && !self.update_in_progress && break_in
            && !(self.tx.keyed && self.tx.source != SRC_CWX)
    }

    fn apply_keyer(&mut self, ko: Vec<keyer::Out>, out: &mut Vec<Tx>) {
        for o in ko {
            match o {
                keyer::Out::On(t) => {
                    if !self.cwx_may_key() {
                        self.keyer.stop(t, &mut Vec::new());
                        continue;
                    }
                    self.now = self.now.max(t);
                    self.tx.keyed = true;
                    self.tx.source = SRC_CWX;
                    self.hw.tx_levels = true;
                    self.hw.chain_a_bit7_ever = true;
                    self.hw.bias_nonzero_ever = true;
                    self.log_key(t, true);
                    let w = self.tx_state_word();
                    self.push(&[(reg::TX_STATE, w)], out);
                }
                keyer::Out::Off(t, why) => {
                    if !(self.tx.keyed && self.tx.source == SRC_CWX) {
                        continue;
                    }
                    if why == keyer::Off::Cap {
                        self.logf("CWX paddle bit held over 1 s: unkey (B10.5a)");
                        let saved = self.now;
                        self.now = t;
                        self.unkey(unkey::CW_CAP, out);
                        self.now = saved.max(t);
                        continue;
                    }
                    // the end of a mark: PA off, the keyer between elements (A3.4d)
                    self.tx.keyed = false;
                    self.hw.tx_levels = false;
                    self.log_key(t, false);
                    let w = self.tx_state_word();
                    self.push(&[(reg::TX_STATE, w)], out);
                }
                keyer::Out::Released(_) => {
                    if self.tx.source == SRC_CWX && !self.tx.keyed {
                        self.tx.source = 0;
                        self.tx.unkey = unkey::RELEASED;
                    }
                }
            }
        }
    }

    /// The key rule of A3.1a, as far as modelled.
    fn try_key(&mut self, out: &mut Vec<Tx>) {
        let Some(s) = self.session.as_ref() else { return };
        let freq = phase_to_hz(s.s.hp.duc0_phase).round() as u32; // within the phase-word resolution
        let inside = cap::TX_BANDS.iter().any(|(lo, hi)| freq >= *lo && freq <= *hi);
        self.tx.outside = !inside;
        let pa = s.s.general.as_ref().map_or(false, |g| g.pa_enable());
        let enc_ok = s.s.general.as_ref().map_or(true, |g| g.b(37) & 0b1000 != 0);
        let eer = s.s.duc.as_ref().map_or(false, |d| d[5] & 1 != 0);
        let ok = self.tx_calibrated() && self.arm.state == 1 && s.arm_confirmed_at.is_some()
            && inside && pa && enc_ok && !eer && self.mode_class != 0 && !self.tx.keyed
            && !self.update_in_progress;
        if ok {
            self.tx.keyed = true;
            self.tx.source = 1;
            self.hw.tx_levels = true;
            self.hw.chain_a_bit7_ever = true;
            self.hw.bias_nonzero_ever = true;
            self.session.as_mut().unwrap().keyed_at = Some(self.now);
        }
        let w = self.tx_state_word();
        self.push(&[(reg::TX_STATE, w)], out);
    }

    fn on_duc_iq(&mut self, rx: &Rx, out: &mut Vec<Tx>) {
        let owner = self.session.as_ref().map(|s| s.owner);
        if owner != Some(rx.src) {
            self.count("drop.duc_iq.not_owner");
            self.maybe_notice(rx.src, out);
            return;
        }
        if rx.data.len() < 1444 {
            self.count("drop.short.duc_iq");
            return;
        }
        if !self.tx_calibrated() {
            // B10.7: while TX is not calibrated the owner's DUC I&Q is discarded and counted.
            self.count("drop.duc_iq.tx_not_calibrated");
        }
    }

    fn maybe_notice(&mut self, src: Host, out: &mut Vec<Tx>) {
        if let Some((h, port, until, last, pkt)) = self.old_owner_notice.clone() {
            if h == src && self.now < until && (last == 0 || self.now - last >= 100 * MS) {
                let mut data = pkt.clone();
                let seq = self.old_seq;
                self.old_seq = seq.wrapping_add(1);
                data[..4].copy_from_slice(&seq.to_be_bytes());
                out.push(Tx { src_port: port::REGMAP, dst: SocketAddrV4::new(*h.ip(), port), data });
                self.old_owner_notice = Some((h, port, until, self.now, pkt));
            }
        }
    }

    // ---------- register map ----------
    fn on_regmap(&mut self, rx: &Rx, out: &mut Vec<Tx>) {
        let d = rx.data;
        if d.len() < 10 {
            self.count("drop.short.regmap");
            return;
        }
        let owner = self.session.as_ref().map(|s| s.owner);
        let is_owner = owner == Some(rx.src);
        if !is_owner && rx.src.port() == port::REGMAP {
            self.count("drop.regmap.from_7250");
            return;
        }
        if !is_owner {
            self.maybe_notice(rx.src, out);
        }
        let n = (d.len() - 4) / 6;
        let mut answer: Vec<(u16, u32)> = Vec::new();
        let mut cap_req = false;
        let mut not_owner_answer = false;
        for i in 0..n {
            let a = be16(d, 4 + 6 * i);
            let v = be32(d, 6 + 6 * i);
            if a == reg::PADDING || (a == 0 && v == 0) {
                continue;
            }
            match self.write(a, v, is_owner, out) {
                W::Answer(pairs) => answer.extend(pairs),
                W::Record => {
                    cap_req = true;
                    answer.push((reg::CAP_REQUEST, v));
                }
                W::Reject(val, reason) => {
                    if reason == wr::NOT_OWNER {
                        not_owner_answer = true;
                    }
                    self.write_rejects = self.write_rejects.wrapping_add(1);
                    answer.push((a, val));
                    answer.push((reg::WRITE_STATUS, (a as u32) << 16 | (self.write_rejects as u32) << 8 | reason as u32));
                }
            }
        }
        if answer.is_empty() {
            return;
        }
        let dst = if is_owner { self.reg_dest().unwrap() } else { rx.src };
        if !is_owner {
            if cap_req {
                if !self.lim_cap.allow(&rx.src, self.now) {
                    self.count("drop.cap_request.rate");
                    return;
                }
            } else if not_owner_answer && !self.lim_notowner.allow(&rx.src, self.now) {
                self.count("drop.not_owner.rate");
                return;
            }
        }
        if cap_req {
            for (i, v) in self.record.clone().iter().enumerate() {
                answer.push((i as u16, *v));
            }
        }
        for chunk in answer.chunks(240) {
            let data = if is_owner { self.reg_packet(chunk) } else { self.reg_packet_nonowner(chunk) };
            out.push(Tx { src_port: port::REGMAP, dst, data });
        }
        if self.link_mbit < 100 && cap_req && self.lim_link.allow(&rx.src, self.now) {
            let pairs = [(reg::LINK_STATUS, self.link_status()), (reg::STREAM_STATUS, self.stream_status())];
            let data = if is_owner { self.reg_packet(&pairs) } else { self.reg_packet_nonowner(&pairs) };
            out.push(Tx { src_port: port::REGMAP, dst: rx.src, data });
        }
    }

    pub fn group_present(&self, a: u16) -> bool {
        let g = (a >> 8) as u32;
        g < 32 && self.record[0x53] & (1 << g) != 0
    }

    /// Current value of a readable register, None if the address is not a register.
    pub fn read(&self, a: u16) -> Option<u32> {
        if (a as usize) < cap::LEN {
            return Some(self.record[a as usize]);
        }
        Some(match a {
            reg::CAP_REQUEST => 0,
            reg::STREAM_STATUS => self.stream_status(),
            reg::LINK_STATUS => self.link_status(),
            reg::WRITE_STATUS => 0,
            reg::TX_ARM_TOKEN => self.token,
            reg::TX_ARM => 0,
            reg::TX_ARM_STATE => self.arm.word(),
            reg::TX_STATE => self.tx_state_word(),
            reg::TX_MODE_CLASS => self.mode_class as u32,
            reg::KEYED_TIMEOUT => self.keyed_timeout_ms,
            reg::WATCHDOG_STATE => self.watchdog_state(),
            reg::SAFETY_STATE | reg::FAULT_CODE | reg::FAULT_FLAGS | reg::FAULT_CLASS => 0,
            reg::TRIP_COUNT | reg::FAULT_CLEAR | reg::LIMIT_STATUS | reg::LIMIT_INHIBIT => 0,
            reg::LIMIT_TX_TIME..=reg::LIMIT_PA_TEMP => self.limit_in_force((a - reg::LIMIT_TX_TIME) as usize),
            // group 3: the HIS 85 sensors exist; values stay invalid (SENSOR_VALID 0) until the
            // calibration set holds P-SEN-1 (record telemetry, 0039). No separate supply current
            // input (telemetry bit 4 = 0), so 0x0305 has no register.
            0x0300..=0x0304 | 0x0306 | reg::SENSOR_VALID => 0,
            reg::ATU_TUNE => 0,
            reg::ATU_STATE => self.atu_state(),
            reg::ANT_STATE => 0, // unknown until P-ANT-1 (Annex P)
            _ => return None,
        })
    }

    /// 5 unknown while the ATU latches are unwritten (P-ATU-1 not set; Annex P, 0032).
    pub fn atu_state(&self) -> u32 {
        if self.has_param("P-ATU-1") { self.atu.state } else { 5 }
    }

    fn limit_in_force(&self, i: usize) -> u32 {
        let p = self.persistent_limits[i];
        let s = self.session_limits[i];
        if s == 0 { p } else { s.min(p) }
    }

    fn write(&mut self, a: u16, v: u32, is_owner: bool, out: &mut Vec<Tx>) -> W {
        let present = self.group_present(a);
        let known = present && self.read(a).is_some();
        let cur = if known { self.read(a).unwrap() } else { 0 };
        if self.update_in_progress && a != reg::CAP_REQUEST {
            return W::Reject(cur, wr::BUSY);
        }
        if a == reg::CAP_REQUEST {
            return if v == 1 { W::Record } else { W::Reject(0, wr::BAD_VALUE) };
        }
        if !is_owner {
            return W::Reject(cur, wr::NOT_OWNER);
        }
        if !known {
            return W::Reject(0, wr::UNKNOWN_ADDRESS);
        }
        match a {
            reg::TX_ARM => W::Answer(self.write_arm(v, out)),
            reg::TX_MODE_CLASS => {
                if !(1..=4).contains(&v) {
                    return W::Reject(cur, wr::BAD_VALUE);
                }
                if v as u8 != self.mode_class && self.tx.keyed {
                    self.unkey(unkey::MODE_CLASS, out);
                }
                self.mode_class = v as u8;
                W::Answer(vec![(a, v)])
            }
            reg::KEYED_TIMEOUT => {
                if !(100..=500).contains(&v) {
                    return W::Reject(cur, wr::BAD_VALUE);
                }
                self.keyed_timeout_ms = v;
                W::Answer(vec![(a, v)])
            }
            reg::FAULT_CLEAR => {
                // No fault is modelled: FAULT_CODE is 0. Writing the current code (0) is
                // "not clearable" (nothing to clear); any other value is BAD_VALUE (SF-09).
                if v == 0 { W::Reject(cur, wr::NOT_CLEARABLE) } else { W::Reject(cur, wr::BAD_VALUE) }
            }
            reg::LIMIT_TX_TIME..=reg::LIMIT_PA_TEMP => {
                let i = (a - reg::LIMIT_TX_TIME) as usize;
                // A1.12 item 4: S2 and T3 (P-PWR-2) are refused until the set holds them.
                if (a == reg::LIMIT_SWR || a == reg::LIMIT_PA_TEMP) && !self.has_param("P-PWR-2") {
                    return W::Reject(cur, wr::NOT_CALIBRATED);
                }
                if v == 0 {
                    self.session_limits[i] = 0;
                } else if v > self.persistent_limits[i] {
                    return W::Reject(cur, wr::LOOSER_LIMIT);
                } else {
                    self.session_limits[i] = v;
                }
                W::Answer(vec![(a, self.limit_in_force(i))])
            }
            reg::ATU_TUNE => match v {
                // A3.9b item 1 order: BUSY, NOT_OWNER, BAD_VALUE, NOT_CALIBRATED, NOT_ARMED, no-op,
                // KEYED, NOT_KEYABLE. The tune cycle itself is not modelled: once calibrated and
                // armed, a start is answered as the no-op with ATU_STATE (model gap).
                // 0 is always accepted and a no-op with no cycle running; its answer is the
                // pair of B8.4 only (ATU_STATE comes with a start's no-op only, item 1).
                0 => {
                    if self.tx.keyed && self.tx.source == SRC_ATU {
                        // an abort: reason 1, ATU_STATE 4 (0047)
                        self.unkey(unkey::RELEASED, out);
                    }
                    W::Answer(vec![(a, 0)])
                }
                1 if !self.has_param("P-ATU-1") => W::Reject(0, wr::NOT_CALIBRATED),
                1 if !self.armed() => W::Reject(0, wr::NOT_ARMED),
                1 if self.atu.cycle.is_some() || self.atu.last_end.map_or(false, |t| self.now < t + 2 * S) => {
                    // the no-op (cycle running or within 2 s of the end of the previous one)
                    let run = self.atu.cycle.is_some() as u32;
                    W::Answer(vec![(a, run), (reg::ATU_STATE, self.atu_state())])
                }
                1 if self.tx.keyed => W::Reject(0, wr::KEYED),
                1 if !self.atu_keyable() => W::Reject(0, wr::NOT_KEYABLE),
                1 => {
                    let now = self.now;
                    let end_at = self.atu.sim_result.map(|_| now + self.atu.sim_after_us);
                    self.atu.cycle = Some((now, end_at, self.atu.sim_result == Some(true)));
                    self.atu.state = 2;
                    self.tx.keyed = true;
                    self.tx.source = SRC_ATU;
                    self.hw.tx_levels = true;
                    self.hw.chain_a_bit7_ever = true;
                    self.hw.bias_nonzero_ever = true;
                    let w = self.tx_state_word();
                    self.push(&[(reg::TX_STATE, w)], out);
                    W::Answer(vec![(a, 1), (reg::ATU_STATE, 2)])
                }
                _ => W::Reject(cur, wr::BAD_VALUE),
            },
            _ => W::Reject(cur, wr::READ_ONLY),
        }
    }

    /// B10.2. Returns the answer pairs (TX_ARM_STATE, and the token on a change).
    fn write_arm(&mut self, v: u32, out: &mut Vec<Tx>) -> Vec<(u16, u32)> {
        if v == 0 {
            if self.armed() {
                self.drop_arm(drop::HOST_DISARM, out);
            }
            return vec![(reg::TX_ARM_STATE, self.arm.word())];
        }
        if v != self.token ^ ARM_MAGIC {
            self.arm.reject = arm::BAD_TOKEN;
            return vec![(reg::TX_ARM_STATE, self.arm.word())];
        }
        if self.armed() {
            return vec![(reg::TX_ARM_STATE, self.arm.word())]; // no-op, same generation
        }
        let s = self.session.as_ref().unwrap();
        let wd_written = s.s.general.as_ref().map_or(false, |g| g.wd_bit());
        // B10.2 item 3 order: BAD_TOKEN (above), NOT_RUNNING, WATCHDOG_DISABLED, IN_FAULT,
        // INHIBITED, PTT_ACTIVE, LEVEL_NOT_ACTIVE, MAINTENANCE, NO_MODE_CLASS,
        // TX_NOT_IMPLEMENTED, NOT_CALIBRATED. NOT_RUNNING is not set over P2 (Table B8-3);
        // fault, inhibit and level are not modelled (always pass); TX is implemented (0036).
        let reason = if !(wd_written && s.s.wd_general_seen) {
            arm::WATCHDOG_DISABLED
        } else if s.s.hp.ptt {
            arm::PTT_ACTIVE
        } else if self.update_in_progress || self.fw.trial.is_some() {
            // an update, and an unconfirmed trial boot (A1.8, 0022 §10)
            arm::MAINTENANCE
        } else if self.mode_class == 0 {
            arm::NO_MODE_CLASS
        } else if !self.tx_calibrated() {
            arm::NOT_CALIBRATED
        } else {
            arm::NONE
        };
        if reason != arm::NONE {
            self.arm.reject = reason;
            return vec![(reg::TX_ARM_STATE, self.arm.word())];
        }
        self.arm.state = 1;
        self.arm.reject = 0;
        self.arm.gen = self.arm.gen.wrapping_add(1);
        self.session.as_mut().unwrap().arm_confirmed_at = Some(self.now);
        self.logf("armed");
        vec![(reg::TX_ARM_STATE, self.arm.word())]
    }

    // ---------- time ----------
    pub fn tick(&mut self, now: u64) -> Vec<Tx> {
        self.now = now;
        let mut out = Vec::new();
        self.confirm_trial();
        self.run_keyer(now, &mut out);
        let mut due: Vec<Tx> = Vec::new();
        self.scheduled.retain(|(t, p)| {
            if *t <= now {
                due.push(p.clone());
                false
            } else {
                true
            }
        });
        out.extend(due);
        let mut m = std::mem::replace(&mut self.mgmt, Mgmt::new(now));
        m.tick(self, &mut out);
        self.mgmt = m;
        // ATU tune cycle (A3.9b item 6; 0047): ended by itself, or by the ATU tune time
        if let Some((start, end_at, _)) = self.atu.cycle {
            if self.tx.keyed && self.tx.source == SRC_ATU {
                let limit_at = start + self.atu.tune_time_us;
                if end_at.map_or(false, |t| now >= t && t < limit_at) {
                    self.unkey(unkey::ATU_ENDED, &mut out);
                } else if now >= limit_at {
                    self.tx.limit = limit::ATU_TIME;
                    self.logf("ATU tune time: unkey, tune failed (A3.9b)");
                    self.unkey(unkey::LIMIT, &mut out);
                }
            }
        }
        let Some(s) = self.session.as_ref() else { return out };
        // watchdog (B5.6): standby between 1000 and 1100 ms
        let wd_on = self.watchdog_state() & 2 != 0;
        if wd_on && now.saturating_sub(s.last_cc) > S {
            self.logf("watchdog standby");
            self.end_session(drop::WATCHDOG, true, &mut out);
            return out;
        }
        // keyed timeout (B10.5)
        if self.tx.keyed && now.saturating_sub(s.last_hp) > self.keyed_timeout_ms as u64 * MS {
            self.unkey(unkey::KEYED_TIMEOUT, &mut out);
        }
        let s = self.session.as_ref().unwrap();
        let owner = s.owner;
        // High Priority status: every 50 ms, every 1 ms while keyed (B9.4)
        if now >= s.next_status {
            let period = if self.tx.keyed { MS } else { 50 * MS };
            self.session.as_mut().unwrap().next_status = now + period;
            let mut p = vec![0u8; 60];
            let seq = self.next_seq(port::DDC_SPECIFIC);
            p[..4].copy_from_slice(&seq.to_be_bytes());
            out.push(Tx { src_port: port::DDC_SPECIFIC, dst: owner, data: p });
        }
        // DDC0 I&Q
        if self.ddc0_reason() == 0 {
            let s = self.session.as_ref().unwrap();
            let rate = be16(s.s.ddc.as_ref().unwrap(), 18) as u64;
            let period = 238 * 1_000_000 / (rate * 1000);
            let mut next = s.next_ddc.max(now.saturating_sub(100 * MS));
            let mut n = 0;
            while next <= now && n < 64 {
                let seq = self.next_seq(port::DDC0_IQ);
                out.push(Tx { src_port: port::DDC0_IQ, dst: owner, data: ddc_packet(seq) });
                next += period;
                n += 1;
            }
            self.session.as_mut().unwrap().next_ddc = next;
        } else {
            self.session.as_mut().unwrap().next_ddc = now;
        }
        out
    }

    /// Simulated reset (RESTART, FACTORY_RESET, power cycle, processor watchdog reset).
    pub fn reset(&mut self, cause: &str) {
        let mut out = Vec::new();
        self.drop_arm(drop::MAINTENANCE, &mut out);
        self.boot_choice(cause);
        self.keyer.reset();
        self.atu.cycle = None;
        self.session = None;
        self.carried = None;
        self.last_owner = None;
        self.setup.clear();
        self.seq.clear();
        self.nonowner_seq = 0; // only a reset of the radio resets it (0051)
        self.scheduled.clear();
        self.old_owner_notice = None;
        self.cfg.boot_counter = self.cfg.boot_counter.wrapping_add(1);
        self.token_n = 0;
        self.new_token();
        self.arm = ArmState { drop: drop::RESET, gen: self.arm.gen, ..Default::default() };
        self.tx = TxState::default();
        self.mode_class = 0;
        self.keyed_timeout_ms = 500;
        self.session_limits = [0; 4];
        self.update_in_progress = false;
        self.hw = Hw::default(); // the trace flags count from boot
        self.booted_at = self.now;
        self.resets += 1;
        self.mgmt.on_reset(self.now);
        self.logf(&format!("reset: {cause}"));
    }
}

enum W {
    Answer(Vec<(u16, u32)>),
    Record,
    Reject(u32, u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CcKind {
    General,
    Ddc,
    Duc,
    Hp,
}
impl CcKind {
    fn name(&self) -> &'static str {
        match self {
            CcKind::General => "general",
            CcKind::Ddc => "ddc_specific",
            CcKind::Duc => "duc_specific",
            CcKind::Hp => "high_priority",
        }
    }
}

impl Settings {
    fn apply_kept(&mut self, kind: CcKind, d: &[u8]) {
        match kind {
            CcKind::General => {
                let g = General { raw: d.to_vec() };
                if g.wd_bit() {
                    self.wd_general_seen = true;
                }
                self.general = Some(g);
            }
            CcKind::Ddc => self.ddc = Some(d.to_vec()),
            CcKind::Duc => self.duc = Some(d.to_vec()),
            CcKind::Hp => {}
        }
    }
    fn merge(&mut self, o: Settings) {
        if o.general.is_some() {
            self.general = o.general;
        }
        if o.wd_general_seen {
            self.wd_general_seen = true;
        }
        if o.ddc.is_some() {
            self.ddc = o.ddc;
        }
        if o.duc.is_some() {
            self.duc = o.duc;
        }
    }
}

/// A DDC0 packet (Table B6-1) with 238 zero samples: the model has no receiver.
fn ddc_packet(seq: u32) -> Vec<u8> {
    let mut p = vec![0u8; 1444];
    p[..4].copy_from_slice(&seq.to_be_bytes());
    p[12..14].copy_from_slice(&24u16.to_be_bytes());
    p[14..16].copy_from_slice(&238u16.to_be_bytes());
    p
}
