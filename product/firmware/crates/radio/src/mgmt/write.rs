// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Management writes (spec A1.6, A1.6a, A1.7, A1.9, A1.10, A1.12; Annex M) and the log ring.
//!
//! The server decides and answers; what touches the hardware it returns as [`Effect`]s, which
//! the board carries out: store writes before the reply is sent, network changes and resets after
//! it (A1.6: "a network change is applied after the reply is sent"; A1.9 commanded reset, step 3
//! before step 5). The update commands (`UPDATE_*`, `ROLLBACK`, A1.8) go to the context's
//! [`Updater`](super::Updater).

use g7250_safety::logring::{Entry, Ring, Words};
use super::{Addr, Ctx, Server};
use g7250_proto::mgmt::payload::{self, Config};
use g7250_proto::mgmt::{self, auth, Command, Request, Status};
use g7250_proto::netcfg;

/// Write session idle time (A1.10).
pub const WRITE_SESSION_MS: u64 = 60_000;
/// Built-in maximum continuous TX time, ms (0007 Resolved 3).
const BUILTIN_TX_TIME_MS: u32 = 300_000;
/// Built-in maximum power of band `i`, 0.1 W: the maximum power of its `tx_band` entry (0079;
/// B4.8), so 0 at 50 MHz until P-PWR-2. `None` for an index with no band.
fn builtin_band_power(i: usize) -> Option<u16> {
    crate::record::TX_BANDS.get(i).map(|b| b.2)
}

/// The configuration as `GET_CONFIG` and `SET_CONFIG` answer it (Table M-3, every item): the
/// stored items, and tag 8 for every `tx_band` the stored record does not tighten, with the
/// persistent value, which is then the built-in maximum of the capability record (A3.4 rule 1:
/// "equal to the built-in value unless tightened"; [`crate::record::TX_BANDS`], SSB/CW/FM). Only
/// the answer: the stored record keeps what was written. A record that does not decode is answered
/// as it is.
pub(super) fn config_reply(stored: &[u8], p: &mut [u8]) -> Option<usize> {
    let Ok(mut c) = Config::decode(stored) else {
        return (stored.len() <= p.len()).then(|| {
            p[..stored.len()].copy_from_slice(stored);
            stored.len()
        });
    };
    for (i, band) in crate::record::TX_BANDS.iter().enumerate() {
        if c.band_limit[i].is_none() {
            c.band_limit[i] = Some(band.2);
        }
    }
    c.encode(p)
}

/// The request holds a C-3 item (A1.6 C-3: tags 7 to 10).
fn has_c3(c: &Config) -> bool {
    c.max_tx_time_ms.is_some() || c.band_limit.iter().any(|l| l.is_some()) || c.swr_s2_x100.is_some() || c.pa_temp_t3.is_some()
}

/// The request holds every C-3 item: the TX time, the power limit of every band, S2 and T3.
fn every_c3(c: &Config) -> bool {
    c.max_tx_time_ms.is_some() && c.band_limit[..crate::record::TX_BANDS.len()].iter().all(|l| l.is_some()) && c.swr_s2_x100.is_some() && c.pa_temp_t3.is_some()
}

/// What the board must do after a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Unkey and drop the arm (`MAINTENANCE`, Table A3-1 event 11) before any flash work of this
    /// request: a `SET_CONFIG` while armed (A1.6; B2.3 "Flash work", 0055). First in the list.
    DisarmForMaintenance,
    /// Store the secret (before the reply).
    SetSecret([u8; 32]),
    /// Store the configuration, the full Table M-3 TLV list (before the reply); `true` if the
    /// request held a C-3 item, which is then written by a compaction (A1.6 C-3; 0072 point 1).
    SetConfig(heapless::Vec<u8, 256>, bool),
    /// Apply the stored network items (C-1) after the reply.
    ApplyNetwork,
    /// Store the calibration set (`SET_CALIBRATION` payload, before the reply).
    SetCalibration(heapless::Vec<u8, 1024>),
    ClearCalibration,
    /// Commanded reset after the reply (A1.9).
    Restart(Reason),
    /// Factory reset (Table A1-4) between steps 4 and 5 of the commanded reset, then the reset.
    FactoryReset,
    /// `UPDATE_BEGIN` accepted: the P2 session ends, streams stop, arms are refused with
    /// `MAINTENANCE` until a restart (A1.8; 0022 §10).
    UpdateStarted,
}

/// A slot in a log line is named by its letter (A1.8 "Log names"; 0067).
fn slot_letter(code: u8) -> char {
    if code == 1 { 'A' } else { 'B' }
}

/// Why the firmware resets (A1.7 "The reason before a commanded reset"; 0068 point 3): the last
/// entry of the run, written after the unkey and RF gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Restart,
    FactoryReset,
    /// `UPDATE_ACTIVATE`: the slot (1 A, 2 B) and version that start as the trial.
    Activated { slot: u8, version: u32 },
    /// `ROLLBACK`: the line of A1.8 "`ROLLBACK`".
    Rollback(super::Rollback),
    UpdateAbandoned,
    TrialUnconfirmed,
}

impl Reason {
    /// Write the reason entry.
    pub fn log<W: Words>(self, r: &mut Ring<W>, uptime_ms: u32) {
        match self {
            Reason::Restart => r.log(uptime_ms, format_args!("restart: restart")),
            Reason::FactoryReset => r.log(uptime_ms, format_args!("restart: factory-reset")),
            Reason::Activated { slot, version: v } => {
                r.log(uptime_ms, format_args!("restart: update-activated slot {} {}.{}.{}", slot_letter(slot), v >> 24, (v >> 16) & 0xFF, v & 0xFFFF));
            }
            Reason::Rollback(super::Rollback { from: (f, fv), to: (t, tv) }) => r.log(
                uptime_ms,
                format_args!(
                    "rollback commanded: slot {} {}.{}.{} to slot {} {}.{}.{}",
                    slot_letter(f),
                    fv >> 24,
                    (fv >> 16) & 0xFF,
                    fv & 0xFFFF,
                    slot_letter(t),
                    tv >> 24,
                    (tv >> 16) & 0xFF,
                    tv & 0xFFFF
                ),
            ),
            Reason::UpdateAbandoned => r.log(uptime_ms, format_args!("restart: update-abandoned")),
            Reason::TrialUnconfirmed => r.log(uptime_ms, format_args!("restart: trial-unconfirmed")),
        }
    }
}

/// The log (A1.7), on the ring kept over a reset (g7250_safety::logring; 0068).
pub struct Log<W>(pub Ring<W>);

impl<W: Words> Log<W> {
    pub fn push(&mut self, uptime_ms: u64, args: core::fmt::Arguments<'_>) {
        self.0.log(uptime_ms as u32, args);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    /// Entries in sequence order from `first`, or from the oldest kept if that is newer.
    fn from(&self, first: u32) -> impl Iterator<Item = Entry> + '_ {
        let (oldest, next) = self.0.range();
        (first.max(oldest)..next).filter_map(|q| self.0.entry(q))
    }
}

fn ip(a: [u8; 4]) -> (u8, u8, u8, u8) {
    (a[0], a[1], a[2], a[3])
}

impl<W: Words> Server<W> {
    /// `READ_LOG` (Table M-1): as many entries as fit in a reply.
    pub(super) fn read_log(&mut self, req: &[u8], p: &mut [u8]) -> (Status, usize) {
        let Some(first) = req.get(..4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) else {
            return (Status::Malformed, 0);
        };
        let room = (mgmt::MAX_REPLY - mgmt::HEADER_LEN).min(p.len());
        let mut at = 0;
        for l in self.log.from(first) {
            let e = payload::LogEntry { seq: l.seq, uptime_ms: l.uptime_ms, text: l.text() };
            if at + e.encoded_len() > room {
                break;
            }
            at += e.encode(&mut p[at..]).unwrap_or(0);
        }
        (Status::Ok, at)
    }

    /// `UPDATE_BLOCK` or `UPDATE_VERIFY` made room in the store (0067 addendum).
    fn note_room(&mut self, now: u64, ctx: &mut Ctx<'_>) {
        if ctx.update.room_made() {
            self.log.push(now, format_args!("store: room made (update)"));
        }
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
            self.writer_executing = true;
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
                let _ = self.effects.push(Effect::Restart(Reason::Restart));
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
                let r = ctx.update.block(blk.number, blk.crc_ok(), blk.data, now);
                self.note_room(now, ctx);
                match r {
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
                self.note_room(now, ctx);
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
                        // The check that failed; a write failure is logged as such, and a verify
                        // with no transfer changes nothing.
                        if let (Status::VerificationFailed, Some(word)) = (s, failed) {
                            self.log.push(now, format_args!("update verify failed: {word}"));
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
                        let (slot, version, _) = ctx.update.verify_note();
                        let _ = self.effects.push(Effect::Restart(Reason::Activated { slot, version }));
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
                // No update state (no UPDATE_BEGIN accepted since the start, also in a trial
                // image): 0 with restart needed 0; nothing changes, no 10-minute restart (A1.8;
                // 0074).
                if !ctx.update.begun() {
                    p[0] = 0;
                    return (Status::Ok, 1);
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
                        // Its line of A1.8 is the last entry before the reset (0068 addendum).
                        let _ = self.effects.push(Effect::Restart(Reason::Rollback(r)));
                        self.log.push(now, format_args!("ROLLBACK by {a}.{b}.{c}.{d}:{}", src.1));
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
        // While C-3 is lost, only a write of every C-3 item ends it; one of some gets 48, before
        // the 62 of S2 and T3 (A1.6 "Lost"; 0072 points 5, A, G).
        if ctx.c3_lost && has_c3(&new) && !every_c3(&new) {
            return (Status::InvalidConfig, 0);
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
                let Some(max) = builtin_band_power(i) else { return (Status::InvalidConfig, 0) };
                if *l > max {
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
        let Some(r) = config_reply(&b[..n], p) else { return (Status::TooLarge, 0) };
        let net_changed = (m.address_mode, m.address, m.netmask, m.gateway) != (cur.address_mode, cur.address, cur.netmask, cur.gateway);
        let mut v = heapless::Vec::new();
        let _ = v.extend_from_slice(&b[..n]);
        // While armed: unkey and disarm first, then write (A1.6; 0055).
        if ctx.armed {
            let _ = self.effects.push(Effect::DisarmForMaintenance);
        }
        let _ = self.effects.push(Effect::SetConfig(v, has_c3(&new)));
        if net_changed {
            let _ = self.effects.push(Effect::ApplyNetwork);
        }
        let (a, bb, c, d) = ip(src.0);
        self.log.push(now, format_args!("configuration changed by {a}.{bb}.{c}.{d}:{}{}", src.1, if net_changed { " (network)" } else { "" }));
        // A C-3 loosening toward the built-in value: one entry per item, with the old value, the
        // new value and the source (A1.6 C-3; T-A1-28). Values as in Table M-3; the longest entry
        // ("band_limit[15] 1000 -> 1000" with a 21-character source) fits the 72 bytes of a line.
        let mut loosened = |item: core::fmt::Arguments<'_>, old: u32, new: u32| {
            if new > old {
                self.log.push(now, format_args!("C-3 loosened: {item} {old} -> {new} by {a}.{bb}.{c}.{d}:{}", src.1));
            }
        };
        if let (Some(o), Some(n)) = (cur.max_tx_time_ms, new.max_tx_time_ms) {
            loosened(format_args!("max_tx_time_ms"), o, n);
        }
        for i in 0..payload::MAX_BANDS {
            if let (Some(o), Some(n)) = (cur.band_limit[i], new.band_limit[i]) {
                loosened(format_args!("band_limit[{i}]"), o as u32, n as u32);
            }
        }
        if let (Some(o), Some(n)) = (cur.swr_s2_x100, new.swr_s2_x100) {
            loosened(format_args!("swr_s2_x100"), o as u32, n as u32);
        }
        if let (Some(o), Some(n)) = (cur.pa_temp_t3, new.pa_temp_t3) {
            loosened(format_args!("pa_temp_t3"), o as u32, n as u32);
        }
        (Status::Ok, r)
    }
}
