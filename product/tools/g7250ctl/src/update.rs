// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Firmware update of milestone M4 (A1.8; Annex M "Package file"; 0043, 0063, 0064, 0067,
//! 0071).
//!
//! Host rules (0070 point 1.3, 1.4; T-A1-93):
//! - a package file is checked (length, magic, header version, signature length, hash) before
//!   anything is sent;
//! - the package for the slot that is not running is sent (`IDENTIFY` tag 23);
//! - every request is authenticated with its own challenge; a block is sent as block *n* =
//!   image bytes 512 *n* … 512 *n* + 511;
//! - status 64 to a block: nothing taken, the same block is sent again as a new request (0067
//!   addendum); a second 64 in a row for that block ends the install;
//! - status 53 to a block: sent again, at most twice;
//! - a failed verify (55): every block sent again, in order, with new request ids, then a new
//!   verify (0067 "Blocks sent again");
//! - a status that ends the transfer: `UPDATE_ABORT` (the radio then needs a restart, A1.8);
//!   a radio that stops answering is left alone, so the transfer can be resumed;
//! - after `UPDATE_ACTIVATE` the result is read back with `IDENTIFY` after the reset: the new
//!   image runs in the target slot, then confirms itself or is rolled back (U-6, 0044).

use std::net::SocketAddr;
use std::path::Path;
use std::time::{Duration, Instant};

use g7250_proto::mgmt::payload::{self, BeginReply, Block, Identify, Package, UpdateStatus, Version};
use g7250_proto::mgmt::{Command, BLOCK_DATA};
use g7250_proto::mgmt_host::{self, PackageFileError};

use crate::reads::{self, Failure};
use crate::secret::Secret;
use crate::transport::{self, Answer, Client};
use crate::writes;

/// Status 64: nothing taken, room made in the store; send the block again (0067 addendum).
pub const STATUS_ROOM_MADE: u16 = 64;
const STATUS_BAD_CRC: u16 = 53;
const STATUS_VERIFY_FAILED: u16 = 55;

/// A checked package file.
#[derive(Clone, Debug)]
pub struct PackageFile {
    pub name: String,
    pub header: Package,
    bytes: Vec<u8>,
}

impl PackageFile {
    pub fn image(&self) -> &[u8] {
        &self.bytes[payload::PACKAGE_LEN..]
    }
    pub fn blocks(&self) -> u32 {
        mgmt_host::block_count(self.image().len())
    }
    pub fn header_bytes(&self) -> &[u8] {
        &self.bytes[..payload::PACKAGE_LEN]
    }

    pub fn from_bytes(name: &str, bytes: Vec<u8>) -> Result<PackageFile, String> {
        let header = match mgmt_host::parse_package_file(&bytes) {
            Ok((h, _)) => h,
            Err(e) => return Err(format!("{name}: refused, nothing sent: {}", package_error(e))),
        };
        Ok(PackageFile { name: name.to_string(), header, bytes })
    }

    pub fn load(path: &Path) -> Result<PackageFile, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_bytes(&path.display().to_string(), bytes)
    }
}

pub fn package_error(e: PackageFileError) -> String {
    match e {
        PackageFileError::Length { file, expected: None } => format!("{file} bytes, shorter than the 53-byte header"),
        PackageFileError::Length { file, expected: Some(x) } => format!("{file} bytes, the header gives 53 + image length = {x}"),
        PackageFileError::Magic => "not a package file (magic is not G7PK)".into(),
        PackageFileError::HeaderVersion(v) => format!("header version {v}, this tool reads version 1"),
        PackageFileError::SignatureLength(n) => format!("signature length {n}; this version carries no signature (U-4)"),
        PackageFileError::Hash => "the image's SHA-256 is not the one in the header".into(),
    }
}

pub fn slot_name(s: u8) -> &'static str {
    match s {
        0 => "either",
        1 => "A",
        2 => "B",
        _ => "?",
    }
}

pub fn version(v: Version) -> String {
    format!("{}.{}.{}", v.major, v.minor, v.build)
}

/// The package to stage, given the running slot (A1.8 "Slot-linked images"; 0043): the one
/// linked for the other slot, else one that runs in either slot.
pub fn choose(packages: &[PackageFile], running_slot: u8) -> Result<&PackageFile, String> {
    let other = match running_slot {
        1 => 2,
        2 => 1,
        s => return Err(format!("IDENTIFY reports running slot {s}; cannot pick a package (tag 23)")),
    };
    packages
        .iter()
        .find(|p| p.header.target_slot == other)
        .or_else(|| packages.iter().find(|p| p.header.target_slot == 0))
        .ok_or_else(|| {
            format!(
                "the radio runs in slot {}; give the package for slot {} (the ones given are for slot {})",
                slot_name(running_slot),
                slot_name(other),
                packages.iter().map(|p| slot_name(p.header.target_slot)).collect::<Vec<_>>().join(", ")
            )
        })
}

/// What an update reports while it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Chosen { name: String, slot: u8, version: Version, running: Version },
    Begun { blocks: u32 },
    Resumed { from: u32, blocks: u32 },
    Block { n: u32, received: u32, blocks: u32 },
    RoomMade { n: u32 },
    CrcResent { n: u32 },
    VerifyFailed { round: u32 },
    Verified,
    Activated,
    /// The reply to `UPDATE_ACTIVATE` or `ROLLBACK` was lost, and `IDENTIFY` shows the reset
    /// (A1.4 "Requests that end in a reset"); with the log's reason line if found.
    ReplyLostResetSeen { reason: Option<String> },
    /// The radio answers again after the reset; with tag 17 when this wait saw the restart.
    Restarted { cause: Option<u8> },
    Trial,
    Confirmed,
    Aborted { restart_needed: bool },
    /// The radio stopped answering during the transfer; it is left as it is, so that
    /// `install --resume` can continue it within 10 min (A1.8).
    LeftForResume,
    /// Status 55 to a block: writing the staging slot failed, and the radio has discarded the
    /// transfer (A1.8 "Write failure"; 0043). Nothing to abort; the update state, with RX
    /// stopped, lasts until a restart or a new install (`UPDATE_BEGIN`).
    WriteFailed,
    /// After a failure, `UPDATE_STATUS` shows no transfer and `IDENTIFY` a lower uptime: the
    /// radio restarted during the transfer (RX runs again). Nothing to abort.
    RadioRestarted,
    /// After a failure, `UPDATE_STATUS` shows no transfer, and `IDENTIFY` shows no restart (or
    /// did not answer): the cause is unknown. Nothing is aborted.
    NoTransfer { state: u8 },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    /// Set the downgrade flag (A1.8, U-8): needed for an image older than the running one.
    pub downgrade: bool,
    /// Continue a transfer that `UPDATE_STATUS` shows in progress for this package. Only the
    /// block count can be compared (`UPDATE_STATUS` gives no hash): a transfer of another
    /// package with the same count is continued too, and then fails at the verify (55).
    pub resume: bool,
    /// How many times a failed verify leads to sending every block again (0067).
    pub verify_retries: u32,
    /// Wait for the restart and the confirmation after the activation.
    pub wait: Option<Waits>,
}

#[derive(Clone, Copy, Debug)]
pub struct Waits {
    /// Until the radio answers again after the reset.
    pub restart: Duration,
    /// Until the trial image has confirmed itself (0044: 10 s; rollback at 60 s).
    pub confirm: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Waits { restart: Duration::from_secs(30), confirm: Duration::from_secs(75) }
    }
}

/// How an install ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Activated; not waited for.
    Activated,
    /// Runs in the target slot and has confirmed itself.
    Confirmed,
}

fn decode(a: &Answer, what: &str) -> Failure {
    Failure::Decode { from: a.from, what: what.to_string() }
}

fn other(what: String, to: SocketAddr) -> Failure {
    Failure::Decode { from: to, what }
}

pub fn update_status(client: &mut Client, to: SocketAddr) -> Result<UpdateStatus, Failure> {
    let a = reads::call(client, to, Command::UpdateStatus, &[])?;
    UpdateStatus::decode(&a.payload).map_err(|_| decode(&a, "UPDATE_STATUS reply does not decode (Table M-1)"))
}

/// The `IDENTIFY` fields an update needs (Table M-2 tags 4, 10, 23).
#[derive(Clone, Copy, Debug)]
struct Ident {
    running: Version,
    update_state: u8,
    running_slot: u8,
    uptime_ms: u32,
}

fn identify(client: &mut Client, to: SocketAddr) -> Result<Ident, Failure> {
    let a = reads::identify(client, to)?;
    let i = Identify::decode(&a.payload).expect("checked by reads::identify");
    Ok(Ident { running: i.running, update_state: i.update_state, running_slot: i.running_slot, uptime_ms: i.uptime_ms })
}

/// Send one block, with the 64 and 53 rules. Returns "blocks received".
fn send_block(client: &mut Client, to: SocketAddr, secret: &Secret, file: &PackageFile, n: u32, ev: &mut dyn FnMut(Event)) -> Result<u32, Failure> {
    let mut buf = [0u8; 8 + BLOCK_DATA];
    let len = Block::encode(n, mgmt_host::block_data(file.image(), n), &mut buf).expect("a block fits");
    let (mut room, mut crc) = (0, 0);
    loop {
        let a = writes::authed_raw(client, to, secret, Command::UpdateBlock, &buf[..len])?;
        match a.status_code {
            0 => {
                let (num, received) = mgmt_host::parse_block_reply(&a.payload).ok_or_else(|| decode(&a, "UPDATE_BLOCK reply is not 8 bytes (Table M-1)"))?;
                if num != n {
                    return Err(decode(&a, &format!("UPDATE_BLOCK reply names block {num}, sent {n}")));
                }
                ev(Event::Block { n, received, blocks: file.blocks() });
                return Ok(received);
            }
            STATUS_ROOM_MADE if room == 0 => {
                room += 1;
                ev(Event::RoomMade { n });
            }
            STATUS_BAD_CRC if crc < 2 => {
                crc += 1;
                ev(Event::CrcResent { n });
            }
            _ => return Err(Failure::Status(a, Command::UpdateBlock)),
        }
    }
}

pub(crate) fn send_blocks(client: &mut Client, to: SocketAddr, secret: &Secret, file: &PackageFile, from: u32, ev: &mut dyn FnMut(Event)) -> Result<(), Failure> {
    for n in from..file.blocks() {
        send_block(client, to, secret, file, n, ev)?;
    }
    Ok(())
}

/// Verify, with the "send every block again" rounds. Ok when the radio's hash is the header's.
pub(crate) fn verify(client: &mut Client, to: SocketAddr, secret: &Secret, file: &PackageFile, retries: u32, ev: &mut dyn FnMut(Event)) -> Result<(), Failure> {
    let mut round = 0;
    loop {
        let a = writes::authed_raw(client, to, secret, Command::UpdateVerify, &[])?;
        match a.status_code {
            0 => {
                let h = mgmt_host::parse_verify_reply(&a.payload).ok_or_else(|| decode(&a, "UPDATE_VERIFY reply is not 32 bytes (Table M-1)"))?;
                if h != file.header.sha256 {
                    return Err(decode(&a, "UPDATE_VERIFY answered 0 with a SHA-256 other than the package header's"));
                }
                ev(Event::Verified);
                return Ok(());
            }
            STATUS_VERIFY_FAILED if round < retries => {
                round += 1;
                ev(Event::VerifyFailed { round });
                send_blocks(client, to, secret, file, 0, ev)?;
            }
            _ => return Err(Failure::Status(a, Command::UpdateVerify)),
        }
    }
}

/// `UPDATE_ABORT`, then `END_WRITE_SESSION`. Returns "restart needed".
pub fn abort(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<bool, Failure> {
    let a = writes::write_once(client, to, secret, Command::UpdateAbort, &[])?;
    mgmt_host::parse_flag_reply(&a.payload).ok_or_else(|| decode(&a, "UPDATE_ABORT reply is not 1 byte (Table M-1)"))
}

/// Whether a failure ends the transfer so that the tool aborts it. A radio that does not
/// answer is left alone (the transfer can be resumed; A1.8 U-3).
fn should_abort(f: &Failure) -> bool {
    !matches!(f, Failure::Transport(transport::Error::NoReply { .. }) | Failure::Transport(transport::Error::Io(_)))
}

/// Status 55 to an `UPDATE_BLOCK`: a write failure, after which the radio has discarded the
/// transfer (A1.8 "Write failure").
fn write_failure(f: &Failure) -> bool {
    matches!(f, Failure::Status(a, Command::UpdateBlock) if a.status_code == 55)
}

/// After a failed transfer: `UPDATE_ABORT` only if `UPDATE_STATUS` shows a transfer in
/// progress (receiving or verified). Without one, `IDENTIFY` tells whether the radio restarted
/// during the transfer (uptime, tag 16, below `before`): it then answers the retransmitted
/// block with 33, because its challenge did not survive the reset. An abort would only put it
/// back in the update state for nothing.
fn abort_in_progress(client: &mut Client, to: SocketAddr, secret: &Secret, uptime_before: u32, ev: &mut dyn FnMut(Event)) {
    match update_status(client, to) {
        Ok(st) if matches!(st.state, 1 | 2) => {
            if let Ok(restart_needed) = abort(client, to, secret) {
                ev(Event::Aborted { restart_needed });
            }
        }
        Ok(st) => match identify(client, to) {
            // Any restart counts here, whatever its cause (tag 17): the transfer is gone.
            Ok(i) if i.uptime_ms < uptime_before => ev(Event::RadioRestarted),
            _ => ev(Event::NoTransfer { state: st.state }),
        },
        Err(_) => ev(Event::LeftForResume),
    }
}

/// Install a package of this firmware: choose, transfer, verify, activate, and (with
/// `opts.wait`) read the result back.
pub fn install(
    client: &mut Client,
    to: SocketAddr,
    secret: &Secret,
    packages: &[PackageFile],
    opts: Options,
    ev: &mut dyn FnMut(Event),
) -> Result<Outcome, Failure> {
    let id = identify(client, to)?;
    let file = choose(packages, id.running_slot).map_err(|e| other(e, to))?;
    ev(Event::Chosen { name: file.name.clone(), slot: file.header.target_slot, version: file.header.version, running: id.running });
    let target = if file.header.target_slot == 0 { 3 - id.running_slot } else { file.header.target_slot };

    // Start, or pick up a transfer in progress for this package.
    let mut from = 0;
    let mut begun = false;
    // Once verified, a failure is never answered with UPDATE_ABORT: the activation may have
    // run (its reply lost), and an abort would then hit the new image. The transfer stays
    // verified; `install --resume` verifies and activates again.
    let mut verified = false;
    // What IDENTIFY showed before the activation, or None once the reset has been seen.
    let mut restart_from = None;
    if opts.resume {
        let st = update_status(client, to)?;
        if !(matches!(st.state, 1 | 2) && st.count == file.blocks()) {
            return Err(other(format!("no transfer of {} blocks to resume (UPDATE_STATUS state {}, {} blocks)", file.blocks(), st.state, st.count), to));
        }
        from = st.received.min(file.blocks());
        ev(Event::Resumed { from, blocks: file.blocks() });
        begun = true;
    }
    let run = (|| -> Result<(), Failure> {
        if !begun {
            let mut p = [0u8; 1 + payload::PACKAGE_LEN];
            p[0] = if opts.downgrade { payload::BEGIN_DOWNGRADE } else { 0 };
            p[1..].copy_from_slice(file.header_bytes());
            let a = writes::authed(client, to, secret, Command::UpdateBegin, &p)?;
            let r = BeginReply::decode(&a.payload).map_err(|_| decode(&a, "UPDATE_BEGIN reply does not decode (Table M-1)"))?;
            begun = true;
            if r.block_size as usize != BLOCK_DATA || r.block_count != file.blocks() || r.one_way {
                return Err(decode(
                    &a,
                    &format!(
                        "UPDATE_BEGIN answered block size {}, {} blocks, one-way {}; expected {BLOCK_DATA}, {}, no",
                        r.block_size,
                        r.block_count,
                        r.one_way,
                        file.blocks()
                    ),
                ));
            }
            ev(Event::Begun { blocks: r.block_count });
        }
        send_blocks(client, to, secret, file, from, ev)?;
        verify(client, to, secret, file, opts.verify_retries, ev)?;
        verified = true;
        match writes::resetting(client, to, secret, Command::UpdateActivate, &mgmt_host::activate_request(false))? {
            writes::Resetting::ResetSeen { reason } => {
                ev(Event::ReplyLostResetSeen { reason });
                restart_from = None
            }
            writes::Resetting::Answered(a, before) if a.ok() => {
                if mgmt_host::parse_flag_reply(&a.payload) != Some(false) {
                    return Err(decode(&a, "UPDATE_ACTIVATE reply is not 'permanent 0' (Table M-1)"));
                }
                restart_from = Some(before);
                // Only with the reply: after a lost one the reset is already seen and told.
                ev(Event::Activated);
            }
            writes::Resetting::Answered(a, _) => return Err(Failure::Status(a, Command::UpdateActivate)),
        }
        Ok(())
    })();
    if let Err(f) = run {
        if !begun {
            // UPDATE_BEGIN refused: it may have opened the write session.
            if let Failure::Status(a, _) = &f
                && !matches!(a.status_code, 16..=19 | 32..=35)
            {
                let _ = writes::end_session(client, to, secret);
            }
        } else if verified {
            ev(Event::LeftForResume);
        } else if write_failure(&f) {
            // The transfer is gone; the write session this install opened is ended.
            let _ = writes::end_session(client, to, secret);
            ev(Event::WriteFailed);
        } else if should_abort(&f) {
            abort_in_progress(client, to, secret, id.uptime_ms, ev);
        } else {
            ev(Event::LeftForResume);
        }
        return Err(f);
    }
    match opts.wait {
        None => Ok(Outcome::Activated),
        Some(w) => {
            confirm(client, to, restart_from, target, Some(file.header.version), w, ev)?;
            Ok(Outcome::Confirmed)
        }
    }
}

/// After an activation or a `ROLLBACK`: wait for the reset (unless `uptime_before` is None:
/// already seen), check that `slot` runs (with `version` if given), then wait for the
/// self-confirmation (tag 10 back to 0) or report the rollback.
pub fn confirm(
    client: &mut Client,
    to: SocketAddr,
    before: Option<writes::Before>,
    slot: u8,
    version_want: Option<Version>,
    w: Waits,
    ev: &mut dyn FnMut(Event),
) -> Result<(), Failure> {
    if let Some(before) = before {
        let o = writes::wait_for_restart(client, to, before, w.restart)?;
        ev(Event::Restarted { cause: Some(o.reset_cause) });
    } else {
        ev(Event::Restarted { cause: None });
    }
    let end = Instant::now() + w.confirm;
    let mut seen_trial = false;
    loop {
        match identify(client, to) {
            Ok(i) => {
                if i.running_slot != slot || version_want.is_some_and(|v| v != i.running) {
                    let why = if i.update_state == 2 { "rolled back" } else { "not started" };
                    return Err(other(
                        format!("the new image {why}: slot {} runs {} (update state {})", slot_name(i.running_slot), version(i.running), i.update_state),
                        to,
                    ));
                }
                match i.update_state {
                    1 if !seen_trial => {
                        seen_trial = true;
                        ev(Event::Trial);
                    }
                    0 => {
                        ev(Event::Confirmed);
                        return Ok(());
                    }
                    _ => {}
                }
            }
            // The trial's own reset or a rollback reset: keep asking.
            Err(Failure::Transport(transport::Error::NoReply { .. })) => {}
            Err(e) => return Err(e),
        }
        if Instant::now() >= end {
            return Err(other(format!("the image in slot {} did not confirm itself within {} s", slot_name(slot), w.confirm.as_secs()), to));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// `ROLLBACK` (A1.8; 0064): the other slot becomes the active one, as a trial; the reset ends
/// the write session. With `wait`, the result is read back as after an install.
pub fn rollback(client: &mut Client, to: SocketAddr, secret: &Secret, wait: Option<Waits>, ev: &mut dyn FnMut(Event)) -> Result<u8, Failure> {
    let id = identify(client, to)?;
    let target = match id.running_slot {
        1 => 2,
        2 => 1,
        s => return Err(other(format!("IDENTIFY reports running slot {s} (tag 23)"), to)),
    };
    let restart_from = match writes::resetting(client, to, secret, Command::Rollback, &[])? {
        writes::Resetting::ResetSeen { reason } => {
            ev(Event::ReplyLostResetSeen { reason });
            None
        }
        writes::Resetting::Answered(a, before) if a.ok() => Some(before),
        writes::Resetting::Answered(a, _) => {
            if !matches!(a.status_code, 16..=19 | 32..=35) {
                let _ = writes::end_session(client, to, secret);
            }
            return Err(Failure::Status(a, Command::Rollback));
        }
    };
    if let Some(w) = wait {
        confirm(client, to, restart_from, target, None, w, ev)?;
    }
    Ok(target)
}
