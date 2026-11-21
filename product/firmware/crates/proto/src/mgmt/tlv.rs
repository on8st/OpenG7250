// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! TLV lists of Annex M: tag (1 byte), length (1 byte), value. A reader MUST skip a TLV with an
//! unknown tag (Annex M, introduction); the caller does that by ignoring tags it does not know.

/// Iterator over the TLVs of a list. Yields `Err(())` once and stops if the list is cut off.
pub struct Tlvs<'a> {
    rest: &'a [u8],
    failed: bool,
}

pub fn iter(list: &[u8]) -> Tlvs<'_> {
    Tlvs { rest: list, failed: false }
}

impl<'a> Iterator for Tlvs<'a> {
    type Item = Result<(u8, &'a [u8]), ()>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.failed || self.rest.is_empty() {
            return None;
        }
        if self.rest.len() < 2 || self.rest.len() < 2 + self.rest[1] as usize {
            self.failed = true;
            return Some(Err(()));
        }
        let (tag, len) = (self.rest[0], self.rest[1] as usize);
        let v = &self.rest[2..2 + len];
        self.rest = &self.rest[2 + len..];
        Some(Ok((tag, v)))
    }
}

/// True if the whole list is well-formed.
pub fn valid(list: &[u8]) -> bool {
    iter(list).all(|t| t.is_ok())
}

/// Writes TLVs into a buffer.
pub struct Writer<'a> {
    buf: &'a mut [u8],
    len: usize,
    overflow: bool,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, len: 0, overflow: false }
    }

    /// Append one TLV. A value longer than 255 bytes, or one that does not fit, marks the writer
    /// as overflowed; [`Writer::finish`] then fails.
    pub fn put(&mut self, tag: u8, value: &[u8]) -> &mut Self {
        if self.overflow || value.len() > 255 || self.len + 2 + value.len() > self.buf.len() {
            self.overflow = true;
            return self;
        }
        self.buf[self.len] = tag;
        self.buf[self.len + 1] = value.len() as u8;
        self.buf[self.len + 2..self.len + 2 + value.len()].copy_from_slice(value);
        self.len += 2 + value.len();
        self
    }

    pub fn u8(&mut self, tag: u8, v: u8) -> &mut Self {
        self.put(tag, &[v])
    }
    pub fn u16(&mut self, tag: u8, v: u16) -> &mut Self {
        self.put(tag, &v.to_be_bytes())
    }
    pub fn u32(&mut self, tag: u8, v: u32) -> &mut Self {
        self.put(tag, &v.to_be_bytes())
    }

    /// The written length, or `None` if anything did not fit.
    pub fn finish(&self) -> Option<usize> {
        if self.overflow { None } else { Some(self.len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_read_skip_unknown() {
        let mut b = [0u8; 32];
        let mut w = Writer::new(&mut b);
        w.u8(1, 7).put(99, &[1, 2, 3]).u32(4, 0x01020304);
        let n = w.finish().unwrap();
        assert_eq!(n, 3 + 5 + 6);
        let got: std::vec::Vec<_> = iter(&b[..n]).map(|t| t.unwrap()).filter(|(t, _)| *t != 99).collect();
        assert_eq!(got, [(1u8, &[7u8][..]), (4, &[1, 2, 3, 4][..])]);
    }

    #[test]
    fn truncated_and_overflow() {
        assert!(!valid(&[1, 4, 0, 0]));
        assert!(!valid(&[1]));
        assert!(valid(&[]));
        let mut b = [0u8; 4];
        let mut w = Writer::new(&mut b);
        w.put(1, &[0, 0, 0]);
        assert_eq!(w.finish(), None);
    }
}
