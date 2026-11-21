//! Management channel, UDP 7251 (spec A1.4–A1.10). Framing, status precedence, challenge and
//! response, write session, stored replies, rate limits and the first-secret window follow the
//! spec. Payload layouts are not fixed by the spec yet ("fixed with the reference model before
//! payload layouts are those of the spec's Annex M (0039).
//!
//! Model gaps (answered with status 56): UPDATE_BEGIN … UPDATE_ACTIVATE, UPDATE_ABORT, ROLLBACK.
//! The network items of SET_CONFIG (C-1) are stored and reported but not applied to the sockets.

use std::net::Ipv4Addr;

use crate::codes::mgmt::{st, *};
use crate::crypto::{crc32, ct_eq, hmac_sha256, sha256};
use crate::radio::{Calibration, Firmware, Image, Radio, Rx, Tx, ALL_PARAMS, LAYOUT, MS, S};
use crate::util::*;

pub const CHALLENGE_LEN: usize = 16; // Annex M
const WINDOW_US: u64 = 300 * S;

#[derive(Debug, Clone)]
struct Stored {
    id: u32,
    cmd: u16,
    hash: [u8; 32],
    reply: Vec<u8>,
    at: u64,
}

pub struct Mgmt {
    pub secret: Option<[u8; 32]>,
    window_start: Option<u64>,
    window_used: bool,
    challenges: Lru<Host, ([u8; CHALLENGE_LEN], u64)>,
    stored: Lru<Host, Stored>,
    pub holder: Option<(Host, u64)>,
    fails: Vec<(Ipv4Addr, u64)>,
    lim_src: Limiter<Host>,
    lim_total: Limiter<u8>,
    lim_ident: Limiter<u8>,
    lim_outside: Limiter<u8>,
    rng: u64,
    pub conf: Conf,
    /// the transfer of an update to an image of this firmware (A1.8)
    pub upd: Option<Upd>,
}

/// One update transfer: the header of Table M-4 and the blocks received so far.
#[derive(Debug, Clone)]
pub struct Upd {
    pub version: (u8, u8, u16),
    pub len: u32,
    pub sha: [u8; 32],
    /// the staging slot: the one not running
    pub slot: u8,
    pub count: u32,
    pub data: Vec<u8>,
    pub received: u32,
    pub verified: bool,
}

pub const BLOCK: usize = 512;
const HW_ID: u16 = 0x00C8;
const PK_MAGIC: u32 = 0x4737_504B; // "G7PK"

fn header(req: &[u8], status: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(18 + payload.len());
    v.extend_from_slice(&MAGIC.to_be_bytes());
    v.push(VERSION.0);
    v.push(VERSION.1);
    v.push(2); // reply
    v.push(0);
    v.extend_from_slice(&req[8..12]);
    v.extend_from_slice(&req[12..14.min(req.len())]);
    while v.len() < 14 {
        v.push(0);
    }
    v.extend_from_slice(&status.to_be_bytes());
    v.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

fn needs_auth(cmd: u16) -> bool {
    matches!(cmd, CHANGE_SECRET | END_WRITE_SESSION | SET_CONFIG | FACTORY_RESET | SWITCH_LEVEL
        | RESTART | CLEAR_LOG | 0x0040..=0x0044 | ROLLBACK | RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY
        | SET_CALIBRATION | CLEAR_CALIBRATION)
}
fn known(cmd: u16) -> bool {
    matches!(cmd, IDENTIFY | GET_STATS | GET_CHALLENGE | SET_FIRST_SECRET | CHANGE_SECRET
        | END_WRITE_SESSION | GET_CONFIG | SET_CONFIG | FACTORY_RESET | SWITCH_LEVEL | RESTART
        | READ_LOG | CLEAR_LOG | 0x0040..=0x0046 | RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY
        | GET_CALIBRATION | SET_CALIBRATION | CLEAR_CALIBRATION)
}
fn is_cal_write(cmd: u16) -> bool {
    cmd == SET_CALIBRATION || cmd == CLEAR_CALIBRATION
}

/// Configuration items of Table M-3 that the model keeps (A1.6). C-3 limits live in
/// `Radio::persistent_limits` (TX time, S2, T3) and `band_limits` here.
#[derive(Debug, Clone)]
pub struct Conf {
    pub addr_mode: u8,
    pub addr: [u8; 4],
    pub mask: [u8; 4],
    pub gw: [u8; 4],
    pub default_level: u8,
    pub region: u8,
    pub band_limits: [u16; 10],
    pub allow: Vec<([u8; 4], u8)>,
}
impl Conf {
    fn factory() -> Self {
        Conf { addr_mode: 0, addr: [0; 4], mask: [0; 4], gw: [0; 4], default_level: 1, region: 0,
               band_limits: band_builtin(), allow: Vec::new() }
    }
}
/// Built-in power per `tx_band`, 0.1 W (Table A3-0; range 10 SIMULATED, P-PWR-2).
fn band_builtin() -> [u16; 10] {
    let mut b = [1000u16; 10];
    b[9] = 900;
    b
}
fn is_update(cmd: u16) -> bool {
    (0x0040..=0x0046).contains(&cmd) && cmd != UPDATE_STATUS
}

impl Mgmt {
    pub fn new(now: u64) -> Self {
        Mgmt {
            secret: None,
            window_start: Some(now),
            window_used: false,
            challenges: Lru::new(4),
            stored: Lru::new(4),
            holder: None,
            fails: Vec::new(),
            lim_src: Limiter::new(S, 20, usize::MAX),
            lim_total: Limiter::new(S, 100, 100),
            lim_ident: Limiter::new(S, 50, 50),
            lim_outside: Limiter::new(S, 10, 10),
            rng: 0x9E37_79B9_7F4A_7C15 ^ now,
            conf: Conf::factory(),
            upd: None,
        }
    }

    pub fn on_reset(&mut self, now: u64) {
        self.challenges.clear();
        self.stored.clear();
        self.holder = None;
        self.window_open(now);
        if self.secret.is_none() && !self.window_used {
            self.window_start = Some(now);
        }
    }

    fn window_open(&mut self, now: u64) -> Option<u64> {
        if self.secret.is_some() || self.window_used {
            return None;
        }
        let st = self.window_start?;
        if now.saturating_sub(st) >= WINDOW_US {
            self.window_used = true;
            return None;
        }
        Some(WINDOW_US - (now - st))
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    pub fn tick(&mut self, _r: &mut Radio, _out: &mut [Tx]) {
        // write-session idle timeout is checked lazily at the next request
    }

    fn holder_valid(&mut self, now: u64) -> Option<Host> {
        if let Some((h, t)) = self.holder {
            if now.saturating_sub(t) < 60 * S {
                return Some(h);
            }
            self.holder = None;
        }
        None
    }

    fn rate_limited(&mut self, ip: Ipv4Addr, now: u64) -> bool {
        self.fails.retain(|(_, t)| now.saturating_sub(*t) < 10 * S);
        self.fails.iter().filter(|(i, _)| *i == ip).count() >= 5
    }

    pub fn on_request(&mut self, r: &mut Radio, rx: &Rx, out: &mut Vec<Tx>) {
        let d = rx.data;
        let now = r.now;
        if d.len() < 4 || be32(d, 0) != MAGIC {
            r.count("mgmt.drop.magic");
            return;
        }
        if d.len() >= 7 && d[6] != 1 {
            r.count("mgmt.drop.not_request");
            return;
        }
        if d.len() < 12 {
            r.count("mgmt.drop.short");
            return;
        }
        let ip = *rx.src.ip();
        let inside = r.own_subnet(ip) && ip != Ipv4Addr::BROADCAST
            && ip != directed_broadcast(r.cfg.ip, r.cfg.netmask);
        let cmd = if d.len() >= 14 { be16(d, 12) } else { 0 };
        // framing and version (A1.4, precedence of Table A1-3)
        let status = if d.len() < 18 {
            Some(st::MALFORMED)
        } else {
            let n = be16(d, 16) as usize;
            let auth = d[7] & 1 != 0;
            let declared = 18 + n + if auth { 32 } else { 0 };
            if declared > 1000 {
                Some(st::TOO_LARGE)
            } else if d.len() != declared {
                Some(st::MALFORMED)
            } else if d[4] != VERSION.0 {
                Some(st::VERSION)
            } else if !known(cmd) {
                Some(st::UNKNOWN_COMMAND)
            } else {
                None
            }
        };
        if !inside && cmd != IDENTIFY {
            r.count("mgmt.drop.outside_subnet");
            return;
        }
        if let Some(s) = status {
            return self.send(r, rx, s, &[], false, cmd, out);
        }
        let n = be16(d, 16) as usize;
        let payload = &d[18..18 + n];
        // 0056: a payload without the size or structure of Annex M is 17, in the framing group,
        // before authentication and every state check
        if payload_malformed(cmd, payload) {
            return self.send(r, rx, st::MALFORMED, &[], false, cmd, out);
        }
        let hash = sha256(payload);
        let id = be32(d, 8);
        // repeated request (A1.4): stored reply, not executed again
        if let Some(s) = self.stored.peek(&rx.src).cloned() {
            if now.saturating_sub(s.at) < 5 * S && s.id == id && s.cmd == cmd && s.hash == hash {
                if self.limits(r, rx, cmd, inside) {
                    out.push(Tx { src_port: crate::codes::port::MGMT, dst: rx.src, data: s.reply });
                }
                r.count("mgmt.repeat");
                return;
            }
        }
        let authed = d[7] & 1 != 0;
        // authentication and rate (32, 33, 34, 36)
        if needs_auth(cmd) || cmd == GET_CHALLENGE {
            if self.rate_limited(ip, now) {
                r.logf(&format!("mgmt: rate-limited {ip}"));
                return self.send(r, rx, st::RATE_LIMITED, &[], false, cmd, out);
            }
        }
        let mut verified = false;
        let mut used_challenge = [0u8; CHALLENGE_LEN];
        if needs_auth(cmd) {
            if !authed || self.secret.is_none() {
                return self.send(r, rx, st::AUTH_REQUIRED, &[], false, cmd, out);
            }
            let ch = self.challenges.remove(&rx.src);
            let ok = match ch {
                Some((c, t)) if now.saturating_sub(t) <= 10 * S => {
                    used_challenge = c;
                    let mut m = c.to_vec();
                    m.extend_from_slice(&d[..18 + n]);
                    ct_eq(&hmac_sha256(self.secret.as_ref().unwrap(), &m), &d[18 + n..])
                }
                _ => false,
            };
            if !ok {
                self.fails.push((ip, now));
                r.logf(&format!("mgmt: authentication failed from {}", rx.src));
                r.count("mgmt.auth_failed");
                return self.send(r, rx, st::AUTH_FAILED, &[], true, cmd, out);
            }
            verified = true;
        }
        if cmd == SET_FIRST_SECRET && self.window_open(now).is_none() {
            return self.send(r, rx, st::WINDOW_CLOSED, &[], true, cmd, out);
        }
        // 35: one writer at a time; RESTART is exempt (A1.10)
        if needs_auth(cmd) && cmd != RESTART {
            match self.holder_valid(now) {
                Some(h) if h != rx.src => return self.send(r, rx, st::BUSY, &[], true, cmd, out),
                _ => self.holder = Some((rx.src, now)),
            }
        }
        let armed_or_keyed = r.armed() || r.tx.keyed;
        // 58, 57, 50
        // during an update the transfer commands go on; a new UPDATE_BEGIN (and ROLLBACK) gets 58,
        // and so does UPDATE_BEGIN during an unconfirmed trial boot (A1.8, 0043)
        if ((cmd == UPDATE_BEGIN || cmd == ROLLBACK) && (r.update_in_progress || r.fw.trial.is_some()))
            || (cmd == SWITCH_LEVEL && r.update_in_progress)
            || ((cmd == RETURN_TO_STOCK || cmd == ENTER_VENDOR_RECOVERY) && r.update_in_progress)
        {
            return self.send(r, rx, st::UPDATE_IN_PROGRESS, &[], true, cmd, out);
        }
        if is_cal_write(cmd) && r.update_in_progress {
            return self.send(r, rx, st::UPDATE_IN_PROGRESS, &[], true, cmd, out);
        }
        if is_cal_write(cmd) && r.session.is_some() {
            return self.send(r, rx, st::SESSION_RUNS, &[], true, cmd, out);
        }
        // 0055: SET_FIRST_SECRET and CHANGE_SECRET write flash too: refused while armed or keyed
        if (is_update(cmd) || cmd == RETURN_TO_STOCK || cmd == ENTER_VENDOR_RECOVERY || is_cal_write(cmd)
            || cmd == SET_FIRST_SECRET || cmd == CHANGE_SECRET)
            && armed_or_keyed
        {
            return self.send(r, rx, st::ARMED, &[], true, cmd, out);
        }
        let (status, reply_payload, after): (u16, Vec<u8>, Option<&str>) = match cmd {
            IDENTIFY => (st::OK, self.identify(r), None),
            GET_STATS => (st::OK, stats_payload(r), None),
            GET_CHALLENGE => {
                let mut c = [0u8; CHALLENGE_LEN];
                for chunk in c.chunks_mut(8) {
                    let v = self.random().to_be_bytes();
                    chunk.copy_from_slice(&v[..chunk.len()]);
                }
                self.challenges.put(rx.src, (c, now));
                (st::OK, c.to_vec(), None)
            }
            SET_FIRST_SECRET => {
                if payload.len() != 32 {
                    (st::MALFORMED, vec![], None)
                } else {
                    let mut s = [0u8; 32];
                    s.copy_from_slice(payload);
                    self.secret = Some(s);
                    self.window_used = true;
                    r.logf(&format!("first secret set by {}", rx.src));
                    (st::OK, vec![], None)
                }
            }
            CHANGE_SECRET => {
                if payload.len() != 32 {
                    (st::MALFORMED, vec![], None)
                } else {
                    let mut m = b"G7NS".to_vec();
                    m.extend_from_slice(&used_challenge);
                    let mask = hmac_sha256(self.secret.as_ref().unwrap(), &m);
                    let mut s = [0u8; 32];
                    for i in 0..32 {
                        s[i] = payload[i] ^ mask[i];
                    }
                    self.secret = Some(s);
                    r.logf(&format!("secret changed by {}", rx.src));
                    (st::OK, vec![], None)
                }
            }
            END_WRITE_SESSION => {
                self.holder = None;
                (st::OK, vec![], None)
            }
            SWITCH_LEVEL => match payload.first() {
                Some(1) => (st::OK, vec![], None),
                Some(2) => (st::NOT_SUPPORTED, vec![], None),
                _ => (st::INVALID_CONFIG, vec![], None),
            },
            RESTART => {
                let mut tmp = Vec::new();
                if r.armed() {
                    r.drop_arm(crate::codes::drop::MAINTENANCE, &mut tmp);
                }
                out.extend(tmp);
                (st::OK, vec![], Some("RESTART"))
            }
            FACTORY_RESET => {
                let mut tmp = Vec::new();
                if r.armed() {
                    r.drop_arm(crate::codes::drop::MAINTENANCE, &mut tmp);
                }
                out.extend(tmp);
                r.logf(&format!("factory reset by {}", rx.src));
                (st::OK, vec![], Some("FACTORY_RESET"))
            }
            READ_LOG => {
                let from = if payload.len() >= 4 { be32(payload, 0) } else { 0 };
                (st::OK, log_payload(r, from), None)
            }
            CLEAR_LOG => {
                r.log.clear();
                r.logf(&format!("log cleared by {}", rx.src));
                (st::OK, vec![], None)
            }
            // Annex M: state (u8), blocks received (u32), block count (u32), sectors written (u8)
            UPDATE_STATUS => {
                let (state, rec, cnt) = match &self.upd {
                    Some(u) if u.verified => (2u8, u.received, u.count),
                    Some(u) => (1, u.received, u.count),
                    None => (0, 0, 0),
                };
                let mut v = vec![state];
                v.extend_from_slice(&rec.to_be_bytes());
                v.extend_from_slice(&cnt.to_be_bytes());
                v.push(0);
                (st::OK, v, None)
            }
            GET_CALIBRATION => (st::OK, cal_payload(r), None),
            SET_CALIBRATION => match parse_cal(payload) {
                Some(c) => {
                    let complete = c.complete();
                    let id = c.id;
                    r.logf(&format!("calibration set by {} ({} entries, complete {complete})", rx.src, c.entries.len()));
                    if let Some((_, v)) = c.entries.iter().find(|(k, _)| k == "P-RX-2") {
                        match crate::prx2::decode(v) {
                            Some(p) => r.logf(&format!("P-RX-2: swap {} invert {} offset {} rates {}", p.swap_iq, p.invert,
                                                       p.tuning_offset, p.rates.len())),
                            None => r.logf("P-RX-2: value not in the encoding of Table P-1 (kept; no status for it)"),
                        }
                    }
                    r.cal = Some(c);
                    r.rebuild_record();
                    let mut v = id.to_vec();
                    v.push(complete as u8);
                    (st::OK, v, None)
                }
                None => (st::INVALID_CONFIG, vec![], None),
            },
            CLEAR_CALIBRATION => {
                r.cal = None;
                r.rebuild_record();
                r.logf(&format!("calibration cleared by {}", rx.src));
                (st::OK, vec![], None)
            }
            GET_CONFIG => (st::OK, self.conf_payload(r), None),
            SET_CONFIG => {
                let mut tmp = Vec::new();
                let s = self.set_config(r, payload, &mut tmp);
                out.extend(tmp);
                let p = if s == st::OK { self.conf_payload(r) } else { vec![] };
                (s, p, None)
            }
            RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY => (st::NOT_SUPPORTED, vec![], None),
            UPDATE_BEGIN => self.update_begin(r, payload, out),
            UPDATE_BLOCK => self.update_block(payload),
            UPDATE_VERIFY => match self.upd.as_mut() {
                Some(u) if u.received == u.count => {
                    let h = sha256(&u.data);
                    if h == u.sha {
                        u.verified = true;
                        (st::OK, h.to_vec(), None)
                    } else {
                        r.logf("update: SHA-256 of the staged image differs from the header");
                        (st::VERIFY_FAILED, vec![], None)
                    }
                }
                _ => (st::VERIFY_FAILED, vec![], None),
            },
            UPDATE_ACTIVATE => match self.upd.as_ref() {
                Some(u) if u.verified => (st::OK, vec![0], Some("ACTIVATE")), // permanent 0: a trial boot follows
                _ => (st::VERIFY_FAILED, vec![], None),
            },
            UPDATE_ABORT => {
                self.upd = None;
                r.logf("update aborted: staging discarded, RESTART needed");
                (st::OK, vec![1], None)
            }
            ROLLBACK => {
                if r.fw.other_image().map_or(false, |i| i.good) {
                    (st::OK, vec![], Some("ROLLBACK"))
                } else {
                    (st::VERIFY_FAILED, vec![], None)
                }
            }
            _ => (st::NOT_SUPPORTED, vec![], None),
        };
        if verified && cmd != RESTART {
            if let Some((h, _)) = self.holder {
                if h == rx.src {
                    self.holder = Some((h, now));
                }
            }
        }
        self.send(r, rx, status, &reply_payload, true, cmd, out);
        match after {
            Some("RESTART") => {
                r.reset("RESTART");
                self.on_reset(r.now);
            }
            Some("ACTIVATE") => {
                // atomic switch, then the trial boot of the new image (A1.8 U-5, U-6)
                if let Some(u) = self.upd.take() {
                    r.fw.slots[(u.slot - 1) as usize] = Some(Image { version: u.version, len: u.len, sha: u.sha, good: false });
                    r.logf(&format!("update activated: slot {} on trial", u.slot));
                    r.reset("UPDATE_ACTIVATE");
                    r.fw.running = u.slot;
                    r.fw.rolled_back = false;
                    r.fw.trial = Some(r.now);
                    self.on_reset(r.now);
                }
            }
            Some("ROLLBACK") => {
                r.logf("ROLLBACK: to the other slot");
                r.reset("ROLLBACK");
                r.fw.running = r.fw.other();
                self.on_reset(r.now);
            }
            Some("FACTORY_RESET") => {
                self.secret = None;
                self.window_used = false;
                self.window_start = Some(r.now);
                // C-3 persistent limits are kept (Table A1-4): nothing to do here.
                r.reset("FACTORY_RESET");
                self.on_reset(r.now);
            }
            _ => {}
        }
    }

    fn limits(&mut self, r: &mut Radio, rx: &Rx, cmd: u16, inside: bool) -> bool {
        let now = r.now;
        if !self.lim_src.check(&rx.src, now) || !self.lim_total.check(&0, now) {
            r.count("mgmt.drop.rate");
            return false;
        }
        if cmd == IDENTIFY {
            if !self.lim_ident.check(&0, now) || (!inside && !self.lim_outside.check(&0, now)) {
                r.count("mgmt.drop.rate_identify");
                return false;
            }
            self.lim_ident.record(&0, now);
            if !inside {
                self.lim_outside.record(&0, now);
            }
        }
        self.lim_src.record(&rx.src, now);
        self.lim_total.record(&0, now);
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn send(&mut self, r: &mut Radio, rx: &Rx, status: u16, payload: &[u8], store: bool, cmd: u16,
            out: &mut Vec<Tx>) {
        let inside = r.own_subnet(*rx.src.ip());
        let reply = header(rx.data, status, payload);
        if store && rx.data.len() >= 18 {
            let n = be16(rx.data, 16) as usize;
            if rx.data.len() >= 18 + n {
                self.stored.put(rx.src, Stored { id: be32(rx.data, 8), cmd, hash: sha256(&rx.data[18..18 + n]),
                                                 reply: reply.clone(), at: r.now });
            }
        }
        if self.limits(r, rx, cmd, inside) {
            out.push(Tx { src_port: crate::codes::port::MGMT, dst: rx.src, data: reply });
        }
    }

    /// IDENTIFY payload, PROVISIONAL TLV layout (docs/mgmt-payloads.md).
    fn identify(&mut self, r: &Radio) -> Vec<u8> {
        let mut v = Vec::new();
        let mut tlv = |t: u8, val: &[u8]| {
            v.push(t);
            v.push(val.len() as u8);
            v.extend_from_slice(val);
        };
        tlv(1, b"G7250");
        tlv(2, &r.cfg.mac);
        let ver = |i: Option<&Image>| i.map_or(vec![0, 0, 0, 0], |i| {
            let mut v = vec![i.version.0, i.version.1];
            v.extend_from_slice(&i.version.2.to_be_bytes());
            v
        });
        tlv(4, &ver(r.fw.running_image()));
        tlv(5, &ver(r.fw.other_image()));
        tlv(6, &[0, 1, 0, 1]); // boot stage version (model)
        tlv(7, &0u32.to_be_bytes());
        tlv(8, &[0]); // bit 0 stock image retained = 0; bit 1 vendor recovery supported = 0
        let integ = |i: Option<&Image>| i.map_or(0, |_| 1);
        tlv(9, &[integ(r.fw.running_image()), integ(r.fw.other_image())]);
        let state = if r.fw.trial.is_some() { 1 } else if r.fw.rolled_back { 2 } else { 0 };
        tlv(10, &[state]);
        tlv(11, &[self.secret.is_some() as u8]);
        let w = self.window_open(r.now).map_or(0, |us| (us / S) as u16);
        tlv(12, &[(w > 0) as u8, (w >> 8) as u8, w as u8]);
        tlv(13, &[1]);
        let mut owner = [0u8; 6];
        if let Some(s) = &r.session {
            owner[..4].copy_from_slice(&s.owner.ip().octets());
            owner[4..].copy_from_slice(&s.owner.port().to_be_bytes());
        }
        tlv(14, &owner);
        tlv(15, &r.arm.word().to_be_bytes());
        tlv(16, &(((r.now - r.booted_at) / MS) as u32).to_be_bytes());
        tlv(17, &[0]);
        tlv(18, &[VERSION.0, VERSION.1, crate::codes::P2_VERSION, 1, 1]);
        tlv(19, &[0]); // controller state unknown (HIS-17)
        tlv(20, &[r.tx_calibrated() as u8]); // I-7: record tx bit 4
        let mut c = vec![0u8; 36];
        if let Some(cal) = &r.cal {
            c[..32].copy_from_slice(&cal.id);
            c[32..].copy_from_slice(&cal.date.to_be_bytes());
        }
        tlv(21, &c);
        tlv(22, &[0]); // sectors written by the streaming installer (none in the model)
        tlv(23, &[r.fw.running]);
        tlv(24, &[0]); // the model's link-local address is never used unprobed
        let mut lay = Vec::new();
        for (first, n) in LAYOUT {
            lay.push(first);
            lay.push(n);
        }
        tlv(25, &lay);
        v
    }
}

impl Mgmt {
    /// UPDATE_BEGIN (A1.8, Table M-4): the command-specific checks after the state checks, in the
    /// order of A1.8 (51, 63, 61), then 52 for an older image without the downgrade flag; a
    /// foreign image (A1.8a item 3) gets 56 after 51 while the F1 gate (P-SYS-5) is closed.
    fn update_begin(&mut self, r: &mut Radio, p: &[u8], out: &mut Vec<Tx>) -> (u16, Vec<u8>, Option<&'static str>) {
        let flags = p[0];
        let h = &p[1..];
        if be16(h, 6) != HW_ID {
            return (st::WRONG_HARDWARE, vec![], None);
        }
        if h[5] != 1 {
            return (st::NOT_SUPPORTED, vec![], None); // a component this version does not know (0063)
        }
        if flags & 1 != 0 {
            return (st::NOT_SUPPORTED, vec![], None); // F1 gate closed (P-SYS-5), A1.8a "Gate"
        }
        let version = (h[10], h[11], be16(h, 12));
        let len = be32(h, 14);
        let mut sha = [0u8; 32];
        sha.copy_from_slice(&h[18..50]);
        let target = h[52];
        if target == r.fw.running {
            return (st::RUNNING_SLOT, vec![], None);
        }
        let slot = r.fw.other();
        if len > Firmware::slot_size(slot) {
            return (st::TOO_LARGE_IMAGE, vec![], None);
        }
        let cur = r.fw.running_image().map_or((0, 0, 0), |i| i.version);
        if version < cur && flags & 2 == 0 {
            return (st::OLDER_IMAGE, vec![], None);
        }
        if version < cur {
            r.logf("update: downgrade requested");
        }
        let count = (len as usize).div_ceil(BLOCK) as u32;
        self.upd = Some(Upd { version, len, sha, slot, count, data: Vec::new(), received: 0, verified: false });
        r.begin_update(out);
        let mut v = (BLOCK as u16).to_be_bytes().to_vec();
        v.extend_from_slice(&count.to_be_bytes());
        v.push(0); // not one-way
        (st::OK, v, None)
    }

    /// UPDATE_BLOCK: blocks in order from 0; a repeat of a received block is harmless; the next
    /// one only (54 otherwise); CRC-32 per block (53); the exact length of the block (17).
    fn update_block(&mut self, p: &[u8]) -> (u16, Vec<u8>, Option<&'static str>) {
        let Some(u) = self.upd.as_mut() else { return (st::OUT_OF_ORDER, vec![], None) };
        let n = be32(p, 0);
        let crc = be32(p, 4);
        let data = &p[8..];
        if n >= u.count || n > u.received {
            return (st::OUT_OF_ORDER, vec![], None);
        }
        let want = if n + 1 < u.count { BLOCK } else { u.len as usize - BLOCK * (u.count as usize - 1) };
        if data.len() != want {
            return (st::MALFORMED, vec![], None);
        }
        if crc32(data) != crc {
            return (st::BAD_CRC, vec![], None);
        }
        if n == u.received {
            u.data.extend_from_slice(data);
            u.received += 1;
            u.verified = false;
        }
        let mut v = n.to_be_bytes().to_vec();
        v.extend_from_slice(&u.received.to_be_bytes());
        (st::OK, v, None)
    }

    /// GET_CONFIG / SET_CONFIG reply: every item of Table M-3.
    fn conf_payload(&self, r: &Radio) -> Vec<u8> {
        let c = &self.conf;
        let mut v = Vec::new();
        let mut tlv = |t: u8, val: &[u8]| {
            v.push(t);
            v.push(val.len() as u8);
            v.extend_from_slice(val);
        };
        tlv(1, &[c.addr_mode]);
        tlv(2, &c.addr);
        tlv(3, &c.mask);
        tlv(4, &c.gw);
        tlv(5, &[c.default_level]);
        tlv(6, &[c.region]);
        tlv(7, &r.persistent_limits[0].to_be_bytes());
        for (i, l) in c.band_limits.iter().enumerate() {
            let b = l.to_be_bytes();
            tlv(8, &[i as u8, b[0], b[1]]);
        }
        // S2 (x100) and T3 (0.1 degC): 0 while the calibration set does not hold P-PWR-2
        let (s2, t3) = if r.has_param("P-PWR-2") {
            ((r.persistent_limits[2] / 10) as u16, (r.persistent_limits[3] / 100) as u16)
        } else {
            (0, 0)
        };
        tlv(9, &s2.to_be_bytes());
        tlv(10, &t3.to_be_bytes());
        let mut a = Vec::new();
        for (ip, pl) in &c.allow {
            a.extend_from_slice(ip);
            a.push(*pl);
        }
        tlv(11, &a);
        v
    }

    /// SET_CONFIG (A1.6): validated as a whole; order of the command-specific codes 48, 49, 62
    /// (Table A1-3). Returns the status.
    fn set_config(&mut self, r: &mut Radio, p: &[u8], out: &mut Vec<Tx>) -> u16 {
        let mut c = self.conf.clone();
        let mut limits = r.persistent_limits;
        let (mut bad, mut looser, mut needs_cal, mut c3) = (false, false, false, false);
        let mut i = 0;
        while i < p.len() {
            if i + 2 > p.len() || i + 2 + p[i + 1] as usize > p.len() {
                bad = true;
                break;
            }
            let (tag, val) = (p[i], &p[i + 2..i + 2 + p[i + 1] as usize]);
            i += 2 + val.len();
            let n = val.len();
            match (tag, n) {
                (1, 1) if val[0] <= 1 => c.addr_mode = val[0],
                (2, 4) => c.addr.copy_from_slice(val),
                (3, 4) => c.mask.copy_from_slice(val),
                (4, 4) => c.gw.copy_from_slice(val),
                (5, 1) if val[0] == 1 => c.default_level = 1,
                (6, 1) if val[0] == 0 => c.region = 0, // one region in this version (Table A3-0)
                (7, 4) => {
                    let ms = be32(val, 0);
                    if ms == 0 {
                        bad = true;
                    } else if ms > r.cfg.builtin_tx_time_ms {
                        looser = true;
                    } else {
                        limits[0] = ms;
                        c3 = true;
                    }
                }
                (8, 3) if (val[0] as usize) < 10 => {
                    let w = be16(val, 1);
                    if w == 0 {
                        bad = true;
                    } else if w > band_builtin()[val[0] as usize] {
                        looser = true;
                    } else {
                        c.band_limits[val[0] as usize] = w;
                        c3 = true;
                    }
                }
                (9, 2) | (10, 2) => {
                    if !r.has_param("P-PWR-2") {
                        needs_cal = true;
                    } else {
                        let x = be16(val, 0) as u32;
                        let (idx, scaled, builtin) = if tag == 9 {
                            (2, x * 10, r.cfg.builtin_swr_s2)
                        } else {
                            (3, x * 100, r.cfg.builtin_pa_temp_t3_mc)
                        };
                        if x == 0 {
                            bad = true;
                        } else if scaled > builtin {
                            looser = true;
                        } else {
                            limits[idx] = scaled;
                            c3 = true;
                        }
                    }
                }
                (11, n) if n % 5 == 0 && n / 5 <= 8 => {
                    let mut a = Vec::new();
                    for e in val.chunks(5) {
                        if e[4] > 32 {
                            bad = true;
                        }
                        a.push(([e[0], e[1], e[2], e[3]], e[4]));
                    }
                    c.allow = a;
                }
                _ => bad = true,
            }
        }
        if bad {
            return st::INVALID_CONFIG;
        }
        if looser {
            return st::NOT_ALLOWED;
        }
        if needs_cal {
            return st::NEEDS_CALIBRATION;
        }
        if c3 && r.armed() {
            r.drop_arm(crate::codes::drop::MAINTENANCE, out); // A1.9: unkey, drop, apply, reply
        }
        for k in 0..4 {
            if limits[k] != r.persistent_limits[k] {
                r.logf(&format!("C-3 limit {k}: {} -> {} by {}", r.persistent_limits[k], limits[k], "management"));
            }
        }
        r.persistent_limits = limits;
        self.conf = c;
        r.logf("configuration changed");
        st::OK
    }
}

fn cal_payload(r: &Radio) -> Vec<u8> {
    let mut v = vec![0u8; 32];
    let Some(c) = &r.cal else {
        v.extend_from_slice(&0u32.to_be_bytes());
        v.push(0);
        return v;
    };
    v[..32].copy_from_slice(&c.id);
    v.extend_from_slice(&c.date.to_be_bytes());
    v.push(c.source.len() as u8);
    v.extend_from_slice(c.source.as_bytes());
    for (id, val) in &c.entries {
        v.push(id.len() as u8);
        v.extend_from_slice(id.as_bytes());
        v.extend_from_slice(&(val.len() as u16).to_be_bytes());
        v.extend_from_slice(val);
    }
    v
}

/// SET_CALIBRATION payload (Annex M Table M-1, M-5); None = status 48.
/// A payload that cannot be read by the structure of Annex M (0056): wrong fixed length, a
/// TLV that runs past the end, a missing fixed field. Values are not judged here.
fn payload_malformed(cmd: u16, p: &[u8]) -> bool {
    match cmd {
        SET_FIRST_SECRET | CHANGE_SECRET => p.len() != 32,
        SWITCH_LEVEL => p.len() != 1,
        // 0063: a header that cannot be read (wrong magic, header version not 1) is 17 as well
        UPDATE_BEGIN => p.len() != 1 + 53 || be32(p, 1) != PK_MAGIC || p[5] != 1,
        UPDATE_BLOCK => p.len() < 8 || p.len() > 8 + BLOCK,
        UPDATE_ACTIVATE => p.len() != 1,
        SET_CALIBRATION => !cal_structure_ok(p),
        SET_CONFIG => {
            let mut i = 0;
            while i < p.len() {
                if i + 2 > p.len() || i + 2 + p[i + 1] as usize > p.len() {
                    return true;
                }
                i += 2 + p[i + 1] as usize;
            }
            false
        }
        _ => false,
    }
}

/// Table M-5 structure only: date, source, then entries of id length, id, value length, value.
fn cal_structure_ok(p: &[u8]) -> bool {
    if p.len() < 5 {
        return false;
    }
    let sl = p[4] as usize;
    let mut i = 5 + sl;
    if i > p.len() {
        return false;
    }
    while i < p.len() {
        let il = p[i] as usize;
        if i + 1 + il + 2 > p.len() {
            return false;
        }
        let vl = be16(p, i + 1 + il) as usize;
        i += 1 + il + 2;
        if i + vl > p.len() {
            return false;
        }
        i += vl;
    }
    true
}

fn parse_cal(p: &[u8]) -> Option<Calibration> {
    if p.len() < 5 {
        return None;
    }
    let date = be32(p, 0);
    let sl = p[4] as usize;
    let source = std::str::from_utf8(p.get(5..5 + sl)?).ok()?.to_string();
    let mut i = 5 + sl;
    let mut entries = Vec::new();
    while i < p.len() {
        let il = *p.get(i)? as usize;
        let id = std::str::from_utf8(p.get(i + 1..i + 1 + il)?).ok()?.to_string();
        i += 1 + il;
        let vl = be16(p.get(i..i + 2)?, 0) as usize;
        let val = p.get(i + 2..i + 2 + vl)?.to_vec();
        i += 2 + vl;
        if !ALL_PARAMS.contains(&id.as_str()) || val.is_empty() || entries.iter().any(|(k, _): &(String, Vec<u8>)| *k == id) {
            return None;
        }
        entries.push((id, val));
    }
    Some(Calibration { id: sha256(p), date, source, entries })
}

fn stats_payload(r: &Radio) -> Vec<u8> {
    let mut keys: Vec<_> = r.stats.iter().collect();
    keys.sort();
    let mut v = Vec::new();
    for (k, n) in keys {
        if v.len() + k.len() + 9 > 1380 {
            break;
        }
        v.push(k.len() as u8);
        v.extend_from_slice(k.as_bytes());
        v.extend_from_slice(&n.to_be_bytes());
    }
    let resets = b"resets";
    v.push(resets.len() as u8);
    v.extend_from_slice(resets);
    v.extend_from_slice(&r.resets.to_be_bytes());
    v
}

fn log_payload(r: &Radio, from: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for (seq, up, text) in r.log.iter().filter(|(s, _, _)| *s >= from) {
        let t = &text.as_bytes()[..text.len().min(200)];
        if v.len() + 9 + t.len() > 1380 {
            break;
        }
        v.extend_from_slice(&seq.to_be_bytes());
        v.extend_from_slice(&(*up as u32).to_be_bytes());
        v.push(t.len() as u8);
        v.extend_from_slice(t);
    }
    v
}
