// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Radio end of the management channel (spec A1.4, A1.5, A1.10, Annex M), step 2: the read
//! commands `IDENTIFY`, `GET_STATS`, `GET_CHALLENGE`, `GET_CONFIG`, `GET_CALIBRATION`.
//!
//! Radio state kept here: the repeat cache (A1.4: last reply per host, 5 s, 4 hosts), one
//! challenge per host (A1.10: 4 hosts, 10 s, single use), the first-secret window (300 s of
//! network uptime while no secret is set), the source rules and reply caps (A1.10: own subnet
//! only except `IDENTIFY`, never a broadcast source; 20/s per source, 100/s in total, 50/s
//! `IDENTIFY`, 10/s `IDENTIFY` from outside the subnet), the authentication-failure limit (5 in
//! 10 s → status 34) and the counters of `GET_STATS`.
//!
//! Not yet: the write commands (step 4). Until then no secret can be set, so every
//! authenticated request fails with 33, and a command not built yet is answered with 56.

pub mod update;
pub mod write;

pub use update::{NoUpdate, Rollback, Updater};
pub use write::{Effect, Log};

use g7250_proto::mgmt::{self, auth, Command, Incoming, Status};
use g7250_proto::netcfg;
use g7250_proto::ratelimit::RateLimit;

type Addr = ([u8; 4], u16);

const HOSTS: usize = 4;
const REPEAT_MS: u64 = 5_000;
const CHALLENGE_MS: u64 = 10_000;
pub const FIRST_SECRET_WINDOW_MS: u64 = 300_000;
const AUTH_FAIL_WINDOW_MS: u64 = 10_000;
const AUTH_FAIL_LIMIT: u8 = 5;

/// `GET_STATS` counters (Table M-1: name, u64). Names are this firmware's; Annex M fixes only the
/// layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub mgmt_requests: u64,
    pub mgmt_replies: u64,
    pub mgmt_dropped_not_request: u64,
    pub mgmt_dropped_source: u64,
    pub mgmt_dropped_rate: u64,
    pub mgmt_repeats: u64,
    pub mgmt_auth_failures: u64,
    pub discovery_answered: u64,
    pub discovery_dropped_rate: u64,
    pub discovery_from_port_1024: u64,
    pub set_ip_ignored: u64,
    /// DUC-specific packets with a hang delay above 2000 ms or a keyer speed above 60 wpm
    /// (Table B10-1, B10.5a item 1; names fixed by 0046).
    pub cw_hang_clamped: u64,
    pub cw_speed_clamped: u64,
    /// Level-1 takeovers since start (A2.4).
    pub takeovers: u64,
    /// The longest net-loop pass that handled a P2 packet since start, µs (0054 margin).
    pub net_busy_pass_max_us: u64,
    /// Datagrams not sent because the previous send on their socket was still in its ARP
    /// (dropped, not queued; 0054).
    pub net_send_dropped: u64,
    /// Link-local addresses used without a complete RFC 3927 probe (A1.11; 0057).
    pub ll_unprobed: u64,
    /// Microseconds from the boot stage's start to the first TX path check (A3.6b; 0066
    /// addendum: at most 100 ms).
    pub tx_check_first_us: u64,
    /// Console bytes dropped because the console's ring was full (the console never blocks).
    pub console_dropped: u64,
    /// High Priority packets read from port 1027, from any source, before the owner check
    /// (B2.3; 0058).
    pub hp_rx: u64,
    /// The longest time of each step of a net-loop pass since start, us ([`STEP_NAMES`]).
    pub steps: [u64; STEPS],
}

/// Steps of the net loop timed for `GET_STATS` (`step_<name>_us`): the loop's tick, the DHCP
/// client, one datagram (read and handled) of each P2 socket, discovery, management, and inside
/// the P2 handler the session events and the slowest single register-packet send.
pub const STEPS: usize = 11;
pub const STEP_NAMES: [&[u8]; STEPS] = [
    b"step_tick_us",
    b"step_dhcp_us",
    b"step_hp_us",
    b"step_ddc_us",
    b"step_duc_us",
    b"step_duc_iq_us",
    b"step_reg_us",
    b"step_discovery_us",
    b"step_mgmt_us",
    b"step_events_us",
    b"step_send_us",
];

impl Stats {
    fn encode(&self, out: &mut [u8]) -> usize {
        let items: [(&[u8], u64); 20] = [
            (b"mgmt_requests", self.mgmt_requests),
            (b"mgmt_replies", self.mgmt_replies),
            (b"mgmt_dropped_not_request", self.mgmt_dropped_not_request),
            (b"mgmt_dropped_source", self.mgmt_dropped_source),
            (b"mgmt_dropped_rate", self.mgmt_dropped_rate),
            (b"mgmt_repeats", self.mgmt_repeats),
            (b"mgmt_auth_failures", self.mgmt_auth_failures),
            (b"discovery_answered", self.discovery_answered),
            (b"discovery_dropped_rate", self.discovery_dropped_rate),
            (b"discovery_from_port_1024", self.discovery_from_port_1024),
            (b"set_ip_ignored", self.set_ip_ignored),
            (b"cw_hang_clamped", self.cw_hang_clamped),
            (b"cw_speed_clamped", self.cw_speed_clamped),
            (b"takeovers", self.takeovers),
            (b"net_busy_pass_max_us", self.net_busy_pass_max_us),
            (b"net_send_dropped", self.net_send_dropped),
            (b"ll_unprobed", self.ll_unprobed),
            (b"tx_check_first_us", self.tx_check_first_us),
            (b"console_dropped", self.console_dropped),
            (b"hp_rx", self.hp_rx),
        ];
        let mut at = 0;
        for (n, v) in items {
            at = mgmt::payload::stat(out, at, n, v).unwrap_or(at);
        }
        for (n, v) in STEP_NAMES.iter().zip(self.steps) {
            at = mgmt::payload::stat(out, at, n, v).unwrap_or(at);
        }
        at
    }
}

/// What the server needs from the rest of the firmware for one request.
pub struct Ctx<'a> {
    pub now_ms: u64,
    /// Our address and netmask (source rule, A1.10).
    pub own_ip: [u8; 4],
    pub own_mask: [u8; 4],
    /// Encoded `IDENTIFY` TLV list without tag 12 (the server adds the window) and with tag 11
    /// as `secret_set`.
    pub identify: &'a mgmt::payload::Identify<'a>,
    /// Encoded `GET_CONFIG` reply payload (Table M-3, every item).
    pub config: &'a [u8],
    /// Encoded `GET_CALIBRATION` reply payload (Table M-1).
    pub calibration: &'a [u8],
    /// Fills a challenge from the hardware RNG; false if it failed.
    pub rng: &'a mut dyn FnMut(&mut [u8; 16]) -> bool,
    /// Set if a secret is stored.
    pub secret: Option<&'a [u8; 32]>,
    /// A G7250-P2 session runs (status 57 for calibration and region writes).
    pub p2_session: bool,
    /// TX armed or keyed on any level (status 50).
    pub armed: bool,
    /// The update path (A1.8).
    pub update: &'a mut dyn Updater,
}

#[derive(Clone, Copy)]
struct Stored {
    addr: Addr,
    id: u32,
    command: u16,
    hash: [u8; 32],
    at_ms: u64,
    len: u16,
    reply: [u8; mgmt::MAX_REPLY],
}

#[derive(Clone, Copy)]
struct Challenge {
    addr: Addr,
    value: [u8; 16],
    at_ms: u64,
}

#[derive(Clone, Copy)]
struct AuthFail {
    ip: [u8; 4],
    count: u8,
    last_ms: u64,
}

pub struct Server {
    repeat: [Option<Stored>; HOSTS],
    challenges: [Option<Challenge>; HOSTS],
    fails: [Option<AuthFail>; HOSTS],
    per_source: RateLimit<16>,
    identify_all: RateLimit<1>,
    identify_off_subnet: RateLimit<1>,
    /// Network up since (window start); `None` until the network is up.
    net_up_ms: Option<u64>,
    window_used: bool,
    pub stats: Stats,
    /// Write session holder and the time of its last refresh (A1.10).
    writer: Option<(Addr, u64)>,
    /// A commanded reset is under way (A1.9 "One commanded reset at a time").
    reset_pending: bool,
    /// The challenge the current authenticated request used (`CHANGE_SECRET` mask, Table M-1).
    used_challenge: [u8; 16],
    /// Effects of the last request for the board (store, network, reset), in order.
    pub effects: heapless::Vec<Effect, 4>,
    pub log: Log,
}

fn is_broadcast_source(ip: [u8; 4], own: [u8; 4], mask: [u8; 4]) -> bool {
    ip == [255; 4] || ip == [0; 4] || (netcfg::valid_mask(mask) && ip == netcfg::directed_broadcast(own, mask)) || ip[0] >= 224
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    pub const fn new() -> Self {
        Server {
            repeat: [None; HOSTS],
            challenges: [None; HOSTS],
            fails: [None; HOSTS],
            per_source: RateLimit::new(20, 100),
            identify_all: RateLimit::new(u16::MAX, 50),
            identify_off_subnet: RateLimit::new(u16::MAX, 10),
            net_up_ms: None,
            window_used: false,
            stats: Stats {
                mgmt_requests: 0,
                mgmt_replies: 0,
                mgmt_dropped_not_request: 0,
                mgmt_dropped_source: 0,
                mgmt_dropped_rate: 0,
                mgmt_repeats: 0,
                mgmt_auth_failures: 0,
                discovery_answered: 0,
                discovery_dropped_rate: 0,
                discovery_from_port_1024: 0,
                set_ip_ignored: 0,
                cw_hang_clamped: 0,
                cw_speed_clamped: 0,
                takeovers: 0,
                net_busy_pass_max_us: 0,
                net_send_dropped: 0,
                ll_unprobed: 0,
                tx_check_first_us: 0,
                console_dropped: 0,
                hp_rx: 0,
                steps: [0; STEPS],
            },
            writer: None,
            reset_pending: false,
            used_challenge: [0; 16],
            effects: heapless::Vec::new(),
            log: Log::new(),
        }
    }

    /// The network came up: the first-secret window starts (A1.10).
    pub fn network_up(&mut self, now_ms: u64) {
        if self.net_up_ms.is_none() {
            self.net_up_ms = Some(now_ms);
        }
    }

    /// First-secret window: open, seconds left (A1.10; IDENTIFY tag 12).
    /// Clear the effects of the previous request (call before `handle`).
    pub fn take_effects(&mut self) -> heapless::Vec<Effect, 4> {
        core::mem::take(&mut self.effects)
    }

    pub fn window(&self, now_ms: u64, secret_set: bool) -> (bool, u16) {
        match self.net_up_ms {
            Some(t0) if !secret_set && !self.window_used => {
                let end = t0 + FIRST_SECRET_WINDOW_MS;
                if now_ms < end { (true, ((end - now_ms + 999) / 1000) as u16) } else { (false, 0) }
            }
            _ => (false, 0),
        }
    }

    fn lru_slot<T: Copy>(slots: &[Option<T>; HOSTS], key: impl Fn(&T) -> bool, age: impl Fn(&T) -> u64) -> usize {
        if let Some(i) = slots.iter().position(|s| s.as_ref().is_some_and(&key)) {
            return i;
        }
        if let Some(i) = slots.iter().position(|s| s.is_none()) {
            return i;
        }
        let mut best = 0;
        for (i, s) in slots.iter().enumerate() {
            if age(s.as_ref().unwrap()) < age(slots[best].as_ref().unwrap()) {
                best = i;
            }
        }
        best
    }

    fn auth_limited(&self, ip: [u8; 4], now: u64) -> bool {
        self.fails.iter().flatten().any(|f| f.ip == ip && f.count >= AUTH_FAIL_LIMIT && now - f.last_ms < AUTH_FAIL_WINDOW_MS)
    }

    fn auth_failed(&mut self, ip: [u8; 4], now: u64) {
        self.stats.mgmt_auth_failures += 1;
        let i = Self::lru_slot(&self.fails, |f| f.ip == ip, |f| f.last_ms);
        let f = match self.fails[i] {
            Some(f) if f.ip == ip && now - f.last_ms < AUTH_FAIL_WINDOW_MS => AuthFail { ip, count: f.count.saturating_add(1), last_ms: now },
            _ => AuthFail { ip, count: 1, last_ms: now },
        };
        self.fails[i] = Some(f);
    }

    /// Handle one datagram from `src`; returns the reply length in `out`, or `None` for no reply.
    pub fn handle(&mut self, src: Addr, d: &[u8], ctx: &mut Ctx<'_>, out: &mut [u8; mgmt::MAX_REPLY]) -> Option<usize> {
        let now = ctx.now_ms;
        let req = match mgmt::check(d) {
            Incoming::Drop => return None,
            Incoming::DropNotRequest => {
                self.stats.mgmt_dropped_not_request += 1;
                return None;
            }
            Incoming::Answer { request_id, command, status } => {
                self.stats.mgmt_requests += 1;
                if !self.source_ok(src, false, ctx) || !self.rate_ok(src, false, ctx) {
                    return None;
                }
                return self.reply(out, request_id, command, status, &[]);
            }
            Incoming::Request(r) => r,
        };
        self.stats.mgmt_requests += 1;
        let cmd = req.command.unwrap();
        let identify = cmd == Command::Identify;
        if !self.source_ok(src, identify, ctx) {
            return None;
        }
        // Repeat (A1.4): same host, id, command and payload hash within 5 s.
        let hash = auth::sha256(req.payload);
        if let Some(s) = self.repeat.iter().flatten().find(|s| {
            s.addr == src && s.id == req.header.request_id && s.command == req.header.command && s.hash == hash && now - s.at_ms < REPEAT_MS
        }) {
            let s = *s;
            if !self.rate_ok(src, identify, ctx) {
                return None;
            }
            self.stats.mgmt_repeats += 1;
            out[..s.len as usize].copy_from_slice(&s.reply[..s.len as usize]);
            self.stats.mgmt_replies += 1;
            return Some(s.len as usize);
        }
        if !self.rate_ok(src, identify, ctx) {
            return None;
        }
        // A payload without its command's size or structure: 17 in the framing group, before
        // authentication and every state check (A1.4 "Malformed payload"; 0056).
        let (status, n) = if mgmt::payload::well_formed(cmd, req.payload) { self.execute(src, cmd, &req, ctx, out) } else { (Status::Malformed, 0) };
        let id = req.header.request_id;
        // Build the reply in place: the payload was written at out[HEADER_LEN..].
        let mut tmp = [0u8; mgmt::MAX_REPLY - mgmt::HEADER_LEN];
        tmp[..n].copy_from_slice(&out[mgmt::HEADER_LEN..mgmt::HEADER_LEN + n]);
        let len = self.reply(out, id, cmd as u16, status, &tmp[..n])?;
        let i = Self::lru_slot(&self.repeat, |s| s.addr == src, |s| s.at_ms);
        let mut st = Stored { addr: src, id, command: cmd as u16, hash, at_ms: now, len: len as u16, reply: [0; mgmt::MAX_REPLY] };
        st.reply[..len].copy_from_slice(&out[..len]);
        self.repeat[i] = Some(st);
        Some(len)
    }

    /// The reply just built for `src` (the newest in the repeat cache) becomes `status` with no
    /// payload, in `out` and in the cache, so that a repeat gets the same: a store write that
    /// failed after the command was accepted (B2.3 "flash work that fails": 55, the old values
    /// kept; 0065 point 2). Returns the new length.
    pub fn fail_reply(&mut self, src: Addr, status: Status, out: &mut [u8; mgmt::MAX_REPLY]) -> Option<usize> {
        let i = self.repeat.iter().enumerate().filter_map(|(i, s)| s.filter(|s| s.addr == src).map(|s| (i, s.at_ms))).max_by_key(|x| x.1)?.0;
        let s = self.repeat[i]?;
        let len = mgmt::build_reply(out, s.id, s.command, status, &[]).ok()?;
        let mut st = s;
        st.len = len as u16;
        st.reply[..len].copy_from_slice(&out[..len]);
        self.repeat[i] = Some(st);
        Some(len)
    }

    fn source_ok(&mut self, src: Addr, identify: bool, ctx: &Ctx<'_>) -> bool {
        let ok = !is_broadcast_source(src.0, ctx.own_ip, ctx.own_mask)
            && (identify || (netcfg::valid_mask(ctx.own_mask) && netcfg::in_subnet(ctx.own_ip, ctx.own_mask, src.0)));
        if !ok {
            self.stats.mgmt_dropped_source += 1;
        }
        ok
    }

    fn rate_ok(&mut self, src: Addr, identify: bool, ctx: &Ctx<'_>) -> bool {
        let now = ctx.now_ms;
        let mut ok = self.per_source.allow(src.0, src.1, now);
        if ok && identify {
            ok = self.identify_all.allow([0; 4], 0, now);
            let inside = netcfg::valid_mask(ctx.own_mask) && netcfg::in_subnet(ctx.own_ip, ctx.own_mask, src.0);
            if ok && !inside {
                ok = self.identify_off_subnet.allow([0; 4], 0, now);
            }
        }
        if !ok {
            self.stats.mgmt_dropped_rate += 1;
        }
        ok
    }

    fn reply(&mut self, out: &mut [u8; mgmt::MAX_REPLY], id: u32, command: u16, status: Status, payload: &[u8]) -> Option<usize> {
        let n = mgmt::build_reply(out, id, command, status, payload).ok()?;
        self.stats.mgmt_replies += 1;
        Some(n)
    }

    /// Run the command; the payload goes to `out[HEADER_LEN..]`. Returns (status, payload length).
    fn execute(&mut self, src: Addr, cmd: Command, req: &mgmt::Request<'_>, ctx: &mut Ctx<'_>, out: &mut [u8; mgmt::MAX_REPLY]) -> (Status, usize) {
        let now = ctx.now_ms;
        let p = &mut out[mgmt::HEADER_LEN..];
        if cmd.needs_auth() {
            // A1.3 order: 32, 33, 34 come before command-specific codes.
            if self.auth_limited(src.0, now) {
                return (Status::RateLimited, 0);
            }
            let Some(trailer) = req.trailer else { return (Status::AuthRequired, 0) };
            let ok = match (ctx.secret, self.take_challenge(src, now)) {
                (Some(secret), Some(ch)) => {
                    self.used_challenge = ch;
                    auth::verify(secret, &ch, req.signed, trailer)
                }
                _ => false,
            };
            if !ok {
                self.auth_failed(src.0, now);
                self.log.push(now, format_args!("auth failure from {}.{}.{}.{}:{}", src.0[0], src.0[1], src.0[2], src.0[3], src.1));
                return (Status::AuthFailed, 0);
            }
            return self.write(src, cmd, req, ctx, p);
        }
        match cmd {
            Command::Identify => {
                let mut id = *ctx.identify;
                id.window = self.window(now, ctx.secret.is_some());
                id.secret_set = ctx.secret.is_some();
                match id.encode(p) {
                    Some(n) => (Status::Ok, n),
                    None => (Status::TooLarge, 0),
                }
            }
            Command::GetStats => {
                let n = self.stats.encode(p);
                (Status::Ok, n)
            }
            Command::GetChallenge => {
                if self.auth_limited(src.0, now) {
                    return (Status::RateLimited, 0);
                }
                let mut c = [0u8; 16];
                if !(ctx.rng)(&mut c) {
                    return (Status::NotSupported, 0);
                }
                let i = Self::lru_slot(&self.challenges, |x| x.addr == src, |x| x.at_ms);
                self.challenges[i] = Some(Challenge { addr: src, value: c, at_ms: now });
                p[..16].copy_from_slice(&c);
                (Status::Ok, 16)
            }
            Command::GetConfig => copy(p, ctx.config),
            Command::GetCalibration => copy(p, ctx.calibration),
            Command::SetFirstSecret => self.set_first_secret(src, req.payload, ctx),
            Command::ReadLog => self.read_log(req.payload, p),
            Command::UpdateStatus => {
                // A read, answered at any time (A1.8; 0022 §51).
                let (state, received, count) = ctx.update.status();
                let u = mgmt::payload::UpdateStatus { state, received, count, sectors_written: 0 };
                p[..10].copy_from_slice(&u.encode());
                (Status::Ok, 10)
            }
            // Unauthenticated but not built yet: SET_FIRST_SECRET (step 4), READ_LOG,
            // UPDATE_STATUS (with the update path).
            _ => (Status::NotSupported, 0),
        }
    }

    /// The host's challenge, if fresh; it is used up (A1.10: one request, 10 s).
    fn take_challenge(&mut self, src: Addr, now: u64) -> Option<[u8; 16]> {
        let i = self.challenges.iter().position(|c| c.as_ref().is_some_and(|c| c.addr == src))?;
        let c = self.challenges[i].take()?;
        if now - c.at_ms < CHALLENGE_MS { Some(c.value) } else { None }
    }
}

fn copy(p: &mut [u8], src: &[u8]) -> (Status, usize) {
    if src.len() > p.len() {
        return (Status::TooLarge, 0);
    }
    p[..src.len()].copy_from_slice(src);
    (Status::Ok, src.len())
}
