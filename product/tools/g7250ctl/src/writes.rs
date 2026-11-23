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
use std::time::Instant;

use g7250_proto::mgmt::auth;
use g7250_proto::mgmt::payload::{Config, Identify};
use g7250_proto::mgmt::{Command, MAX_REQUEST};
use g7250_proto::mgmt_host;

use crate::reads::{self, Calibration, Failure, LogShown, Observed};
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
pub(crate) fn may_hold_session(code: u16) -> bool {
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
/// A network change is applied after the reply (A1.6 C-1, C-4), so the read-back goes to the
/// new static address, or to the old one if the radio fell back to it (C-4) or kept it; after
/// a change to DHCP the radio may be at an address only `discover` finds.
///
/// A change of address or netmask ends the write session and discards the challenges (A1.10;
/// 0074 item 9): then no `END_WRITE_SESSION` is sent, and any later write takes a new challenge
/// (every authenticated request here does). The change is certain when the radio answers at
/// another address, or, with static addressing before and after, its netmask changed. Otherwise
/// (DHCP involved, the C-4 fallback) the session is ended as usual: with no session held it is
/// answered 0 and changes nothing (0074 item 8).
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
    // The address and netmask before, to tell whether the radio ended the session.
    let before = reads::get_config(client, to)?;
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
                // Certain only where the values are the ones in use: the address the radio
                // answers at, and with static addressing before and after, the netmask. A
                // configured value can change while the address in use does not (DHCP to a
                // static address equal to the lease); then the session is ended as usual.
                let both_static = before.address_mode == Some(1) && back.address_mode == Some(1);
                let moved = at != to || (both_static && back.netmask != before.netmask);
                if !moved {
                    let _ = end_session(client, at, secret);
                }
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

/// How a reset request started: answered (wait for the reset from `Before`), or its reply was
/// lost and the reset is already seen, with the evidence that it was this request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetStart {
    Answered(Before),
    Seen { evidence: ResetEvidence },
}

/// `RESTART` (A1.9) or `FACTORY_RESET` (A1.6a): the reset ends the write session, so no
/// `END_WRITE_SESSION` follows. A refusal (59 RF present, 35, ...) is reported; then the
/// session is ended if one may be held.
pub fn reset(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command) -> Result<ResetStart, Failure> {
    debug_assert!(matches!(cmd, Command::Restart | Command::FactoryReset));
    match resetting(client, to, secret, cmd, &[])? {
        Resetting::ResetSeen { evidence } => Ok(ResetStart::Seen { evidence }),
        Resetting::Answered(a, before) if a.ok() => Ok(ResetStart::Answered(before)),
        Resetting::Answered(a, _) => {
            if cmd == Command::FactoryReset && may_hold_session(a.status_code) {
                let _ = end_session(client, to, secret);
            }
            Err(Failure::Status(a, cmd))
        }
    }
}

/// What `IDENTIFY` showed just before a request that ends in a reset, and when the request
/// went out: the evidence a later `IDENTIFY` is compared with to see that the reset happened
/// (A1.4 "Requests that end in a reset"; 0074 item 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Before {
    /// Tag 16 (ms). An uptime below it shows a restart.
    pub uptime_ms: u32,
    /// Tag 23, compared only when `slot_changes`: after `UPDATE_ACTIVATE` and `ROLLBACK` the
    /// other slot runs, so a changed running slot also shows the reset.
    pub running_slot: u8,
    pub slot_changes: bool,
    /// Tag 11: a factory reset clears a secret that was set.
    pub secret_set: bool,
    /// When the request was sent: an uptime below the time since then shows a start after it.
    pub sent: Instant,
}

/// Tag 17 (Table M-2): a commanded reset is a software reset.
pub const RESET_CAUSE_SOFTWARE: u8 = 2;

impl Before {
    /// Taken now, from `IDENTIFY` just before the request.
    pub fn now(i: &Identify, slot_changes: bool) -> Before {
        Before { uptime_ms: i.uptime_ms, running_slot: i.running_slot, slot_changes, secret_set: i.secret_set, sent: Instant::now() }
    }

    fn started_after(&self, i: &Observed) -> bool {
        let since = self.sent.elapsed().as_millis().min(u32::MAX as u128) as u32;
        i.uptime_ms < self.uptime_ms || i.uptime_ms < since
    }

    /// Whether this `IDENTIFY` shows a restart after the request, of any cause: the radio
    /// started after it (uptime below the time since the request, or below the one before), or
    /// for `UPDATE_ACTIVATE` and `ROLLBACK` the other slot runs. For the wait after an answered
    /// request, which ran.
    pub fn restarted(&self, i: &Observed) -> bool {
        self.started_after(i) || (self.slot_changes && i.running_slot != self.running_slot)
    }

    /// Whether this `IDENTIFY` fits that the request ran, when its reply was lost (A1.4
    /// "Requests that end in a reset"; 0074 item 10); which request it was, the log says: the radio started after the request, the
    /// last reset cause is software (tag 17 = 2, exactly), and for `UPDATE_ACTIVATE` and
    /// `ROLLBACK` the running slot changed as well. The version alone is no evidence.
    pub fn ran(&self, i: &Observed) -> bool {
        self.started_after(i) && i.reset_cause == RESET_CAUSE_SOFTWARE && (!self.slot_changes || i.running_slot != self.running_slot)
    }
}

/// The outcome of a request that ends in a commanded reset.
#[derive(Debug)]
pub enum Resetting {
    /// The reply came (any status); with what `IDENTIFY` showed just before the request, for
    /// [`wait_for_restart`].
    Answered(Answer, Before),
    /// The reply was lost, but `IDENTIFY` shows the radio restarted, and `evidence` that it was
    /// this request: it ran.
    ResetSeen { evidence: ResetEvidence },
}

/// What showed, besides `IDENTIFY` ([`Before::ran`]), that a reset was the request's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResetEvidence {
    /// The log's last entry before the start entry, which names the request (A1.7).
    Log(String),
    /// `FACTORY_RESET` with the log lost: the secret that was set before is gone (tag 11 = 0).
    SecretCleared,
}

impl std::fmt::Display for ResetEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResetEvidence::Log(l) => write!(f, "the log says \"{l}\""),
            ResetEvidence::SecretCleared => write!(
                f,
                "the log was lost at the reset; the evidence used is that IDENTIFY showed a secret set before the request and shows none now (tag 11 = 0)"
            ),
        }
    }
}

/// How long [`resetting`] waits to see a reset after a lost reply.
pub const RESET_PROBE: std::time::Duration = std::time::Duration::from_secs(30);

/// An authenticated request that ends in a commanded reset (`RESTART`, `FACTORY_RESET`,
/// `UPDATE_ACTIVATE`, `ROLLBACK`). Its stored reply does not survive the reset (Table A1-4), so
/// if the reply is lost, a retransmission after the reset meets a spent challenge and gets 33,
/// or gets no reply at all while the radio restarts. Either is ambiguous: the request may have
/// run. By the host rule of A1.4 (0074 item 10) retransmitting stops at the first 33 (the
/// first reply ends the exchange), and the radio is then asked with `IDENTIFY` whether it ran
/// ([`Before::ran`]), and the log which request it was ([`log_shown`]): both must show it. With
/// the log lost, a `FACTORY_RESET` is also shown by tag 11 going from set to not set; a secret
/// still set speaks against it, whatever the log says. The request is never sent again with a
/// new challenge here: without that evidence the result is [`Failure::ResetNotShown`], with
/// what was seen.
/// A 33 to the first send is a plain authentication failure.
pub fn resetting(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command, payload: &[u8]) -> Result<Resetting, Failure> {
    let a = reads::identify(client, to)?;
    let before = Before::now(&Identify::decode(&a.payload).expect("checked by reads::identify"), matches!(cmd, Command::UpdateActivate | Command::Rollback));
    let ambiguous = |r: &Result<Answer, Failure>| match r {
        Ok(a) => a.status_code == 33 && a.sends > 1,
        Err(Failure::Transport(crate::transport::Error::NoReply { .. })) => true,
        Err(_) => false,
    };
    let r = authed_raw(client, to, secret, cmd, payload);
    if !ambiguous(&r) {
        return r.map(|a| Resetting::Answered(a, before));
    }
    let seen = wait_for_restart(client, to, before, RESET_PROBE).ok();
    let mut log = None;
    if let Some(o) = seen.filter(|o| before.ran(o)) {
        let factory = cmd == Command::FactoryReset;
        log = match log_shown(client, to, cmd) {
            LogShown::Names(l) if !(factory && o.secret_set) => return Ok(Resetting::ResetSeen { evidence: ResetEvidence::Log(l) }),
            LogShown::Lost if factory && before.secret_set && !o.secret_set => {
                return Ok(Resetting::ResetSeen { evidence: ResetEvidence::SecretCleared });
            }
            other => Some(other),
        };
    }
    let failure = match r {
        Ok(a) => Failure::Status(a, cmd),
        Err(f) => f,
    };
    Err(Failure::ResetNotShown { failure: Box::new(failure), cmd, seen, log })
}

/// What the log shows about the last reset, for `cmd`: the entry just before the last start
/// entry (A1.7 "The reason before a commanded reset": the last entry before the reset; 0068),
/// the start entry's own "log lost", and `start: unexpected stop` after it (0075, 0078).
pub fn log_shown(client: &mut Client, to: SocketAddr, cmd: Command) -> LogShown {
    let mut lines = Vec::new();
    let r = reads::read_log(client, to, 0, &mut |e| {
        if let reads::LogEvent::Entry(l) = e {
            lines.push(l.text)
        }
    });
    if let Err(e) = r {
        return LogShown::Unreadable(e.to_string());
    }
    let Some(start) = lines.iter().rposition(|t| t.starts_with("start after ")) else { return LogShown::NoStart };
    if lines[start].ends_with(": log lost") {
        return LogShown::Lost;
    }
    let before = lines[..start].last().cloned();
    if lines[start + 1..].iter().any(|t| t == "start: unexpected stop") {
        return LogShown::Unexpected { before };
    }
    match before {
        Some(l) if names(&l, cmd) => LogShown::Names(l),
        Some(l) => LogShown::Other(l),
        None => LogShown::NoReason,
    }
}

/// Whether this log line is the reason line of `cmd` (A1.7; 0068).
fn names(line: &str, cmd: Command) -> bool {
    match cmd {
        Command::Restart => line == "restart: restart",
        Command::FactoryReset => line == "restart: factory-reset",
        Command::UpdateActivate => line.starts_with("restart: update-activated "),
        Command::Rollback => line.starts_with("rollback commanded:"),
        _ => false,
    }
}

/// After a reset: wait until the radio answers `IDENTIFY` again showing a restart of any cause
/// ([`Before::restarted`]), within `limit`. Returns what it showed; the caller names a cause
/// other than software.
pub fn wait_for_restart(client: &mut Client, to: SocketAddr, before: Before, limit: std::time::Duration) -> Result<Observed, Failure> {
    let end = std::time::Instant::now() + limit;
    loop {
        match reads::identify(client, to) {
            Ok(a) => {
                let i = Identify::decode(&a.payload).expect("checked by reads::identify");
                let o = Observed { uptime_ms: i.uptime_ms, reset_cause: i.reset_cause, running_slot: i.running_slot, secret_set: i.secret_set };
                if before.restarted(&o) {
                    return Ok(o);
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
