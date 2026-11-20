//! Management channel, UDP 7251 (spec A1.4–A1.10). Framing, status precedence, challenge and
//! response, write session, stored replies, rate limits and the first-secret window follow the
//! spec. Payload layouts are not fixed by the spec yet ("fixed with the reference model before
//! M5", A1 intro); the layouts here are PROVISIONAL and documented in docs/mgmt-payloads.md.
//!
//! Model gaps in T1 (answered with status 56): GET_CONFIG, SET_CONFIG, UPDATE_BEGIN …
//! UPDATE_ACTIVATE, UPDATE_ABORT, ROLLBACK.

use std::net::Ipv4Addr;

use crate::codes::mgmt::{st, *};
use crate::crypto::{ct_eq, hmac_sha256, sha256};
use crate::radio::{Radio, Rx, Tx, MS, S};
use crate::util::*;

pub const CHALLENGE_LEN: usize = 16; // PROVISIONAL (finding SF-08)
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
}

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
        | RESTART | CLEAR_LOG | 0x0040..=0x0044 | ROLLBACK | RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY)
}
fn known(cmd: u16) -> bool {
    matches!(cmd, IDENTIFY | GET_STATS | GET_CHALLENGE | SET_FIRST_SECRET | CHANGE_SECRET
        | END_WRITE_SESSION | GET_CONFIG | SET_CONFIG | FACTORY_RESET | SWITCH_LEVEL | RESTART
        | READ_LOG | CLEAR_LOG | 0x0040..=0x0046 | RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY)
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
        if (is_update(cmd) && r.update_in_progress) || (cmd == SWITCH_LEVEL && r.update_in_progress)
            || ((cmd == RETURN_TO_STOCK || cmd == ENTER_VENDOR_RECOVERY) && r.update_in_progress)
        {
            return self.send(r, rx, st::UPDATE_IN_PROGRESS, &[], true, cmd, out);
        }
        if (is_update(cmd) || cmd == RETURN_TO_STOCK || cmd == ENTER_VENDOR_RECOVERY) && armed_or_keyed {
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
            UPDATE_STATUS => (st::OK, vec![if r.update_in_progress { 1 } else { 0 }], None),
            RETURN_TO_STOCK | ENTER_VENDOR_RECOVERY => (st::NOT_SUPPORTED, vec![], None),
            _ => (st::NOT_SUPPORTED, vec![], None), // model gap in T1
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
        let (maj, min, b) = r.cfg.fw;
        let mut fw = vec![maj, min];
        fw.extend_from_slice(&b.to_be_bytes());
        tlv(4, &fw);
        tlv(5, &[0, 0, 0, 0]); // other slot empty
        tlv(6, &[0, 1, 0, 1]); // boot stage version (model)
        tlv(7, &0u32.to_be_bytes());
        tlv(8, &[0]); // bit 0 stock image retained = 0; bit 1 vendor recovery supported = 0
        tlv(9, &[1, 0]); // integrity: running ok, other slot empty
        tlv(10, &[0]); // update state normal
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
        tlv(20, &[(!r.cfg.tx_locked) as u8]); // TX calibrated (proposal SF-05)
        v
    }
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
