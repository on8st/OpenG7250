// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The update subcommands (M4), part of the `g7250ctl` binary.

use std::path::PathBuf;
use std::time::Duration;

use clap::Subcommand;

use g7250ctl::render::Json;
use g7250ctl::update::{self, Event, Options, PackageFile, Waits};
use g7250ctl::{exit, writes};

use crate::cli_write::SecretArg;
use crate::{fail, open, Outcome};

#[derive(Subcommand)]
pub enum UpdateCmd {
    /// Install new firmware (A1.8): give the packages (.g7pk) for both slots; the one for the
    /// slot that is not running is sent, verified and activated, and the new image is watched
    /// until it has confirmed itself.
    Install {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        /// Package files; usually one per slot (0043).
        #[arg(required = true, value_name = "PACKAGE")]
        packages: Vec<PathBuf>,
        #[command(flatten)]
        secret: SecretArg,
        /// Allow an image older than the running one (A1.8, U-8; logged by the radio).
        #[arg(long)]
        downgrade: bool,
        /// Continue a transfer in progress for this package (UPDATE_STATUS), after a break.
        /// Only the block count is compared: another package of the same count is continued
        /// too, and then fails at the verify.
        #[arg(long)]
        resume: bool,
        /// After a failed verify, send every block again this many times (0067).
        #[arg(long, default_value_t = 1, value_name = "N")]
        verify_retries: u32,
        /// Do not wait for the restart and the self-confirmation.
        #[arg(long)]
        no_wait: bool,
    },
    /// Show the progress of an update (UPDATE_STATUS; no authentication).
    Status {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
    },
    /// Abandon an update (UPDATE_ABORT). The radio stays in the update state until a restart.
    Abort {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Restart the radio afterwards, so that RX is available again.
        #[arg(long)]
        restart: bool,
    },
    /// Return to the image in the other slot (ROLLBACK; 0064), started as a trial.
    Rollback {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Do not wait for the restart and the self-confirmation.
        #[arg(long)]
        no_wait: bool,
    },
    /// Check a package file and show its header (Annex M "Package file"); sends nothing.
    Package {
        #[arg(value_name = "PACKAGE")]
        file: PathBuf,
    },
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn package_json(p: &PackageFile) -> String {
    let h = &p.header;
    Json::obj()
        .s("file", &p.name)
        .raw("component", h.component)
        .raw("hardware_id", h.hardware_id)
        .raw("min_hw_revision", h.min_hw_revision)
        .s("version", &update::version(h.version))
        .raw("length", h.length)
        .s("sha256", &hex(&h.sha256))
        .s("slot", update::slot_name(h.target_slot))
        .raw("blocks", p.blocks())
        .end()
}

/// Progress on stderr (stdout holds the result).
fn progress(json: bool) -> impl FnMut(Event) {
    let mut last_pct = u32::MAX;
    move |e| {
        if json {
            return;
        }
        match e {
            Event::Chosen { name, slot, version, running } => eprintln!(
                "installing {name}: {} for slot {} (running {})",
                update::version(version),
                update::slot_name(slot),
                update::version(running)
            ),
            Event::Begun { blocks } => eprintln!("transfer of {blocks} blocks"),
            Event::Resumed { from, blocks } => eprintln!("resuming at block {from} of {blocks}"),
            Event::Block { n, blocks, .. } => {
                let pct = (n + 1) * 100 / blocks.max(1);
                if pct / 10 != last_pct / 10 || n + 1 == blocks {
                    eprintln!("  {pct:>3}% ({} of {blocks})", n + 1);
                    last_pct = pct;
                }
            }
            Event::RoomMade { n } => eprintln!("  block {n}: room made in the store, sent again"),
            Event::CrcResent { n } => eprintln!("  block {n}: bad CRC reported, sent again"),
            Event::VerifyFailed { round } => eprintln!("verify failed; sending every block again (round {round})"),
            Event::Verified => eprintln!("verified"),
            Event::Activated => eprintln!("activated; the radio restarts into the new image"),
            Event::Restarted => eprintln!("the radio answers again"),
            Event::Trial => eprintln!("trial boot; waiting for the self-confirmation"),
            Event::Confirmed => eprintln!("confirmed"),
            Event::Aborted { restart_needed } => eprintln!(
                "update aborted{}",
                if restart_needed { "; the radio needs a restart (g7250ctl restart) before RX works again" } else { "" }
            ),
            Event::NoTransfer { state } => eprintln!("the radio has no transfer in progress (UPDATE_STATUS state {state}; it restarted?): nothing aborted, start the install again"),
            Event::LeftForResume => eprintln!("the transfer is left as it is (no abort): `install --resume` within 10 min continues it"),
        }
    }
}

pub fn run(json: bool, policy: g7250ctl::transport::Policy, cmd: &UpdateCmd) -> Outcome {
    match cmd {
        UpdateCmd::Package { file } => {
            let p = PackageFile::load(file).map_err(|e| (exit::ERROR, e))?;
            if json {
                println!("{}", package_json(&p));
            } else {
                let h = &p.header;
                println!("{}: version {} for slot {}, {} bytes ({} blocks), hardware 0x{:04x} rev >= {}, sha256 {}", p.name, update::version(h.version), update::slot_name(h.target_slot), h.length, p.blocks(), h.hardware_id, h.min_hw_revision, hex(&h.sha256));
            }
            Ok(exit::OK)
        }
        UpdateCmd::Install { addr, packages, secret, downgrade, resume, verify_retries, no_wait } => {
            // Every file is checked before anything is sent (0071).
            let files = packages.iter().map(|p| PackageFile::load(p)).collect::<Result<Vec<_>, _>>().map_err(|e| (exit::ERROR, e))?;
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            let opts = Options { downgrade: *downgrade, resume: *resume, verify_retries: *verify_retries, wait: if *no_wait { None } else { Some(Waits::default()) } };
            let out = update::install(&mut c, to, &s, &files, opts, &mut progress(json)).map_err(fail)?;
            let what = match out {
                update::Outcome::Activated => "activated",
                update::Outcome::Confirmed => "installed and confirmed",
            };
            if json {
                println!("{}", Json::obj().s("result", what).end());
            } else {
                println!("{what}");
            }
            Ok(exit::OK)
        }
        UpdateCmd::Status { addr } => {
            let (mut c, to) = open(policy, addr)?;
            let st = update::update_status(&mut c, to).map_err(fail)?;
            let state = match st.state {
                0 => "idle",
                1 => "receiving",
                2 => "verified",
                3 => "installer, before the first erase",
                4 => "installer writing",
                _ => "?",
            };
            if json {
                println!("{}", Json::obj().s("state", state).raw("received", st.received).raw("blocks", st.count).raw("sectors_written", st.sectors_written).end());
            } else {
                println!("{state}: {} of {} blocks received, {} sectors written", st.received, st.count, st.sectors_written);
            }
            Ok(exit::OK)
        }
        UpdateCmd::Abort { addr, secret, restart } => {
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            let needed = update::abort(&mut c, to, &s).map_err(fail)?;
            if *restart {
                writes::reset(&mut c, to, &s, g7250_proto::mgmt::Command::Restart).map_err(fail)?;
            }
            let what = match (needed, restart) {
                (_, true) => "update aborted; restart requested",
                (true, false) => "update aborted; the radio needs a restart (g7250ctl restart) before RX works again",
                (false, false) => "update aborted",
            };
            if json {
                println!("{}", Json::obj().raw("restart_needed", needed && !restart).s("done", what).end());
            } else {
                println!("{what}");
            }
            Ok(exit::OK)
        }
        UpdateCmd::Rollback { addr, secret, no_wait } => {
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            let wait = if *no_wait { None } else { Some(Waits { restart: Duration::from_secs(30), confirm: Duration::from_secs(75) }) };
            let slot = update::rollback(&mut c, to, &s, wait, &mut progress(json)).map_err(fail)?;
            let what = format!("rolled back to slot {}{}", update::slot_name(slot), if *no_wait { "" } else { "; confirmed" });
            if json {
                println!("{}", Json::obj().s("slot", update::slot_name(slot)).s("done", &what).end());
            } else {
                println!("{what}");
            }
            Ok(exit::OK)
        }
    }
}
