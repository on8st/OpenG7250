// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The authenticated subcommands (M3), part of the `g7250ctl` binary.

use std::io::{BufRead, IsTerminal, Write as _};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Subcommand};
use g7250_proto::mgmt::payload::{Config, Identify};
use g7250_proto::mgmt::Command;

use g7250ctl::render::{self, Json};
use g7250ctl::secret::Secret;
use g7250ctl::writes::{self, Whereabouts};
use g7250ctl::{args, exit, reads};

use crate::{fail, open, Outcome};

/// Where the secret comes from. Never the command line.
#[derive(Args, Clone)]
pub struct SecretArg {
    /// File with the secret (64 hex digits). Without it, the environment variable G7250_SECRET.
    #[arg(long, value_name = "FILE")]
    pub secret_file: Option<PathBuf>,
}

impl SecretArg {
    fn load(&self) -> Result<Secret, (u8, String)> {
        Secret::load(self.secret_file.as_deref()).map_err(|e| (exit::USAGE, e))
    }
}

#[derive(Subcommand)]
pub enum WriteCmd {
    /// Set the first secret, in the window after the first start or a factory reset (A1.10).
    /// The secret crosses the network in the clear: do it on a network you trust.
    SetSecret {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Generate a new random secret and write it to FILE (which must not exist) first.
        #[arg(long, value_name = "FILE", conflicts_with = "secret_file")]
        generate: Option<PathBuf>,
    },
    /// Change the secret (A1.10): sent masked, authenticated with the old one.
    ChangeSecret {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        /// The current secret.
        #[command(flatten)]
        secret: SecretArg,
        /// File with the new secret.
        #[arg(long, value_name = "FILE", required_unless_present = "generate")]
        new_secret_file: Option<PathBuf>,
        /// Generate the new secret and write it to FILE (which must not exist) first.
        #[arg(long, value_name = "FILE", conflicts_with = "new_secret_file")]
        generate: Option<PathBuf>,
    },
    /// Release the write session held by this host (A1.10). Each run of g7250ctl releases its
    /// own; this is for a session left by another tool on the same address and port.
    EndSession {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
    },
    /// Restart the radio (A1.9): unkey and disarm, wait until RF is gone, reply, reset.
    Restart {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Wait until the radio answers again.
        #[arg(long)]
        wait: bool,
        /// How long --wait waits, in seconds.
        #[arg(long, default_value_t = 30, value_name = "S")]
        wait_limit: u64,
    },
    /// Factory reset (A1.6a, Table A1-4): network to DHCP, secret cleared (a new first-secret
    /// window opens), level 1, region 0, TCI allow-list empty; limits, log, calibration and
    /// images kept.
    FactoryReset {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
        /// Do not ask for confirmation.
        #[arg(long)]
        yes: bool,
        /// Wait until the radio answers again (at the same address only if DHCP gives it).
        #[arg(long)]
        wait: bool,
        /// How long --wait waits, in seconds (17 s more than after a restart, 0069).
        #[arg(long, default_value_t = 47, value_name = "S")]
        wait_limit: u64,
    },
    /// Switch the active level now (A1.6 C-2; A2.5).
    SwitchLevel {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        /// 1 (G7250-P2) or 2 (TCI, once it exists).
        #[arg(value_parser = clap::value_parser!(u8).range(1..=2))]
        level: u8,
        #[command(flatten)]
        secret: SecretArg,
    },
    /// Clear the log (A1.7).
    ClearLog {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: SecretArg,
    },
}

/// `config set` options: one per item of Table M-3; only the items given are changed.
#[derive(Args)]
pub struct ConfigSetArgs {
    /// Radio address, `host` or `host:port` (port 7251 by default).
    pub addr: String,
    #[command(flatten)]
    pub secret: SecretArg,
    /// C-1: DHCP, with a link-local address while no lease is held (A1.11).
    #[arg(long, conflicts_with = "static_")]
    pub dhcp: bool,
    /// C-1: static address (give --address and --netmask too, unless already set).
    #[arg(long = "static")]
    pub static_: bool,
    /// C-1 static address.
    #[arg(long, value_name = "IP")]
    pub address: Option<Ipv4Addr>,
    /// C-1 netmask.
    #[arg(long, value_name = "MASK")]
    pub netmask: Option<Ipv4Addr>,
    /// C-1 gateway (0.0.0.0 for none).
    #[arg(long, value_name = "IP")]
    pub gateway: Option<Ipv4Addr>,
    /// C-2 default level at power-on.
    #[arg(long, value_name = "1|2", value_parser = clap::value_parser!(u8).range(1..=2))]
    pub default_level: Option<u8>,
    /// C-6 region (0 is the only one in this version).
    #[arg(long, value_name = "N")]
    pub region: Option<u8>,
    /// C-3 maximum continuous TX time, ms.
    #[arg(long, value_name = "MS")]
    pub max_tx_time: Option<u32>,
    /// C-3 power limit of one band, INDEX=WATTS (0.1 W steps; 0 closes the band). Repeatable.
    #[arg(long, value_name = "INDEX=W", value_parser = args::parse_band_limit)]
    pub band_limit: Vec<(usize, u16)>,
    /// C-3 SWR limit S2, for example 2.5 (needs the calibration set, status 62).
    #[arg(long, value_name = "SWR", value_parser = |s: &str| args::parse_fixed(s, 2))]
    pub swr_limit: Option<u16>,
    /// C-3 PA temperature limit T3 in °C, 0.1 steps (needs the calibration set, status 62).
    #[arg(long, value_name = "C", value_parser = |s: &str| args::parse_fixed(s, 1))]
    pub pa_temp_limit: Option<u16>,
    /// C-7 TCI allow-list entry ADDRESS/PREFIX; the list given replaces the stored one. Repeatable.
    #[arg(long, value_name = "CIDR", value_parser = args::parse_cidr)]
    pub allow: Vec<([u8; 4], u8)>,
    /// C-7: empty the TCI allow-list (any address in the radio's own subnet).
    #[arg(long, conflicts_with = "allow")]
    pub allow_clear: bool,
}

impl ConfigSetArgs {
    fn change(&self) -> Result<Config, String> {
        let mut c = Config {
            address_mode: if self.dhcp { Some(0) } else if self.static_ { Some(1) } else { None },
            address: self.address.map(|a| a.octets()),
            netmask: self.netmask.map(|a| a.octets()),
            gateway: self.gateway.map(|a| a.octets()),
            default_level: self.default_level,
            region: self.region,
            max_tx_time_ms: self.max_tx_time,
            swr_s2_x100: self.swr_limit,
            pa_temp_t3: self.pa_temp_limit,
            ..Default::default()
        };
        for (i, w) in &self.band_limit {
            c.band_limit[*i] = Some(*w);
        }
        if self.allow_clear {
            c.allow_count = Some(0);
        } else if !self.allow.is_empty() {
            args::check_allow_len(self.allow.len())?;
            c.allow[..self.allow.len()].copy_from_slice(&self.allow);
            c.allow_count = Some(self.allow.len() as u8);
        }
        if c == Config::default() {
            return Err("config set: give at least one item to change (see --help)".into());
        }
        Ok(c)
    }
}

/// `calibration set` options.
#[derive(Args)]
pub struct CalibrationSetArgs {
    /// Radio address, `host` or `host:port` (port 7251 by default).
    pub addr: String,
    #[command(flatten)]
    pub secret: SecretArg,
    /// Where the values come from (ASCII, at most 255 characters).
    #[arg(long)]
    pub source: String,
    /// Date of the set: `now` or seconds since 1970-01-01 UTC.
    #[arg(long, default_value = "now", value_parser = args::parse_date)]
    pub date: u32,
    /// An entry of Table M-5, ID=HEX (for example P-RF-2=000a). Repeatable; the entries given
    /// replace the stored set as a whole.
    #[arg(long, value_name = "ID=HEX", value_parser = args::parse_entry)]
    pub entry: Vec<(String, Vec<u8>)>,
}

fn done(json: bool, what: &str) -> Outcome {
    if json {
        println!("{}", Json::obj().raw("ok", true).s("done", what).end());
    } else {
        println!("{what}");
    }
    Ok(exit::OK)
}

pub fn run(json: bool, policy: g7250ctl::transport::Policy, cmd: &WriteCmd) -> Outcome {
    match cmd {
        WriteCmd::SetSecret { addr, secret, generate } => {
            let s = match generate {
                Some(path) => {
                    let s = Secret::generate().map_err(|e| (exit::ERROR, e))?;
                    s.write_new(path).map_err(|e| (exit::ERROR, e))?;
                    s
                }
                None => secret.load()?,
            };
            let (mut c, to) = open(policy, addr)?;
            writes::set_first_secret(&mut c, to, &s).map_err(fail)?;
            done(json, "secret set; IDENTIFY shows a secret is set")
        }
        WriteCmd::ChangeSecret { addr, secret, new_secret_file, generate } => {
            let old = secret.load()?;
            let new = match (generate, new_secret_file) {
                (Some(path), _) => {
                    let s = Secret::generate().map_err(|e| (exit::ERROR, e))?;
                    s.write_new(path).map_err(|e| (exit::ERROR, e))?;
                    s
                }
                (None, Some(f)) => Secret::from_file(f).map_err(|e| (exit::USAGE, e))?,
                (None, None) => unreachable!("clap requires one"),
            };
            if new == old {
                return Err((exit::USAGE, "the new secret is the same as the old one".into()));
            }
            let (mut c, to) = open(policy, addr)?;
            writes::change_secret(&mut c, to, &old, &new).map_err(fail)?;
            done(json, "secret changed; checked with a request signed with the new secret")
        }
        WriteCmd::EndSession { addr, secret } => {
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            writes::end_session(&mut c, to, &s).map_err(fail)?;
            done(json, "write session ended")
        }
        WriteCmd::Restart { addr, secret, wait, wait_limit } => {
            let s = secret.load()?;
            reset(json, policy, addr, &s, Command::Restart, *wait, *wait_limit)
        }
        WriteCmd::FactoryReset { addr, secret, yes, wait, wait_limit } => {
            let s = secret.load()?;
            if !yes {
                confirm(&format!(
                    "Factory reset {addr}: network to DHCP, secret cleared (anyone on the LAN can then set the first secret), level 1, region 0, TCI allow-list empty. Type yes to go on: "
                ))?;
            }
            reset(json, policy, addr, &s, Command::FactoryReset, *wait, *wait_limit)
        }
        WriteCmd::SwitchLevel { addr, level, secret } => {
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            writes::switch_level(&mut c, to, &s, *level).map_err(fail)?;
            done(json, &format!("level {level} requested"))
        }
        WriteCmd::ClearLog { addr, secret } => {
            let s = secret.load()?;
            let (mut c, to) = open(policy, addr)?;
            writes::clear_log(&mut c, to, &s).map_err(fail)?;
            done(json, "log cleared")
        }
    }
}

fn confirm(prompt: &str) -> Result<(), (u8, String)> {
    if !std::io::stdin().is_terminal() {
        return Err((exit::USAGE, "not confirmed: give --yes when not run from a terminal".into()));
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| (exit::ERROR, e.to_string()))?;
    if line.trim() == "yes" { Ok(()) } else { Err((exit::USAGE, "not confirmed".into())) }
}

fn reset(json: bool, policy: g7250ctl::transport::Policy, addr: &str, s: &Secret, cmd: Command, wait: bool, limit: u64) -> Outcome {
    let (mut c, to) = open(policy, addr)?;
    let before = reads::identify(&mut c, to).map_err(fail)?;
    let up = Identify::decode(&before.payload).expect("checked by reads::identify").uptime_ms;
    writes::reset(&mut c, to, s, cmd).map_err(fail)?;
    let what = if cmd == Command::Restart { "restart" } else { "factory reset" };
    if !wait {
        return done(json, &format!("{what} accepted"));
    }
    match writes::wait_for_restart(&mut c, to, up, Duration::from_secs(limit)) {
        Ok(ms) => done(json, &format!("{what} done; the radio answers again (uptime {})", render::uptime(ms))),
        Err(e) if cmd == Command::FactoryReset => Err((
            e.exit_code(),
            format!("{e}; after a factory reset the radio takes its address from DHCP (or link-local): find it with `g7250ctl discover`"),
        )),
        Err(e) => Err(fail(e)),
    }
}

pub fn config_set(json: bool, policy: g7250ctl::transport::Policy, a: &ConfigSetArgs) -> Outcome {
    let change = a.change().map_err(|e| (exit::USAGE, e))?;
    let s = a.secret.load()?;
    let (mut c, to) = open(policy, &a.addr)?;
    let r = writes::set_config(&mut c, to, &s, &change).map_err(fail)?;
    let shown = r.read_back.as_ref().unwrap_or(&r.after);
    if json {
        let at = match &r.whereabouts {
            Whereabouts::At(at) => Json::str(&at.to_string()),
            Whereabouts::Unknown => "null".into(),
        };
        println!("{}", Json::obj().raw("address", at).raw("read_back", r.read_back.is_some()).raw("config", render::config_json(shown)).end());
    } else {
        print!("{}", render::config_human(shown));
        match &r.whereabouts {
            Whereabouts::At(at) if *at != to => println!("the radio now answers at {at}"),
            Whereabouts::At(_) => {}
            Whereabouts::Unknown => println!("the radio no longer answers at {to}; find it with `g7250ctl discover`"),
        }
    }
    if r.read_back.is_none() { Err((exit::NO_REPLY, "the change was not read back".into())) } else { Ok(exit::OK) }
}

pub fn calibration_set(json: bool, policy: g7250ctl::transport::Policy, a: &CalibrationSetArgs) -> Outcome {
    if a.source.len() > 255 || !a.source.is_ascii() {
        return Err((exit::USAGE, "--source is at most 255 ASCII characters".into()));
    }
    let s = a.secret.load()?;
    let (mut c, to) = open(policy, &a.addr)?;
    let (id, tx) = writes::set_calibration(&mut c, to, &s, a.date, &a.source, &a.entry).map_err(fail)?;
    let hex: String = id.iter().map(|b| format!("{b:02x}")).collect();
    if json {
        println!("{}", Json::obj().s("id", &hex).raw("tx_calibrated", tx).end());
    } else {
        println!("calibration set {hex} stored; TX calibrated: {}", if tx { "yes" } else { "no" });
    }
    Ok(exit::OK)
}

pub fn calibration_clear(json: bool, policy: g7250ctl::transport::Policy, addr: &str, secret: &SecretArg) -> Outcome {
    let s = secret.load()?;
    let (mut c, to) = open(policy, addr)?;
    writes::clear_calibration(&mut c, to, &s).map_err(fail)?;
    done(json, "calibration set cleared")
}
