// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Request/response over the management channel (spec A1.4, Table A1-1).
//!
//! Host rules implemented here (0070 point 1.1; T-A1-93):
//! - every new request gets a new request id; the ids of one client increase by one from a
//!   random start;
//! - a retransmission is the identical datagram (same id, command and payload, the same HMAC
//!   trailer), so that the radio answers it from its stored reply and does not run it again;
//! - a retransmission follows every 500 ms without a reply: 5 times for an ordinary request,
//!   and for a request that may do flash work for at least 16 s (0069 item 4);
//! - a reply counts only if it comes from the address the request went to and echoes the
//!   request id and the command; anything else (a late copy of an earlier reply among them)
//!   is ignored, and of several copies of the reply the first is used (0069 item 4).

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use g7250_proto::mgmt::{self, Command, Status};

/// Requests that may do flash work (0069 item 3; spec B2.3 "Flash work").
pub fn may_do_flash_work(cmd: Command) -> bool {
    matches!(
        cmd,
        Command::SetConfig
            | Command::SetFirstSecret
            | Command::ChangeSecret
            | Command::FactoryReset
            | Command::SetCalibration
            | Command::ClearCalibration
            | Command::UpdateBegin
            | Command::UpdateBlock
            | Command::UpdateActivate
            | Command::Rollback
    )
}

/// Retransmission policy. The default is the one of A1.4 and 0069; tests shorten it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Time without a reply before the next send: 500 ms (A1.4).
    pub interval: Duration,
    /// Retransmissions of an ordinary request: 5 (A1.4).
    pub retransmits: u32,
    /// How long a request that may do flash work keeps being retransmitted: 16 s (0069 item 4).
    pub flash_window: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { interval: Duration::from_millis(500), retransmits: 5, flash_window: Duration::from_secs(16) }
    }
}

impl Policy {
    /// Number of times the datagram is sent in total (first send included). The window of a
    /// flash-work request is per command, as 0069 asks: its last send is at least
    /// `flash_window` after the first.
    pub fn sends(&self, cmd: Command) -> u32 {
        let ordinary = 1 + self.retransmits;
        if !may_do_flash_work(cmd) {
            return ordinary;
        }
        let i = self.interval.as_nanos().max(1);
        let n = self.flash_window.as_nanos().div_ceil(i);
        (1 + n.min(u32::MAX as u128 - 1) as u32).max(ordinary)
    }
}

/// A reply, copied out of the receive buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub from: SocketAddr,
    pub request_id: u32,
    pub command: u16,
    /// Protocol version of the replying radio (bytes 4-5); with status 18 it is the radio's own.
    pub version: (u8, u8),
    /// Raw status (bytes 14-15).
    pub status_code: u16,
    /// The status if it is one of Table A1-3.
    pub status: Option<Status>,
    pub payload: Vec<u8>,
    /// How many times the request was sent before this reply came.
    pub sends: u32,
}

impl Answer {
    pub fn ok(&self) -> bool {
        self.status_code == 0
    }
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// The request does not fit the 1000-byte request limit (A1.4).
    TooLarge,
    /// No matching reply after every send of the policy.
    NoReply { to: SocketAddr, command: Command, sends: u32 },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "network error: {e}"),
            Error::TooLarge => write!(f, "request larger than 1000 bytes (A1.4)"),
            Error::NoReply { to, command, sends } => {
                write!(f, "no reply from {to} to {command:?} after {sends} sends")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// A management client: one UDP socket and the request-id counter.
pub struct Client {
    sock: UdpSocket,
    next_id: u32,
    pub policy: Policy,
}

fn random_u32() -> u32 {
    let mut h = RandomState::new().build_hasher();
    h.write_u128(Instant::now().elapsed().as_nanos());
    h.write_u32(std::process::id());
    h.finish() as u32
}

impl Client {
    /// Bind an ephemeral port on every interface.
    pub fn new() -> io::Result<Client> {
        Self::with_socket(UdpSocket::bind("0.0.0.0:0")?)
    }

    pub fn with_socket(sock: UdpSocket) -> io::Result<Client> {
        sock.set_broadcast(true)?;
        Ok(Client { sock, next_id: random_u32(), policy: Policy::default() })
    }

    pub fn socket(&self) -> &UdpSocket {
        &self.sock
    }

    /// The id the next new request will carry.
    pub fn peek_id(&self) -> u32 {
        self.next_id
    }

    /// Take a fresh request id.
    pub fn new_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Build a request datagram with a fresh id.
    pub fn build(&mut self, cmd: Command, payload: &[u8], auth: Option<(&[u8; 32], &[u8; 16])>) -> Result<(u32, Vec<u8>), Error> {
        let id = self.new_id();
        let mut buf = vec![0u8; mgmt::MAX_REQUEST];
        let n = mgmt::build_request(&mut buf, id, cmd, payload, auth).map_err(|_| Error::TooLarge)?;
        buf.truncate(n);
        Ok((id, buf))
    }

    /// Send one request and wait for its reply, retransmitting by the policy.
    pub fn request(&mut self, to: SocketAddr, cmd: Command, payload: &[u8], auth: Option<(&[u8; 32], &[u8; 16])>) -> Result<Answer, Error> {
        let (id, dgram) = self.build(cmd, payload, auth)?;
        self.exchange(to, cmd, id, &dgram)
    }

    /// Send a built datagram and wait for the reply that matches it.
    pub fn exchange(&mut self, to: SocketAddr, cmd: Command, id: u32, dgram: &[u8]) -> Result<Answer, Error> {
        let sends = self.policy.sends(cmd);
        let mut buf = [0u8; 2048];
        for n in 1..=sends {
            self.sock.send_to(dgram, to)?;
            let deadline = Instant::now() + self.policy.interval;
            while let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
                self.sock.set_read_timeout(Some(left))?;
                let (len, from) = match self.sock.recv_from(&mut buf) {
                    Ok(x) => x,
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => break,
                    // Windows reports an ICMP port unreachable from an earlier send this way.
                    Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue,
                    Err(e) => return Err(e.into()),
                };
                if from != to {
                    continue;
                }
                if let Some(a) = matching(&buf[..len], from, id, cmd, n) {
                    return Ok(a);
                }
            }
        }
        Err(Error::NoReply { to, command: cmd, sends })
    }

    /// Receive one datagram within `timeout` (for discovery, which collects several replies).
    pub fn recv(&self, buf: &mut [u8], timeout: Duration) -> io::Result<Option<(usize, SocketAddr)>> {
        if timeout.is_zero() {
            return Ok(None);
        }
        self.sock.set_read_timeout(Some(timeout))?;
        match self.sock.recv_from(buf) {
            Ok(x) => Ok(Some(x)),
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::ConnectionReset) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// The reply to request `id` / `cmd`, or `None` for any other datagram.
pub fn matching(d: &[u8], from: SocketAddr, id: u32, cmd: Command, sends: u32) -> Option<Answer> {
    let r = mgmt::parse_reply(d)?;
    if r.header.request_id != id || r.header.command != cmd as u16 {
        return None;
    }
    Some(Answer {
        from,
        request_id: id,
        command: r.header.command,
        version: (r.header.major, r.header.minor),
        status_code: r.header.status,
        status: r.status,
        payload: r.payload.to_vec(),
        sends,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_counts() {
        let p = Policy::default();
        // A1.4: first send plus 5 retransmissions.
        assert_eq!(p.sends(Command::Identify), 6);
        assert_eq!(p.sends(Command::GetStats), 6);
        assert_eq!(p.sends(Command::UpdateStatus), 6);
        // 0069: every 500 ms for at least 16 s: sends at 0, 0.5, ..., 16.0 s.
        for c in [Command::SetConfig, Command::UpdateBlock, Command::FactoryReset, Command::Rollback] {
            assert_eq!(p.sends(c), 33, "{c:?}");
        }
        let short = Policy { interval: Duration::from_millis(20), retransmits: 5, flash_window: Duration::from_millis(50) };
        assert_eq!(short.sends(Command::UpdateBlock), 6, "never fewer than an ordinary request");
    }

    #[test]
    fn flash_work_list_is_0069_item_3() {
        let flash: Vec<_> = Command::ALL.iter().copied().filter(|c| may_do_flash_work(*c)).collect();
        assert_eq!(flash.len(), 10);
        assert!(!may_do_flash_work(Command::Restart));
        assert!(!may_do_flash_work(Command::UpdateVerify));
    }
}
