// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Host end of management payloads that [`crate::mgmt::payload`] only reads on the radio end
//! (Annex M, Table M-1), for `g7250ctl` (0037). Nothing here is used by the firmware.

use crate::mgmt::auth::{sha256, CHALLENGE_LEN};
use crate::mgmt::payload::{Package, PACKAGE_HEADER_VERSION, PACKAGE_LEN, PACKAGE_MAGIC};
use crate::mgmt::BLOCK_DATA;

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

/// Why a package file is refused before anything is sent (Annex M "Package file"; 0071).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageFileError {
    /// Shorter than the 53-byte header, or not 53 + the image length of bytes 14-17.
    Length { file: usize, expected: Option<u64> },
    /// Bytes 0-3 are not "G7PK".
    Magic,
    /// Byte 4 is not the header version of Table M-4.
    HeaderVersion(u8),
    /// Bytes 50-51 are not 0 (no signature in this version, U-4).
    SignatureLength(u16),
    /// The SHA-256 of the image is not bytes 18-49.
    Hash,
}

/// Check a package file (Annex M "Package file"; 0071): the 53 bytes of Table M-4, then the
/// image and nothing else. Returns the header and the image. The checks are those a host MUST
/// make before it sends anything: length, magic, header version, signature length, hash.
pub fn parse_package_file(file: &[u8]) -> Result<(Package, &[u8]), PackageFileError> {
    if file.len() < PACKAGE_LEN {
        return Err(PackageFileError::Length { file: file.len(), expected: None });
    }
    if file[0..4] != PACKAGE_MAGIC.to_be_bytes() {
        return Err(PackageFileError::Magic);
    }
    if file[4] != PACKAGE_HEADER_VERSION {
        return Err(PackageFileError::HeaderVersion(file[4]));
    }
    let pkg = Package::decode(&file[..PACKAGE_LEN]).map_err(|_| PackageFileError::Magic)?;
    let expected = PACKAGE_LEN as u64 + pkg.length as u64;
    if file.len() as u64 != expected {
        return Err(PackageFileError::Length { file: file.len(), expected: Some(expected) });
    }
    if pkg.signature_len != 0 {
        return Err(PackageFileError::SignatureLength(pkg.signature_len));
    }
    let image = &file[PACKAGE_LEN..];
    if sha256(image) != pkg.sha256 {
        return Err(PackageFileError::Hash);
    }
    Ok((pkg, image))
}

/// Number of `UPDATE_BLOCK`s for an image: block *n* is bytes 512 *n* to 512 *n* + 511
/// (Annex M "Package file"; Table M-1).
pub fn block_count(image_len: usize) -> u32 {
    image_len.div_ceil(BLOCK_DATA) as u32
}

/// The data of block `n` of `image`.
pub fn block_data(image: &[u8], n: u32) -> &[u8] {
    let start = n as usize * BLOCK_DATA;
    &image[start.min(image.len())..(start + BLOCK_DATA).min(image.len())]
}

/// `UPDATE_BLOCK` reply (Table M-1): block number, blocks received so far.
pub fn parse_block_reply(p: &[u8]) -> Option<(u32, u32)> {
    let b: [u8; 8] = p.try_into().ok()?;
    Some((u32::from_be_bytes([b[0], b[1], b[2], b[3]]), u32::from_be_bytes([b[4], b[5], b[6], b[7]])))
}

/// `UPDATE_VERIFY` reply (Table M-1): the SHA-256 computed over the received image.
pub fn parse_verify_reply(p: &[u8]) -> Option<[u8; 32]> {
    p.try_into().ok()
}

/// `UPDATE_ACTIVATE` request (Table M-1): flags, bit 0 foreign image.
pub fn activate_request(foreign: bool) -> [u8; 1] {
    [foreign as u8]
}

/// `UPDATE_ACTIVATE` reply (Table M-1): permanent (1 for a foreign image, no trial boot).
/// `UPDATE_ABORT` reply: restart needed (1). Both one u8.
pub fn parse_flag_reply(p: &[u8]) -> Option<bool> {
    match p {
        [b] => Some(*b != 0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mgmt::payload::{calibration_entries, parse_set_calibration, Version};

    fn package_file(image: &[u8]) -> Vec<u8> {
        let pkg = Package {
            header_version: PACKAGE_HEADER_VERSION,
            component: 1,
            hardware_id: crate::mgmt::payload::HARDWARE_ID_G7250,
            min_hw_revision: 0,
            version: Version { major: 1, minor: 2, build: 3 },
            length: image.len() as u32,
            sha256: sha256(image),
            signature_len: 0,
            target_slot: 2,
        };
        let mut f = pkg.encode().to_vec();
        f.extend_from_slice(image);
        f
    }

    #[test]
    fn package_file_checks() {
        let image: Vec<u8> = (0..1300u32).map(|i| i as u8).collect();
        let f = package_file(&image);
        let (pkg, img) = parse_package_file(&f).unwrap();
        assert_eq!((pkg.target_slot, img.len()), (2, 1300));
        assert_eq!(parse_package_file(&f[..52]), Err(PackageFileError::Length { file: 52, expected: None }));
        assert_eq!(parse_package_file(&f[..f.len() - 1]), Err(PackageFileError::Length { file: 1352, expected: Some(1353) }));
        let mut longer = f.clone();
        longer.push(0);
        assert!(matches!(parse_package_file(&longer), Err(PackageFileError::Length { .. })));
        let mut m = f.clone();
        m[0] = b'X';
        assert_eq!(parse_package_file(&m), Err(PackageFileError::Magic));
        let mut v = f.clone();
        v[4] = 2;
        assert_eq!(parse_package_file(&v), Err(PackageFileError::HeaderVersion(2)));
        let mut s = f.clone();
        s[51] = 64;
        assert_eq!(parse_package_file(&s), Err(PackageFileError::SignatureLength(64)));
        let mut h = f.clone();
        h[100] ^= 1;
        assert_eq!(parse_package_file(&h), Err(PackageFileError::Hash));
    }

    #[test]
    fn blocks_and_update_replies() {
        let image = [7u8; 1300];
        assert_eq!(block_count(1300), 3);
        assert_eq!(block_count(1024), 2);
        assert_eq!(block_count(0), 0);
        assert_eq!(block_data(&image, 0).len(), 512);
        assert_eq!(block_data(&image, 2).len(), 1300 - 1024);
        assert!(block_data(&image, 3).is_empty());
        assert_eq!(parse_block_reply(&crate::mgmt::payload::block_reply(5, 6)), Some((5, 6)));
        assert_eq!(parse_block_reply(&[0; 7]), None);
        assert_eq!(parse_verify_reply(&[1; 32]), Some([1; 32]));
        assert_eq!(activate_request(true), [1]);
        assert_eq!(parse_flag_reply(&[1]), Some(true));
        assert_eq!(parse_flag_reply(&[]), None);
    }

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
