//! P-RX-2 (Annex P, Table P-1, spec/draft 90097d9): the FPGA configuration frame values from the
//! unit's factory calibration. Encoding: flags (u8: bit 0 I and Q swapped, bit 1 spectrum
//! inverted), tuning-word offset (u8: 1 or 5), the other receive tuning word (u32), the word at
//! offset 19 (u32), then per offered rate: rate index (u8), byte 0 (u8), bytes 9-18 (10 bytes).
//! The flags act on the 0x04 samples bit 0 first, then bit 1 (negate Q after any swap): both set
//! give (Q, -I).
//!
//! GPL-3.0-or-later, (c) 2026 ON8ST.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    pub index: u8,
    pub byte0: u8,
    pub bytes9_18: [u8; 10],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prx2 {
    pub swap_iq: bool,
    pub invert: bool,
    pub tuning_offset: u8,
    pub other_tuning_word: u32,
    pub word19: u32,
    pub rates: Vec<Rate>,
}

/// Decode a P-RX-2 value; None if it does not have the structure above.
pub fn decode(v: &[u8]) -> Option<Prx2> {
    if v.len() < 10 || (v.len() - 10) % 12 != 0 {
        return None;
    }
    let off = v[1];
    if off != 1 && off != 5 {
        return None;
    }
    let rd = |i: usize| u32::from_be_bytes([v[i], v[i + 1], v[i + 2], v[i + 3]]);
    let rates = v[10..]
        .chunks(12)
        .map(|c| {
            let mut b = [0u8; 10];
            b.copy_from_slice(&c[2..12]);
            Rate { index: c[0], byte0: c[1], bytes9_18: b }
        })
        .collect();
    Some(Prx2 { swap_iq: v[0] & 1 != 0, invert: v[0] & 2 != 0, tuning_offset: off, other_tuning_word: rd(2), word19: rd(6), rates })
}

impl Prx2 {
    /// One 0x04 sample (I, Q) as DDC0 delivers it: bit 0 first (swap), then bit 1 (negate Q).
    pub fn apply(&self, i: i32, q: i32) -> (i32, i32) {
        let (i, q) = if self.swap_iq { (q, i) } else { (i, q) };
        if self.invert { (i, -q) } else { (i, q) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(flags: u8, off: u8, rates: u8) -> Vec<u8> {
        let mut v = vec![flags, off];
        v.extend_from_slice(&0x1122_3344u32.to_be_bytes());
        v.extend_from_slice(&0x5566_7788u32.to_be_bytes());
        for r in 0..rates {
            v.push(r);
            v.push(0xA0 + r);
            v.extend_from_slice(&[r; 10]);
        }
        v
    }

    #[test]
    fn decodes_the_encoding() {
        let p = decode(&value(3, 5, 3)).unwrap();
        assert!(p.swap_iq && p.invert);
        assert_eq!((p.tuning_offset, p.other_tuning_word, p.word19), (5, 0x1122_3344, 0x5566_7788));
        assert_eq!(p.rates.len(), 3);
        assert_eq!(p.rates[2], Rate { index: 2, byte0: 0xA2, bytes9_18: [2; 10] });
    }

    #[test]
    fn rejects_bad_structure() {
        assert!(decode(&value(0, 2, 1)).is_none()); // offset neither 1 nor 5
        assert!(decode(&value(0, 1, 1)[..15]).is_none()); // a rate entry cut short
        assert!(decode(&[1]).is_none());
    }

    #[test]
    fn flag_order_bit0_then_bit1() {
        let p = |f| decode(&value(f, 1, 0)).unwrap();
        assert_eq!(p(0).apply(3, 7), (3, 7));
        assert_eq!(p(1).apply(3, 7), (7, 3));
        assert_eq!(p(2).apply(3, 7), (3, -7));
        assert_eq!(p(3).apply(3, 7), (7, -3)); // (Q, -I)
    }
}
