// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The return-to-stock subcommands (M5; spec A1.8a), part of the `g7250ctl` binary.

use std::io::Write as _;
use std::path::PathBuf;

use clap::Subcommand;

use g7250_proto::mgmt::Command;
use g7250ctl::reads::Failure;
use g7250ctl::render::Json;
use g7250ctl::stock::{self, StockLeft, StockOptions, StockOutcome};
use g7250ctl::exit;

use crate::cli_update::progress;
use crate::cli_write::{confirm, SecretArg};
use crate::{fail, open, Outcome};

#[derive(Subcommand)]
pub enum StockCmd {
    /// RETURN_TO_STOCK (A1.8a item 2). Reserved: this firmware keeps no stock image, so the
    /// radio answers 56. The way back is `install-stock`.
    ReturnToStock {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
    },
    /// ENTER_VENDOR_RECOVERY (A1.8a item 4). Reserved: no software entry into the vendor path
    /// is known, so the radio answers 56. The vendor path is the update menu on the controller.
    VendorRecovery {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
    },
    /// Install an owner-supplied stock image (A1.8a item 3): one-way once the first sector is
    /// erased; the management secret is erased with it.
    InstallStock {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        /// The stock Main image, as supplied by the vendor (not a .g7pk).
        #[arg(value_name = "IMAGE")]
        image: PathBuf,
        /// The image's SHA-256 as the owner has it (64 hex digits); the file must match it.
        #[arg(long, value_name = "HEX")]
        sha256: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
        /// Continue an installation the installer shows in progress, after a break.
        #[arg(long)]
        resume: bool,
        /// After a failed verify, send every block again this many times.
        #[arg(long, default_value_t = 1, value_name = "N")]
        verify_retries: u32,
        /// Also save the package (header and image, Annex M "Package file") to this new file.
        #[arg(long, value_name = "FILE")]
        save_package: Option<PathBuf>,
    },
}

const RESERVED_56: &str = "status 56: not supported in this firmware";

fn explain_reserved(cmd: Command, f: Failure) -> (u8, String) {
    let is_56 = matches!(&f, Failure::Status(a, _) if a.status_code == 56);
    let (code, text) = fail(f);
    if !is_56 {
        return (code, text);
    }
    let why = match cmd {
        Command::ReturnToStock => "this firmware keeps no stock image (A1.8a item 2); return to stock with `g7250ctl install-stock <addr> <image> --sha256 <hash>`",
        _ => "no software entry into the vendor path is known (A1.8a item 4); the vendor path is the update menu on the controller (USB drive)",
    };
    (code, format!("{RESERVED_56}: {why}"))
}

pub fn run(json: bool, policy: g7250ctl::transport::Policy, cmd: &StockCmd) -> Outcome {
    match cmd {
        StockCmd::ReturnToStock { addr, secret } | StockCmd::VendorRecovery { addr, secret } => {
            let c = if matches!(cmd, StockCmd::ReturnToStock { .. }) { Command::ReturnToStock } else { Command::EnterVendorRecovery };
            let s = secret.load()?;
            let (mut client, to) = open(policy, addr)?;
            stock::reserved(&mut client, to, &s, c).map_err(|f| explain_reserved(c, f))?;
            // A later firmware may implement them; this one never answers 0.
            let what = format!("{c:?} accepted");
            if json {
                println!("{}", Json::obj().raw("ok", true).s("done", &what).end());
            } else {
                println!("{what}");
            }
            Ok(exit::OK)
        }
        StockCmd::InstallStock { addr, image, sha256, secret, yes, resume, verify_retries, save_package } => {
            let want = stock::parse_sha256(sha256).map_err(|e| (exit::USAGE, e))?;
            let bytes = std::fs::read(image).map_err(|e| (exit::ERROR, format!("{}: {e}", image.display())))?;
            let name = image.display().to_string();
            let file = stock::stock_package(&name, &bytes, want).map_err(|e| (exit::ERROR, e))?;
            if let Some(out) = save_package {
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(out)
                    .map_err(|e| (exit::ERROR, format!("{}: {e} (an existing file is never overwritten)", out.display())))?;
                f.write_all(file.header_bytes()).and_then(|_| f.write_all(file.image())).map_err(|e| (exit::ERROR, format!("{}: {e}", out.display())))?;
            }
            let s = secret.load()?;
            if !yes && !resume {
                confirm(&format!(
                    "Install the stock image {name} ({} bytes, SHA-256 {}) on {addr}.\n\
                     This is one-way: once the installer erases the first sector there is no return to this firmware,\n\
                     the management secret is erased, and a transfer broken by a power loss needs the vendor's\n\
                     update path on the controller. Type yes to go on: ",
                    bytes.len(),
                    stock::hex(&want)
                ))?;
            }
            let (mut c, to) = open(policy, addr)?;
            let opts = StockOptions { resume: *resume, verify_retries: *verify_retries };
            let mut left = None;
            match stock::install_stock(&mut c, to, &s, &file, opts, &mut progress(json), &mut left) {
                Ok(o) => {
                    let what = match o {
                        StockOutcome::Activated => "activated (permanent): the radio resets into the stock firmware",
                        StockOutcome::ActivatedUnconfirmed => {
                            "the reply to UPDATE_ACTIVATE was lost and the radio no longer answers: the stock firmware has most likely started (it does not answer this channel); check the radio"
                        }
                    };
                    if json {
                        println!("{}", Json::obj().raw("confirmed", o == StockOutcome::Activated).s("done", what).end());
                    } else {
                        println!("{what}");
                    }
                    Ok(exit::OK)
                }
                Err(f) => {
                    let (code, text) = fail(f);
                    let after = match left {
                        Some(StockLeft::AbortedBeforeErase) => "; aborted before the first erase: the radio restarts into this firmware, secret and configuration unchanged".to_string(),
                        Some(StockLeft::Writing { sectors }) => format!(
                            "; the installer has erased and written {sectors} sectors: there is no way back. It waits for blocks with no time limit: run `install-stock --resume` with the same image. Keep the power on; after a power loss only the vendor's update path on the controller helps"
                        ),
                        Some(StockLeft::Unknown) => "; the radio does not answer UPDATE_STATUS: if the installer runs, `install-stock --resume` continues; see `g7250ctl status`".into(),
                        Some(StockLeft::Nothing) | None => String::new(),
                    };
                    Err((code, format!("{text}{after}")))
                }
            }
        }
    }
}
