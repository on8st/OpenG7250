// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Capability record, version 1.1 (spec B4.8, Table B4-1; 0003, 0020 §26), 207 registers from
//! `CAP_BASE` = 0x0000. Values are constant for a session (B4.7).

/// Registers in the record (`cap_length`, version 1.1).
pub const LEN: usize = 0xCF;

/// Transmit ranges of region 0 (Table A3-0), in Hz, with the maximum powers in 0.1 W
/// (SSB/CW/FM, AM). Range 10 (50 MHz): maximum power pending hardware (P-PWR-2), reported as 0.
pub const TX_BANDS: [(u32, u32, u16, u16); 10] = [
    (1_800_000, 2_000_000, 1000, 250),
    (3_500_000, 3_900_000, 1000, 250),
    (7_000_000, 7_200_000, 1000, 250),
    (10_100_000, 10_150_000, 1000, 250),
    (14_000_000, 14_350_000, 1000, 250),
    (18_068_000, 18_168_000, 1000, 250),
    (21_000_000, 21_450_000, 1000, 250),
    (24_890_000, 24_990_000, 1000, 250),
    (28_000_000, 29_700_000, 1000, 250),
    (50_000_000, 54_000_000, 0, 0),
];

/// What the record needs from the build and the radio.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    /// [31:24] major, [23:16] minor, [15:0] build.
    pub fw_version: u32,
    /// Record `tx` bit 1: TX implemented in this firmware (B4.8; B10).
    pub tx_implemented: bool,
    /// Record `tx` bit 4: the calibration set is complete (A1.12); always 0 while bit 1 is 0.
    pub tx_calibrated: bool,
    /// 384 ksps offered (only once the SPI link to the CH394 is shown fast enough, P-NET-1).
    pub rate_384: bool,
}

const fn ascii4(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

pub fn build(p: &Params) -> [u32; LEN] {
    let mut r = [0u32; LEN];
    r[0x00] = ascii4(b"G725"); // cap_magic
    r[0x01] = 0x0001_0001; // cap_version 1.1
    r[0x02] = LEN as u32; // cap_length
    r[0x03] = 0x0001_0000; // regmap_version 1.0
    r[0x04] = ascii4(b"G725"); // product_name "G7250-P2"
    r[0x05] = ascii4(b"0-P2");
    r[0x08] = p.fw_version;
    r[0x09] = 44; // p2_version
    r[0x0A] = 0; // hw_revision: not identified (HIS-18)
    r[0x0B] = 0x01 | (1 << 4); // level 1 present; active level 1
    r[0x0C] = 1 | ((p.tx_implemented as u32) << 1); // RX IQ; TX IQ per tx bit 1; no audio, mic, wideband
    r[0x0D] = 1; // num_adc
    r[0x0E] = 1; // num_ddc (0004)
    r[0x0F] = 1; // num_duc
    r[0x10] = if p.rate_384 { 0x0F } else { 0x07 }; // ddc_rates
    r[0x11] = 0x00C0_0018; // duc_format: 192 ksps, 24 bits
    r[0x12] = 122_880_000; // dsp_clock_hz (B3.8)
    r[0x13] = 2; // rx_range_count
    r[0x14] = 500_000;
    r[0x15] = 30_000_000;
    r[0x16] = 50_000_000;
    r[0x17] = 54_000_000;
    r[0x1C] = 2; // two antenna connectors; host cannot select (P-ANT-1); no RX-only input
    r[0x1D] = 0b0_1111; // atu: present, host-controlled (0032), per-band memory, external tuner present; bit 4 0 (P-SYS-8)
    r[0x1E] = (20 << 16) | 150; // atu_range
    r[0x1F] = 0b110_1111; // telemetry: fwd, rev, PA temp, supply V, PA current, ADC overload; no separate supply current
    let tx_cal = p.tx_implemented && p.tx_calibrated;
    r[0x20] = 1 | ((p.tx_implemented as u32) << 1) | (1 << 2) | (1 << 3) | ((tx_cal as u32) << 4);
    r[0x21] = TX_BANDS.len() as u32;
    for (i, (lo, hi, pmax, pam)) in TX_BANDS.iter().enumerate() {
        r[0x22 + 3 * i] = *lo;
        r[0x23 + 3 * i] = *hi;
        r[0x24 + 3 * i] = ((*pmax as u32) << 16) | *pam as u32;
    }
    r[0x52] = (7251 << 16) | 1; // mgmt
    r[0x53] = 0b11_1111; // regmap_groups: 0-5 present; 6, 7 not in this version
    r[0x54] = 100; // link: minimum 100 Mbit/s
    // wb_format, wb_frame: 0 (no wideband before M2).
    r[0x57] = (4095 << 16) | (1 << 8); // cal_info: N = 4095, one constant set, no constants yet (P-SEN-1)
    // cal_supply_c, cal_power: 0 until calibrated.
    r[0xB9] = 0; // status_fill: no standard status field filled with a measurement yet (HIS-11, P-SEN-1)
    r[0xBA] = 0; // clock_ref: no reference PLL known (HIS-05)
    r[0xBB] = 0; // coupler: not reported (P-SEN-1)
    r[0xBC] = (1000 << 16) | (10 << 8) | 1; // tlm_rates
    r[0xBD] = (50 << 16) | 1; // status_rates
    r[0xBE] = 0; // tlm_count: no telemetry descriptors yet (HIS-11)
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> [u32; LEN] {
        build(&Params { fw_version: 0x0001_0007, tx_implemented: false, tx_calibrated: false, rate_384: false })
    }

    #[test]
    fn header_and_identity() {
        let r = rec();
        assert_eq!(r[0], 0x4737_3235);
        assert_eq!(r[1], 0x0001_0001);
        assert_eq!(r[2], 0xCF);
        assert_eq!(&r[4..8], &[0x4737_3235, 0x302D_5032, 0, 0]);
        assert_eq!(r[9], 44);
        assert_eq!(r[0x0E], 1);
        assert_eq!(r[0x12], 122_880_000);
        assert_eq!(r[0x52], 0x1C53_0001);
        assert_eq!(r[0x54], 100);
    }

    #[test]
    fn tx_bits() {
        let r = rec();
        assert_eq!(r[0x20], 0b0_1101); // present, arm required, mode class required; not implemented, not calibrated
        assert_eq!(r[0x0C] & 2, 0);
        let r = build(&Params { fw_version: 0, tx_implemented: true, tx_calibrated: false, rate_384: false });
        assert_eq!(r[0x20], 0b0_1111);
        // Bit 4 is never set while bit 1 is 0 (0039).
        let r = build(&Params { fw_version: 0, tx_implemented: false, tx_calibrated: true, rate_384: false });
        assert_eq!(r[0x20] & 0x10, 0);
    }

    #[test]
    fn bands_table_a3_0() {
        let r = rec();
        assert_eq!(r[0x21], 10);
        assert_eq!((r[0x22], r[0x23], r[0x24]), (1_800_000, 2_000_000, (1000 << 16) | 250));
        assert_eq!((r[0x22 + 27], r[0x23 + 27]), (50_000_000, 54_000_000));
        // Unused band entries are 0.
        assert!(r[0x22 + 30..0x52].iter().all(|&v| v == 0));
    }
}
