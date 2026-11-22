// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Host end of the capability record (spec B4), carried in memory-mapped register packets
//! (B8.1), for `g7250ctl` (0037). Written from B4 and B8 only. Nothing here is used by the
//! firmware.
//!
//! The host asks with a write of 1 to `CAP_REQUEST` and reads the answer: one packet with the
//! echo of that pair and the whole record (B4.4 item 1). A packet without the echo, or with the
//! echo but without every register up to `cap_length`, is not a record. The unasked record
//! in parts (B4.4 item 2) belongs to a session and is not read here.

/// The radio's register port (B8.1).
pub const REGISTER_PORT: u16 = 7250;
/// `CAP_REQUEST` (Table B8-2): 0x00000001 asks for the record.
pub const CAP_REQUEST: u16 = 0x0100;
/// `CAP_BASE` (B4.2).
pub const CAP_BASE: u16 = 0x0000;
/// The record has at most 239 registers, 0x0000-0x00EE (B4.2, Table B8-1).
pub const CAP_MAX: usize = 239;
/// `cap_magic`, ASCII "G725" (Table B4-1; B4.10 rule 5).
pub const CAP_MAGIC: u32 = 0x4737_3235;
/// The record format this file knows: major 1, minor 1 (Table B4-1).
pub const KNOWN_MAJOR: u16 = 1;
pub const KNOWN_MINOR: u16 = 1;

/// Offsets of Table B4-1 (record version 1.1), relative to `CAP_BASE`.
pub mod off {
    pub const MAGIC: usize = 0x00;
    pub const VERSION: usize = 0x01;
    pub const LENGTH: usize = 0x02;
    pub const REGMAP_VERSION: usize = 0x03;
    /// 4 registers of text.
    pub const PRODUCT_NAME: usize = 0x04;
    pub const FW_VERSION: usize = 0x08;
    pub const P2_VERSION: usize = 0x09;
    pub const HW_REVISION: usize = 0x0A;
    pub const LEVEL: usize = 0x0B;
    pub const STREAMS: usize = 0x0C;
    pub const NUM_ADC: usize = 0x0D;
    pub const NUM_DDC: usize = 0x0E;
    pub const NUM_DUC: usize = 0x0F;
    pub const DDC_RATES: usize = 0x10;
    pub const DUC_FORMAT: usize = 0x11;
    pub const DSP_CLOCK_HZ: usize = 0x12;
    pub const RX_RANGE_COUNT: usize = 0x13;
    /// 4 pairs: start Hz, stop Hz.
    pub const RX_RANGE: usize = 0x14;
    pub const ANTENNAS: usize = 0x1C;
    pub const ATU: usize = 0x1D;
    pub const ATU_RANGE: usize = 0x1E;
    pub const TELEMETRY: usize = 0x1F;
    pub const TX: usize = 0x20;
    pub const TX_BAND_COUNT: usize = 0x21;
    /// 16 bands of 3 registers.
    pub const TX_BAND: usize = 0x22;
    pub const MGMT: usize = 0x52;
    pub const REGMAP_GROUPS: usize = 0x53;
    pub const LINK: usize = 0x54;
    pub const WB_FORMAT: usize = 0x55;
    pub const WB_FRAME: usize = 0x56;
    pub const CAL_INFO: usize = 0x57;
    pub const CAL_SUPPLY_C: usize = 0x58;
    /// 16 sets of 6 registers.
    pub const CAL_POWER: usize = 0x59;
    /// Version 1.1 from here (B4.9).
    pub const STATUS_FILL: usize = 0xB9;
    pub const CLOCK_REF: usize = 0xBA;
    pub const COUPLER: usize = 0xBB;
    pub const TLM_RATES: usize = 0xBC;
    pub const STATUS_RATES: usize = 0xBD;
    pub const TLM_COUNT: usize = 0xBE;
    /// 8 descriptors of 2 registers.
    pub const TLM_DESC: usize = 0xBF;
    /// One past the last field of version 1.1 (`cap_length` 0xCF).
    pub const END_V1_1: usize = 0xCF;
    /// `cap_length` of version 1.0 (0xB9).
    pub const END_V1_0: usize = 0xB9;
}

/// The request: sequence number 0 (not checked, B2.7), then the pair (`CAP_REQUEST`, 1);
/// 4 + 6 bytes (B8.1).
pub fn cap_request() -> [u8; 10] {
    let mut p = [0u8; 10];
    p[4..6].copy_from_slice(&CAP_REQUEST.to_be_bytes());
    p[6..10].copy_from_slice(&1u32.to_be_bytes());
    p
}

/// The pairs of a memory-mapped packet (B8.1): bytes 4 onwards, 2-byte address and 4-byte data,
/// big-endian. A trailing incomplete pair is dropped; padding (address 0xFFFF, and the pair
/// 0x0000 with 0x00000000) is skipped.
pub fn pairs(packet: &[u8]) -> impl Iterator<Item = (u16, u32)> + '_ {
    packet
        .get(4..)
        .unwrap_or(&[])
        .as_chunks::<6>()
        .0
        .iter()
        .map(|c| (u16::from_be_bytes([c[0], c[1]]), u32::from_be_bytes([c[2], c[3], c[4], c[5]])))
        .filter(|&(a, d)| a != 0xFFFF && !(a == 0 && d == 0))
}

/// Why a packet is not a capability record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// No echo of the `CAP_REQUEST` pair: not the answer to a request (a register push, or a
    /// part of the unasked record).
    NoEcho,
    /// A record register appears twice in the packet.
    Repeated(u16),
    /// `cap_magic` is not "G725" (B4.10 rule 5), or register 0 is missing.
    Magic(Option<u32>),
    /// A major version this file does not know (B4.10 rule 4): the record must not be used.
    Major(u16),
    /// `cap_length` is missing, below the header (3) or above 239 (B4.2).
    Length(Option<u32>),
    /// The packet lacks registers below `cap_length`: `have` of `need` present.
    Incomplete { have: usize, need: usize },
}

/// A capability record read from one answer packet: registers 0 to `cap_length` - 1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapRecord {
    regs: [u32; CAP_MAX],
    len: usize,
}

/// Read the answer to a `CAP_REQUEST` (B4.4 item 1).
pub fn parse_answer(packet: &[u8]) -> Result<CapRecord, RecordError> {
    let mut regs: [Option<u32>; CAP_MAX] = [None; CAP_MAX];
    let mut echo = false;
    for (a, d) in pairs(packet) {
        if a == CAP_REQUEST && d == 1 {
            echo = true;
        } else if let Some(i) = (a as usize).checked_sub(CAP_BASE as usize)
            && i < CAP_MAX
        {
            if regs[i].is_some() {
                return Err(RecordError::Repeated(a));
            }
            regs[i] = Some(d);
        }
    }
    if !echo {
        return Err(RecordError::NoEcho);
    }
    match regs[off::MAGIC] {
        Some(CAP_MAGIC) => {}
        m => return Err(RecordError::Magic(m)),
    }
    let version = regs[off::VERSION].ok_or(RecordError::Incomplete { have: count(&regs), need: off::LENGTH + 1 })?;
    let major = (version >> 16) as u16;
    if major != KNOWN_MAJOR {
        return Err(RecordError::Major(major));
    }
    let len = match regs[off::LENGTH] {
        Some(l) if (3..=CAP_MAX as u32).contains(&l) => l as usize,
        l => return Err(RecordError::Length(l)),
    };
    let have = regs[..len].iter().filter(|r| r.is_some()).count();
    if have != len {
        return Err(RecordError::Incomplete { have, need: len });
    }
    let mut out = [0u32; CAP_MAX];
    for (o, r) in out.iter_mut().zip(&regs[..len]) {
        *o = r.unwrap();
    }
    Ok(CapRecord { regs: out, len })
}

fn count(regs: &[Option<u32>]) -> usize {
    regs.iter().filter(|r| r.is_some()).count()
}

/// One TX band of `tx_band[]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TxBand {
    pub start_hz: u32,
    pub stop_hz: u32,
    /// 0.1 W.
    pub max_ssb_cw_fm: u16,
    /// 0.1 W.
    pub max_am: u16,
}

/// One telemetry descriptor of `tlm_desc[]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlmDesc {
    pub address: u16,
    pub update_ms: u16,
    /// In the register's milli-units.
    pub resolution: u16,
    pub hysteresis: u16,
}

impl CapRecord {
    /// `cap_length`: the number of registers.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every register, in order (for a raw dump).
    pub fn registers(&self) -> &[u32] {
        &self.regs[..self.len]
    }

    /// (major, minor) of the record format.
    pub fn version(&self) -> (u16, u16) {
        let v = self.regs[off::VERSION];
        ((v >> 16) as u16, v as u16)
    }

    /// A register by offset; `None` beyond `cap_length` (absent, B4.10 rule 2).
    pub fn get(&self, offset: usize) -> Option<u32> {
        self.registers().get(offset).copied()
    }

    /// Four registers of text from `offset` (B4.3: 4 ASCII characters per register, first in
    /// bits 31-24, padded with 0x00), with the length up to the padding. `None` if any is absent.
    pub fn text4(&self, offset: usize) -> Option<([u8; 16], usize)> {
        let mut t = [0u8; 16];
        for i in 0..4 {
            t[4 * i..4 * i + 4].copy_from_slice(&self.get(offset + i)?.to_be_bytes());
        }
        let n = t.iter().rposition(|b| *b != 0).map_or(0, |p| p + 1);
        Some((t, n))
    }

    /// `rx_range[i]`, i < 4: (start Hz, stop Hz).
    pub fn rx_range(&self, i: usize) -> Option<(u32, u32)> {
        (i < 4).then_some(())?;
        Some((self.get(off::RX_RANGE + 2 * i)?, self.get(off::RX_RANGE + 2 * i + 1)?))
    }

    /// `tx_band[i]`, i < 16.
    pub fn tx_band(&self, i: usize) -> Option<TxBand> {
        (i < 16).then_some(())?;
        let b = off::TX_BAND + 3 * i;
        let p = self.get(b + 2)?;
        Some(TxBand { start_hz: self.get(b)?, stop_hz: self.get(b + 1)?, max_ssb_cw_fm: (p >> 16) as u16, max_am: p as u16 })
    }

    /// `cal_power[i]`, i < 16: forward c1, c2; reverse c1, c2; exciter c1, c2 (units of 1e-6).
    pub fn cal_power(&self, i: usize) -> Option<[u32; 6]> {
        (i < 16).then_some(())?;
        let b = off::CAL_POWER + 6 * i;
        let mut s = [0u32; 6];
        for (k, v) in s.iter_mut().enumerate() {
            *v = self.get(b + k)?;
        }
        Some(s)
    }

    /// `tlm_desc[i]`, i < 8.
    pub fn tlm_desc(&self, i: usize) -> Option<TlmDesc> {
        (i < 8).then_some(())?;
        let b = off::TLM_DESC + 2 * i;
        let (a, r) = (self.get(b)?, self.get(b + 1)?);
        Some(TlmDesc { address: (a >> 16) as u16, update_ms: a as u16, resolution: (r >> 16) as u16, hysteresis: r as u16 })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of `len` registers built by hand: magic, version 1.1, length, then the offset in
    /// every register, so that a register's value shows where it came from.
    fn regs(len: usize, minor: u32) -> Vec<u32> {
        let mut r: Vec<u32> = (0..len as u32).map(|i| 0x1000_0000 | i).collect();
        r[0] = 0x4737_3235;
        r[1] = 0x0001_0000 | minor;
        r[2] = len as u32;
        r
    }

    /// An answer packet by hand: sequence number, the echo, then the pairs.
    fn packet(pairs: &[(u16, u32)], echo: bool) -> Vec<u8> {
        let mut p = vec![0, 0, 0, 7];
        if echo {
            p.extend([0x01, 0x00, 0, 0, 0, 1]);
        }
        for (a, d) in pairs {
            p.extend(a.to_be_bytes());
            p.extend(d.to_be_bytes());
        }
        p
    }

    fn record_pairs(r: &[u32]) -> Vec<(u16, u32)> {
        r.iter().enumerate().map(|(i, v)| (i as u16, *v)).collect()
    }

    #[test]
    fn request_is_one_pair() {
        assert_eq!(cap_request(), [0, 0, 0, 0, 0x01, 0x00, 0, 0, 0, 1]);
    }

    #[test]
    fn a_whole_v1_1_record_is_read() {
        let mut r = regs(0xCF, 1);
        r[4..8].copy_from_slice(&[0x4737_3235, 0x302D_5032, 0, 0]);
        r[off::TX_BAND..off::TX_BAND + 3].copy_from_slice(&[1_800_000, 2_000_000, (1000 << 16) | 250]);
        r[off::TLM_DESC..off::TLM_DESC + 2].copy_from_slice(&[(0x0300 << 16) | 100, (10 << 16) | 5]);
        let rec = parse_answer(&packet(&record_pairs(&r), true)).unwrap();
        assert_eq!((rec.len(), rec.version()), (0xCF, (1, 1)));
        let (t, n) = rec.text4(off::PRODUCT_NAME).unwrap();
        assert_eq!(&t[..n], b"G7250-P2");
        assert_eq!(rec.tx_band(0), Some(TxBand { start_hz: 1_800_000, stop_hz: 2_000_000, max_ssb_cw_fm: 1000, max_am: 250 }));
        assert_eq!(rec.tlm_desc(0), Some(TlmDesc { address: 0x0300, update_ms: 100, resolution: 10, hysteresis: 5 }));
        assert_eq!(rec.get(off::COUPLER), Some(0x1000_0000 | off::COUPLER as u32));
        assert_eq!(rec.rx_range(4), None);
    }

    #[test]
    fn a_v1_0_record_has_no_v1_1_fields() {
        let rec = parse_answer(&packet(&record_pairs(&regs(0xB9, 0)), true)).unwrap();
        assert_eq!(rec.version(), (1, 0));
        assert_eq!(rec.get(off::CAL_POWER + 6 * 16 - 1), Some(0x1000_0000 | 0xB8));
        assert_eq!(rec.get(off::STATUS_FILL), None);
        assert_eq!(rec.tlm_desc(0), None);
    }

    #[test]
    fn padding_and_other_registers_are_skipped() {
        let r = regs(0xB9, 0);
        let mut p = record_pairs(&r);
        p.insert(5, (0xFFFF, 0xDEAD_BEEF));
        p.push((0x0101, 7)); // WRITE_STATUS or the like: not part of the record
        p.push((0, 0)); // padding
        assert!(parse_answer(&packet(&p, true)).is_ok());
    }

    #[test]
    fn a_packet_without_the_echo_is_not_a_record() {
        assert_eq!(parse_answer(&packet(&record_pairs(&regs(0xCF, 1)), false)), Err(RecordError::NoEcho));
    }

    #[test]
    fn a_record_cut_in_the_middle_of_a_pair_is_incomplete() {
        let mut p = packet(&record_pairs(&regs(0xCF, 1)), true);
        p.truncate(p.len() - 3); // the last pair loses half its data word
        assert_eq!(parse_answer(&p), Err(RecordError::Incomplete { have: 0xCE, need: 0xCF }));
    }

    #[test]
    fn a_repeated_register_is_refused() {
        let mut p = record_pairs(&regs(0xCF, 1));
        p.push((0x0010, 0x0F));
        assert_eq!(parse_answer(&packet(&p, true)), Err(RecordError::Repeated(0x0010)));
    }

    #[test]
    fn a_cap_length_larger_than_the_packet_is_incomplete() {
        let mut r = regs(0xCF, 1);
        r[2] = 0xD5; // six more registers announced than sent
        assert_eq!(parse_answer(&packet(&record_pairs(&r), true)), Err(RecordError::Incomplete { have: 0xCF, need: 0xD5 }));
        r[2] = 240;
        assert_eq!(parse_answer(&packet(&record_pairs(&r), true)), Err(RecordError::Length(Some(240))));
    }

    #[test]
    fn magic_and_major_are_checked() {
        let mut r = regs(0xCF, 1);
        r[0] = 0x4737_3236;
        assert_eq!(parse_answer(&packet(&record_pairs(&r), true)), Err(RecordError::Magic(Some(0x4737_3236))));
        let mut r = regs(0xCF, 1);
        r[1] = 0x0002_0000;
        assert_eq!(parse_answer(&packet(&record_pairs(&r), true)), Err(RecordError::Major(2)));
        assert_eq!(parse_answer(&packet(&[], true)), Err(RecordError::Magic(None)));
    }

    #[test]
    fn a_higher_minor_version_keeps_its_extra_registers_raw() {
        let rec = parse_answer(&packet(&record_pairs(&regs(0xD2, 4)), true)).unwrap();
        assert_eq!((rec.version(), rec.len()), ((1, 4), 0xD2));
        assert_eq!(rec.registers()[0xD1], 0x1000_00D1);
    }
}
