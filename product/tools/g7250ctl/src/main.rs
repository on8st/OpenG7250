// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! `g7250ctl` command line. See README.md.

use std::net::Ipv4Addr;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use g7250_proto::mgmt::payload::{self, Identify};
use g7250_proto::mgmt::Command;
use g7250_proto::{discovery, mgmt};

use g7250ctl::render::{self, Json};
use g7250ctl::transport::{Answer, Client, Policy};
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

type Outcome = Result<u8, (u8, String)>;

fn client(policy: Policy) -> Result<Client, (u8, String)> {
    let mut c = Client::new().map_err(|e| (exit::ERROR, format!("cannot open a UDP socket: {e}")))?;
    c.policy = policy;
    Ok(c)
}

/// One request to one radio; a non-zero status is reported and ends the command.
fn call(policy: Policy, addr: &str, cmd: Command, payload: &[u8]) -> Result<Answer, (u8, String)> {
    let to = resolve(addr, mgmt::PORT).map_err(|e| (exit::USAGE, e))?;
    let mut c = client(policy)?;
    let a = c.request(to, cmd, payload, None).map_err(|e| match e {
        g7250ctl::transport::Error::NoReply { .. } => (exit::NO_REPLY, e.to_string()),
        _ => (exit::ERROR, e.to_string()),
    })?;
    if !a.ok() {
        let mut msg = format!("{} answered {cmd:?} with status {}: {}", a.from, a.status_code, render::status_text(a.status_code));
        if a.status_code == 18 {
            msg.push_str(&format!(" (radio speaks management {}.{}, this tool {}.{})", a.version.0, a.version.1, mgmt::VERSION_MAJOR, mgmt::VERSION_MINOR));
        }
        return Err((exit::STATUS, msg));
    }
    Ok(a)
}

fn decode_identify(a: &Answer) -> Result<Identify<'_>, (u8, String)> {
    Identify::decode(&a.payload).map_err(|_| (exit::ERROR, format!("{}: IDENTIFY reply does not decode (Table M-2)", a.from)))
}

fn run(cli: &Cli, policy: Policy) -> Outcome {
    match &cli.cmd {
        Cmd::Discover { wait, broadcast, port, p2 } => {
            let wait = Duration::from_secs_f64(wait.max(0.0));
            if *p2 {
                let found = discover::p2_all(&discover::targets(broadcast, discovery::PORT), discovery::PORT, wait, policy.interval, policy.retransmits)
                    .map_err(|e| (exit::ERROR, e.to_string()))?;
                if cli.json {
                    println!("{}", Json::array(found.iter().map(|f| render::p2_json(f.from, &f.reply))));
                } else {
                    for f in &found {
                        println!("{}", render::p2_human(f.from, &f.reply));
                    }
                }
                return if found.is_empty() { Err((exit::NO_REPLY, "no P2 radio answered".into())) } else { Ok(exit::OK) };
            }
            let mut c = client(policy)?;
            let found = discover::identify_all(&mut c, &discover::targets(broadcast, *port), wait).map_err(|e| (exit::ERROR, e.to_string()))?;
            let mut rows = Vec::new();
            for a in &found {
                let id = if a.ok() { Identify::decode(&a.payload).ok() } else { None };
                rows.push((a, id));
            }
            if cli.json {
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
                            if i.owner == ([0; 4], 0) { "free".to_string() } else { "in use".to_string() }
                        ),
                        None => println!("{:<21} status {}: {}", a.from.to_string(), a.status_code, render::status_text(a.status_code)),
                    }
                }
            }
            if found.is_empty() { Err((exit::NO_REPLY, "no G7250 answered".into())) } else { Ok(exit::OK) }
        }
        Cmd::Info { addr } => {
            let a = call(policy, addr, Command::Identify, &[])?;
            let i = decode_identify(&a)?;
            if cli.json {
                println!("{}", render::identify_json(&i));
            } else {
                print!("{}", render::identify_human(&i));
            }
            Ok(exit::OK)
        }
        Cmd::Stats { addr } => {
            let a = call(policy, addr, Command::GetStats, &[])?;
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
    }
}
