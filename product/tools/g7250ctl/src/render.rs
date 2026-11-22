// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Human-readable and JSON output. JSON is written by hand (no dependency): objects of
//! strings, numbers, booleans and arrays, keys in a fixed order.

use std::fmt::Write as _;

use g7250_proto::discovery_host;
use g7250_proto::mgmt::payload::{Identify, Version};
use g7250_proto::mgmt::Status;

/// Meaning of a status code (Table A1-3).
pub fn status_text(code: u16) -> &'static str {
    use Status::*;
    match Status::from_u16(code) {
        Some(Ok) => "OK",
        Some(InProgress) => "accepted, in progress",
        Some(UnknownCommand) => "unknown command",
        Some(Malformed) => "malformed",
        Some(UnsupportedVersion) => "unsupported protocol version",
        Some(TooLarge) => "too large",
        Some(AuthRequired) => "authentication required",
        Some(AuthFailed) => "authentication failed",
        Some(RateLimited) => "rate-limited",
        Some(Busy) => "busy (another writer)",
        Some(FirstSecretClosed) => "first-secret window closed or secret already set",
        Some(InvalidConfig) => "invalid configuration",
        Some(NotAllowed) => "not allowed (would loosen a limit beyond its built-in value, or touch TX)",
        Some(Armed) => "refused while armed or transmitting",
        Some(WrongHardware) => "wrong hardware",
        Some(OlderImage) => "older image without downgrade flag",
        Some(BadBlockCrc) => "bad block CRC",
        Some(BlockOutOfOrder) => "block out of order",
        Some(VerificationFailed) => "verification failed",
        Some(NotSupported) => "not supported in this firmware",
        Some(P2SessionRuns) => "refused while a P2 session runs",
        Some(UpdateInProgress) => "refused while a firmware update is in progress",
        Some(RfPresent) => "reset refused: RF present",
        Some(OneWayInProgress) => "not possible: one-way installation in progress",
        Some(ImageTooLarge) => "image too large",
        Some(NeedsCalibration) => "refused: the value needs the calibration set",
        Some(RunningSlot) => "image linked for the running slot",
        None => "unknown status code",
    }
}

pub fn mac(m: &[u8; 6]) -> String {
    m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

fn version(v: Version) -> String {
    format!("{}.{}.{}", v.major, v.minor, v.build)
}

fn version_or_empty(v: Version) -> String {
    if v.is_empty() { "empty".into() } else { version(v) }
}

fn integrity(b: u8) -> &'static str {
    match b {
        0 => "none",
        1 => "good",
        2 => "bad",
        _ => "?",
    }
}

fn update_state(b: u8) -> &'static str {
    match b {
        0 => "normal",
        1 => "trial boot",
        2 => "rolled back",
        3 => "installer",
        _ => "?",
    }
}

fn slot(b: u8) -> &'static str {
    match b {
        1 => "A",
        2 => "B",
        _ => "?",
    }
}

fn controller(b: u8) -> &'static str {
    match b {
        0 => "unknown",
        1 => "not detected",
        2 => "detected",
        _ => "?",
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Uptime as `1d 02:03:04.567`.
pub fn uptime(ms: u32) -> String {
    let (d, rest) = (ms / 86_400_000, ms % 86_400_000);
    let (h, m, s, f) = (rest / 3_600_000, rest / 60_000 % 60, rest / 1000 % 60, rest % 1000);
    if d > 0 { format!("{d}d {h:02}:{m:02}:{s:02}.{f:03}") } else { format!("{h:02}:{m:02}:{s:02}.{f:03}") }
}

/// Seconds since 1970-01-01 UTC as `YYYY-MM-DD hh:mm:ss UTC` (civil-from-days, proleptic
/// Gregorian).
pub fn utc(secs: u32) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem / 60 % 60, rem % 60)
}

fn layout(l: &[u8; 6]) -> String {
    let one = |name: &str, first: u8, n: u8| {
        if n == 0 { format!("{name} -") } else { format!("{name} sectors {first}-{}", first as u16 + n as u16 - 1) }
    };
    format!("{}, {}, {}", one("slot A", l[0], l[1]), one("slot B", l[2], l[3]), one("store", l[4], l[5]))
}

/// The `IDENTIFY` record for a person.
pub fn identify_human(i: &Identify<'_>) -> String {
    let mut s = String::new();
    let mut line = |k: &str, v: String| {
        let _ = writeln!(s, "{k:<13}{v}");
    };
    line("model", text(i.model));
    line("mac", mac(&i.mac));
    line("serial", i.serial.map(text).unwrap_or_else(|| "-".into()));
    line(
        "running",
        format!("{} in slot {} ({})", version(i.running), slot(i.running_slot), integrity(i.integrity[0])),
    );
    line("other slot", format!("{} ({})", version_or_empty(i.other_slot), integrity(i.integrity[1])));
    line("boot stage", version(i.boot_stage));
    line(
        "hardware",
        if i.hw_revision == 0 { "revision 0 (not identified)".into() } else { format!("revision {}", i.hw_revision) },
    );
    let mut upd = update_state(i.update_state).to_string();
    if i.update_state == 3 || i.sectors_written != 0 {
        let _ = write!(upd, ", {} sectors written", i.sectors_written);
    }
    line("update", upd);
    line(
        "stock",
        format!(
            "stock image {}, vendor recovery {}",
            if i.stock_flags & 1 != 0 { "retained" } else { "not retained" },
            if i.stock_flags & 2 != 0 { "supported" } else { "not supported" }
        ),
    );
    let mut sec = if i.secret_set { "set".to_string() } else { "not set".to_string() };
    if i.window.0 {
        let _ = write!(sec, "; first-secret window open, {} s left", i.window.1);
    }
    line("secret", sec);
    line("level", i.active_level.to_string());
    line(
        "owner",
        if i.owner == ([0; 4], 0) {
            "none".into()
        } else {
            let o = i.owner.0;
            format!("{}.{}.{}.{}:{}", o[0], o[1], o[2], o[3], i.owner.1)
        },
    );
    line("TX_ARM_STATE", format!("0x{:08x}", i.tx_arm_state));
    line("uptime", uptime(i.uptime_ms));
    line("reset cause", i.reset_cause.to_string());
    let v = i.versions;
    line("protocols", format!("management {}.{}, P2 {}, record {}.{}", v[0], v[1], v[2], v[3], v[4]));
    line("controller", controller(i.controller).into());
    line(
        "calibration",
        if i.calibration_id == [0; 32] {
            format!("none; TX calibrated: {}", if i.tx_calibrated { "yes" } else { "no" })
        } else {
            format!(
                "set {}…, {}; TX calibrated: {}",
                hex(&i.calibration_id[..8]),
                utc(i.calibration_date),
                if i.tx_calibrated { "yes" } else { "no" }
            )
        },
    );
    if i.ll_unprobed {
        line("link-local", "address taken without a complete probe".into());
    }
    line("flash", layout(&i.layout));
    s
}

/// Minimal JSON writer.
pub struct Json(String);

impl Json {
    pub fn str(s: &str) -> String {
        let mut o = String::with_capacity(s.len() + 2);
        o.push('"');
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(o, "\\u{:04x}", c as u32);
                }
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }
    pub fn obj() -> Json {
        Json(String::from("{"))
    }
    fn key(&mut self, k: &str) {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        self.0.push_str(&Json::str(k));
        self.0.push(':');
    }
    /// A raw JSON value (number, bool, nested object or array).
    pub fn raw(mut self, k: &str, v: impl std::fmt::Display) -> Json {
        self.key(k);
        let _ = write!(self.0, "{v}");
        self
    }
    pub fn s(mut self, k: &str, v: &str) -> Json {
        self.key(k);
        self.0.push_str(&Json::str(v));
        self
    }
    pub fn end(mut self) -> String {
        self.0.push('}');
        self.0
    }
    pub fn array(items: impl IntoIterator<Item = String>) -> String {
        format!("[{}]", items.into_iter().collect::<Vec<_>>().join(","))
    }
}

fn json_version(v: Version) -> String {
    Json::obj().raw("major", v.major).raw("minor", v.minor).raw("build", v.build).end()
}

/// The `IDENTIFY` record as JSON, every tag of Table M-2 under a stable name.
pub fn identify_json(i: &Identify<'_>) -> String {
    let o = i.owner.0;
    let l = i.layout;
    let part = |f: u8, n: u8| Json::obj().raw("first_sector", f).raw("sectors", n).end();
    Json::obj()
        .s("model", &text(i.model))
        .s("mac", &mac(&i.mac))
        .raw("serial", i.serial.map(|s| Json::str(&text(s))).unwrap_or_else(|| "null".into()))
        .raw("running", json_version(i.running))
        .raw("other_slot", json_version(i.other_slot))
        .raw("boot_stage", json_version(i.boot_stage))
        .raw("hw_revision", i.hw_revision)
        .raw("stock_image_retained", i.stock_flags & 1 != 0)
        .raw("vendor_recovery_supported", i.stock_flags & 2 != 0)
        .s("integrity_running", integrity(i.integrity[0]))
        .s("integrity_other", integrity(i.integrity[1]))
        .s("update_state", update_state(i.update_state))
        .raw("secret_set", i.secret_set)
        .raw("first_secret_window_open", i.window.0)
        .raw("first_secret_window_seconds", i.window.1)
        .raw("active_level", i.active_level)
        .raw(
            "owner",
            if i.owner == ([0; 4], 0) {
                "null".into()
            } else {
                Json::str(&format!("{}.{}.{}.{}:{}", o[0], o[1], o[2], o[3], i.owner.1))
            },
        )
        .raw("tx_arm_state", i.tx_arm_state)
        .raw("uptime_ms", i.uptime_ms)
        .raw("reset_cause", i.reset_cause)
        .raw(
            "protocols",
            Json::obj()
                .s("management", &format!("{}.{}", i.versions[0], i.versions[1]))
                .raw("p2", i.versions[2])
                .s("record", &format!("{}.{}", i.versions[3], i.versions[4]))
                .end(),
        )
        .s("controller", controller(i.controller))
        .raw("tx_calibrated", i.tx_calibrated)
        .s("calibration_id", &hex(&i.calibration_id))
        .raw("calibration_date", i.calibration_date)
        .raw("sectors_written", i.sectors_written)
        .s("running_slot", slot(i.running_slot))
        .raw("link_local_unprobed", i.ll_unprobed)
        .raw("flash", Json::obj().raw("slot_a", part(l[0], l[1])).raw("slot_b", part(l[2], l[3])).raw("store", part(l[4], l[5])).end())
        .end()
}

/// `GET_STATS` entries for a person: one `name value` per line, names aligned.
pub fn stats_human(entries: &[(String, u64)]) -> String {
    let w = entries.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
    entries.iter().map(|(n, v)| format!("{n:<w$}  {v}\n")).collect()
}

pub fn stats_json(entries: &[(String, u64)]) -> String {
    let mut o = Json::obj();
    for (n, v) in entries {
        o = o.raw(n, v);
    }
    o.end()
}

pub fn p2_human(from: std::net::SocketAddr, r: &discovery_host::Reply) -> String {
    format!(
        "{:<21} {}  board {}{}  P2 {}.{}  fw {}.{}{}  {}  ddc {}",
        from.ip().to_string(),
        mac(&r.mac),
        r.board_type,
        if r.is_g7250() { " (G7250)" } else { "" },
        r.p2_version / 10,
        r.p2_version % 10,
        r.fw_code / 10,
        r.fw_code % 10,
        if r.beta != 0 { " beta" } else { "" },
        if r.status == discovery_host::STATUS_IN_USE { "in use" } else { "available" },
        r.num_ddc
    )
}

pub fn p2_json(from: std::net::SocketAddr, r: &discovery_host::Reply) -> String {
    Json::obj()
        .s("address", &from.ip().to_string())
        .s("mac", &mac(&r.mac))
        .raw("board_type", r.board_type)
        .raw("p2_version", r.p2_version)
        .raw("fw_code", r.fw_code)
        .raw("beta", r.beta != 0)
        .s("status", if r.status == discovery_host::STATUS_IN_USE { "in_use" } else { "available" })
        .raw("num_ddc", r.num_ddc)
        .raw("phase_word", r.phase_word)
        .raw("endian_modes", r.endian_modes)
        .end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_uptime() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc(1_795_305_600), "2026-11-22 00:00:00 UTC");
        assert_eq!(uptime(3_723_456), "01:02:03.456");
        assert_eq!(uptime(90_061_001), "1d 01:01:01.001");
    }

    #[test]
    fn json_escapes() {
        assert_eq!(Json::str("a\"b\\c\n\u{1}"), r#""a\"b\\c\n\u0001""#);
        assert_eq!(Json::obj().raw("n", 1).s("s", "x").end(), r#"{"n":1,"s":"x"}"#);
    }

    #[test]
    fn every_table_a1_3_code_has_a_text() {
        for c in 0..=70u16 {
            if Status::from_u16(c).is_some() {
                assert_ne!(status_text(c), "unknown status code", "{c}");
            }
        }
        assert_eq!(status_text(2), "unknown status code");
    }
}
