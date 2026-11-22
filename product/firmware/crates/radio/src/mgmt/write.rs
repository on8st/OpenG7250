// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Management writes (spec A1.6, A1.6a, A1.7, A1.9, A1.10, A1.12; Annex M) and the log ring.
//!
//! The server decides and answers; what touches the hardware it returns as [`Effect`]s, which
//! the board carries out: store writes before the reply is sent, network changes and resets after
//! it (A1.6: "a network change is applied after the reply is sent"; A1.9 commanded reset, step 3
//! before step 5). The update commands (`UPDATE_*`, `ROLLBACK`, A1.8) go to the context's
//! [`Updater`](super::Updater).

use super::{Addr, Ctx, Server};
use g7250_proto::mgmt::payload::{self, Config};
use g7250_proto::mgmt::{self, auth, Command, Request, Status};
use g7250_proto::netcfg;

/// Write session idle time (A1.10).
pub const WRITE_SESSION_MS: u64 = 60_000;
/// Built-in maximum continuous TX time, ms (0007 Resolved 3).
const BUILTIN_TX_TIME_MS: u32 = 300_000;
/// Built-in maximum power per band, 0.1 W (Table A3-0: 100 W).
const BUILTIN_BAND_POWER: u16 = 1000;
const TX_BANDS: usize = 10;

/// What the board must do after a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Unkey and drop the arm (`MAINTENANCE`, Table A3-1 event 11) before any flash work of this
    /// request: a `SET_CONFIG` while armed (A1.6; B2.3 "Flash work", 0055). First in the list.
    DisarmForMaintenance,
    /// Store the secret (before the reply).
    SetSecret([u8; 32]),
    /// Store the configuration, the full Table M-3 TLV list (before the reply).
    SetConfig(heapless::Vec<u8, 256>),
    /// Apply the stored network items (C-1) after the reply.
    ApplyNetwork,
    /// Store the calibration set (`SET_CALIBRATION` payload, before the reply).
    SetCalibration(heapless::Vec<u8, 1024>),
    ClearCalibration,
    /// Commanded reset after the reply (A1.9).
    Restart,
    /// Factory reset (Table A1-4) between steps 4 and 5 of the commanded reset, then the reset.
    FactoryReset,
    /// `UPDATE_BEGIN` accepted: the P2 session ends, streams stop, arms are refused with
    /// `MAINTENANCE` until a restart (A1.8; 0022 §10).
    UpdateStarted,
}

const LOG_ENTRIES: usize = 64;
const LOG_TEXT: usize = 72;

/// A slot in a log line is named by its letter (A1.8 "Log names"; 0067).
fn slot_letter(code: u8) -> char {
    if code == 1 { 'A' } else { 'B' }
}

/// RAM log ring (A1.7: kept in RAM, lost at power loss; size a firmware choice).
pub struct Log {
    entries: [(u32, u32, u8, [u8; LOG_TEXT]); LOG_ENTRIES],
    next_seq: u32,
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}

impl Log {
    pub const fn new() -> Self {
        Log { entries: [(0, 0, 0, [0; LOG_TEXT]); LOG_ENTRIES], next_seq: 1 }
    }

    pub fn push(&mut self, uptime_ms: u64, args: core::fmt::Arguments<'_>) {
        use core::fmt::Write;
        let mut s: heapless::String<LOG_TEXT> = heapless::String::new();
        let _ = s.write_fmt(args);
        let mut t = [0u8; LOG_TEXT];
        t[..s.len()].copy_from_slice(s.as_bytes());
        let i = (self.next_seq as usize) % LOG_ENTRIES;
        self.entries[i] = (self.next_seq, uptime_ms as u32, s.len() as u8, t);
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
    }

    pub fn clear(&mut self) {
        for e in &mut self.entries {
            e.0 = 0;
        }
    }

    /// Entries in sequence order from `first` (0 = oldest kept).
    fn from(&self, first: u32) -> impl Iterator<Item = &(u32, u32, u8, [u8; LOG_TEXT])> {
        let oldest = self.next_seq.saturating_sub(LOG_ENTRIES as u32).max(1);
        let start = if first == 0 { oldest } else { first.max(oldest) };
        (start..self.next_seq).filter_map(move |q| {
            let e = &self.entries[(q as usize) % LOG_ENTRIES];
            (e.0 == q).then_some(e)
        })
    }
}

fn ip(a: [u8; 4]) -> (u8, u8, u8, u8) {
    (a[0], a[1], a[2], a[3])
}

impl Server {
    /// `READ_LOG` (Table M-1): as many entries as fit in a reply.
    pub(super) fn read_log(&mut self, req: &[u8], p: &mut [u8]) -> (Status, usize) {
        let Some(first) = req.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) else {
            return (Status::Malformed, 0);
        };
        let room = (mgmt::MAX_REPLY - mgmt::HEADER_LEN).min(p.len());
        let mut at = 0;
        for (seq, up, len, text) in self.log.from(first) {
            let e = payload::LogEntry { seq: *seq, uptime_ms: *up, text: &text[..*len as usize] };
            if at + e.encoded_len() > room {
                break;
            }
            at += e.encode(&mut p[at..]).unwrap_or(0);
        }
        (Status::Ok, at)
    }

    /// `SET_FIRST_SECRET` (A1.10): only in the one-time window while no secret is set.
    pub(super) fn set_first_secret(&mut self, src: Addr, payload: &[u8], ctx: &mut Ctx<'_>) -> (Status, usize) {
        let now = ctx.now_ms;
        let open = self.window(now, ctx.secret.is_some()).0;
        if !open {
            return (Status::FirstSecretClosed, 0);
        }
        // Table A1-3 order: 36 (above), 17, then 50.
        let Ok(secret) = <[u8; 32]>::try_from(payload) else { return (Status::Malformed, 0) };
        // No flash work while TX is armed or keyed (B2.3 "Flash work"; 0055; Table A1-2).
        if ctx.armed {
            return (Status::Armed, 0);
        }
        self.window_used = true;
        let _ = self.effects.push(Effect::SetSecret(secret));
        let (a, b, c, d) = ip(src.0);
        self.log.push(now, format_args!("first secret set by {a}.{b}.{c}.{d}:{}", src.1));
        (Status::Ok, 0)
    }

    /// An authenticated request whose HMAC verified.
    pub(super) fn write(&mut self, src: Addr, cmd: Command, req: &Request<'_>, ctx: &mut Ctx<'_>, p: &mut [u8]) -> (Status, usize) {
        let now = ctx.now_ms;
        // One commanded reset at a time (A1.9): RESTART gets 1, the others 35.
        if self.reset_pending {
            match cmd {
                Command::Restart => return (Status::InProgress, 0),
                Command::FactoryReset | Command::UpdateActivate | Command::Rollback | Command::UpdateBegin => return (Status::Busy, 0),
                _ => {}
            }
        }
        // Write session (A1.10): one holder; RESTART is exempt and neither opens nor refreshes it.
        if cmd != Command::Restart {
            if let Some((h, t)) = self.writer {
                if h != src && now.saturating_sub(t) < WRITE_SESSION_MS {
                    return (Status::Busy, 0);
                }
            }
            self.writer = Some((src, now));
        }
        let (a, b, c, d) = ip(src.0);
        match cmd {
            Command::EndWriteSession => {
                self.writer = None;
                (Status::Ok, 0)
            }
            Command::ChangeSecret => {
                let (Some(old), Ok(v)) = (ctx.secret, <[u8; 32]>::try_from(req.payload)) else { return (Status::Malformed, 0) };
                // No flash work while TX is armed or keyed (B2.3 "Flash work"; 0055; Table
                // A1-2), after 17 (Table A1-3 order).
                if ctx.armed {
                    return (Status::Armed, 0);
                }
                let new = auth::change_secret_mask(old, &self.used_challenge, &v);
                let _ = self.effects.push(Effect::SetSecret(new));
                self.log.push(now, format_args!("secret changed by {a}.{b}.{c}.{d}:{}", src.1));
                (Status::Ok, 0)
            }
            Command::SetConfig => self.set_config(src, req.payload, ctx, p),
            Command::ClearLog => {
                self.log.clear();
                self.log.push(now, format_args!("log cleared by {a}.{b}.{c}.{d}:{}", src.1));
                (Status::Ok, 0)
            }
            Command::Restart => {
                self.reset_pending = true;
                let _ = self.effects.push(Effect::Restart);
                self.log.push(now, format_args!("RESTART by {a}.{b}.{c}.{d}:{}", src.1));
                (Status::Ok, 0)
            }
            Command::FactoryReset => {
                self.reset_pending = true;
                let _ = self.effects.push(Effect::FactoryReset);
                self.log.push(now, format_args!("FACTORY_RESET by {a}.{b}.{c}.{d}:{}", src.1));
                (Status::Ok, 0)
            }
            Command::SwitchLevel if ctx.update.in_progress() => (Status::UpdateInProgress, 0), // T-A1-31
            // One byte (checked as structure, 0056); its value: 1, 2 (only after G6, 0011), else 48.
            Command::SwitchLevel => match payload::parse_switch_level(req.payload) {
                Ok(1) => (Status::Ok, 0),
                Ok(_) => (Status::NotSupported, 0),
                Err(_) => (Status::InvalidConfig, 0),
            },
            Command::SetCalibration | Command::ClearCalibration => {
                // Structure (17) is checked before authentication (0056). Then, in the Table
                // A1-3 order, 58, 57, 50, and the values (48: an id outside Annex P, A1.12).
                if ctx.update.in_progress() {
                    return (Status::UpdateInProgress, 0);
                }
                if ctx.p2_session {
                    return (Status::P2SessionRuns, 0);
                }
                if ctx.armed {
                    return (Status::Armed, 0);
                }
                if cmd == Command::ClearCalibration {
                    let _ = self.effects.push(Effect::ClearCalibration);
                    self.log.push(now, format_args!("calibration cleared by {a}.{b}.{c}.{d}:{}", src.1));
                    return (Status::Ok, 0);
                }
                let known = payload::parse_set_calibration(req.payload)
                    .map(|h| payload::calibration_entries(h.entries).all(|e| e.is_ok_and(|(id, _)| payload::PARAMETER_IDS.iter().any(|k| k.as_bytes() == id))))
                    .unwrap_or(false);
                if !known {
                    return (Status::InvalidConfig, 0);
                }
                // A value that does not decode: 48 (A1.12 item 1). P-RX-2 has its encoding in
                // Table P-1 (0060).
                if payload::parse_set_calibration(req.payload).is_ok_and(|h| crate::calset::rx_cal(h.entries).is_err()) {
                    return (Status::InvalidConfig, 0);
                }
                let mut v = heapless::Vec::new();
                let _ = v.extend_from_slice(req.payload);
                let _ = self.effects.push(Effect::SetCalibration(v));
                self.log.push(now, format_args!("calibration set by {a}.{b}.{c}.{d}:{}", src.1));
                // Reply: set id (SHA-256 of the set), TX calibrated. No parameter encoding of
                // Table P-1 is fixed yet, so no set is complete: 0 (A1.12; Annex P).
                p[..32].copy_from_slice(&auth::sha256(req.payload));
                p[32] = 0;
                (Status::Ok, 33)
            }
            Command::UpdateBegin
            | Command::UpdateBlock
            | Command::UpdateVerify
            | Command::UpdateActivate
            | Command::UpdateAbort
            | Command::Rollback
            | Command::ReturnToStock
            | Command::EnterVendorRecovery => {
                if ctx.armed {
                    return (Status::Armed, 0);
                }
                self.update_command(src, cmd, req.payload, ctx, p)
            }
            _ => (Status::NotSupported, 0),
        }
    }

    /// The update commands (A1.8; Table M-1), unarmed.
    fn update_command(&mut self, src: Addr, cmd: Command, payload: &[u8], ctx: &mut Ctx<'_>, p: &mut [u8]) -> (Status, usize) {
        let now = ctx.now_ms;
        let (a, b, c, d) = ip(src.0);
        match cmd {
            Command::UpdateBegin => {
                let Ok((flags, pkg)) = payload::parse_update_begin(payload) else { return (Status::Malformed, 0) };
                let foreign = flags & payload::BEGIN_FOREIGN != 0;
                let downgrade = flags & payload::BEGIN_DOWNGRADE != 0;
                match ctx.update.begin(foreign, downgrade, &pkg, now) {
                    Ok(count) => {
                        let _ = self.effects.push(Effect::UpdateStarted);
                        let v = pkg.version;
                        let staging = slot_letter(ctx.update.verify_note().0);
                        self.log.push(
                            now,
                            format_args!(
                                "update begun by {a}.{b}.{c}.{d}:{}: version {}.{}.{}{}, slot {}",
                                src.1,
                                v.major,
                                v.minor,
                                v.build,
                                if downgrade { " (downgrade)" } else { "" },
                                staging
                            ),
                        );
                        let r = payload::BeginReply { block_size: mgmt::BLOCK_DATA as u16, block_count: count, one_way: false };
                        p[..7].copy_from_slice(&r.encode());
                        (Status::Ok, 7)
                    }
                    Err(s) => (s, 0),
                }
            }
            Command::UpdateBlock => {
                let Ok(blk) = payload::Block::decode(payload) else { return (Status::Malformed, 0) };
                match ctx.update.block(blk.number, blk.crc_ok(), blk.data, now) {
                    Ok(received) => {
                        p[..8].copy_from_slice(&payload::block_reply(blk.number, received));
                        (Status::Ok, 8)
                    }
                    Err(Status::VerificationFailed) => {
                        // 0043: a write failure discards the transfer and is logged.
                        self.log.push(now, format_args!("update: write of block {} to slot {} failed; transfer discarded", blk.number, slot_letter(ctx.update.verify_note().0)));
                        (Status::VerificationFailed, 0)
                    }
                    Err(s) => (s, 0),
                }
            }
            Command::UpdateVerify => {
                if !payload.is_empty() {
                    return (Status::Malformed, 0);
                }
                let r = ctx.update.verify(now);
                // Every update step is logged (A1.7): a good verify with the slot and version,
                // a failed one with the check that failed.
                let (slot, v, failed) = ctx.update.verify_note();
                let slot = slot_letter(slot);
                match r {
                    Ok(h) => {
                        self.log.push(now, format_args!("update verified: slot {slot} {}.{}.{}", v >> 24, (v >> 16) & 0xFF, v & 0xFFFF));
                        p[..32].copy_from_slice(&h);
                        (Status::Ok, 32)
                    }
                    Err(s) => {
                        if s == Status::VerificationFailed {
                            self.log.push(now, format_args!("update verify failed: {}", failed.unwrap_or("no transfer")));
                        }
                        (s, 0)
                    }
                }
            }
            Command::UpdateActivate => {
                // Flags (one byte, checked as structure, 0056): bit 0 foreign image, not supported in
                // this firmware (A1.8a item 3): 56.
                if payload != [0] {
                    return (Status::NotSupported, 0);
                }
                match ctx.update.activate(now) {
                    Ok(()) => {
                        self.reset_pending = true;
                        let _ = self.effects.push(Effect::Restart);
                        self.log.push(now, format_args!("update activated by {a}.{b}.{c}.{d}:{}: trial boot follows", src.1));
                        p[0] = 0; // not permanent: a trial boot (A1.8a item 5)
                        (Status::Ok, 1)
                    }
                    Err(s) => (s, 0),
                }
            }
            Command::UpdateAbort => {
                if !payload.is_empty() {
                    return (Status::Malformed, 0);
                }
                // A second abort: 0, no effect, not logged again (0063 addendum).
                if ctx.update.abort(now) {
                    self.log.push(now, format_args!("update aborted by {a}.{b}.{c}.{d}:{}", src.1));
                }
                p[0] = 1; // a RESTART is needed to leave the update state (A1.8)
                (Status::Ok, 1)
            }
            Command::Rollback => {
                if !payload.is_empty() {
                    return (Status::Malformed, 0);
                }
                match ctx.update.rollback(now) {
                    Ok(r) => {
                        self.reset_pending = true;
                        let _ = self.effects.push(Effect::Restart);
                        let slot = slot_letter;
                        let v = |x: u32| (x >> 24, (x >> 16) & 0xFF, x & 0xFFFF);
                        let (f, t) = (v(r.from.1), v(r.to.1));
                        self.log.push(
                            now,
                            format_args!(
                                "rollback commanded: slot {} {}.{}.{} to slot {} {}.{}.{} (by {a}.{b}.{c}.{d}:{})",
                                slot(r.from.0),
                                f.0,
                                f.1,
                                f.2,
                                slot(r.to.0),
                                t.0,
                                t.1,
                                t.2,
                                src.1
                            ),
                        );
                        (Status::Ok, 0)
                    }
                    Err(s) => (s, 0),
                }
            }
            // RETURN_TO_STOCK, ENTER_VENDOR_RECOVERY: always 56 (A1.8a items 2, 4).
            _ => (Status::NotSupported, 0),
        }
    }

    /// `SET_CONFIG` (A1.6; Table M-3): validated as a whole, then stored; the reply carries every
    /// item after the change; a network change is applied after the reply.
    fn set_config(&mut self, src: Addr, payload: &[u8], ctx: &mut Ctx<'_>, p: &mut [u8]) -> (Status, usize) {
        let now = ctx.now_ms;
        let Ok(new) = Config::decode(payload) else { return (Status::Malformed, 0) };
        let cur = Config::decode(ctx.config).unwrap_or_default();
        // Region change while a P2 session runs: 57, before the command-specific codes (A1.6 C-6).
        if new.region.is_some_and(|r| Some(r) != cur.region) && ctx.p2_session {
            return (Status::P2SessionRuns, 0);
        }
        // S2 and T3 need the calibration set (Table M-3 note; A1.12).
        if new.touches_calibrated_limits() {
            return (Status::NeedsCalibration, 0);
        }
        let mut m = cur;
        macro_rules! take {
            ($f:ident) => {
                if new.$f.is_some() {
                    m.$f = new.$f;
                }
            };
        }
        take!(address_mode);
        take!(address);
        take!(netmask);
        take!(gateway);
        take!(default_level);
        take!(region);
        take!(max_tx_time_ms);
        for i in 0..payload::MAX_BANDS {
            if new.band_limit[i].is_some() {
                m.band_limit[i] = new.band_limit[i];
            }
        }
        if new.allow_count.is_some() {
            m.allow_count = new.allow_count;
            m.allow = new.allow;
        }
        // Validate the result as a whole (A1.6 C-4): 48 invalid, 49 looser than built-in.
        if let Some(t) = m.max_tx_time_ms {
            if t > BUILTIN_TX_TIME_MS {
                return (Status::NotAllowed, 0);
            }
            if t == 0 {
                return (Status::InvalidConfig, 0);
            }
        }
        for (i, l) in m.band_limit.iter().enumerate() {
            if let Some(l) = l {
                if i >= TX_BANDS {
                    return (Status::InvalidConfig, 0);
                }
                if *l > BUILTIN_BAND_POWER {
                    return (Status::NotAllowed, 0);
                }
            }
        }
        if !matches!(m.address_mode, None | Some(0) | Some(1)) || !matches!(m.default_level, None | Some(1)) || !matches!(m.region, None | Some(0)) {
            return (Status::InvalidConfig, 0);
        }
        if m.address_mode == Some(1) {
            let (Some(a), Some(k)) = (m.address, m.netmask) else { return (Status::InvalidConfig, 0) };
            if !netcfg::valid_host(a, k) {
                return (Status::InvalidConfig, 0);
            }
            if let Some(g) = m.gateway {
                if g != [0; 4] && !netcfg::in_subnet(a, k, g) {
                    return (Status::InvalidConfig, 0);
                }
            }
        }
        if let Some(n) = m.allow_count {
            if m.allow[..n as usize].iter().any(|(_, len)| *len > 32) {
                return (Status::InvalidConfig, 0);
            }
        }
        let mut b = [0u8; 256];
        let Some(n) = m.encode(&mut b) else { return (Status::InvalidConfig, 0) };
        let net_changed = (m.address_mode, m.address, m.netmask, m.gateway) != (cur.address_mode, cur.address, cur.netmask, cur.gateway);
        let mut v = heapless::Vec::new();
        let _ = v.extend_from_slice(&b[..n]);
        // While armed: unkey and disarm first, then write (A1.6; 0055).
        if ctx.armed {
            let _ = self.effects.push(Effect::DisarmForMaintenance);
        }
        let _ = self.effects.push(Effect::SetConfig(v));
        if net_changed {
            let _ = self.effects.push(Effect::ApplyNetwork);
        }
        let (a, bb, c, d) = ip(src.0);
        self.log.push(now, format_args!("configuration changed by {a}.{bb}.{c}.{d}:{}{}", src.1, if net_changed { " (network)" } else { "" }));
        p[..n].copy_from_slice(&b[..n]);
        (Status::Ok, n)
    }
}
