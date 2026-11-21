// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! P-RX-2: the DDC0 values of the 0x01 configuration frame, from the unit's calibration set
//! (A1.12; Annex P "P-RX-2 route"; decision 0060), and the frame built from them.
//!
//! The values are per unit and come from the unit's factory calibration, confirmed on the radio
//! (T-B6-28): they are never in this image. The firmware writes byte 0 and bytes 9-18 as given
//! and does not interpret them (0060 point 2).
//!
//! Frame layout ([HIS] HIS 80, `5292ba8`): byte 0 status flags; three big-endian 32-bit NCO
//! words at offsets 1, 5 and 19 (HIGH); bytes 9-18 other fields incl. per-rate calibration;
//! byte 23 most likely the rate index (MEDIUM). Rate index 0-3 is 384/192/96/48 kS/s (MEDIUM).
//!
//! A frame is built only with a [`BusPermit`](crate::fpga_gate::BusPermit): P-RX-2 alone does
//! not open the FPGA bus gate, whose timing parameters still come from a cleared HIS.

use crate::fpga_gate::BusPermit;

/// The value of P-RX-2 does not decode (A1.12 item 1: status 48).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bad;

/// The values for one offered DDC0 rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateFrame {
    /// Byte 0 of the frame in receive.
    pub byte0: u8,
    /// Bytes 9-18 of the frame for this rate.
    pub bytes_9_18: [u8; 10],
}

/// Flags byte of the P-RX-2 encoding (Table P-1).
pub const FLAG_IQ_SWAPPED: u8 = 0x01;
pub const FLAG_SPECTRUM_INVERTED: u8 = 0x02;

/// A decoded P-RX-2 value (Table P-1 encoding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RxCal {
    pub flags: u8,
    /// Byte offset of the DDC0 tuning word: 1 or 5.
    pub ddc0_offset: u8,
    /// The other receive tuning word (at offset 5 or 1) and the word at offset 19 (transmit,
    /// HYPOTHESIS; frozen until P-TX-1), from the capture at its lowest frequency: written
    /// unchanged whatever DDC0 is tuned to, as the bytes given (big-endian in the frame).
    pub other_word: [u8; 4],
    pub word_19: [u8; 4],
    /// Per rate index 0-3 (384, 192, 96, 48 kS/s): its frame values, if offered.
    pub rates: [Option<RateFrame>; 4],
}

/// The rate of each rate index ([HIS] HIS 80), kS/s.
pub const RATE_KSPS: [u16; 4] = [384, 192, 96, 48];

const ENTRY: usize = 12;
const HEAD: usize = 10;

impl RxCal {
    /// Decode the Table P-1 encoding: flags (u8), tuning-word offset (u8: 1 or 5), the other
    /// receive tuning word (u32), the word at offset 19 (u32), then per offered rate: rate index
    /// (u8), byte 0 (u8), bytes 9-18 (10 bytes). At least one rate; no rate twice; no flag bit
    /// other than bits 0 and 1; a rate index of 0-3.
    pub fn decode(v: &[u8]) -> Result<RxCal, Bad> {
        if v.len() < HEAD {
            return Err(Bad);
        }
        let (flags, off, rest) = (&v[0], &v[1], &v[HEAD..]);
        if flags & !(FLAG_IQ_SWAPPED | FLAG_SPECTRUM_INVERTED) != 0 || !matches!(off, 1 | 5) {
            return Err(Bad);
        }
        let mut other_word = [0u8; 4];
        other_word.copy_from_slice(&v[2..6]);
        let mut word_19 = [0u8; 4];
        word_19.copy_from_slice(&v[6..10]);
        if rest.is_empty() || rest.len() % ENTRY != 0 {
            return Err(Bad);
        }
        let mut rates = [None; 4];
        for e in rest.chunks_exact(ENTRY) {
            let i = e[0] as usize;
            if i >= 4 || rates[i].is_some() {
                return Err(Bad);
            }
            let mut b = [0u8; 10];
            b.copy_from_slice(&e[2..12]);
            rates[i] = Some(RateFrame { byte0: e[1], bytes_9_18: b });
        }
        Ok(RxCal { flags: *flags, ddc0_offset: *off, other_word, word_19, rates })
    }

    /// The `ddc_rates` bits (B4.8: bit 0 48, bit 1 96, bit 2 192, bit 3 384) of the rates that
    /// have a frame; a rate without one is not offered (0060 point 2.4).
    pub fn rate_mask(&self) -> u8 {
        let mut m = 0;
        for (i, r) in self.rates.iter().enumerate() {
            if r.is_some() {
                m |= 1 << (3 - i);
            }
        }
        m
    }

    /// The rate index of a rate in kS/s, if this set has a frame for it.
    pub fn rate_index(&self, ksps: u16) -> Option<u8> {
        let i = RATE_KSPS.iter().position(|&r| r == ksps)?;
        self.rates[i].map(|_| i as u8)
    }

    /// One 0x04 sample pair as read from the FPGA, in host order: I and Q swapped back if flag
    /// bit 0, then Q negated after any swap if flag bit 1; both give (Q, -I) (Table P-1). The
    /// flags act on the samples only, never on a tuning word.
    pub fn sample(&self, first: i32, second: i32) -> (i32, i32) {
        let (i, q) = if self.flags & FLAG_IQ_SWAPPED != 0 { (second, first) } else { (first, second) };
        if self.flags & FLAG_SPECTRUM_INVERTED != 0 { (i, q.wrapping_neg()) } else { (i, q) }
    }
}

/// The 0x01 configuration frame in receive for DDC0 at `ksps`, tuned by the phase word `word`
/// (B3.8; High Priority bytes 9-12), or `None` if P-RX-2 has no frame for that rate. The other
/// receive tuning word and the word at offset 19 are P-RX-2's, unchanged (Table P-1).
pub fn config_frame(_permit: &BusPermit, cal: &RxCal, ksps: u16, word: u32) -> Option<[u8; 24]> {
    let idx = cal.rate_index(ksps)?;
    let r = cal.rates[idx as usize]?;
    let mut f = [0u8; 24];
    f[0] = r.byte0;
    let o = cal.ddc0_offset as usize;
    f[o..o + 4].copy_from_slice(&word.to_be_bytes());
    let other = if o == 1 { 5 } else { 1 };
    f[other..other + 4].copy_from_slice(&cal.other_word);
    f[19..23].copy_from_slice(&cal.word_19);
    f[9..19].copy_from_slice(&r.bytes_9_18);
    f[23] = idx;
    Some(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(idx: u8, b0: u8, fill: u8) -> [u8; 12] {
        let mut e = [fill; 12];
        e[0] = idx;
        e[1] = b0;
        e
    }

    fn value(flags: u8, off: u8, entries: &[[u8; 12]]) -> std::vec::Vec<u8> {
        let mut v = std::vec![flags, off, 0xA1, 0xA2, 0xA3, 0xA4, 0xB1, 0xB2, 0xB3, 0xB4];
        for e in entries {
            v.extend_from_slice(e);
        }
        v
    }

    #[test]
    fn table_p1_decode() {
        let v = value(0x03, 5, &[entry(3, 0x40, 0x11), entry(1, 0x41, 0x22)]);
        let c = RxCal::decode(&v).unwrap();
        assert_eq!((c.flags, c.ddc0_offset), (3, 5));
        assert_eq!((c.other_word, c.word_19), ([0xA1, 0xA2, 0xA3, 0xA4], [0xB1, 0xB2, 0xB3, 0xB4]));
        assert_eq!(c.rates[3], Some(RateFrame { byte0: 0x40, bytes_9_18: [0x11; 10] }));
        assert_eq!(c.rates[1], Some(RateFrame { byte0: 0x41, bytes_9_18: [0x22; 10] }));
        assert_eq!((c.rates[0], c.rates[2]), (None, None));
        // 48 (index 3) and 192 (index 1): ddc_rates bits 0 and 2.
        assert_eq!(c.rate_mask(), 0b0101);
        assert_eq!(c.rate_index(48), Some(3));
        assert_eq!(c.rate_index(96), None, "a rate without a frame is not offered");
        assert_eq!(c.rate_index(1536), None);
    }

    #[test]
    fn table_p1_rejects() {
        let e = entry(3, 0, 0);
        assert_eq!(RxCal::decode(&[]), Err(Bad));
        assert_eq!(RxCal::decode(&[0, 1]), Err(Bad), "no words");
        assert_eq!(RxCal::decode(&value(0, 1, &[])), Err(Bad), "no rate");
        assert_eq!(RxCal::decode(&value(0, 1, &[e])[..13]), Err(Bad), "short entry after the words");
        assert_eq!(RxCal::decode(&value(0x04, 1, &[e])), Err(Bad), "unknown flag bit");
        for off in [0u8, 2, 4, 6, 19] {
            assert_eq!(RxCal::decode(&value(0, off, &[e])), Err(Bad), "offset {off}");
        }
        assert_eq!(RxCal::decode(&value(0, 1, &[entry(4, 0, 0)])), Err(Bad), "rate index 4");
        assert_eq!(RxCal::decode(&value(0, 1, &[e, e])), Err(Bad), "a rate twice");
        let mut v = value(0, 1, &[e]);
        v.pop();
        assert_eq!(RxCal::decode(&v), Err(Bad), "short entry");
        v.extend_from_slice(&[0, 0]);
        assert_eq!(RxCal::decode(&v), Err(Bad), "trailing bytes");
        assert!(RxCal::decode(&value(0, 1, &[entry(0, 0, 0), entry(1, 0, 0), entry(2, 0, 0), entry(3, 0, 0)])).is_ok());
    }

    #[test]
    fn samples_follow_the_flags() {
        let mut c = RxCal::decode(&value(0, 1, &[entry(3, 0, 0)])).unwrap();
        assert_eq!(c.sample(10, -20), (10, -20));
        c.flags = FLAG_IQ_SWAPPED;
        assert_eq!(c.sample(10, -20), (-20, 10));
        c.flags = FLAG_SPECTRUM_INVERTED;
        assert_eq!(c.sample(10, -20), (10, 20));
        c.flags = FLAG_IQ_SWAPPED | FLAG_SPECTRUM_INVERTED;
        assert_eq!(c.sample(10, -20), (-20, -10));
    }

    #[test]
    fn frame_layout() {
        // No permit can exist in this build (gate closed); the layout is checked with a
        // test-only permit.
        let p = BusPermit::for_test();
        let mut e = entry(2, 0xA5, 0);
        e[2..12].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        for off in [1u8, 5] {
            let c = RxCal::decode(&value(0, off, &[e])).unwrap();
            let f = config_frame(&p, &c, 96, 0x1234_5678).unwrap();
            assert_eq!(f[0], 0xA5);
            assert_eq!(&f[off as usize..off as usize + 4], &[0x12, 0x34, 0x56, 0x78]);
            let other = if off == 1 { 5 } else { 1 };
            assert_eq!(&f[other..other + 4], &[0xA1, 0xA2, 0xA3, 0xA4], "the other receive word unchanged");
            assert_eq!(&f[9..19], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
            assert_eq!(&f[19..23], &[0xB1, 0xB2, 0xB3, 0xB4], "the offset-19 word unchanged");
            // Unchanged whatever DDC0 is tuned to; the flags never touch a word.
            let g = config_frame(&p, &RxCal { flags: 3, ..c }, 96, 0).unwrap();
            assert_eq!((&g[other..other + 4], &g[19..23]), (&f[other..other + 4], &f[19..23]));
            assert_eq!(f[23], 2);
            assert_eq!(config_frame(&p, &c, 48, 0), None);
        }
    }
}
