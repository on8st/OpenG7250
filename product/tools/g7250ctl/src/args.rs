// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Parsers for command-line values, in the units of Table M-3 and Table M-5.

use std::net::Ipv4Addr;

use g7250_proto::mgmt::payload::{MAX_ALLOW, MAX_BANDS};

/// A decimal number with at most `decimals` places, scaled to an integer: `parse_fixed("2.5",
/// 2)` = 250. Rejects more places than given, signs and values above `u16::MAX`.
pub fn parse_fixed(s: &str, decimals: u32) -> Result<u16, String> {
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let digits = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
    if int.is_empty() || !digits(int) || !digits(frac) || frac.len() as u32 > decimals {
        return Err(format!("{s:?}: a number with at most {decimals} decimal places"));
    }
    let scale = 10u64.pow(decimals);
    let f: u64 = if frac.is_empty() { 0 } else { frac.parse::<u64>().unwrap() * 10u64.pow(decimals - frac.len() as u32) };
    let v = int.parse::<u64>().map_err(|_| format!("{s:?}: too large"))?.checked_mul(scale).and_then(|x| x.checked_add(f));
    v.and_then(|x| u16::try_from(x).ok()).ok_or_else(|| format!("{s:?}: too large"))
}

/// `INDEX=WATTS`: a C-3 power limit for `tx_band[INDEX]`, in 0.1 W (Table M-3 tag 8).
pub fn parse_band_limit(s: &str) -> Result<(usize, u16), String> {
    let (i, w) = s.split_once('=').ok_or_else(|| format!("{s:?}: INDEX=WATTS, for example 3=50 or 3=12.5"))?;
    let i: usize = i.trim().parse().map_err(|_| format!("{s:?}: bad band index"))?;
    if i >= MAX_BANDS {
        return Err(format!("{s:?}: band index 0-{}", MAX_BANDS - 1));
    }
    Ok((i, parse_fixed(w.trim(), 1)?))
}

/// `A.B.C.D/LEN`: a C-7 allow-list entry (Table M-3 tag 11).
pub fn parse_cidr(s: &str) -> Result<([u8; 4], u8), String> {
    let (a, l) = s.split_once('/').ok_or_else(|| format!("{s:?}: ADDRESS/PREFIX, for example 192.168.1.0/24"))?;
    let a: Ipv4Addr = a.parse().map_err(|_| format!("{s:?}: bad IPv4 address"))?;
    let l: u8 = l.parse().ok().filter(|l| *l <= 32).ok_or_else(|| format!("{s:?}: prefix length 0-32"))?;
    Ok((a.octets(), l))
}

pub fn check_allow_len(n: usize) -> Result<(), String> {
    if n > MAX_ALLOW { Err(format!("the TCI allow-list holds at most {MAX_ALLOW} entries")) } else { Ok(()) }
}

/// `ID=HEX`: a Table M-5 calibration entry with its value in hex (an empty value is `ID=`).
pub fn parse_entry(s: &str) -> Result<(String, Vec<u8>), String> {
    let (id, hex) = s.split_once('=').ok_or_else(|| format!("{s:?}: ID=HEX, for example P-RF-2=000a"))?;
    if id.is_empty() || id.len() > 255 || !id.is_ascii() {
        return Err(format!("{s:?}: the id is 1-255 ASCII characters"));
    }
    if hex.len() % 2 != 0 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("{s:?}: the value is an even number of hex digits"));
    }
    let v = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
    Ok((id.to_string(), v))
}

/// `now` or seconds since 1970-01-01 UTC.
pub fn parse_date(s: &str) -> Result<u32, String> {
    if s == "now" {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?;
        return u32::try_from(t.as_secs()).map_err(|_| "the date does not fit u32".into());
    }
    s.parse().map_err(|_| format!("{s:?}: `now` or seconds since 1970-01-01 UTC"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed() {
        assert_eq!(parse_fixed("2.5", 2), Ok(250));
        assert_eq!(parse_fixed("2.05", 2), Ok(205));
        assert_eq!(parse_fixed("3", 2), Ok(300));
        assert_eq!(parse_fixed("75.5", 1), Ok(755));
        assert!(parse_fixed("2.555", 2).is_err());
        assert!(parse_fixed("-1", 1).is_err());
        assert!(parse_fixed("6553.6", 1).is_err());
        assert_eq!(parse_fixed("6553.5", 1), Ok(65535));
        assert!(parse_fixed(".5", 1).is_err());
    }

    #[test]
    fn band_cidr_entry_date() {
        assert_eq!(parse_band_limit("3=12.5"), Ok((3, 125)));
        assert_eq!(parse_band_limit("0=0"), Ok((0, 0)));
        assert!(parse_band_limit("16=10").is_err());
        assert_eq!(parse_cidr("10.0.0.0/8"), Ok(([10, 0, 0, 0], 8)));
        assert!(parse_cidr("10.0.0.0/33").is_err());
        assert!(parse_cidr("10.0.0.0").is_err());
        assert_eq!(parse_entry("P-RF-2=000a"), Ok(("P-RF-2".into(), vec![0, 10])));
        assert_eq!(parse_entry("P-X="), Ok(("P-X".into(), vec![])));
        assert!(parse_entry("P-RF-2=00a").is_err());
        assert!(parse_entry("=00").is_err());
        assert_eq!(parse_date("1795305600"), Ok(1_795_305_600));
        assert!(parse_date("now").unwrap() > 1_795_000_000);
        assert!(check_allow_len(9).is_err());
    }
}
