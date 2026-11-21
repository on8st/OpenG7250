// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! CRC-32 (IEEE 802.3, reflected, polynomial 0xEDB88320, init and final XOR 0xFFFFFFFF), the
//! CRC the management channel uses for update blocks (spec A1.8). Table-free; fast enough for
//! one 256 KiB image check at boot.

pub const fn update(mut crc: u32, data: &[u8]) -> u32 {
    let mut i = 0;
    while i < data.len() {
        crc ^= data[i] as u32;
        let mut b = 0;
        while b < 8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            b += 1;
        }
        i += 1;
    }
    crc
}

pub const fn init() -> u32 {
    0xFFFF_FFFF
}

pub const fn finish(crc: u32) -> u32 {
    crc ^ 0xFFFF_FFFF
}

pub const fn crc32(data: &[u8]) -> u32 {
    finish(update(init(), data))
}

#[cfg(test)]
mod tests {
    #[test]
    fn check_value() {
        // The standard check value of CRC-32/ISO-HDLC.
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
    #[test]
    fn incremental() {
        let c = super::update(super::init(), b"12345");
        let c = super::update(c, b"6789");
        assert_eq!(super::finish(c), 0xCBF4_3926);
    }
}
