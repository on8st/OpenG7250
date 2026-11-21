// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Authentication of management requests (A1.10; Annex M "Authentication"): the trailer is
//! HMAC-SHA256(secret, challenge ‖ bytes 0 … 17 + N) [RFC2104], [FIPS180-4]; the secret is
//! 32 bytes, the challenge 16 bytes from the hardware RNG. Challenge lifetime, the write
//! session and rate limits are radio state and live in the firmware, not here.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub const SECRET_LEN: usize = 32;
pub const CHALLENGE_LEN: usize = 16;

/// The 32-byte trailer for `signed` (header and payload) under `challenge`.
pub fn trailer(secret: &[u8; SECRET_LEN], challenge: &[u8; CHALLENGE_LEN], signed: &[u8]) -> [u8; 32] {
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes any key length");
    m.update(challenge);
    m.update(signed);
    m.finalize().into_bytes().into()
}

/// Check a trailer in constant time.
pub fn verify(secret: &[u8; SECRET_LEN], challenge: &[u8; CHALLENGE_LEN], signed: &[u8], got: &[u8; 32]) -> bool {
    let mut m = HmacSha256::new_from_slice(secret).expect("HMAC takes any key length");
    m.update(challenge);
    m.update(signed);
    m.verify_slice(got).is_ok()
}

/// `CHANGE_SECRET` payload (Table M-1): new secret XOR HMAC-SHA256(old secret, "G7NS" ‖
/// challenge). The same function encodes (host) and decodes (radio), since XOR is its own
/// inverse.
pub fn change_secret_mask(old: &[u8; SECRET_LEN], challenge: &[u8; CHALLENGE_LEN], value: &[u8; SECRET_LEN]) -> [u8; SECRET_LEN] {
    let mut m = HmacSha256::new_from_slice(old).expect("HMAC takes any key length");
    m.update(b"G7NS");
    m.update(challenge);
    let pad: [u8; 32] = m.finalize().into_bytes().into();
    let mut out = [0u8; SECRET_LEN];
    for i in 0..SECRET_LEN {
        out[i] = value[i] ^ pad[i];
    }
    out
}

/// SHA-256, used for repeat detection (A1.4: hash of the payload), image and calibration ids.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4231_case_2() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?". Our trailer takes
        // a 32-byte key, so check the primitive directly.
        let mut m = HmacSha256::new_from_slice(b"Jefe").unwrap();
        m.update(b"what do ya want ");
        m.update(b"for nothing?");
        let out: [u8; 32] = m.finalize().into_bytes().into();
        assert_eq!(out[..4], [0x5b, 0xdc, 0xc1, 0x46]);
        assert_eq!(out[28..], [0x64, 0xec, 0x38, 0x43]);
    }

    #[test]
    fn trailer_covers_challenge_then_message() {
        let s = [7u8; 32];
        let c = [1u8; 16];
        let t = trailer(&s, &c, b"hdr+payload");
        assert!(verify(&s, &c, b"hdr+payload", &t));
        assert!(!verify(&s, &[2u8; 16], b"hdr+payload", &t));
        assert!(!verify(&s, &c, b"hdr+payloaX", &t));
        assert!(!verify(&[8u8; 32], &c, b"hdr+payload", &t));
        // Equal to HMAC over the concatenation.
        let mut all = std::vec::Vec::from(c);
        all.extend_from_slice(b"hdr+payload");
        let mut m = HmacSha256::new_from_slice(&s).unwrap();
        m.update(&all);
        let want: [u8; 32] = m.finalize().into_bytes().into();
        assert_eq!(t, want);
    }

    #[test]
    fn change_secret_roundtrip() {
        let old = [3u8; 32];
        let new = [9u8; 32];
        let c = [5u8; 16];
        let wire = change_secret_mask(&old, &c, &new);
        assert_ne!(wire, new);
        assert_eq!(change_secret_mask(&old, &c, &wire), new);
    }

    #[test]
    fn sha256_abc() {
        assert_eq!(sha256(b"abc")[..4], [0xba, 0x78, 0x16, 0xbf]);
    }
}
