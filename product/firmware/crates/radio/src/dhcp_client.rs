// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! DHCP client state (spec A1.11, A1.11a; RFC 2131): power-on exchange, keep trying while on a
//! link-local address, renewal at T1 (unicast to the server), rebinding at T2 (broadcast), end
//! of lease, DHCPNAK. The board executes the [`Action`]s; it borrows the management socket for
//! each exchange (A1.11) without blocking the P2 sockets.
//!
//! Timers (RFC 2131 sections 4.1, 4.4.5): retransmission of DISCOVER with an exponential back-off
//! 4, 8, 16, 32, 64 s (then every 64 s); T1 = lease / 2 and T2 = lease × 7/8 when the server
//! gives no option 58/59; a reply is awaited for [`WAIT_MS`].

use g7250_proto::dhcp::{self, Reply};
use g7250_proto::netcfg::{Ip, Mode, NetConfig};

/// Longest borrow of the management socket by one exchange, from its first send (0041: at most
/// 2 s, since a host gives up after about 2.5 s). An exchange not finished by then is abandoned
/// and retried at the next interval.
pub const WAIT_MS: u64 = 2_000;
const BACKOFF_MS: [u64; 5] = [4_000, 8_000, 16_000, 32_000, 64_000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// No lease: link-local address in use; next DISCOVER at `next_ms`.
    Init { attempt: u8, next_ms: u64 },
    /// DISCOVER sent, waiting for an OFFER until `until_ms`.
    Selecting { attempt: u8, until_ms: u64 },
    /// REQUEST sent for `offer`, waiting for the ACK.
    Requesting { attempt: u8, until_ms: u64, offer: Reply },
    /// Lease held.
    Bound,
    /// REQUEST unicast to the server, waiting until `until_ms`.
    Renewing { until_ms: u64 },
    /// REQUEST broadcast, waiting until `until_ms`.
    Rebinding { until_ms: u64 },
}

/// What the board must do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Borrow the management socket (port 68) and send this message to `to`:67; keep listening
    /// on 68 until [`Client::listening`] is false, then give the socket back to 7251.
    Send { to: Ip, msg: Msg },
    /// Use this configuration (a lease, or the link-local fall-back): an address change under
    /// A1.11a if it differs from the one in use.
    Use(NetConfig),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    Discover,
    Request { offered: Ip, server: Ip },
    Renew { ciaddr: Ip },
}

impl Msg {
    pub fn encode(&self, xid: u32, mac: [u8; 6], buf: &mut [u8; dhcp::MSG_LEN]) -> usize {
        match *self {
            Msg::Discover => dhcp::discover(xid, mac, buf),
            Msg::Request { offered, server } => dhcp::request(xid, mac, offered, server, buf),
            Msg::Renew { ciaddr } => dhcp::renew(xid, mac, ciaddr, buf),
        }
    }
}

pub struct Client {
    pub state: State,
    pub xid: u32,
    mac: [u8; 6],
    link_local: NetConfig,
    /// The lease in use: config, server, time it was obtained, and its length.
    lease: Option<(NetConfig, Ip, u64, u32)>,
    /// After a renewal without answer: no new try before this time.
    retry_at: Option<u64>,
}

impl Client {
    /// After the power-on exchange: `lease` if it gave one (config, server, lease seconds), else
    /// the radio is on `link_local` and keeps trying.
    pub fn new(mac: [u8; 6], xid: u32, link_local: NetConfig, lease: Option<(NetConfig, Ip, u32)>, now: u64) -> Self {
        let (state, lease) = match lease {
            Some((c, s, l)) => (State::Bound, Some((c, s, now, l))),
            None => (State::Init { attempt: 0, next_ms: now + BACKOFF_MS[0] }, None),
        };
        Client { state, xid, mac, link_local, lease, retry_at: None }
    }

    /// The lease in use, if any: configuration and server.
    pub fn lease(&self) -> Option<(NetConfig, Ip)> {
        self.lease.map(|(c, s, _, _)| (c, s))
    }

    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }

    /// The borrowed socket is still needed.
    pub fn listening(&self) -> bool {
        matches!(self.state, State::Selecting { .. } | State::Requesting { .. } | State::Renewing { .. } | State::Rebinding { .. })
    }

    fn t1_t2_end(&self) -> Option<(u64, u64, u64)> {
        self.lease.map(|(_, _, t0, l)| {
            let l = l as u64 * 1000;
            (t0 + l / 2, t0 + l * 7 / 8, t0 + l)
        })
    }

    fn backoff(attempt: u8) -> u64 {
        BACKOFF_MS[(attempt as usize).min(BACKOFF_MS.len() - 1)]
    }

    /// Call often.
    pub fn poll(&mut self, now: u64) -> Action {
        match self.state {
            State::Init { attempt, next_ms } if now >= next_ms => {
                self.xid = self.xid.wrapping_add(1);
                self.state = State::Selecting { attempt, until_ms: now + WAIT_MS };
                Action::Send { to: [255; 4], msg: Msg::Discover }
            }
            State::Selecting { attempt, until_ms } | State::Requesting { attempt, until_ms, .. } if now >= until_ms => {
                let a = attempt.saturating_add(1);
                self.state = State::Init { attempt: a, next_ms: now + Self::backoff(a) };
                Action::None
            }
            State::Bound | State::Renewing { .. } | State::Rebinding { .. } => {
                let Some((t1, t2, end)) = self.t1_t2_end() else { return Action::None };
                let (cfg, server, _, _) = self.lease.unwrap();
                if now >= end {
                    // Lease ended without a new one: stop using the address (A1.11a), back to
                    // link-local, keep trying.
                    self.lease = None;
                    self.state = State::Init { attempt: 0, next_ms: now };
                    return Action::Use(self.link_local);
                }
                let paused = self.retry_at.is_some_and(|t| now < t);
                match self.state {
                    State::Bound if paused => Action::None,
                    State::Bound if now >= t2 => {
                        self.xid = self.xid.wrapping_add(1);
                        self.state = State::Rebinding { until_ms: now + WAIT_MS };
                        Action::Send { to: [255; 4], msg: Msg::Renew { ciaddr: cfg.ip } }
                    }
                    State::Bound if now >= t1 => {
                        self.xid = self.xid.wrapping_add(1);
                        self.state = State::Renewing { until_ms: now + WAIT_MS };
                        Action::Send { to: server, msg: Msg::Renew { ciaddr: cfg.ip } }
                    }
                    State::Renewing { until_ms } | State::Rebinding { until_ms } if now >= until_ms => {
                        // No answer: back to Bound and try again after a pause (RFC 2131
                        // section 4.4.5): half the time remaining until T2 while renewing, until
                        // the lease end while rebinding, at least 60 s, but never past that
                        // point, so that T2 always gives the broadcast REQUEST (DH-03).
                        let limit = if matches!(self.state, State::Renewing { .. }) && now < t2 { t2 } else { end };
                        let remaining = limit.saturating_sub(now);
                        let pause = (remaining / 2).max(60_000).min(remaining);
                        self.state = State::Bound;
                        self.retry_at = Some(now + pause);
                        Action::None
                    }
                    _ => Action::None,
                }
            }
            _ => Action::None,
        }
    }

    /// A DHCP reply on the borrowed socket.
    pub fn on_reply(&mut self, b: &[u8], now: u64) -> Action {
        let Some(r) = dhcp::parse(b, self.xid, self.mac) else { return Action::None };
        match self.state {
            State::Selecting { attempt, until_ms } if r.msg_type == dhcp::OFFER => {
                // Same deadline: the whole DISCOVER-OFFER-REQUEST-ACK exchange is one borrow.
                let _ = now;
                self.state = State::Requesting { attempt, until_ms, offer: r };
                Action::Send { to: [255; 4], msg: Msg::Request { offered: r.yiaddr, server: r.server } }
            }
            State::Requesting { .. } | State::Renewing { .. } | State::Rebinding { .. } if r.msg_type == dhcp::ACK => {
                let mask = r.mask.or(match self.state {
                    State::Requesting { offer, .. } => offer.mask,
                    _ => self.lease.map(|l| l.0.mask),
                });
                let mask = mask.unwrap_or([255, 255, 255, 0]);
                let gw = r.router.or(self.lease.map(|l| l.0.gateway)).unwrap_or([0; 4]);
                if !g7250_proto::netcfg::valid_host(r.yiaddr, mask) {
                    return Action::None;
                }
                let server = if r.server == [0; 4] { self.lease.map_or([0; 4], |l| l.1) } else { r.server };
                let cfg = NetConfig { mode: Mode::Dhcp, ip: r.yiaddr, mask, gateway: gw };
                let lease_s = r.lease_s.unwrap_or(3600).max(60);
                self.lease = Some((cfg, server, now, lease_s));
                self.state = State::Bound;
                self.retry_at = None;
                // A renewal that keeps address and netmask changes nothing (A1.11a); the board
                // compares with the configuration in use.
                Action::Use(cfg)
            }
            State::Requesting { attempt, .. } if r.msg_type == dhcp::NAK => {
                let a = attempt.saturating_add(1);
                self.state = State::Init { attempt: a, next_ms: now + Self::backoff(a) };
                Action::None
            }
            State::Renewing { .. } | State::Rebinding { .. } if r.msg_type == dhcp::NAK => {
                // DHCPNAK to a renewal: stop using the address (A1.11a), link-local, start over.
                self.lease = None;
                self.state = State::Init { attempt: 0, next_ms: now };
                Action::Use(self.link_local)
            }
            _ => Action::None,
        }
    }
}
