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
pub use write::{Effect, Log, Reason};

use g7250_safety::logring;
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

/// `GET_STATS` counters (Table M-1: name, u64): first those of Table M-6 in its order, every one
/// also at 0 (0074), then this firmware's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub mgmt_requests: u64,
    pub mgmt_replies: u64,
    pub mgmt_dropped_not_request: u64,
    pub mgmt_dropped_source: u64,
    pub mgmt_dropped_rate: u64,
    pub mgmt_repeats: u64,
    pub mgmt_auth_failures: u64,
    /// Requests answered with status 32 (Table M-6; 0080).
    pub mg_auth_required: u64,
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
    /// A3.5 "Counters" since power-on, over resets: entries of SWR_REPEATED, PA_OVERTEMP,
    /// WDT_RESET_AT_START; resets by cause (pin, software, independent watchdog, window watchdog,
    /// low-power).
    pub fault_entries: [u64; 3],
    pub resets: [u64; 5],
    /// The last whole second, per task (net, tx_path, watchdog, console, main): wakes and busy
    /// microseconds (idle analysis; emulator time.advance).
    pub wakes_per_s: [u64; 5],
    pub busy_us_per_s: [u64; 5],
    /// Console bytes dropped because the console's ring was full (the console never blocks).
    pub console_dropped: u64,
    /// High Priority packets read from port 1027, from any source, before the owner check
    /// (B2.3; 0058).
    pub hp_rx: u64,
    /// The longest time of each step of a net-loop pass since start, us ([`STEP_NAMES`]).
    pub steps: [u64; STEPS],
    /// Table M-6 (0074) counters not kept above. Management requests above a total reply cap
    /// (`IDENTIFY` in all, or from outside the subnet), dropped.
    pub mgmt_dropped_cap: u64,
    /// General packets on port 1024 at level 2: 0 in this version (no level 2).
    pub gen1024_ignored: u64,
    /// Non-owner `CAP_REQUEST`s above the answer limit (B4.4 item 3); rejected register writes
    /// above the `NOT_OWNER` limit; register packets from a non-owner with source port 7250 (B8.5).
    pub cap_rate_drop: u64,
    pub notowner_drop: u64,
    pub reg7250_drop: u64,
    /// Not counted yet by this firmware (always 0): ICMP port unreachable (B11.10), the numbered
    /// mode of port 1027 (B11.6a), sequence gaps (B11.6).
    pub icmp_unreach: u64,
    pub hp_stale: u64,
    pub hp_resync: u64,
    pub hp_release: u64,
    pub gap_1027: u64,
    pub gap_1029: u64,
    /// DUC I&Q discarded (B10.7): from another source or with no session; from the owner while
    /// record `tx` bit 1 or 4 is 0. Received while keyed in CW; samples dropped from the FIFO.
    pub duc_foreign: u64,
    pub duc_not_tx: u64,
    pub duc_cw_drop: u64,
    pub duc_fifo_drop: u64,
    /// Packets discarded per receiving port of Table B2-1 ([`DROP_PORTS`]).
    pub drop_port: [u64; 7],
    /// `TRIP_COUNT` (A3.5).
    pub trip_count: u64,
}

/// The ports of the `drop_<port>` counters, in the order of Table M-6.
pub const DROP_PORTS: [u16; 7] = [1024, 1025, 1026, 1027, 1029, 7250, 7251];
/// The fault codes of the `fault_<code>` counters (Table B8-3), in the order of Table M-6, and the
/// index into [`Stats::fault_entries`] of the three kept over resets (A3.5 "Counters"); the
/// others are not kept yet (always 0).
const FAULT_NAMES: [(&[u8], Option<usize>); 8] = [
    (b"fault_2", Some(0)),
    (b"fault_3", Some(1)),
    (b"fault_9", None),
    (b"fault_10", None),
    (b"fault_16", None),
    (b"fault_17", Some(2)),
    (b"fault_18", None),
    (b"fault_19", None),
];

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
    b"step_disc_us",
    b"step_mgmt_us",
    b"step_events_us",
    b"step_send_us",
];

impl Stats {
    /// The names and values of Table M-6, in its order (0074; 46 names, 0080).
    pub fn table_m6(&self) -> [(&'static [u8], u64); 46] {
        let d = &self.drop_port;
        let f = |i: usize| FAULT_NAMES[i].1.map_or(0, |k| self.fault_entries[k]);
        [
            (b"mg_not_request", self.mgmt_dropped_not_request),
            (b"mg_rate_drop", self.mgmt_dropped_rate),
            (b"mg_cap_drop", self.mgmt_dropped_cap),
            (b"mg_auth_required", self.mg_auth_required),
            (b"disc_rate_drop", self.discovery_dropped_rate),
            (b"disc_src1024", self.discovery_from_port_1024),
            (b"setip_ignored", self.set_ip_ignored),
            (b"gen1024_ignored", self.gen1024_ignored),
            (b"cap_rate_drop", self.cap_rate_drop),
            (b"notowner_drop", self.notowner_drop),
            (b"reg7250_drop", self.reg7250_drop),
            (b"takeovers", self.takeovers),
            (b"icmp_unreach", self.icmp_unreach),
            (b"hp_rx", self.hp_rx),
            (b"hp_stale", self.hp_stale),
            (b"hp_resync", self.hp_resync),
            (b"hp_release", self.hp_release),
            (b"gap_1027", self.gap_1027),
            (b"gap_1029", self.gap_1029),
            (b"duc_foreign", self.duc_foreign),
            (b"duc_not_tx", self.duc_not_tx),
            (b"duc_cw_drop", self.duc_cw_drop),
            (b"duc_fifo_drop", self.duc_fifo_drop),
            (b"cw_speed_clamped", self.cw_speed_clamped),
            (b"cw_hang_clamped", self.cw_hang_clamped),
            (b"ll_unprobed", self.ll_unprobed),
            (b"drop_1024", d[0]),
            (b"drop_1025", d[1]),
            (b"drop_1026", d[2]),
            (b"drop_1027", d[3]),
            (b"drop_1029", d[4]),
            (b"drop_7250", d[5]),
            (b"drop_7251", d[6]),
            (FAULT_NAMES[0].0, f(0)),
            (FAULT_NAMES[1].0, f(1)),
            (FAULT_NAMES[2].0, f(2)),
            (FAULT_NAMES[3].0, f(3)),
            (FAULT_NAMES[4].0, f(4)),
            (FAULT_NAMES[5].0, f(5)),
            (FAULT_NAMES[6].0, f(6)),
            (FAULT_NAMES[7].0, f(7)),
            // Causes since power-on (A3.5 "Counters"): unknown is 0 while a start with no flag
            // counts as power loss (0068 addendum); watchdog is the independent and the window one.
            (b"reset_unknown", 0),
            (b"reset_software", self.resets[1]),
            (b"reset_watchdog", self.resets[2] + self.resets[3]),
            (b"reset_pin", self.resets[0]),
            (b"trip_count", self.trip_count),
        ]
    }

    fn encode(&self, out: &mut [u8]) -> usize {
        let mut at = 0;
        // Table M-6 first; it always fits (at most 48 names of at most 25 bytes, Table M-1).
        for (n, v) in self.table_m6() {
            at = mgmt::payload::stat(out, at, n, v).unwrap_or(at);
        }
        // Then this firmware's own; one that does not fit is left out.
        let items: [(&[u8], u64); 21] = [
            (b"mgmt_requests", self.mgmt_requests),
            (b"mgmt_replies", self.mgmt_replies),
            (b"mg_drop_source", self.mgmt_dropped_source),
            (b"mgmt_repeats", self.mgmt_repeats),
            (b"mg_auth_failures", self.mgmt_auth_failures),
            (b"disc_answered", self.discovery_answered),
            (b"busy_pass_max_us", self.net_busy_pass_max_us),
            (b"net_send_dropped", self.net_send_dropped),
            (b"tx_check_1st_us", self.tx_check_first_us),
            (b"resets_low_power", self.resets[4]),
            (b"wake_net_ps", self.wakes_per_s[0]),
            (b"wake_txpath_ps", self.wakes_per_s[1]),
            (b"wake_wdog_ps", self.wakes_per_s[2]),
            (b"wake_console_ps", self.wakes_per_s[3]),
            (b"wake_main_ps", self.wakes_per_s[4]),
            (b"busy_net_us_ps", self.busy_us_per_s[0]),
            (b"busy_txpath_usps", self.busy_us_per_s[1]),
            (b"busy_wdog_us_ps", self.busy_us_per_s[2]),
            (b"busy_cons_us_ps", self.busy_us_per_s[3]),
            (b"busy_main_us_ps", self.busy_us_per_s[4]),
            (b"console_dropped", self.console_dropped),
        ];
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
    /// C-3 is lost (A1.6 "Lost"; 0072 point 5): only a `SET_CONFIG` of every C-3 item ends it.
    pub c3_lost: bool,
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

/// The management server; `W` is the memory of its log ring (the board: RAM kept over a reset).
pub struct Server<W = [u64; logring::WORDS]> {
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
    /// The request being handled refreshed the write session: its idle time does not run until
    /// the reply is sent ([`Self::reply_sent`]; A1.10; 0077 point 2).
    writer_executing: bool,
    /// The radio's address and netmask at the last request: a change of either ends the write
    /// session and drops the challenges and stored replies (A1.10; 0074 point 9).
    own: Option<([u8; 4], [u8; 4])>,
    /// A commanded reset is under way (A1.9 "One commanded reset at a time").
    reset_pending: bool,
    /// The challenge the current authenticated request used (`CHANGE_SECRET` mask, Table M-1).
    used_challenge: [u8; 16],
    /// Effects of the last request for the board (store, network, reset), in order.
    pub effects: heapless::Vec<Effect, 4>,
    pub log: Log<W>,
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
    /// A server with an empty log in its own memory (host tests).
    pub fn new() -> Self {
        Self::with_log(logring::Ring::open([0; logring::WORDS], true).0)
    }
}

impl<W: logring::Words> Server<W> {
    pub const fn with_log(ring: logring::Ring<W>) -> Self {
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
                mg_auth_required: 0,
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
                fault_entries: [0; 3],
                resets: [0; 5],
                wakes_per_s: [0; 5],
                busy_us_per_s: [0; 5],
                console_dropped: 0,
                hp_rx: 0,
                steps: [0; STEPS],
                mgmt_dropped_cap: 0,
                gen1024_ignored: 0,
                cap_rate_drop: 0,
                notowner_drop: 0,
                reg7250_drop: 0,
                icmp_unreach: 0,
                hp_stale: 0,
                hp_resync: 0,
                hp_release: 0,
                gap_1027: 0,
                gap_1029: 0,
                duc_foreign: 0,
                duc_not_tx: 0,
                duc_cw_drop: 0,
                duc_fifo_drop: 0,
                drop_port: [0; 7],
                trip_count: 0,
            },
            writer: None,
            writer_executing: false,
            own: None,
            reset_pending: false,
            used_challenge: [0; 16],
            effects: heapless::Vec::new(),
            log: Log(ring),
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
        self.writer_executing = false;
        // An address or netmask change since the last request (a SET_CONFIG answered first, a
        // DHCP lease, a link-local fall-back): the write session ends and no challenge or stored
        // reply of the old address counts. A renewal with the same address changes nothing
        // (A1.10; 0074 point 9).
        let own = (ctx.own_ip, ctx.own_mask);
        if self.own.is_some_and(|o| o != own) {
            self.writer = None;
            self.challenges = [None; HOSTS];
            self.repeat = [None; HOSTS];
        }
        self.own = Some(own);
        let req = match mgmt::check(d) {
            Incoming::Drop => {
                self.stats.drop_port[6] += 1;
                return None;
            }
            Incoming::DropNotRequest => {
                self.stats.mgmt_dropped_not_request += 1;
                self.stats.drop_port[6] += 1;
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

    /// The reply just built for `src` was sent at `now_ms` (after any flash work of its request,
    /// in the request or after it): the stored reply's time becomes that, so that the A1.4
    /// repeat window counts from the reply. Retransmissions sent during the flash work wait in
    /// the socket buffer and are read only after the reply; they are then repeats, answered
    /// from the stored reply, never executed again (0069 point 3).
    /// For the write-session holder's own request that refreshed the session, the 60 s idle time
    /// runs again from the reply (A1.10; 0077 point 2).
    pub fn reply_sent(&mut self, src: Addr, now_ms: u64) {
        if core::mem::take(&mut self.writer_executing) {
            if let Some((h, t)) = self.writer.as_mut() {
                if *h == src {
                    *t = (*t).max(now_ms);
                }
            }
        }
        if let Some(i) = self.repeat.iter().enumerate().filter_map(|(i, s)| s.filter(|s| s.addr == src).map(|s| (i, s.at_ms))).max_by_key(|x| x.1).map(|x| x.0) {
            if let Some(s) = self.repeat[i].as_mut() {
                s.at_ms = s.at_ms.max(now_ms);
            }
        }
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
            self.stats.drop_port[6] += 1;
        }
        ok
    }

    fn rate_ok(&mut self, src: Addr, identify: bool, ctx: &Ctx<'_>) -> bool {
        let now = ctx.now_ms;
        // 20 replies per second per source (mg_rate_drop), then the caps on IDENTIFY in all and
        // from outside the subnet (mg_cap_drop; A1.10; Table M-6).
        if !self.per_source.allow(src.0, src.1, now) {
            self.stats.mgmt_dropped_rate += 1;
            self.stats.drop_port[6] += 1;
            return false;
        }
        let mut ok = true;
        if identify {
            ok = self.identify_all.allow([0; 4], 0, now);
            let inside = netcfg::valid_mask(ctx.own_mask) && netcfg::in_subnet(ctx.own_ip, ctx.own_mask, src.0);
            if ok && !inside {
                ok = self.identify_off_subnet.allow([0; 4], 0, now);
            }
        }
        if !ok {
            self.stats.mgmt_dropped_cap += 1;
            self.stats.drop_port[6] += 1;
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
            // No trailer, or no secret to verify it with: 32, executed not, counted, and not an
            // authentication failure: toward no 34, no challenge used, no session refreshed
            // (A1.10 "No secret, no HMAC"; 0080).
            let (Some(trailer), Some(secret)) = (req.trailer, ctx.secret) else {
                self.stats.mg_auth_required += 1;
                return (Status::AuthRequired, 0);
            };
            let ok = match self.take_challenge(src, now) {
                Some(ch) => {
                    self.used_challenge = ch;
                    auth::verify(secret, &ch, req.signed, trailer)
                }
                None => false,
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
            Command::GetConfig => write::config_reply(ctx.config, p).map_or((Status::TooLarge, 0), |n| (Status::Ok, n)),
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
