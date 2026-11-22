// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Return to the stock firmware (M5; spec A1.8a).
//!
//! - `RETURN_TO_STOCK` and `ENTER_VENDOR_RECOVERY` are reserved in this version: no stock
//!   image is retained (item 2) and no software entry into the vendor path is known (item 4).
//!   Both always get status 56; the tool sends them and explains the answer.
//! - The way back is the streaming installation of an owner-supplied stock image (item 3):
//!   `UPDATE_BEGIN` with the "foreign image" flag and a package header the host builds (Table
//!   M-4: version 0, target slot 0), the blocks, `UPDATE_VERIFY`, and `UPDATE_ACTIVATE` with
//!   the flag again. It is one-way once the installer has erased the first sector: then it is
//!   never aborted (that gets 60) and a break is left for `--resume`, since the installer waits
//!   for blocks with no time limit. Before the first erase, an abort restarts the radio into
//!   this firmware with its secret and configuration unchanged.
//! - The stock firmware does not speak this channel, so nothing is read back after the
//!   activation.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use g7250_proto::mgmt::auth::sha256;
use g7250_proto::mgmt::payload::{self, BeginReply, Identify, Package, Version, HARDWARE_ID_G7250, PACKAGE_HEADER_VERSION};
use g7250_proto::mgmt::{Command, BLOCK_DATA};
use g7250_proto::mgmt_host;

use crate::reads::{self, Failure};
use crate::secret::Secret;
use crate::transport::{self, Client};
use crate::update::{self, Event, PackageFile};
use crate::writes;

/// The application area the installer writes: sectors 1 to 7 of 128 KiB (A1.8a item 3
/// "Writing"); a longer image gets 61 from the radio and is refused here before anything is
/// sent.
pub const MAX_STOCK_LEN: usize = 7 * 128 * 1024;

/// `UPDATE_STATUS` / `IDENTIFY` tag 10 states of the installer (Table M-1).
pub const STATE_INSTALLER_BEFORE_ERASE: u8 = 3;
pub const STATE_INSTALLER_WRITING: u8 = 4;

/// The package for a stock image (Annex M "Package file": the host builds the header at install
/// time). Checks the owner-supplied SHA-256 and the length before anything is sent.
pub fn stock_package(name: &str, image: &[u8], owner_sha256: [u8; 32]) -> Result<PackageFile, String> {
    if image.is_empty() {
        return Err(format!("{name}: the image is empty"));
    }
    if image.len() > MAX_STOCK_LEN {
        return Err(format!("{name}: {} bytes; the application area (sectors 1-7) holds {MAX_STOCK_LEN} (A1.8a item 3)", image.len()));
    }
    let got = sha256(image);
    if got != owner_sha256 {
        return Err(format!("{name}: its SHA-256 is {}, not the one given ({}); nothing sent", hex(&got), hex(&owner_sha256)));
    }
    let header = Package {
        header_version: PACKAGE_HEADER_VERSION,
        component: 1,
        hardware_id: HARDWARE_ID_G7250,
        min_hw_revision: 0,
        version: Version { major: 0, minor: 0, build: 0 },
        length: image.len() as u32,
        sha256: owner_sha256,
        signature_len: 0,
        target_slot: 0,
    };
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(image);
    PackageFile::from_bytes(name, bytes)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Parse a SHA-256 given as 64 hex digits.
pub fn parse_sha256(s: &str) -> Result<[u8; 32], String> {
    let s = s.trim();
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("a SHA-256 is 64 hex digits".into());
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    Ok(out)
}

/// `RETURN_TO_STOCK` or `ENTER_VENDOR_RECOVERY` (A1.8a items 2, 4): sent as an authenticated
/// write; the answer (56 in this version) is returned as a failure like any refusal.
pub fn reserved(client: &mut Client, to: SocketAddr, secret: &Secret, cmd: Command) -> Result<(), Failure> {
    debug_assert!(matches!(cmd, Command::ReturnToStock | Command::EnterVendorRecovery));
    writes::write_once(client, to, secret, cmd, &[]).map(|_| ())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StockOptions {
    /// Continue an installation the installer shows in progress (`UPDATE_STATUS` 3 or 4) with
    /// the same block count.
    pub resume: bool,
    /// After a failed verify, send every block again this many times (A1.8a item 3 "Check and
    /// switch").
    pub verify_retries: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockOutcome {
    /// `UPDATE_ACTIVATE` answered "permanent": the radio resets into the stock firmware.
    Activated,
    /// The activation reply was lost and the radio no longer answers this channel: the stock
    /// firmware has most likely started (it does not speak it). Checked on the radio itself.
    ActivatedUnconfirmed,
}

/// What a failure left behind, for the message (the error itself is returned as well).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockLeft {
    /// Before the first erase: `UPDATE_ABORT` was sent; the radio restarts into this firmware,
    /// secret and configuration unchanged.
    AbortedBeforeErase,
    /// The installer has erased (the secret too) and is writing: no way back; it waits for
    /// blocks with no time limit (`install-stock --resume`). A power loss now needs the vendor
    /// path through the controller (A1.8a item 3 "No way back").
    Writing { sectors: u8 },
    /// `UPDATE_STATUS` did not answer: nothing was sent; the installer may still wait.
    Unknown,
    /// No transfer in progress (refused `UPDATE_BEGIN`, or this firmware runs again).
    Nothing,
}

/// The streaming installation of a stock image (A1.8a item 3).
pub fn install_stock(
    client: &mut Client,
    to: SocketAddr,
    secret: &Secret,
    file: &PackageFile,
    opts: StockOptions,
    ev: &mut dyn FnMut(Event),
    left: &mut Option<StockLeft>,
) -> Result<StockOutcome, Failure> {
    let blocks = file.blocks();
    let mut from = 0;
    let mut begun = false;
    if opts.resume {
        let st = update::update_status(client, to)?;
        if !(matches!(st.state, STATE_INSTALLER_BEFORE_ERASE | STATE_INSTALLER_WRITING) && st.count == blocks) {
            return Err(other(format!("no stock installation of {blocks} blocks to resume (UPDATE_STATUS state {}, {} blocks)", st.state, st.count), to));
        }
        from = st.received.min(blocks);
        ev(Event::Resumed { from, blocks });
        begun = true;
    }
    let run = (|| -> Result<StockOutcome, Failure> {
        if !begun {
            let mut p = [0u8; 1 + payload::PACKAGE_LEN];
            p[0] = payload::BEGIN_FOREIGN;
            p[1..].copy_from_slice(file.header_bytes());
            let a = writes::authed(client, to, secret, Command::UpdateBegin, &p)?;
            begun = true;
            let r = BeginReply::decode(&a.payload).map_err(|_| Failure::Decode { from: a.from, what: "UPDATE_BEGIN reply does not decode (Table M-1)".into() })?;
            if r.block_size as usize != BLOCK_DATA || r.block_count != blocks || !r.one_way {
                return Err(Failure::Decode {
                    from: a.from,
                    what: format!(
                        "UPDATE_BEGIN for a foreign image answered block size {}, {} blocks, one-way {}; expected {BLOCK_DATA}, {blocks}, yes (A1.8a item 3)",
                        r.block_size, r.block_count, r.one_way
                    ),
                });
            }
            ev(Event::Begun { blocks });
        }
        update::send_blocks(client, to, secret, file, from, ev)?;
        update::verify(client, to, secret, file, opts.verify_retries, ev)?;
        activate(client, to, secret)
    })();
    match run {
        Ok(o) => {
            ev(Event::Activated);
            Ok(o)
        }
        Err(f) => {
            *left = Some(if begun { after_failure(client, to, secret, ev) } else { StockLeft::Nothing });
            if !begun
                && let Failure::Status(a, _) = &f
                && writes::may_hold_session(a.status_code)
            {
                let _ = writes::end_session(client, to, secret);
            }
            Err(f)
        }
    }
}

/// How long to look for the installer after a lost activation reply.
pub const ACTIVATE_PROBE: Duration = Duration::from_secs(10);

/// `UPDATE_ACTIVATE` with the foreign flag (A1.8a item 3 "Check and switch"); the reply says
/// "permanent" (item 5). A lost reply is resolved with `IDENTIFY`: the installer still
/// answering means it did not switch; silence means the stock firmware runs.
fn activate(client: &mut Client, to: SocketAddr, secret: &Secret) -> Result<StockOutcome, Failure> {
    let r = writes::authed_raw(client, to, secret, Command::UpdateActivate, &mgmt_host::activate_request(true));
    let lost = match &r {
        Ok(a) => a.status_code == 33 && a.sends > 1,
        Err(Failure::Transport(transport::Error::NoReply { .. })) => true,
        Err(_) => false,
    };
    if lost {
        let end = Instant::now() + ACTIVATE_PROBE;
        while Instant::now() < end {
            match reads::identify(client, to) {
                Ok(a) => {
                    let i = Identify::decode(&a.payload).expect("checked by reads::identify");
                    if i.update_state == 3 {
                        // IDENTIFY tag 10 = installer: it answers, so it has not switched.
                        return Err(other("the reply to UPDATE_ACTIVATE was lost and the installer still runs: it has not switched; `install-stock --resume` verifies and activates again".into(), to));
                    }
                    return Err(other(format!("after UPDATE_ACTIVATE this firmware answers again (update state {})", i.update_state), to));
                }
                Err(Failure::Transport(transport::Error::NoReply { .. })) => {}
                Err(e) => return Err(e),
            }
        }
        return Ok(StockOutcome::ActivatedUnconfirmed);
    }
    let a = r?;
    if !a.ok() {
        return Err(Failure::Status(a, Command::UpdateActivate));
    }
    match mgmt_host::parse_flag_reply(&a.payload) {
        Some(true) => Ok(StockOutcome::Activated),
        _ => Err(Failure::Decode { from: a.from, what: "UPDATE_ACTIVATE for a foreign image did not answer 'permanent 1' (Table M-1; A1.8a item 5)".into() }),
    }
}

/// After a failure: abort only before the first erase (A1.8a item 3); once writing, leave the
/// installer waiting.
fn after_failure(client: &mut Client, to: SocketAddr, secret: &Secret, ev: &mut dyn FnMut(Event)) -> StockLeft {
    match update::update_status(client, to) {
        Ok(st) if st.state == STATE_INSTALLER_BEFORE_ERASE => {
            if let Ok(restart_needed) = update::abort(client, to, secret) {
                ev(Event::Aborted { restart_needed });
            }
            StockLeft::AbortedBeforeErase
        }
        Ok(st) if st.state == STATE_INSTALLER_WRITING => StockLeft::Writing { sectors: st.sectors_written },
        Ok(_) => StockLeft::Nothing,
        Err(_) => StockLeft::Unknown,
    }
}

fn other(what: String, to: SocketAddr) -> Failure {
    Failure::Decode { from: to, what }
}
