// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The read commands of milestone M2: `READ_LOG` in pages (A1.7 L-3), `GET_CONFIG` (A1.6,
//! Table M-3), `GET_CALIBRATION` (A1.12, Tables M-1 and M-5). None needs authentication.

use std::fmt;
use std::net::SocketAddr;
use std::time::Duration;

use g7250_proto::mgmt::payload::{self, Config, Identify};
use g7250_proto::mgmt::Command;

use crate::render;
use crate::transport::{self, Answer, Client};

/// Why a command failed, with its exit code.
#[derive(Debug)]
pub enum Failure {
    Transport(transport::Error),
    /// The radio answered with a status other than 0.
    Status(Answer, Command),
    /// The reply does not decode as Annex M gives it.
    Decode { from: SocketAddr, what: String },
    /// A request that ends in a reset lost its reply (`failure`: the 33, or no reply), and
    /// `IDENTIFY` does not show that it ran (A1.4 "Requests that end in a reset"): `seen` is the
    /// restart it showed, if any. The request is not sent again; the operator decides.
    ResetNotShown { failure: Box<Failure>, cmd: Command, seen: Option<Observed> },
}

/// What `IDENTIFY` showed after a restart: tags 16, 17 and 23.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Observed {
    pub uptime_ms: u32,
    pub reset_cause: u8,
    pub running_slot: u8,
}

/// Tag 17 (Table M-2).
pub fn reset_cause_name(c: u8) -> &'static str {
    match c {
        0 => "unknown",
        1 => "power-on",
        2 => "software",
        3 => "watchdog",
        4 => "brown-out",
        5 => "pin",
        _ => "not defined",
    }
}

impl Failure {
    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Transport(transport::Error::NoReply { .. }) => crate::exit::NO_REPLY,
            Failure::Transport(_) | Failure::Decode { .. } => crate::exit::ERROR,
            Failure::Status(..) => crate::exit::STATUS,
            Failure::ResetNotShown { failure, .. } => failure.exit_code(),
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failure::Transport(e) => e.fmt(f),
            Failure::Status(a, cmd) => {
                write!(f, "{} answered {cmd:?} with status {}: {}", a.from, a.status_code, render::status_text(a.status_code))?;
                if a.status_code == 18 {
                    write!(
                        f,
                        " (radio speaks management {}.{}, this tool {}.{})",
                        a.version.0,
                        a.version.1,
                        g7250_proto::mgmt::VERSION_MAJOR,
                        g7250_proto::mgmt::VERSION_MINOR
                    )?;
                }
                Ok(())
            }
            Failure::Decode { from, what } => write!(f, "{from}: {what}"),
            Failure::ResetNotShown { failure, cmd, seen } => {
                write!(f, "{failure}; its reply was lost, and IDENTIFY does not show that {cmd:?} ran")?;
                match seen {
                    None => write!(f, " (no restart seen)")?,
                    Some(o) if o.reset_cause == 0 => write!(
                        f,
                        ": the radio restarted, cause unknown (tag 17 = 0); uptime {}, slot {} runs",
                        render::uptime(o.uptime_ms),
                        crate::update::slot_name(o.running_slot)
                    )?,
                    Some(o) => write!(
                        f,
                        ": the radio restarted, reset cause {}; uptime {}, slot {} runs",
                        reset_cause_name(o.reset_cause),
                        render::uptime(o.uptime_ms),
                        crate::update::slot_name(o.running_slot)
                    )?,
                }
                write!(f, ". It is not sent again: check `g7250ctl info` and `g7250ctl logs` (A1.4)")
            }
        }
    }
}

impl From<transport::Error> for Failure {
    fn from(e: transport::Error) -> Self {
        Failure::Transport(e)
    }
}

/// One request that must succeed with status 0.
pub fn call(client: &mut Client, to: SocketAddr, cmd: Command, payload: &[u8]) -> Result<Answer, Failure> {
    let a = client.request(to, cmd, payload, None)?;
    if !a.ok() {
        return Err(Failure::Status(a, cmd));
    }
    Ok(a)
}

fn bad(a: &Answer, what: &str) -> Failure {
    Failure::Decode { from: a.from, what: what.to_string() }
}

pub fn identify(client: &mut Client, to: SocketAddr) -> Result<Answer, Failure> {
    let a = call(client, to, Command::Identify, &[])?;
    Identify::decode(&a.payload).map_err(|_| bad(&a, "IDENTIFY reply does not decode (Table M-2)"))?;
    Ok(a)
}

/// One log entry, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub seq: u32,
    pub uptime_ms: u32,
    pub text: String,
}

/// What reading the log reports besides entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogEvent {
    Entry(LogLine),
    /// The first entry returned is later than the one asked for: the ring dropped the ones
    /// between (A1.7).
    Gap { wanted: u32, got: u32 },
    /// The radio restarted and its log started empty (0068 "log lost"): sequence numbers start
    /// at 0 again.
    Restarted,
}

/// Read the log from sequence `from` (0 = oldest) to its end, page by page: each page asks
/// for the sequence after the last entry received (A1.7 L-3; Table M-1). The end is a page
/// with flag bit 1 ("more blocks follow") clear, or an empty page (A1.7; 0074 item 6).
/// Returns the sequence to ask for next.
pub fn read_log(client: &mut Client, to: SocketAddr, from: u32, each: &mut dyn FnMut(LogEvent)) -> Result<u32, Failure> {
    let mut next = from;
    let mut first_page = true;
    loop {
        let a = call(client, to, Command::ReadLog, &next.to_be_bytes())?;
        let mut page = Vec::new();
        for e in payload::log_entries(&a.payload) {
            let e = e.map_err(|_| bad(&a, "READ_LOG reply does not decode (Table M-1)"))?;
            page.push(LogLine { seq: e.seq, uptime_ms: e.uptime_ms, text: String::from_utf8_lossy(e.text).into_owned() });
        }
        // Defensive: entries before the one asked for are not repeated.
        let page: Vec<_> = page.into_iter().filter(|e| next == 0 || e.seq >= next).collect();
        let Some(last) = page.last() else { return Ok(next) };
        if first_page && next != 0 && page[0].seq > next {
            each(LogEvent::Gap { wanted: next, got: page[0].seq });
        }
        first_page = false;
        let after = last.seq.wrapping_add(1);
        for e in page {
            each(LogEvent::Entry(e));
        }
        if after <= next && next != 0 {
            return Err(bad(&a, "READ_LOG pages do not advance"));
        }
        next = after;
        if a.flags & g7250_proto::mgmt::FLAG_MORE == 0 {
            return Ok(next);
        }
    }
}

/// Follow the log: read it to the end, then every `poll` read what is new, until `stop`
/// returns true. A restart is detected from the `IDENTIFY` uptime going back; if the log was
/// kept, the start entry of 0068 continues the sequence; if the page after a restart is empty,
/// the log was lost and reading starts again at 0. While the radio does not answer (a reset),
/// the follow goes on; `on_silence` is told once per silence.
pub fn follow_log(
    client: &mut Client,
    to: SocketAddr,
    from: u32,
    poll: Duration,
    each: &mut dyn FnMut(LogEvent),
    on_silence: &mut dyn FnMut(&Failure),
    stop: &dyn Fn() -> bool,
) -> Result<(), Failure> {
    let mut next = read_log(client, to, from, each)?;
    let mut uptime = uptime_of(client, to)?;
    let mut silent = false;
    while !stop() {
        std::thread::sleep(poll);
        let step = (|| -> Result<(), Failure> {
            let now = uptime_of(client, to)?;
            let restarted = now < uptime;
            uptime = now;
            let before = next;
            next = read_log(client, to, next, each)?;
            if restarted && next == before && before != 0 {
                each(LogEvent::Restarted);
                next = read_log(client, to, 0, each)?;
            }
            Ok(())
        })();
        match step {
            Ok(()) => silent = false,
            Err(e @ Failure::Transport(transport::Error::NoReply { .. })) => {
                if !silent {
                    on_silence(&e);
                }
                silent = true;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn uptime_of(client: &mut Client, to: SocketAddr) -> Result<u32, Failure> {
    let a = identify(client, to)?;
    Ok(Identify::decode(&a.payload).map(|i| i.uptime_ms).unwrap_or(0))
}

pub fn get_config(client: &mut Client, to: SocketAddr) -> Result<Config, Failure> {
    let a = call(client, to, Command::GetConfig, &[])?;
    Config::decode(&a.payload).map_err(|_| bad(&a, "GET_CONFIG reply does not decode (Table M-3)"))
}

/// A calibration set as `GET_CALIBRATION` returns it (Table M-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calibration {
    /// SHA-256 of the set; zeros if there is none.
    pub id: [u8; 32],
    /// Seconds since 1970-01-01 UTC.
    pub date: u32,
    pub source: String,
    /// Table M-5 entries: parameter id, value.
    pub entries: Vec<(String, Vec<u8>)>,
}

impl Calibration {
    pub fn is_none(&self) -> bool {
        self.id == [0; 32]
    }
}

/// Decode a `GET_CALIBRATION` reply: id (32 bytes), then date, source and entries as in
/// `SET_CALIBRATION`.
pub fn decode_calibration(p: &[u8]) -> Option<Calibration> {
    let id: [u8; 32] = p.get(..32)?.try_into().ok()?;
    // No set: Table M-1 gives only "zeros if none" for the id; accept the id alone as well as
    // an id followed by an empty head.
    if p.len() == 32 && id == [0; 32] {
        return Some(Calibration { id, date: 0, source: String::new(), entries: Vec::new() });
    }
    let head = payload::parse_set_calibration(&p[32..]).ok()?;
    let entries = payload::calibration_entries(head.entries)
        .map(|e| e.map(|(k, v)| (String::from_utf8_lossy(k).into_owned(), v.to_vec())))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    Some(Calibration { id, date: head.date, source: String::from_utf8_lossy(head.source).into_owned(), entries })
}

pub fn get_calibration(client: &mut Client, to: SocketAddr) -> Result<Calibration, Failure> {
    let a = call(client, to, Command::GetCalibration, &[])?;
    decode_calibration(&a.payload).ok_or_else(|| bad(&a, "GET_CALIBRATION reply does not decode (Tables M-1, M-5)"))
}
