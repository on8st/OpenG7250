//! Capability record, version 1.1 (spec B4.8, Table B4-1). Values marked PLACEHOLDER are not
//! fixed by the spec (TBD with an HIS id, or given only by reference to [G7250-MAN]); the model
//! uses the stated assumption and the test bench must not assert them.

use crate::codes::{reg, DSP_CLOCK_HZ, P2_VERSION};
use crate::radio::Config;

pub const LEN: usize = reg::CAP_LENGTH as usize; // 207

fn ascii4(s: &[u8]) -> u32 {
    let mut b = [0u8; 4];
    b[..s.len().min(4)].copy_from_slice(&s[..s.len().min(4)]);
    u32::from_be_bytes(b)
}

/// TX bands, factory default region = intersection of the transmit ranges of all regions
/// (A1.6 C-6). PLACEHOLDER: the spec gives the ranges only by reference to [G7250-MAN]
/// (finding SF-07); these are the IARU-region intersections of the ten HF/6 m bands.
pub const TX_BANDS: [(u32, u32); 10] = [
    (1_810_000, 2_000_000),
    (3_500_000, 3_800_000),
    (7_000_000, 7_200_000),
    (10_100_000, 10_150_000),
    (14_000_000, 14_350_000),
    (18_068_000, 18_168_000),
    (21_000_000, 21_450_000),
    (24_890_000, 24_990_000),
    (28_000_000, 29_700_000),
    (50_000_000, 52_000_000),
];

pub fn regmap_groups(cfg: &Config) -> u32 {
    // bits 0, 1 always; bit 2 TX group (r1.0 has TX, locked until calibrated); bit 4 ATU (0032)
    let mut g = 0b0000_0011 | 0b0000_0100;
    if cfg.atu_group {
        g |= 1 << 4;
    }
    g
}

pub fn build(cfg: &Config) -> Vec<u32> {
    let mut r = vec![0u32; LEN];
    r[0x00] = 0x4737_3235; // "G725"
    r[0x01] = 0x0001_0001;
    r[0x02] = LEN as u32;
    r[0x03] = 0x0001_0000;
    r[0x04] = ascii4(b"G725");
    r[0x05] = ascii4(b"0-P2");
    let (maj, min, build) = cfg.fw;
    r[0x08] = (maj as u32) << 24 | (min as u32) << 16 | build as u32;
    r[0x09] = P2_VERSION as u32;
    r[0x0A] = 0; // hw_revision: not identified (HIS-18)
    r[0x0B] = 0x1 | (1 << 4); // level 1 present, active level 1
    let tx_impl = !cfg.tx_locked;
    r[0x0C] = 0x1 | if tx_impl { 0x2 } else { 0 }; // RX IQ; TX IQ per tx bit 1
    r[0x0D] = 1; // num_adc (HIS-01, assumed)
    r[0x0E] = 1; // num_ddc, 0004
    r[0x0F] = 1; // num_duc
    r[0x10] = cfg.ddc_rates;
    r[0x11] = 0x00C0_0018;
    r[0x12] = DSP_CLOCK_HZ as u32;
    r[0x13] = 2;
    r[0x14] = 500_000;
    r[0x15] = 30_000_000;
    r[0x16] = 50_000_000;
    r[0x17] = 54_000_000;
    r[0x1C] = 2; // two connectors; bit 8, 9 per HIS-06 (0)
    r[0x1D] = 0b1101 | if cfg.atu_group { 0b10 } else { 0 };
    r[0x1E] = (20 << 16) | 150;
    r[0x1F] = 0; // telemetry: nothing reported until calibrated (HIS-11)
    r[0x20] = 0b1101 | if tx_impl { 0b10 } else { 0 };
    r[0x21] = TX_BANDS.len() as u32;
    for (i, (lo, hi)) in TX_BANDS.iter().enumerate() {
        let base = 0x22 + 3 * i;
        r[base] = *lo;
        r[base + 1] = *hi;
        // 100 W / 25 W; 50 MHz PLACEHOLDER 90 W / 22.5 W (HIS-12)
        r[base + 2] = if *lo >= 50_000_000 { (900 << 16) | 225 } else { (1000 << 16) | 250 };
    }
    r[0x52] = 0x1C53_0001;
    r[0x53] = regmap_groups(cfg);
    r[0x54] = 100;
    r[0x55] = 0; // wideband before M2
    r[0x56] = 0;
    r[0x57] = (4095 << 16) | (1 << 8); // N = 4095, one set, nothing filled (not calibrated)
    r[0xB9] = (1 << 5) | (1 << 6) | (1 << 7) | (1 << 8) | (1 << 10); // PLACEHOLDER (HIS-11)
    r[0xBA] = 0; // no reference PLL (HIS-05)
    r[0xBB] = 0; // coupler not reported
    r[0xBC] = (1000 << 16) | (10 << 8) | 1;
    r[0xBD] = (50 << 16) | 1;
    r[0xBE] = 0;
    r
}
