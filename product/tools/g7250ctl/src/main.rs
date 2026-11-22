// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! `g7250ctl` command line. See README.md.

mod cli_stock;
mod cli_update;
mod cli_write;

use std::net::{Ipv4Addr, SocketAddr};
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use g7250_proto::mgmt::payload::{self, Identify};
use g7250_proto::mgmt::Command;
use g7250_proto::{discovery, mgmt};

use g7250ctl::reads::{self, Failure, LogEvent};
use g7250ctl::render::{self, Json};
use g7250ctl::transport::{Client, Policy};
use g7250ctl::{discover, exit, resolve};

#[derive(Parser)]
#[command(name = "g7250ctl", version, about = "Manage a G7250 running the OpenG7250 firmware (management channel, UDP 7251)")]
struct Cli {
    /// Print JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    /// Time without a reply before a retransmission, in ms (A1.4: 500).
    #[arg(long, global = true, default_value_t = 500, value_name = "MS")]
    interval: u64,
    /// Retransmissions of an ordinary request (A1.4: 5). Requests that may do flash work keep
    /// retransmitting for at least 16 s (0069).
    #[arg(long, global = true, default_value_t = 5, value_name = "N")]
    retries: u32,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Find G7250s: IDENTIFY to the broadcast address on the management port (A1.5).
    Discover {
        /// How long to collect replies, in seconds.
        #[arg(long, default_value_t = 2.0, value_name = "S")]
        wait: f64,
        /// Also send to this address (a directed broadcast such as 192.168.1.255, or a unicast
        /// address). Repeatable. The limited broadcast 255.255.255.255 is always used.
        #[arg(long, value_name = "ADDR")]
        broadcast: Vec<Ipv4Addr>,
        /// Management port to send to (A1.3: 7251).
        #[arg(long, default_value_t = mgmt::PORT)]
        port: u16,
        /// Use P2 discovery on port 1024 (B3.2) instead; lists any P2 radio.
        #[arg(long)]
        p2: bool,
        /// Discovery port for --p2 (B3.1: 1024); replies are taken only from this port.
        #[arg(long, default_value_t = discovery::PORT, value_name = "PORT")]
        p2_port: u16,
    },
    /// Show identification: model, MAC, firmware versions, update state, owner (A1.5).
    Info {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
    },
    /// Show the packet, error and gap counters (GET_STATS; A1.7, B11).
    Stats {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
    },
    /// Read the log (READ_LOG, in pages; A1.7).
    Logs {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        /// First sequence number wanted; 0 = the oldest entry kept.
        #[arg(long, default_value_t = 0, value_name = "SEQ")]
        from: u32,
        /// Keep reading new entries until interrupted; goes on across a restart of the radio.
        #[arg(long, short)]
        follow: bool,
        /// Poll period of --follow, in seconds.
        #[arg(long, default_value_t = 1.0, value_name = "S")]
        poll: f64,
    },
    /// Read or change the configuration (GET_CONFIG, SET_CONFIG; A1.6, Table M-3).
    Config {
        #[command(subcommand)]
        op: ConfigOp,
    },
    /// Read, set or clear the calibration set (A1.12).
    Calibration {
        #[command(subcommand)]
        op: CalibrationOp,
    },
    #[command(flatten)]
    Write(cli_write::WriteCmd),
    #[command(flatten)]
    Update(cli_update::UpdateCmd),
    #[command(flatten)]
    Stock(cli_stock::StockCmd),
}

#[derive(Subcommand)]
enum ConfigOp {
    /// Show every configuration item.
    Get {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
    },
    /// Change the items given, validated as a whole by the radio; read back afterwards.
    Set(cli_write::ConfigSetArgs),
}

#[derive(Subcommand)]
enum CalibrationOp {
    /// Show the calibration set: id, date, source and the Table M-5 entries.
    Get {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
    },
    /// Replace the calibration set as a whole (SET_CALIBRATION); read back afterwards.
    Set(cli_write::CalibrationSetArgs),
    /// Remove the calibration set (CLEAR_CALIBRATION).
    Clear {
        /// Radio address, `host` or `host:port` (port 7251 by default).
        addr: String,
        #[command(flatten)]
        secret: cli_write::SecretArg,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let policy = Policy { interval: Duration::from_millis(cli.interval.max(1)), retransmits: cli.retries, ..Policy::default() };
    let code = match run(&cli, policy) {
        Ok(c) => c,
        Err((c, msg)) => {
            eprintln!("g7250ctl: {msg}");
            c
        }
    };
    ExitCode::from(code)
}

pub(crate) type Outcome = Result<u8, (u8, String)>;

pub(crate) fn fail(f: Failure) -> (u8, String) {
    (f.exit_code(), f.to_string())
}

fn client(policy: Policy) -> Result<Client, (u8, String)> {
    let mut c = Client::new().map_err(|e| (exit::ERROR, format!("cannot open a UDP socket: {e}")))?;
    c.policy = policy;
    Ok(c)
}

/// A client and the resolved radio address.
pub(crate) fn open(policy: Policy, addr: &str) -> Result<(Client, SocketAddr), (u8, String)> {
    let to = resolve(addr, mgmt::PORT).map_err(|e| (exit::USAGE, e))?;
    Ok((client(policy)?, to))
}

fn run(cli: &Cli, policy: Policy) -> Outcome {
    match &cli.cmd {
        Cmd::Discover { wait, broadcast, port, p2, p2_port } => discover_cmd(cli.json, policy, *wait, broadcast, *port, *p2, *p2_port),
        Cmd::Info { addr } => {
            let (mut c, to) = open(policy, addr)?;
            let a = reads::identify(&mut c, to).map_err(fail)?;
            let i = Identify::decode(&a.payload).expect("checked by reads::identify");
            if cli.json {
                println!("{}", render::identify_json(&i));
            } else {
                print!("{}", render::identify_human(&i));
            }
            Ok(exit::OK)
        }
        Cmd::Stats { addr } => {
            let (mut c, to) = open(policy, addr)?;
            let a = reads::call(&mut c, to, Command::GetStats, &[]).map_err(fail)?;
            let mut entries = Vec::new();
            for e in payload::stats(&a.payload) {
                let (name, v) = e.map_err(|_| (exit::ERROR, format!("{}: GET_STATS reply does not decode (Table M-1)", a.from)))?;
                entries.push((String::from_utf8_lossy(name).into_owned(), v));
            }
            if cli.json {
                println!("{}", render::stats_json(&entries));
            } else {
                print!("{}", render::stats_human(&entries));
            }
            Ok(exit::OK)
        }
        Cmd::Logs { addr, from, follow, poll } => logs_cmd(cli.json, policy, addr, *from, *follow, *poll),
        Cmd::Config { op: ConfigOp::Get { addr } } => {
            let (mut c, to) = open(policy, addr)?;
            let cfg = reads::get_config(&mut c, to).map_err(fail)?;
            if cli.json {
                println!("{}", render::config_json(&cfg));
            } else {
                print!("{}", render::config_human(&cfg));
            }
            Ok(exit::OK)
        }
        Cmd::Config { op: ConfigOp::Set(a) } => cli_write::config_set(cli.json, policy, a),
        Cmd::Calibration { op: CalibrationOp::Set(a) } => cli_write::calibration_set(cli.json, policy, a),
        Cmd::Calibration { op: CalibrationOp::Clear { addr, secret } } => cli_write::calibration_clear(cli.json, policy, addr, secret),
        Cmd::Write(w) => cli_write::run(cli.json, policy, w),
        Cmd::Update(u) => cli_update::run(cli.json, policy, u),
        Cmd::Stock(c) => cli_stock::run(cli.json, policy, c),
        Cmd::Calibration { op: CalibrationOp::Get { addr } } => {
            let (mut c, to) = open(policy, addr)?;
            let cal = reads::get_calibration(&mut c, to).map_err(fail)?;
            if cli.json {
                println!("{}", render::calibration_json(&cal));
            } else {
                print!("{}", render::calibration_human(&cal));
            }
            Ok(exit::OK)
        }
    }
}

fn logs_cmd(json: bool, policy: Policy, addr: &str, from: u32, follow: bool, poll: f64) -> Outcome {
    let (mut c, to) = open(policy, addr)?;
    // Without --follow, JSON is one array; with it, one object per line as entries come.
    let mut collected = Vec::new();
    let mut each = |ev: LogEvent| match ev {
        LogEvent::Entry(e) => {
            if json && !follow {
                collected.push(render::log_json(&e));
            } else if json {
                println!("{}", render::log_json(&e));
            } else {
                println!("{}", render::log_human(&e));
            }
        }
        LogEvent::Gap { wanted, got } => eprintln!("g7250ctl: entries {wanted}..{} are no longer in the log", got.wrapping_sub(1)),
        LogEvent::Restarted => eprintln!("g7250ctl: the radio restarted and its log started empty; reading from 0"),
    };
    if !follow {
        reads::read_log(&mut c, to, from, &mut each).map_err(fail)?;
        drop(each);
        if json {
            println!("{}", Json::array(collected));
        }
        return Ok(exit::OK);
    }
    // Runs until interrupted (Ctrl-C ends the process).
    let mut on_silence = |f: &Failure| eprintln!("g7250ctl: {f}; still following");
    reads::follow_log(&mut c, to, from, Duration::from_secs_f64(poll.max(0.05)), &mut each, &mut on_silence, &|| false)
        .map_err(fail)?;
    Ok(exit::OK)
}

fn discover_cmd(json: bool, policy: Policy, wait: f64, broadcast: &[Ipv4Addr], port: u16, p2: bool, p2_port: u16) -> Outcome {
    let wait = Duration::from_secs_f64(wait.max(0.0));
    if p2 {
        let found = discover::p2_all(&discover::targets(broadcast, p2_port), p2_port, wait, policy.interval, policy.retransmits)
            .map_err(|e| (exit::ERROR, e.to_string()))?;
        if json {
            println!("{}", Json::array(found.iter().map(|f| render::p2_json(f.from, &f.reply))));
        } else {
            for f in &found {
                println!("{}", render::p2_human(f.from, &f.reply));
            }
        }
        return if found.is_empty() { Err((exit::NO_REPLY, "no P2 radio answered".into())) } else { Ok(exit::OK) };
    }
    let mut c = client(policy)?;
    let found = discover::identify_all(&mut c, &discover::targets(broadcast, port), wait).map_err(|e| (exit::ERROR, e.to_string()))?;
    let rows: Vec<_> = found.iter().map(|a| (a, if a.ok() { Identify::decode(&a.payload).ok() } else { None })).collect();
    if json {
        println!(
            "{}",
            Json::array(rows.iter().map(|(a, id)| {
                Json::obj()
                    .s("address", &a.from.to_string())
                    .raw("status", a.status_code)
                    .raw("identify", id.as_ref().map(render::identify_json).unwrap_or_else(|| "null".into()))
                    .end()
            }))
        );
    } else {
        for (a, id) in &rows {
            match id {
                Some(i) => println!(
                    "{:<21} {}  {}  fw {}.{}.{} slot {}  level {}  {}",
                    a.from.to_string(),
                    render::mac(&i.mac),
                    String::from_utf8_lossy(i.model),
                    i.running.major,
                    i.running.minor,
                    i.running.build,
                    match i.running_slot {
                        1 => "A",
                        2 => "B",
                        _ => "?",
                    },
                    i.active_level,
                    if i.owner == ([0; 4], 0) { "free" } else { "in use" }
                ),
                None => println!("{:<21} status {}: {}", a.from.to_string(), a.status_code, render::status_text(a.status_code)),
            }
        }
    }
    if found.is_empty() { Err((exit::NO_REPLY, "no G7250 answered".into())) } else { Ok(exit::OK) }
}
