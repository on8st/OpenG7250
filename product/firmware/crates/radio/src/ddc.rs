// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! DDC0 I&Q packets (spec B6.2-B6.4, Table B6-1) and the mapping to the FPGA rate index
//! ([HIS] HIS 80: rate index 0-3 → 384/192/96/48 kS/s; 8/4/2/1 samples per DRDY).
//!
//! The samples come from the FPGA bus, which stays closed until its parameters are cleared
//! (`g7250_safety::fpga_gate`). This module only formats what it is given.

/// DDC0 local port (B6.1).
pub const PORT: u16 = 1035;
/// Samples per packet (B6.4).
pub const SAMPLES: usize = 238;
/// Packet length: 16 + 238 × 6 (B6.4).
pub const LEN: usize = 16 + SAMPLES * 6;

/// FPGA rate index for an offered DDC0 rate ([HIS] HIS 80, rate index MEDIUM).
pub fn rate_index(ksps: u16) -> Option<u8> {
    match ksps {
        384 => Some(0),
        192 => Some(1),
        96 => Some(2),
        48 => Some(3),
        _ => None,
    }
}

/// Samples the FPGA delivers per DRDY at a rate index ([HIS] HIS 80).
pub fn samples_per_drdy(index: u8) -> usize {
    8 >> index.min(3)
}

/// Collects I&Q samples and emits full packets.
pub struct Packer {
    buf: [u8; LEN],
    n: usize,
}

impl Default for Packer {
    fn default() -> Self {
        Self::new()
    }
}

impl Packer {
    pub const fn new() -> Self {
        Packer { buf: [0; LEN], n: 0 }
    }

    /// Drop a partial packet (rate change, stop, refusal).
    pub fn reset(&mut self) {
        self.n = 0;
    }

    /// Add one sample (24-bit signed values in the low bits of `i` and `q`). Returns the full
    /// packet when 238 samples are in, with `seq` in bytes 0-3.
    pub fn push(&mut self, i: i32, q: i32, seq: u32) -> Option<&[u8; LEN]> {
        let at = 16 + 6 * self.n;
        let ib = (i as u32).to_be_bytes();
        let qb = (q as u32).to_be_bytes();
        self.buf[at..at + 3].copy_from_slice(&ib[1..4]);
        self.buf[at + 3..at + 6].copy_from_slice(&qb[1..4]);
        self.n += 1;
        if self.n < SAMPLES {
            return None;
        }
        self.n = 0;
        self.buf[0..4].copy_from_slice(&seq.to_be_bytes());
        self.buf[4..12].fill(0); // no time stamp (B6.6)
        self.buf[12..14].copy_from_slice(&24u16.to_be_bytes());
        self.buf[14..16].copy_from_slice(&(SAMPLES as u16).to_be_bytes());
        Some(&self.buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_layout_table_b6_1() {
        let mut p = Packer::new();
        for k in 0..SAMPLES as i32 - 1 {
            assert!(p.push(k, -k, 7).is_none());
        }
        let b = p.push(-1, 0x7F_FFFF, 7).unwrap();
        assert_eq!(b.len(), 1444);
        assert_eq!(&b[0..4], &[0, 0, 0, 7]);
        assert!(b[4..12].iter().all(|&x| x == 0));
        assert_eq!(&b[12..16], &[0, 24, 0, 238]);
        // Sample 1: I = 1, Q = -1 (two's complement, MSB first, I before Q).
        assert_eq!(&b[22..28], &[0, 0, 1, 0xFF, 0xFF, 0xFF]);
        // Sample 237.
        assert_eq!(&b[1438..1444], &[0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF]);
    }

    #[test]
    fn rates() {
        assert_eq!(rate_index(384), Some(0));
        assert_eq!(rate_index(48), Some(3));
        assert_eq!(rate_index(768), None);
        assert_eq!((samples_per_drdy(0), samples_per_drdy(1), samples_per_drdy(3)), (8, 4, 1));
    }
}
