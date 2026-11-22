// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The authenticated commands of milestone M3 (A1.6, A1.9, A1.10, A1.12).
//!
//! Host rules (0070 point 1.2; A1.10):
//! - one fresh challenge per authenticated request: `GET_CHALLENGE`, then the request signed
//!   with HMAC-SHA256(secret, challenge ‖ header ‖ payload), both from the same socket, because
//!   the radio keeps one challenge per IP address and port; a retransmission is the identical
//!   signed datagram (A1.4), never a new challenge;
//! - a write session belongs to the IP address and port of the host (A1.10), and every run of
//!   this tool has its own port, so a command that may have opened one ends it with
//!   `END_WRITE_SESSION` before it exits; `RESTART` and `FACTORY_RESET` end it by the reset;
//! - status 33 is never retried with a new challenge: a lost reply may mean the request was
//!   executed (the stored reply lasts 5 s, a challenge 10 s; A1.4), so it is reported;
//! - replies are not authenticated (A1.10, "Replies are not authenticated"), so a write is
//!   checked by reading the value back where a read exists.

use std::net::SocketAddr;

use g7250_proto::mgmt::auth;
use g7250_proto::mgmt::payload::Config;
use g7250_proto::mgmt::{Command, MAX_REQUEST};
use g7250_proto::mgmt_host;

use crate::reads::{self, Calibration, Failure};
use crate::secret::Secret;
use crate::transport::{Answer, Client};

fn decode(a: &Answer, what: &str) -> Failure {
    Failure::Decode { from: a.from, what: what.to_string() }
}

/// A fresh challenge for the next authenticated request from this socket.
pub fn challenge(client: &mut Client, to: SocketAddr) -> Result<[u8; auth::CHALLENGE_LEN], Failure> {
    let a = reads::call(client, to, Command::GetChallenge, &[])?;
    mgmt_host::parse_challenge(&a.payload).ok_or_else(|| decode(&a, "GET_CHALLENGE reply is not 16 bytes (Table M-1)"))
}

/// One authenticated request with its own challenge. The answer is returned whatever its
/// status; see [`authed`] for one that must succeed.
pub fn authed_raw(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command, payload: &[u8]) -> Result<Answer, Failure> {
    let ch = challenge(client, to)?;
    Ok(client.request(to, cmd, payload, Some((secret.bytes(), &ch)))?)
}

pub fn authed(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command, payload: &[u8]) -> Result<Answer, Failure> {
    let a = authed_raw(client, to, secret, cmd, payload)?;
    if !a.ok() {
        return Err(Failure::Status(a, cmd));
    }
    Ok(a)
}

pub fn end_session(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<(), Failure> {
    authed(client, to, secret, Command::EndWriteSession, &[]).map(|_| ())
}

/// Whether a reply to an authenticated write means the radio may now hold a write session for
/// us: the HMAC was checked and the request not refused for authentication or another
/// writer. A refused write of another kind is treated as possibly opening one (A1.10 does not
/// say), so the session is ended either way.
fn may_hold_session(code: u16) -> bool {
    !matches!(code, 16..=19 | 32..=35)
}

/// An authenticated write, followed by `END_WRITE_SESSION` (its own result ignored if the
/// write already failed; reported if the write succeeded and only the release failed).
pub fn write_once(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command, payload: &[u8]) -> Result<Answer, Failure> {
    let r = authed_raw(client, to, secret, cmd, payload);
    let open = matches!(&r, Ok(a) if may_hold_session(a.status_code));
    let end = if open { Some(end_session(client, to, secret)) } else { None };
    let a = r?;
    if !a.ok() {
        return Err(Failure::Status(a, cmd));
    }
    if let Some(Err(e)) = end {
        return Err(e);
    }
    Ok(a)
}

/// `SET_FIRST_SECRET` (A1.10): unauthenticated, accepted only in the first-secret window. The
/// result is read back from `IDENTIFY` tag 11.
pub fn set_first_secret(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<(), Failure> {
    reads::call(client, to, Command::SetFirstSecret, secret.bytes())?;
    let a = reads::identify(client, to)?;
    let i = g7250_proto::mgmt::payload::Identify::decode(&a.payload).expect("checked by reads::identify");
    if !i.secret_set {
        return Err(decode(&a, "SET_FIRST_SECRET was answered 0, but IDENTIFY says no secret is set"));
    }
    Ok(())
}

/// `CHANGE_SECRET` (A1.10): the new secret masked with HMAC-SHA256(old, "G7NS" ‖ challenge),
/// authenticated with the old secret and the same challenge. Checked, and the write session
/// ended, by an `END_WRITE_SESSION` signed with the new secret.
pub fn change_secret(client: &mut Client, to: SocketAddr, old: &Secret, new: &Secret) -> Result<(), Failure> {
    let ch = challenge(client, to)?;
    let masked = auth::change_secret_mask(old.bytes(), &ch, new.bytes());
    let a = client.request(to, Command::ChangeSecret, &masked, Some((old.bytes(), &ch)))?;
    if !a.ok() {
        if may_hold_session(a.status_code) {
            let _ = end_session(client, to, old);
        }
        return Err(Failure::Status(a, Command::ChangeSecret));
    }
    end_session(client, to, new)
}

/// Where the radio is after a `SET_CONFIG`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Whereabouts {
    /// Read back at this address (the old one, or the new static one).
    At(SocketAddr),
    /// The network changed to DHCP and the radio no longer answers at the old address: find it
    /// with `discover`.
    Unknown,
}

/// The result of a `SET_CONFIG`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigSet {
    /// Every item after the change, from the reply.
    pub after: Config,
    /// The configuration read back with `GET_CONFIG`, if the radio could be reached.
    pub read_back: Option<Config>,
    pub whereabouts: Whereabouts,
}

/// Whether a change touches C-1 (tags 1-4), which the radio applies after its reply (A1.6).
pub fn touches_network(c: &Config) -> bool {
    c.address_mode.is_some() || c.address.is_some() || c.netmask.is_some() || c.gateway.is_some()
}

/// `SET_CONFIG` (A1.6): the items to change, validated as a whole by the radio. Its reply
/// holds every item after the change; the change is also read back with `GET_CONFIG` and
/// every item that was sent must match.
///
/// A network change is applied after the reply (A1.6 C-1, C-4), so the read-back and the
/// `END_WRITE_SESSION` go to the new static address, or to the old one if the radio fell back
/// to it (C-4) or kept it; after a change to DHCP the radio may be at an address only
/// `discover` finds.
pub fn set_config(client: &mut Client, to: SocketAddr, secret: &Secret, change: &Config) -> Result<ConfigSet, Failure> {
    let mut p = [0u8; MAX_REQUEST];
    let n = change.encode(&mut p).ok_or(Failure::Transport(crate::transport::Error::TooLarge))?;
    if !touches_network(change) {
        let a = write_once(client, to, secret, Command::SetConfig, &p[..n])?;
        let after = Config::decode(&a.payload).map_err(|_| decode(&a, "SET_CONFIG reply does not decode (Table M-3)"))?;
        let back = reads::get_config(client, to)?;
        if !applied(change, &back) {
            return Err(Failure::Decode { from: a.from, what: "GET_CONFIG after SET_CONFIG does not show the change".into() });
        }
        return Ok(ConfigSet { after, read_back: Some(back), whereabouts: Whereabouts::At(to) });
    }
    let a = authed(client, to, secret, Command::SetConfig, &p[..n])?;
    let after = Config::decode(&a.payload).map_err(|_| decode(&a, "SET_CONFIG reply does not decode (Table M-3)"))?;
    let mut candidates = Vec::new();
    if after.address_mode == Some(1)
        && let Some(ip) = after.address
    {
        candidates.push(SocketAddr::from((ip, to.port())));
    }
    if !candidates.contains(&to) {
        candidates.push(to);
    }
    for at in candidates {
        match reads::get_config(client, at) {
            Ok(back) => {
                let _ = end_session(client, at, secret);
                if at == to && candidates_changed(&after, to) {
                    // Still at the old address while a new static one was set: the fallback of C-4.
                    return Err(Failure::Decode { from: at, what: "the radio kept its old address: the new network configuration did not come up (A1.6 C-4); see the log".into() });
                }
                if !applied(change, &back) {
                    return Err(Failure::Decode { from: at, what: "GET_CONFIG after SET_CONFIG does not show the change".into() });
                }
                return Ok(ConfigSet { after, read_back: Some(back), whereabouts: Whereabouts::At(at) });
            }
            // No reply, or no route to the new address from here: try the next one.
            Err(Failure::Transport(_)) => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(ConfigSet { after, read_back: None, whereabouts: Whereabouts::Unknown })
}

/// The reply names a static address other than the one the request went to.
fn candidates_changed(after: &Config, to: SocketAddr) -> bool {
    after.address_mode == Some(1) && after.address.is_some_and(|ip| SocketAddr::from((ip, to.port())) != to)
}

/// Every item present in `change` has that value in `got`.
pub fn applied(change: &Config, got: &Config) -> bool {
    fn same<T: PartialEq>(want: &Option<T>, got: &Option<T>) -> bool {
        want.is_none() || want == got
    }
    let allow_ok = match change.allow_count {
        None => true,
        Some(n) => got.allow_count == Some(n) && change.allow[..n as usize] == got.allow[..n as usize],
    };
    same(&change.address_mode, &got.address_mode)
        && same(&change.address, &got.address)
        && same(&change.netmask, &got.netmask)
        && same(&change.gateway, &got.gateway)
        && same(&change.default_level, &got.default_level)
        && same(&change.region, &got.region)
        && same(&change.max_tx_time_ms, &got.max_tx_time_ms)
        && change.band_limit.iter().zip(got.band_limit.iter()).all(|(w, g)| same(w, g))
        && same(&change.swr_s2_x100, &got.swr_s2_x100)
        && same(&change.pa_temp_t3, &got.pa_temp_t3)
        && allow_ok
}

/// `SET_CALIBRATION` (A1.12): replaces the set as a whole; checked with `GET_CALIBRATION`.
/// Returns the set id and whether TX is now calibrated.
pub fn set_calibration(
    client: &mut Client,
    to: SocketAddr,
    secret: &Secret,
    date: u32,
    source: &str,
    entries: &[(String, Vec<u8>)],
) -> Result<([u8; 32], bool), Failure> {
    let mut p = [0u8; MAX_REQUEST];
    let n = mgmt_host::set_calibration_request(date, source.as_bytes(), entries.iter().map(|(k, v)| (k.as_bytes(), v.as_slice())), &mut p)
        .ok_or(Failure::Transport(crate::transport::Error::TooLarge))?;
    let a = write_once(client, to, secret, Command::SetCalibration, &p[..n])?;
    let (id, tx) = mgmt_host::parse_set_calibration_reply(&a.payload).ok_or_else(|| decode(&a, "SET_CALIBRATION reply is not 33 bytes (Table M-1)"))?;
    let back: Calibration = reads::get_calibration(client, to)?;
    if back.id != id || back.date != date || back.source != source || back.entries != entries {
        return Err(Failure::Decode { from: a.from, what: "GET_CALIBRATION after SET_CALIBRATION does not show the new set".into() });
    }
    Ok((id, tx))
}

pub fn clear_calibration(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<(), Failure> {
    let a = write_once(client, to, secret, Command::ClearCalibration, &[])?;
    let back = reads::get_calibration(client, to)?;
    if !back.is_none() {
        return Err(Failure::Decode { from: a.from, what: "GET_CALIBRATION after CLEAR_CALIBRATION still shows a set".into() });
    }
    Ok(())
}

pub fn clear_log(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<(), Failure> {
    write_once(client, to, secret, Command::ClearLog, &[]).map(|_| ())
}

pub fn switch_level(client: &mut Client, to: SocketAddr, secret: &Secret, level: u8) -> Result<(), Failure> {
    write_once(client, to, secret, Command::SwitchLevel, &mgmt_host::switch_level_request(level)).map(|_| ())
}

/// `RESTART` (A1.9) or `FACTORY_RESET` (A1.6a): the reset ends the write session, so no
/// `END_WRITE_SESSION` follows. A refusal (59 RF present, 35, ...) is reported; then the
/// session is ended if one may be held.
pub fn reset(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command) -> Result<(), Failure> {
    debug_assert!(matches!(cmd, Command::Restart | Command::FactoryReset));
    let a = authed_raw(client, to, secret, cmd, &[])?;
    if !a.ok() {
        if cmd == Command::FactoryReset && may_hold_session(a.status_code) {
            let _ = end_session(client, to, secret);
        }
        return Err(Failure::Status(a, cmd));
    }
    Ok(())
}

/// After a reset: wait until the radio answers `IDENTIFY` again with an uptime below `before`
/// (it has restarted), within `limit`. Returns the new uptime in ms.
pub fn wait_for_restart(client: &mut Client, to: SocketAddr, before_ms: u32, limit: std::time::Duration) -> Result<u32, Failure> {
    let end = std::time::Instant::now() + limit;
    loop {
        match reads::identify(client, to) {
            Ok(a) => {
                let i = g7250_proto::mgmt::payload::Identify::decode(&a.payload).expect("checked by reads::identify");
                if i.uptime_ms < before_ms {
                    return Ok(i.uptime_ms);
                }
            }
            Err(Failure::Transport(crate::transport::Error::NoReply { .. })) => {}
            Err(e) => return Err(e),
        }
        if std::time::Instant::now() >= end {
            return Err(Failure::Decode { from: to, what: format!("the radio did not come back within {} s", limit.as_secs()) });
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
