// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The capability record dump (M6; spec B4). Read-only: a `CAP_REQUEST` write starts no
//! session and takes nothing over (B4.5).
//!
//! - The request goes to the radio's register port (7250, B8.1) from a socket whose own port is
//!   never 7250 (the radio drops those, B8.5).
//! - Only a packet from the radio's address and register port, with the echo of the request and
//!   the whole record, is taken (B4.4 item 1). Anything else from there (a register push, a part
//!   of the unasked record) is not a record and is skipped.
//! - No answer after 1 s: the request is sent again, at most once per second, up to 5 times
//!   (B4.4 item 4).

use std::fmt::Write as _;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use g7250_proto::caprec_host::{self, off, CapRecord, RecordError};

use crate::render::Json;

/// B4.4 item 4: 1 s between requests, and the user is told after 5.
pub const INTERVAL: Duration = Duration::from_secs(1);
pub const TRIES: u32 = 5;

#[derive(Debug)]
pub enum CapsError {
    Io(std::io::Error),
    /// No answer after `tries` requests.
    NoAnswer { to: SocketAddr, tries: u32 },
    /// The answer (it held the echo) is not a usable record.
    Bad { from: SocketAddr, why: RecordError },
}

impl std::fmt::Display for CapsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapsError::Io(e) => write!(f, "{e}"),
            CapsError::NoAnswer { to, tries } => write!(f, "no capability record from {to} after {tries} requests, 1 s apart (B4.4 item 4)"),
            CapsError::Bad { from, why } => match why {
                RecordError::Major(m) => write!(
                    f,
                    "{from} sends capability record major version {m}; this tool knows {}: the firmware is newer than this tool, so the record is not read (B4.10 rule 4)",
                    caprec_host::KNOWN_MAJOR
                ),
                RecordError::Incomplete { have, need } => write!(f, "{from}: incomplete capability record: {have} of {need} registers in the answer; not a record"),
                RecordError::Repeated(a) => write!(f, "{from}: register 0x{a:04x} appears twice in the answer; not a record"),
                RecordError::Magic(m) => write!(f, "{from}: not a G7250 capability record (cap_magic {})", m.map_or("missing".into(), |m| format!("0x{m:08x}"))),
                RecordError::Length(l) => write!(f, "{from}: cap_length {} is outside 3 to 239 (B4.2)", l.map_or("missing".into(), |l| l.to_string())),
                RecordError::NoEcho => write!(f, "{from}: no echo of the request"),
            },
        }
    }
}

/// A socket for the request: any local port but 7250 (B8.5).
pub fn socket() -> std::io::Result<UdpSocket> {
    for _ in 0..4 {
        let s = UdpSocket::bind("0.0.0.0:0")?;
        if s.local_addr()?.port() != caprec_host::REGISTER_PORT {
            return Ok(s);
        }
    }
    Err(std::io::Error::other("the system keeps giving local port 7250"))
}

/// What came in besides the answer, for `--verbose`-like reporting and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Skipped {
    /// From another address or port than the radio's register port.
    pub other_source: u32,
    /// From the register port, but without the echo (pushes, parts of the unasked record).
    pub not_an_answer: u32,
}

/// Ask for the record and read the answer.
pub fn request(sock: &UdpSocket, to: SocketAddr, interval: Duration, tries: u32, skipped: &mut Skipped) -> Result<CapRecord, CapsError> {
    if sock.local_addr().map_err(CapsError::Io)?.port() == caprec_host::REGISTER_PORT {
        return Err(CapsError::Io(std::io::Error::other("refusing to send from local port 7250 (B8.5)")));
    }
    let req = caprec_host::cap_request();
    let mut buf = [0u8; 1500];
    for _ in 0..tries {
        sock.send_to(&req, to).map_err(CapsError::Io)?;
        let end = Instant::now() + interval;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            sock.set_read_timeout(Some(left)).map_err(CapsError::Io)?;
            let (n, from) = match sock.recv_from(&mut buf) {
                Ok(r) => r,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
                Err(e) => return Err(CapsError::Io(e)),
            };
            if from != to {
                skipped.other_source += 1;
                continue;
            }
            match caprec_host::parse_answer(&buf[..n]) {
                Ok(r) => return Ok(r),
                Err(RecordError::NoEcho) => skipped.not_an_answer += 1,
                Err(why) => return Err(CapsError::Bad { from, why }),
            }
        }
    }
    Err(CapsError::NoAnswer { to, tries })
}

// Rendering. Every line shows the raw register; bits and values the spec leaves open are
// named "not defined" and shown raw.

fn bits(v: u32, names: &[(u32, &str)]) -> String {
    let mut known = 0u32;
    let mut set = Vec::new();
    for (b, n) in names {
        known |= 1 << b;
        if v & (1 << b) != 0 {
            set.push(*n);
        }
    }
    let mut s = if set.is_empty() { "none".to_string() } else { set.join(", ") };
    undefined(&mut s, v & !known);
    s
}

fn undefined(s: &mut String, rest: u32) {
    if rest != 0 {
        let _ = write!(s, "; bits not defined: 0x{rest:08x}");
    }
}

fn hz(v: u32) -> String {
    format!("{:.6} MHz", v as f64 / 1e6)
}

fn watts(tenths: u16) -> String {
    format!("{}.{} W", tenths / 10, tenths % 10)
}

/// One field: name, offset, and its meaning as text.
fn fields(r: &CapRecord) -> Vec<(String, usize, Option<String>)> {
    let mut out: Vec<(String, usize, Option<String>)> = Vec::new();
    let mut f = |name: &str, o: usize, show: &dyn Fn(u32) -> String| out.push((name.to_string(), o, r.get(o).map(show)));
    f("cap_magic", off::MAGIC, &|_| "\"G725\"".into());
    f("cap_version", off::VERSION, &|v| format!("{}.{}", v >> 16, v & 0xFFFF));
    f("cap_length", off::LENGTH, &|v| format!("{v} registers"));
    f("regmap_version", off::REGMAP_VERSION, &|v| format!("{}.{}", v >> 16, v & 0xFFFF));
    f("fw_version", off::FW_VERSION, &|v| format!("{}.{}.{}", v >> 24, (v >> 16) & 0xFF, v & 0xFFFF));
    f("p2_version", off::P2_VERSION, &|v| v.to_string());
    f("hw_revision", off::HW_REVISION, &|v| if v == 0 { "0 (not identified)".into() } else { v.to_string() });
    f("level", off::LEVEL, &|v| {
        let mut s = bits(v & 0x07, &[(0, "level 1 present"), (1, "level 2 present"), (2, "level 2 active")]);
        let _ = write!(s, "; active level {}", (v >> 4) & 0xF);
        undefined(&mut s, v & !0xF7);
        s
    });
    f("streams", off::STREAMS, &|v| bits(v, &[(0, "RX IQ"), (1, "TX IQ"), (2, "DDC audio"), (3, "microphone"), (4, "wideband"), (5, "IQ time stamps")]));
    f("num_adc", off::NUM_ADC, &|v| v.to_string());
    f("num_ddc", off::NUM_DDC, &|v| v.to_string());
    f("num_duc", off::NUM_DUC, &|v| v.to_string());
    f("ddc_rates", off::DDC_RATES, &|v| bits(v, &[(0, "48"), (1, "96"), (2, "192"), (3, "384"), (4, "768"), (5, "1536 ksps")]));
    f("duc_format", off::DUC_FORMAT, &|v| {
        let mut s = format!("{} ksps, {} bits", v >> 16, v & 0xFF);
        undefined(&mut s, v & 0xFF00);
        s
    });
    f("dsp_clock_hz", off::DSP_CLOCK_HZ, &|v| format!("{v} Hz"));
    f("rx_range_count", off::RX_RANGE_COUNT, &|v| if v <= 4 { v.to_string() } else { format!("{v} (above 4: not defined)") });
    for i in 0..4 {
        if r.get(off::RX_RANGE_COUNT).is_some_and(|c| (i as u32) < c.min(4)) {
            let (a, b) = r.rx_range(i).unwrap_or((0, 0));
            out.push((format!("rx_range[{i}]"), off::RX_RANGE + 2 * i, r.rx_range(i).map(|_| format!("{} to {}", hz(a), hz(b)))));
        }
    }
    let mut f = |name: &str, o: usize, show: &dyn Fn(u32) -> String| out.push((name.to_string(), o, r.get(o).map(show)));
    f("antennas", off::ANTENNAS, &|v| {
        let mut s = format!("{} connectors", v & 0xFF);
        for (b, n) in [(8, "host can select"), (9, "separate RX-only input")] {
            if v & (1 << b) != 0 {
                let _ = write!(s, ", {n}");
            }
        }
        undefined(&mut s, v & !0x3FF);
        s
    });
    f("atu", off::ATU, &|v| bits(v, &[(0, "internal ATU"), (1, "host controls it"), (2, "per-band memory"), (3, "external tuner interface"), (4, "host controls the external tuner")]));
    f("atu_range", off::ATU_RANGE, &|v| format!("{} to {} ohm", v >> 16, v & 0xFFFF));
    f("telemetry", off::TELEMETRY, &|v| {
        bits(v, &[(0, "forward power"), (1, "reverse power"), (2, "PA temperature"), (3, "supply voltage"), (4, "supply current"), (5, "PA current"), (6, "ADC overload")])
    });
    f("tx", off::TX, &|v| bits(v, &[(0, "TX hardware"), (1, "TX implemented"), (2, "arm required"), (3, "mode class required"), (4, "TX calibrated")]));
    f("tx_band_count", off::TX_BAND_COUNT, &|v| if v <= 16 { v.to_string() } else { format!("{v} (above 16: not defined)") });
    for i in 0..16 {
        if r.get(off::TX_BAND_COUNT).is_some_and(|c| (i as u32) < c.min(16)) {
            out.push((
                format!("tx_band[{i}]"),
                off::TX_BAND + 3 * i,
                r.tx_band(i).map(|b| format!("{} to {}, max {} SSB/CW/FM, {} AM", hz(b.start_hz), hz(b.stop_hz), watts(b.max_ssb_cw_fm), watts(b.max_am))),
            ));
        }
    }
    let mut f = |name: &str, o: usize, show: &dyn Fn(u32) -> String| out.push((name.to_string(), o, r.get(o).map(show)));
    f("mgmt", off::MGMT, &|v| {
        let mut s = if v & 1 != 0 { format!("present, port {}", v >> 16) } else { "absent".into() };
        undefined(&mut s, v & 0xFFFE);
        s
    });
    f("regmap_groups", off::REGMAP_GROUPS, &|v| {
        bits(
            v,
            &[(0, "0 capability"), (1, "1 session"), (2, "2 TX"), (3, "3 telemetry"), (4, "4 ATU"), (5, "5 antenna"), (6, "6 front end"), (7, "7 external interfaces")],
        )
    });
    f("link", off::LINK, &|v| {
        let mut s = format!("{} Mbit/s minimum", v & 0xFFFF);
        undefined(&mut s, v & 0xFFFF_0000);
        s
    });
    f("wb_format", off::WB_FORMAT, &|v| if v == 0 { "0 (no wideband)".into() } else { format!("{} samples, {} bits, every {} ms", v >> 16, (v >> 8) & 0xFF, v & 0xFF) });
    f("wb_frame", off::WB_FRAME, &|v| {
        let mut s = format!("{} packets per frame", v & 0xFF);
        undefined(&mut s, v & !0xFF);
        s
    });
    f("cal_info", off::CAL_INFO, &|v| {
        let mut s = bits(v & 0x1F, &[(0, "constants per band"), (1, "forward"), (2, "reverse"), (3, "exciter"), (4, "supply volts")]);
        let _ = write!(s, "; {} sets; N = {}", (v >> 8) & 0xFF, v >> 16);
        undefined(&mut s, v & 0xE0);
        s
    });
    f("cal_supply_c", off::CAL_SUPPLY_C, &|v| format!("{v} x 1e-6"));
    let sets = r.get(off::CAL_INFO).map_or(0, |v| ((v >> 8) & 0xFF) as usize).min(16);
    for i in 0..sets {
        out.push((
            format!("cal_power[{i}]"),
            off::CAL_POWER + 6 * i,
            r.cal_power(i).map(|c| format!("forward {} {}, reverse {} {}, exciter {} {} (x 1e-6)", c[0], c[1], c[2], c[3], c[4], c[5])),
        ));
    }
    let mut f = |name: &str, o: usize, show: &dyn Fn(u32) -> String| out.push((name.to_string(), o, r.get(o).map(show)));
    f("status_fill", off::STATUS_FILL, &|v| {
        bits(
            v,
            &[
                (0, "byte 4 [0]"),
                (1, "byte 4 [1]-[2]"),
                (2, "byte 4 [4]"),
                (3, "byte 5"),
                (4, "bytes 6-7"),
                (5, "bytes 14-15"),
                (6, "bytes 22-23"),
                (7, "byte 30"),
                (8, "bytes 31-36"),
                (9, "bytes 39-40"),
                (10, "bytes 49-50"),
            ],
        )
    });
    f("clock_ref", off::CLOCK_REF, &|v| bits(v, &[(0, "reference PLL present")]));
    f("coupler", off::COUPLER, &|v| {
        let mut s = match v & 0xF {
            0 => "not reported".to_string(),
            1 => "between PA and ATU".into(),
            2 => "antenna side of the ATU".into(),
            x => format!("{x} (not defined)"),
        };
        undefined(&mut s, v & !0xF);
        s
    });
    f("tlm_rates", off::TLM_RATES, &|v| format!("snapshot every {} ms, {} ms during TX, events at least {} ms apart", v >> 16, (v >> 8) & 0xFF, v & 0xFF));
    f("status_rates", off::STATUS_RATES, &|v| format!("{} ms, {} ms while transmitting", v >> 16, v & 0xFFFF));
    f("tlm_count", off::TLM_COUNT, &|v| if v <= 8 { v.to_string() } else { format!("{v} (above 8: not defined)") });
    for i in 0..8 {
        if r.get(off::TLM_COUNT).is_some_and(|c| (i as u32) < c.min(8)) {
            out.push((
                format!("tlm_desc[{i}]"),
                off::TLM_DESC + 2 * i,
                r.tlm_desc(i).map(|d| format!("register 0x{:04x} every {} ms, resolution {}, hysteresis {} (milli-units)", d.address, d.update_ms, d.resolution, d.hysteresis)),
            ));
        }
    }
    out
}

fn product(r: &CapRecord) -> String {
    match r.text4(off::PRODUCT_NAME) {
        Some((t, n)) if t[..n].iter().all(|b| b.is_ascii_graphic() || *b == b' ') => String::from_utf8_lossy(&t[..n]).into_owned(),
        Some((t, n)) => format!("{:02x?} (not ASCII)", &t[..n]),
        None => "absent".into(),
    }
}

/// Text output: one line per field, with the raw register.
pub fn human(r: &CapRecord, raw: bool) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{:<16}{}", "product_name", product(r));
    for (name, o, v) in fields(r) {
        match v {
            Some(v) => {
                let _ = writeln!(s, "{name:<16}{v}  [0x{:02x}: 0x{:08x}]", o, r.get(o).unwrap());
            }
            None => {
                let _ = writeln!(s, "{name:<16}absent (beyond cap_length)");
            }
        }
    }
    let (major, minor) = r.version();
    if (major, minor) > (caprec_host::KNOWN_MAJOR, caprec_host::KNOWN_MINOR) || r.len() > off::END_V1_1 {
        let _ = writeln!(s, "registers 0x{:02x}-0x{:02x}: fields of a later minor version, not defined here (raw below)", off::END_V1_1, r.len().saturating_sub(1));
    }
    if raw || r.len() > off::END_V1_1 {
        for (i, v) in r.registers().iter().enumerate() {
            if raw || i >= off::END_V1_1 {
                let _ = writeln!(s, "  0x{i:02x}  0x{v:08x}");
            }
        }
    }
    s
}

/// JSON output: every field as its raw register value, the product name as text, and every
/// register.
pub fn json(r: &CapRecord) -> String {
    let mut fields_obj = Json::obj();
    for (name, o, _) in fields(r) {
        if let Some(v) = r.get(o) {
            fields_obj = fields_obj.raw(&name, v);
        }
    }
    let (major, minor) = r.version();
    let regs: Vec<String> = r.registers().iter().map(|v| v.to_string()).collect();
    Json::obj()
        .s("product_name", &product(r))
        .s("version", &format!("{major}.{minor}"))
        .raw("length", r.len())
        .raw("fields", fields_obj.end())
        .raw("registers", format!("[{}]", regs.join(",")))
        .end()
}
