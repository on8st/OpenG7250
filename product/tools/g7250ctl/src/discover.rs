// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Finding radios: `IDENTIFY` to a broadcast address on the management port (A1.5: "MUST
//! also be answered to a broadcast request"), and optionally P2 discovery on port 1024 (B3.2).

use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use g7250_proto::discovery_host;
use g7250_proto::mgmt::{self, Command};

use crate::transport::{matching, Answer, Client};

/// The limited broadcast address (B3.2; A1.5).
pub const LIMITED_BROADCAST: [u8; 4] = [255, 255, 255, 255];

/// Send one `IDENTIFY` (one request id) to every target, retransmit it every policy interval
/// (at most the A1.4 count), and collect every matching reply until `wait` has passed.
/// Replies are keyed by source address; a radio reached through two targets is listed once.
pub fn identify_all(client: &mut Client, targets: &[SocketAddr], wait: Duration) -> io::Result<Vec<Answer>> {
    let (id, dgram) = client.build(Command::Identify, &[], None).map_err(|e| io::Error::other(e.to_string()))?;
    let end = Instant::now() + wait;
    let mut found = BTreeMap::new();
    let mut buf = [0u8; 2048];
    let mut sends = 0u32;
    let mut next_send = Instant::now();
    loop {
        let now = Instant::now();
        if now >= end {
            break;
        }
        if now >= next_send && sends < 1 + client.policy.retransmits {
            for t in targets {
                // A target that is not reachable (no route for a directed broadcast) is skipped.
                let _ = client.socket().send_to(&dgram, t);
            }
            sends += 1;
            next_send = now + client.policy.interval;
        }
        let until = if sends < 1 + client.policy.retransmits { next_send.min(end) } else { end };
        if let Some((n, from)) = client.recv(&mut buf, until.saturating_duration_since(Instant::now()))?
            && let Some(a) = matching(&buf[..n], from, id, Command::Identify, sends)
        {
            found.entry(from).or_insert(a);
        }
    }
    Ok(found.into_values().collect())
}

/// A P2 discovery reply and where it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct P2Found {
    pub from: SocketAddr,
    pub reply: discovery_host::Reply,
}

/// P2 discovery (B3.2): send the request to `targets` (port 1024 normally) from an ephemeral
/// port (never 1024), and collect replies that come from `radio_port` until `wait` has passed.
pub fn p2_all(targets: &[SocketAddr], radio_port: u16, wait: Duration, interval: Duration, retransmits: u32) -> io::Result<Vec<P2Found>> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    let req = discovery_host::request();
    let end = Instant::now() + wait;
    let mut found = BTreeMap::new();
    let mut buf = [0u8; 2048];
    let mut sends = 0u32;
    let mut next_send = Instant::now();
    loop {
        let now = Instant::now();
        if now >= end {
            break;
        }
        if now >= next_send && sends < 1 + retransmits {
            for t in targets {
                let _ = sock.send_to(&req, t);
            }
            sends += 1;
            next_send = now + interval;
        }
        let until = if sends < 1 + retransmits { next_send.min(end) } else { end };
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            continue;
        }
        sock.set_read_timeout(Some(left))?;
        match sock.recv_from(&mut buf) {
            Ok((n, from)) => {
                // A request of ours that loops back has the same shape; only port 1024 answers.
                if from.port() == radio_port
                    && let Some(reply) = discovery_host::parse_reply(&buf[..n])
                {
                    found.entry(from).or_insert(P2Found { from, reply });
                }
            }
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::ConnectionReset) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(found.into_values().collect())
}

/// Default discovery targets: the limited broadcast on `port`, plus any extra addresses.
pub fn targets(extra: &[std::net::Ipv4Addr], port: u16) -> Vec<SocketAddr> {
    let mut v = vec![SocketAddr::from((LIMITED_BROADCAST, port))];
    v.extend(extra.iter().map(|a| SocketAddr::from((a.octets(), port))));
    v.dedup();
    v
}

/// The management port (A1.3), re-exported for callers.
pub const MGMT_PORT: u16 = mgmt::PORT;
