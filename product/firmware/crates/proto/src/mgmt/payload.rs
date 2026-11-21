// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Payloads of the management commands (Annex M, Tables M-1 to M-5). Each type encodes into a
//! caller buffer and decodes from a received payload, so both ends use the same layout.

use super::tlv;

/// Payload error: too short, cut off, or a field outside its range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bad;

fn u16_at(b: &[u8], o: usize) -> Result<u16, Bad> {
    b.get(o..o + 2).map(|s| u16::from_be_bytes([s[0], s[1]])).ok_or(Bad)
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, Bad> {
    b.get(o..o + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or(Bad)
}
fn arr<const N: usize>(b: &[u8]) -> Result<[u8; N], Bad> {
    b.try_into().map_err(|_| Bad)
}

/// A version as major (u8), minor (u8), build (u16) (Table M-2 tags 4-6; Table M-4 bytes 10-13).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub build: u16,
}

impl Version {
    pub fn to_bytes(self) -> [u8; 4] {
        let b = self.build.to_be_bytes();
        [self.major, self.minor, b[0], b[1]]
    }
    pub fn from_bytes(b: [u8; 4]) -> Self {
        Self { major: b[0], minor: b[1], build: u16::from_be_bytes([b[2], b[3]]) }
    }
    pub fn is_empty(self) -> bool {
        self == Version::default()
    }
}

/// `IDENTIFY` reply (Table M-2). Text fields are borrowed when decoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Identify<'a> {
    pub model: &'a [u8],
    pub mac: [u8; 6],
    pub serial: Option<&'a [u8]>,
    pub running: Version,
    pub other_slot: Version,
    pub boot_stage: Version,
    pub hw_revision: u32,
    /// bit 0 stock image retained, bit 1 vendor recovery supported (tag 8).
    pub stock_flags: u8,
    /// integrity of the running image and of the other slot: 0 none, 1 good, 2 bad (tag 9).
    pub integrity: [u8; 2],
    /// 0 normal, 1 trial boot, 2 rolled back, 3 installer (tag 10).
    pub update_state: u8,
    pub secret_set: bool,
    /// first-secret window open, seconds left (tag 12).
    pub window: (bool, u16),
    pub active_level: u8,
    /// owner IP and UDP port; zeros if none (tag 14).
    pub owner: ([u8; 4], u16),
    pub tx_arm_state: u32,
    pub uptime_ms: u32,
    pub reset_cause: u8,
    /// management major, minor; P2 version; record major, minor (tag 18).
    pub versions: [u8; 5],
    pub controller: u8,
    pub tx_calibrated: bool,
    pub calibration_id: [u8; 32],
    pub calibration_date: u32,
    pub sectors_written: u8,
    /// Running slot: 1 slot A, 2 slot B (tag 23; 0043).
    pub running_slot: u8,
    /// The link-local address in use was taken without a complete RFC 3927 probe (tag 24, u8,
    /// 1 = unprobed; A1.11, 0057).
    pub ll_unprobed: bool,
}

impl<'a> Identify<'a> {
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        let mut w = tlv::Writer::new(out);
        w.put(1, self.model).put(2, &self.mac);
        if let Some(s) = self.serial {
            w.put(3, s);
        }
        w.put(4, &self.running.to_bytes())
            .put(5, &self.other_slot.to_bytes())
            .put(6, &self.boot_stage.to_bytes())
            .u32(7, self.hw_revision)
            .u8(8, self.stock_flags)
            .put(9, &self.integrity)
            .u8(10, self.update_state)
            .u8(11, self.secret_set as u8);
        let s = self.window.1.to_be_bytes();
        w.put(12, &[self.window.0 as u8, s[0], s[1]]).u8(13, self.active_level);
        let mut o = [0u8; 6];
        o[..4].copy_from_slice(&self.owner.0);
        o[4..].copy_from_slice(&self.owner.1.to_be_bytes());
        w.put(14, &o).u32(15, self.tx_arm_state).u32(16, self.uptime_ms).u8(17, self.reset_cause).put(18, &self.versions);
        w.u8(19, self.controller).u8(20, self.tx_calibrated as u8);
        let mut c = [0u8; 36];
        c[..32].copy_from_slice(&self.calibration_id);
        c[32..].copy_from_slice(&self.calibration_date.to_be_bytes());
        w.put(21, &c).u8(22, self.sectors_written).u8(23, self.running_slot).u8(24, self.ll_unprobed as u8);
        w.finish()
    }

    /// Decode; unknown tags are skipped, a cut-off list or a wrong length is an error.
    pub fn decode(p: &'a [u8]) -> Result<Self, Bad> {
        let mut r = Identify::default();
        for t in tlv::iter(p) {
            let (tag, v) = t.map_err(|_| Bad)?;
            match tag {
                1 => r.model = v,
                2 => r.mac = arr(v)?,
                3 => r.serial = Some(v),
                4 => r.running = Version::from_bytes(arr(v)?),
                5 => r.other_slot = Version::from_bytes(arr(v)?),
                6 => r.boot_stage = Version::from_bytes(arr(v)?),
                7 => r.hw_revision = u32::from_be_bytes(arr(v)?),
                8 => r.stock_flags = arr::<1>(v)?[0],
                9 => r.integrity = arr(v)?,
                10 => r.update_state = arr::<1>(v)?[0],
                11 => r.secret_set = arr::<1>(v)?[0] != 0,
                12 => {
                    let a: [u8; 3] = arr(v)?;
                    r.window = (a[0] != 0, u16::from_be_bytes([a[1], a[2]]));
                }
                13 => r.active_level = arr::<1>(v)?[0],
                14 => {
                    let a: [u8; 6] = arr(v)?;
                    r.owner = ([a[0], a[1], a[2], a[3]], u16::from_be_bytes([a[4], a[5]]));
                }
                15 => r.tx_arm_state = u32::from_be_bytes(arr(v)?),
                16 => r.uptime_ms = u32::from_be_bytes(arr(v)?),
                17 => r.reset_cause = arr::<1>(v)?[0],
                18 => r.versions = arr(v)?,
                19 => r.controller = arr::<1>(v)?[0],
                20 => r.tx_calibrated = arr::<1>(v)?[0] != 0,
                21 => {
                    let a: [u8; 36] = arr(v)?;
                    r.calibration_id.copy_from_slice(&a[..32]);
                    r.calibration_date = u32::from_be_bytes([a[32], a[33], a[34], a[35]]);
                }
                22 => r.sectors_written = arr::<1>(v)?[0],
                23 => r.running_slot = arr::<1>(v)?[0],
                24 => r.ll_unprobed = arr::<1>(v)?[0] != 0,
                _ => {}
            }
        }
        Ok(r)
    }
}

/// Number of `tx_band` entries (record field `tx_band_count`, at most 16, B4.8).
pub const MAX_BANDS: usize = 16;
/// Entries of the C-7 TCI allow-list (A1.6).
pub const MAX_ALLOW: usize = 8;

/// Configuration items of Table M-3. `None` = item not present (in `SET_CONFIG`: not changed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// C-1 address mode: 0 DHCP (with link-local fallback), 1 static (tag 1).
    pub address_mode: Option<u8>,
    pub address: Option<[u8; 4]>,
    pub netmask: Option<[u8; 4]>,
    pub gateway: Option<[u8; 4]>,
    /// C-2 default level at power-on, 1 or 2 (tag 5).
    pub default_level: Option<u8>,
    /// C-6 region, 0 in this version (tag 6).
    pub region: Option<u8>,
    /// C-3 maximum continuous TX time, ms (tag 7).
    pub max_tx_time_ms: Option<u32>,
    /// C-3 power limit per band, 0.1 W; index = `tx_band` index (tag 8, one TLV per band).
    pub band_limit: [Option<u16>; MAX_BANDS],
    /// C-3 SWR limit S2 × 100 (tag 9).
    pub swr_s2_x100: Option<u16>,
    /// C-3 PA temperature limit T3, 0.1 °C (tag 10).
    pub pa_temp_t3: Option<u16>,
    /// C-7 TCI allow-list (tag 11): `Some(n)` with the first `n` entries of `allow` valid.
    pub allow_count: Option<u8>,
    pub allow: [([u8; 4], u8); MAX_ALLOW],
}

impl Config {
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        let mut w = tlv::Writer::new(out);
        if let Some(v) = self.address_mode {
            w.u8(1, v);
        }
        if let Some(v) = self.address {
            w.put(2, &v);
        }
        if let Some(v) = self.netmask {
            w.put(3, &v);
        }
        if let Some(v) = self.gateway {
            w.put(4, &v);
        }
        if let Some(v) = self.default_level {
            w.u8(5, v);
        }
        if let Some(v) = self.region {
            w.u8(6, v);
        }
        if let Some(v) = self.max_tx_time_ms {
            w.u32(7, v);
        }
        for (i, l) in self.band_limit.iter().enumerate() {
            if let Some(l) = l {
                let b = l.to_be_bytes();
                w.put(8, &[i as u8, b[0], b[1]]);
            }
        }
        if let Some(v) = self.swr_s2_x100 {
            w.u16(9, v);
        }
        if let Some(v) = self.pa_temp_t3 {
            w.u16(10, v);
        }
        if let Some(n) = self.allow_count {
            let n = (n as usize).min(MAX_ALLOW);
            let mut b = [0u8; 5 * MAX_ALLOW];
            for (i, (ip, len)) in self.allow[..n].iter().enumerate() {
                b[5 * i..5 * i + 4].copy_from_slice(ip);
                b[5 * i + 4] = *len;
            }
            w.put(11, &b[..5 * n]);
        }
        w.finish()
    }

    /// Decode. Structural errors only (lengths, band index, list size); whether the values make
    /// a valid configuration is the radio's check (A1.6, status 48).
    pub fn decode(p: &[u8]) -> Result<Self, Bad> {
        let mut c = Config::default();
        for t in tlv::iter(p) {
            let (tag, v) = t.map_err(|_| Bad)?;
            match tag {
                1 => c.address_mode = Some(arr::<1>(v)?[0]),
                2 => c.address = Some(arr(v)?),
                3 => c.netmask = Some(arr(v)?),
                4 => c.gateway = Some(arr(v)?),
                5 => c.default_level = Some(arr::<1>(v)?[0]),
                6 => c.region = Some(arr::<1>(v)?[0]),
                7 => c.max_tx_time_ms = Some(u32::from_be_bytes(arr(v)?)),
                8 => {
                    let a: [u8; 3] = arr(v)?;
                    let i = a[0] as usize;
                    if i >= MAX_BANDS {
                        return Err(Bad);
                    }
                    c.band_limit[i] = Some(u16::from_be_bytes([a[1], a[2]]));
                }
                9 => c.swr_s2_x100 = Some(u16::from_be_bytes(arr(v)?)),
                10 => c.pa_temp_t3 = Some(u16::from_be_bytes(arr(v)?)),
                11 => {
                    if v.len() % 5 != 0 || v.len() / 5 > MAX_ALLOW {
                        return Err(Bad);
                    }
                    for (i, e) in v.chunks_exact(5).enumerate() {
                        c.allow[i] = ([e[0], e[1], e[2], e[3]], e[4]);
                    }
                    c.allow_count = Some((v.len() / 5) as u8);
                }
                _ => {}
            }
        }
        Ok(c)
    }

    /// Tags 9 and 10 need the calibration set (S2, T3; Table M-3 note: status 62).
    pub fn touches_calibrated_limits(&self) -> bool {
        self.swr_s2_x100.is_some() || self.pa_temp_t3.is_some()
    }
}

/// Package header of Table M-4 (the payload of `UPDATE_BEGIN` after the flags byte).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Package {
    pub header_version: u8,
    pub component: u8,
    pub hardware_id: u16,
    pub min_hw_revision: u16,
    pub version: Version,
    pub length: u32,
    pub sha256: [u8; 32],
    pub signature_len: u16,
    /// Target slot: 0 either slot (or a foreign image), 1 slot A, 2 slot B (byte 52; 0043).
    pub target_slot: u8,
}

pub const PACKAGE_MAGIC: u32 = 0x4737_504B; // "G7PK"
pub const PACKAGE_LEN: usize = 53;
/// Target hardware id of the G7250 (Table M-4 bytes 6-7).
pub const HARDWARE_ID_G7250: u16 = 0x00C8;
pub const COMPONENT_MAIN: u8 = 1;

impl Package {
    pub fn encode(&self) -> [u8; PACKAGE_LEN] {
        let mut b = [0u8; PACKAGE_LEN];
        b[0..4].copy_from_slice(&PACKAGE_MAGIC.to_be_bytes());
        b[4] = self.header_version;
        b[5] = self.component;
        b[6..8].copy_from_slice(&self.hardware_id.to_be_bytes());
        b[8..10].copy_from_slice(&self.min_hw_revision.to_be_bytes());
        b[10..14].copy_from_slice(&self.version.to_bytes());
        b[14..18].copy_from_slice(&self.length.to_be_bytes());
        b[18..50].copy_from_slice(&self.sha256);
        b[50..52].copy_from_slice(&self.signature_len.to_be_bytes());
        b[52] = self.target_slot;
        b
    }

    pub fn decode(b: &[u8]) -> Result<Self, Bad> {
        if b.len() < PACKAGE_LEN || u32_at(b, 0)? != PACKAGE_MAGIC {
            return Err(Bad);
        }
        Ok(Package {
            header_version: b[4],
            component: b[5],
            hardware_id: u16_at(b, 6)?,
            min_hw_revision: u16_at(b, 8)?,
            version: Version::from_bytes(arr(&b[10..14])?),
            length: u32_at(b, 14)?,
            sha256: arr(&b[18..50])?,
            signature_len: u16_at(b, 50)?,
            target_slot: b[52],
        })
    }
}

/// `UPDATE_BEGIN` request flags (Table M-1).
pub const BEGIN_FOREIGN: u8 = 0x01;
pub const BEGIN_DOWNGRADE: u8 = 0x02;

/// `UPDATE_BEGIN` request: flags, then the package header.
pub fn update_begin(flags: u8, pkg: &Package) -> [u8; 1 + PACKAGE_LEN] {
    let mut b = [0u8; 1 + PACKAGE_LEN];
    b[0] = flags;
    b[1..].copy_from_slice(&pkg.encode());
    b
}

pub fn parse_update_begin(p: &[u8]) -> Result<(u8, Package), Bad> {
    if p.len() != 1 + PACKAGE_LEN {
        return Err(Bad);
    }
    Ok((p[0], Package::decode(&p[1..])?))
}

/// `UPDATE_BEGIN` reply: block size (u16, 512), block count (u32), one-way (u8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BeginReply {
    pub block_size: u16,
    pub block_count: u32,
    pub one_way: bool,
}

impl BeginReply {
    pub fn encode(&self) -> [u8; 7] {
        let s = self.block_size.to_be_bytes();
        let c = self.block_count.to_be_bytes();
        [s[0], s[1], c[0], c[1], c[2], c[3], self.one_way as u8]
    }
    pub fn decode(p: &[u8]) -> Result<Self, Bad> {
        if p.len() != 7 {
            return Err(Bad);
        }
        Ok(Self { block_size: u16_at(p, 0)?, block_count: u32_at(p, 2)?, one_way: p[6] != 0 })
    }
}

/// `UPDATE_BLOCK` request: block number, CRC-32 of the data, data (512 bytes, the last one may
/// be shorter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block<'a> {
    pub number: u32,
    pub crc: u32,
    pub data: &'a [u8],
}

impl<'a> Block<'a> {
    /// Encode; the CRC is computed here (CRC-32/IEEE, the same as `g7250_layout::crc32`).
    pub fn encode(number: u32, data: &[u8], out: &mut [u8]) -> Option<usize> {
        if data.is_empty() || data.len() > super::BLOCK_DATA || out.len() < 8 + data.len() {
            return None;
        }
        out[0..4].copy_from_slice(&number.to_be_bytes());
        out[4..8].copy_from_slice(&crc32(data).to_be_bytes());
        out[8..8 + data.len()].copy_from_slice(data);
        Some(8 + data.len())
    }
    pub fn decode(p: &'a [u8]) -> Result<Self, Bad> {
        if p.len() < 9 || p.len() > 8 + super::BLOCK_DATA {
            return Err(Bad);
        }
        Ok(Self { number: u32_at(p, 0)?, crc: u32_at(p, 4)?, data: &p[8..] })
    }
    /// The CRC carried matches the data (status 53 if not).
    pub fn crc_ok(&self) -> bool {
        crc32(self.data) == self.crc
    }
}

/// CRC-32 [IEEE802.3], reflected, init and final XOR 0xFFFFFFFF.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ (0xEDB8_8320 & (c & 1).wrapping_neg());
        }
    }
    !c
}

/// `UPDATE_BLOCK` reply: block number, blocks received so far.
pub fn block_reply(number: u32, received: u32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[..4].copy_from_slice(&number.to_be_bytes());
    b[4..].copy_from_slice(&received.to_be_bytes());
    b
}

/// `UPDATE_STATUS` reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UpdateStatus {
    /// 0 idle, 1 receiving, 2 verified, 3 installer before the first erase, 4 installer writing.
    pub state: u8,
    pub received: u32,
    pub count: u32,
    pub sectors_written: u8,
}

impl UpdateStatus {
    pub fn encode(&self) -> [u8; 10] {
        let mut b = [0u8; 10];
        b[0] = self.state;
        b[1..5].copy_from_slice(&self.received.to_be_bytes());
        b[5..9].copy_from_slice(&self.count.to_be_bytes());
        b[9] = self.sectors_written;
        b
    }
    pub fn decode(p: &[u8]) -> Result<Self, Bad> {
        if p.len() != 10 || p[0] > 4 {
            return Err(Bad);
        }
        Ok(Self { state: p[0], received: u32_at(p, 1)?, count: u32_at(p, 5)?, sectors_written: p[9] })
    }
}

/// One `READ_LOG` entry: sequence, uptime ms, text (at most 255 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogEntry<'a> {
    pub seq: u32,
    pub uptime_ms: u32,
    pub text: &'a [u8],
}

impl<'a> LogEntry<'a> {
    pub fn encoded_len(&self) -> usize {
        9 + self.text.len().min(255)
    }
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        let t = &self.text[..self.text.len().min(255)];
        let n = 9 + t.len();
        if out.len() < n {
            return None;
        }
        out[0..4].copy_from_slice(&self.seq.to_be_bytes());
        out[4..8].copy_from_slice(&self.uptime_ms.to_be_bytes());
        out[8] = t.len() as u8;
        out[9..n].copy_from_slice(t);
        Some(n)
    }
}

/// Iterate the entries of a `READ_LOG` reply.
pub fn log_entries(mut p: &[u8]) -> impl Iterator<Item = Result<LogEntry<'_>, Bad>> {
    core::iter::from_fn(move || {
        if p.is_empty() {
            return None;
        }
        if p.len() < 9 || p.len() < 9 + p[8] as usize {
            p = &[];
            return Some(Err(Bad));
        }
        let n = 9 + p[8] as usize;
        let e = LogEntry { seq: u32::from_be_bytes([p[0], p[1], p[2], p[3]]), uptime_ms: u32::from_be_bytes([p[4], p[5], p[6], p[7]]), text: &p[9..n] };
        p = &p[n..];
        Some(Ok(e))
    })
}

/// Append one `GET_STATS` entry: name length (u8), name (ASCII), value (u64).
pub fn stat(out: &mut [u8], at: usize, name: &[u8], value: u64) -> Option<usize> {
    let n = 1 + name.len() + 8;
    if name.len() > 255 || out.len() < at + n {
        return None;
    }
    out[at] = name.len() as u8;
    out[at + 1..at + 1 + name.len()].copy_from_slice(name);
    out[at + 1 + name.len()..at + n].copy_from_slice(&value.to_be_bytes());
    Some(at + n)
}

/// Iterate the entries of a `GET_STATS` reply.
pub fn stats(mut p: &[u8]) -> impl Iterator<Item = Result<(&[u8], u64), Bad>> {
    core::iter::from_fn(move || {
        if p.is_empty() {
            return None;
        }
        let l = p[0] as usize;
        if p.len() < 1 + l + 8 {
            p = &[];
            return Some(Err(Bad));
        }
        let name = &p[1..1 + l];
        let mut v = [0u8; 8];
        v.copy_from_slice(&p[1 + l..9 + l]);
        p = &p[9 + l..];
        Some(Ok((name, u64::from_be_bytes(v))))
    })
}

/// `SWITCH_LEVEL` request: target level 1 or 2.
pub fn parse_switch_level(p: &[u8]) -> Result<u8, Bad> {
    match p {
        [l @ (1 | 2)] => Ok(*l),
        _ => Err(Bad),
    }
}

/// Head of a calibration set: date, source, then the Table M-5 entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalibrationHead<'a> {
    pub date: u32,
    pub source: &'a [u8],
    pub entries: &'a [u8],
}

/// Parse the `SET_CALIBRATION` request (date, source, entries).
pub fn parse_set_calibration(p: &[u8]) -> Result<CalibrationHead<'_>, Bad> {
    let date = u32_at(p, 0)?;
    let sl = *p.get(4).ok_or(Bad)? as usize;
    let source = p.get(5..5 + sl).ok_or(Bad)?;
    let entries = &p[5 + sl..];
    if calibration_entries(entries).any(|e| e.is_err()) {
        return Err(Bad);
    }
    Ok(CalibrationHead { date, source, entries })
}

/// The value of parameter `id` in a calibration set's entries, if the set holds it.
pub fn calibration_value<'a>(entries: &'a [u8], id: &str) -> Option<&'a [u8]> {
    calibration_entries(entries).filter_map(Result::ok).find(|(k, _)| *k == id.as_bytes()).map(|(_, v)| v)
}

/// Iterate the entries of Table M-5: (parameter id, value).
pub fn calibration_entries(mut p: &[u8]) -> impl Iterator<Item = Result<(&[u8], &[u8]), Bad>> {
    core::iter::from_fn(move || {
        if p.is_empty() {
            return None;
        }
        let il = p[0] as usize;
        let ok = p.len() >= 1 + il + 2 && p.len() >= 3 + il + u16::from_be_bytes([p[1 + il], p[2 + il]]) as usize;
        if !ok {
            p = &[];
            return Some(Err(Bad));
        }
        let vl = u16::from_be_bytes([p[1 + il], p[2 + il]]) as usize;
        let id = &p[1..1 + il];
        let v = &p[3 + il..3 + il + vl];
        p = &p[3 + il + vl..];
        Some(Ok((id, v)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identify_roundtrip_and_tags() {
        let id = Identify {
            model: b"G7250",
            mac: [2, 1, 2, 3, 4, 5],
            running: Version { major: 0, minor: 1, build: 1 },
            integrity: [1, 0],
            active_level: 1,
            owner: ([192, 168, 1, 5], 50000),
            versions: [1, 0, 44, 1, 1],
            ..Default::default()
        };
        let mut b = [0u8; 400];
        let n = id.encode(&mut b).unwrap();
        // Tag 1 first, "G7250"; tag 2 the MAC.
        assert_eq!(&b[..7], &[1, 5, b'G', b'7', b'2', b'5', b'0']);
        assert_eq!(&b[7..15], &[2, 6, 2, 1, 2, 3, 4, 5]);
        assert_eq!(Identify::decode(&b[..n]).unwrap(), id);
        // Unknown tags are skipped; a wrong length for a known tag is an error.
        let mut v = std::vec::Vec::from(&b[..n]);
        v.extend_from_slice(&[200, 2, 9, 9]);
        assert_eq!(Identify::decode(&v).unwrap(), id);
        assert_eq!(Identify::decode(&[2, 5, 0, 0, 0, 0, 0]), Err(Bad));
    }

    #[test]
    fn t_a1_76_identify_tag_24_reports_an_unprobed_link_local_address() {
        // 0057: tag 24, u8, 1 = the link-local address in use is unprobed.
        let mut b = [0u8; 400];
        for un in [false, true] {
            let id = Identify { model: b"G7250", ll_unprobed: un, ..Default::default() };
            let n = id.encode(&mut b).unwrap();
            assert!(b[..n].windows(3).any(|w| w == [24, 1, un as u8]));
            assert_eq!(Identify::decode(&b[..n]).unwrap().ll_unprobed, un);
        }
    }

    #[test]
    fn config_roundtrip_and_limits() {
        let mut c = Config { address_mode: Some(1), address: Some([192, 168, 1, 20]), netmask: Some([255, 255, 255, 0]), ..Default::default() };
        c.band_limit[3] = Some(500);
        c.allow_count = Some(1);
        c.allow[0] = ([192, 168, 1, 0], 24);
        let mut b = [0u8; 200];
        let n = c.encode(&mut b).unwrap();
        assert_eq!(Config::decode(&b[..n]).unwrap(), c);
        assert!(!c.touches_calibrated_limits());
        c.swr_s2_x100 = Some(250);
        assert!(c.touches_calibrated_limits());
        assert_eq!(Config::decode(&[8, 3, 16, 0, 1]), Err(Bad)); // band index 16
        assert_eq!(Config::decode(&[11, 4, 1, 2, 3, 4]), Err(Bad)); // not a multiple of 5
    }

    #[test]
    fn package_header_table_m4() {
        let p = Package {
            header_version: 1,
            component: COMPONENT_MAIN,
            hardware_id: HARDWARE_ID_G7250,
            min_hw_revision: 0,
            version: Version { major: 0, minor: 2, build: 7 },
            length: 36_000,
            sha256: [0xAB; 32],
            signature_len: 0,
            target_slot: 2,
        };
        let b = p.encode();
        assert_eq!(b.len(), 53);
        assert_eq!(b[52], 2); // 0043: target slot B
        assert_eq!(&b[0..4], b"G7PK");
        assert_eq!(&b[4..8], &[1, 1, 0x00, 0xC8]);
        assert_eq!(&b[10..14], &[0, 2, 0, 7]);
        assert_eq!(Package::decode(&b).unwrap(), p);
        let req = update_begin(BEGIN_FOREIGN, &p);
        assert_eq!(parse_update_begin(&req).unwrap(), (BEGIN_FOREIGN, p));
        let r = BeginReply { block_size: 512, block_count: 71, one_way: false };
        assert_eq!(BeginReply::decode(&r.encode()).unwrap(), r);
    }

    #[test]
    fn blocks() {
        let mut out = [0u8; 600];
        let n = Block::encode(5, b"123456789", &mut out).unwrap();
        let b = Block::decode(&out[..n]).unwrap();
        assert_eq!((b.number, b.crc, b.data), (5, 0xCBF4_3926, &b"123456789"[..]));
        assert!(b.crc_ok());
        out[8] ^= 1;
        assert!(!Block::decode(&out[..n]).unwrap().crc_ok());
        assert!(Block::encode(0, &[0u8; 513], &mut out).is_none());
        assert_eq!(Block::decode(&[0u8; 8]), Err(Bad));
    }

    #[test]
    fn log_stats_status_level_calibration() {
        let mut b = [0u8; 64];
        let n = LogEntry { seq: 1, uptime_ms: 2, text: b"boot" }.encode(&mut b).unwrap();
        let m = LogEntry { seq: 2, uptime_ms: 3, text: b"" }.encode(&mut b[n..]).unwrap();
        let v: std::vec::Vec<_> = log_entries(&b[..n + m]).map(|e| e.unwrap()).collect();
        assert_eq!(v[0].text, b"boot");
        assert_eq!(v[1].seq, 2);
        let at = stat(&mut b, 0, b"rx", 7).unwrap();
        let at = stat(&mut b, at, b"dropped_1024", 1).unwrap();
        let s: std::vec::Vec<_> = stats(&b[..at]).map(|e| e.unwrap()).collect();
        assert_eq!(s, [(&b"rx"[..], 7u64), (&b"dropped_1024"[..], 1)]);
        let u = UpdateStatus { state: 1, received: 3, count: 71, sectors_written: 0 };
        assert_eq!(UpdateStatus::decode(&u.encode()).unwrap(), u);
        assert_eq!(parse_switch_level(&[2]), Ok(2));
        assert_eq!(parse_switch_level(&[3]), Err(Bad));
        let req = [0, 0, 0, 1, 3, b'l', b'a', b'b', 6, b'P', b'-', b'R', b'F', b'-', b'2', 0, 2, 0x12, 0x34];
        let h = parse_set_calibration(&req).unwrap();
        assert_eq!((h.date, h.source), (1, &b"lab"[..]));
        let e: std::vec::Vec<_> = calibration_entries(h.entries).map(|e| e.unwrap()).collect();
        assert_eq!(e, [(&b"P-RF-2"[..], &[0x12u8, 0x34][..])]);
        assert_eq!(parse_set_calibration(&req[..req.len() - 1]), Err(Bad));
    }
}

/// The parameter ids of Annex P (Table P-1): a calibration entry with any other id is not
/// allowed (A1.12: status 48).
pub const PARAMETER_IDS: [&str; 37] = [
    "P-RF-1", "P-RF-2", "P-RF-3", "P-RF-4", "P-RF-5", "P-ANT-1", "P-ATU-1", "P-PWR-1", "P-PWR-2", "P-PWR-3", "P-PWR-4", "P-PWR-5",
    "P-SEN-1", "P-SEN-2", "P-SEN-3", "P-SEN-4", "P-SEN-5", "P-TIM-1", "P-TIM-2", "P-NET-1", "P-NET-2", "P-NET-3", "P-SYS-1",
    "P-SYS-2", "P-SYS-3", "P-SYS-4", "P-SYS-5", "P-SYS-6", "P-SYS-7", "P-SYS-8", "P-SYS-9", "P-RX-1", "P-RX-2", "P-TX-1",
    "P-L2-1", "P-L2-2", "P-L2-3",
];

/// Whether a request payload has the size and structure Annex M gives for its command (A1.4
/// "Malformed payload", 0056): if not, the request cannot be read and gets 17 in the framing
/// group, before authentication and every state check. Whether its values are allowed is the
/// command's own check, after the state checks (48, 49, 51-56, 59-63).
pub fn well_formed(cmd: super::Command, p: &[u8]) -> bool {
    use super::Command as C;
    match cmd {
        C::SetFirstSecret | C::ChangeSecret => p.len() == 32,
        C::SetConfig => Config::decode(p).is_ok(),
        C::SwitchLevel | C::UpdateActivate => p.len() == 1,
        C::ReadLog => p.len() >= 4,
        C::UpdateBegin => parse_update_begin(p).is_ok(),
        C::UpdateBlock => Block::decode(p).is_ok(),
        C::UpdateVerify | C::UpdateAbort | C::Rollback => p.is_empty(),
        C::SetCalibration => p.len() <= 1024 && parse_set_calibration(p).is_ok(),
        _ => true,
    }
}
