// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Host end of management payloads that [`crate::mgmt::payload`] only reads on the radio end
//! (Annex M, Table M-1), for `g7250ctl` (0037). Nothing here is used by the firmware.

use crate::mgmt::auth::CHALLENGE_LEN;

/// `READ_LOG` request: the first sequence number wanted, 0 = oldest (Table M-1).
pub fn read_log_request(first: u32) -> [u8; 4] {
    first.to_be_bytes()
}

/// `GET_CHALLENGE` reply: the 16-byte challenge (Table M-1).
pub fn parse_challenge(p: &[u8]) -> Option<[u8; CHALLENGE_LEN]> {
    p.try_into().ok()
}

/// `SWITCH_LEVEL` request: the target level, 1 or 2 (Table M-1).
pub fn switch_level_request(level: u8) -> [u8; 1] {
    [level]
}

/// `SET_CALIBRATION` request (Table M-1): date (u32), source length (u8), source (ASCII), then
/// the entries of Table M-5: id length (u8), id (ASCII), value length (u16), value. Returns
/// the length written; `None` if it does not fit `out`, or a source, id or value is too long
/// for its length field.
pub fn set_calibration_request<'e>(
    date: u32,
    source: &[u8],
    entries: impl IntoIterator<Item = (&'e [u8], &'e [u8])>,
    out: &mut [u8],
) -> Option<usize> {
    let mut n = 0;
    let mut put = |b: &[u8], n: &mut usize| -> Option<()> {
        out.get_mut(*n..*n + b.len())?.copy_from_slice(b);
        *n += b.len();
        Some(())
    };
    put(&date.to_be_bytes(), &mut n)?;
    put(&[u8::try_from(source.len()).ok()?], &mut n)?;
    put(source, &mut n)?;
    for (id, value) in entries {
        put(&[u8::try_from(id.len()).ok()?], &mut n)?;
        put(id, &mut n)?;
        put(&u16::try_from(value.len()).ok()?.to_be_bytes(), &mut n)?;
        put(value, &mut n)?;
    }
    Some(n)
}

/// `SET_CALIBRATION` reply (Table M-1): calibration set id (32 bytes), TX calibrated (u8).
pub fn parse_set_calibration_reply(p: &[u8]) -> Option<([u8; 32], bool)> {
    if p.len() != 33 {
        return None;
    }
    let mut id = [0u8; 32];
    id.copy_from_slice(&p[..32]);
    Some((id, p[32] != 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::payload::{calibration_entries, parse_set_calibration};

    #[test]
    fn calibration_request_reads_back_on_the_radio_end() {
        let mut b = [0u8; 200];
        let e: [(&[u8], &[u8]); 2] = [(b"P-RF-2", &[0, 10]), (b"P-PWR-2", &[1, 2, 3])];
        let n = set_calibration_request(1_795_305_600, b"bench", e, &mut b).unwrap();
        assert_eq!(n, 4 + 1 + 5 + (1 + 6 + 2 + 2) + (1 + 7 + 2 + 3));
        let h = parse_set_calibration(&b[..n]).unwrap();
        assert_eq!((h.date, h.source), (1_795_305_600, &b"bench"[..]));
        let got: Vec<_> = calibration_entries(h.entries).map(|r| r.unwrap()).collect();
        assert_eq!(got, e.to_vec());
    }

    #[test]
    fn calibration_request_limits() {
        let mut small = [0u8; 8];
        assert!(set_calibration_request(0, b"bench", [], &mut small).is_none());
        let mut b = [0u8; 600];
        let long = [b'x'; 256];
        assert!(set_calibration_request(0, &long, [], &mut b).is_none());
        let e: [(&[u8], &[u8]); 1] = [(&long, &[])];
        assert!(set_calibration_request(0, b"", e, &mut b).is_none());
        assert_eq!(set_calibration_request(7, b"", [], &mut b), Some(5));
    }

    #[test]
    fn small_payloads() {
        assert_eq!(read_log_request(0x0102_0304), [1, 2, 3, 4]);
        assert_eq!(switch_level_request(2), [2]);
        assert_eq!(parse_challenge(&[7; 16]), Some([7; 16]));
        assert_eq!(parse_challenge(&[7; 15]), None);
        let mut r = [0xAB; 33];
        r[32] = 1;
        assert_eq!(parse_set_calibration_reply(&r), Some(([0xAB; 32], true)));
        assert_eq!(parse_set_calibration_reply(&r[..32]), None);
    }
}
