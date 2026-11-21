// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! G7250-P2 discovery on UDP port 1024 (spec B3; [P2] Discovery packet and Discovery reply).

/// Local port for discovery, General packet and Set IP (Table B2-1 row 1).
pub const PORT: u16 = 1024;
/// Length of the [P2] short reply, format 1 (B3.3).
pub const REPLY_LEN: usize = 60;
/// Board type of the G7250 (0002; Table B3-1 byte 11).
pub const BOARD_TYPE: u8 = 200;
/// openHPSDR protocol version, V4.4 (B3.5; B1.3).
pub const P2_VERSION: u8 = 44;

/// Command byte 4 of a port-1024 packet from the host ([P2]).
pub const CMD_DISCOVERY: u8 = 0x02;
pub const CMD_SET_IP: u8 = 0x03;
pub const CMD_ERASE: u8 = 0x04;
pub const CMD_PROGRAM: u8 = 0x05;

/// What a datagram to port 1024 is, before session logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port1024 {
    /// A discovery request (B3.2): byte 4 = 0x02, at least 5 bytes. Bytes 0-3 and 5-59 are not
    /// required to be zero (B3.2 third item; B11.3).
    Discovery,
    /// Set IP Address (B3.9), with the target MAC and the new address.
    SetIp { mac: [u8; 6], ip: [u8; 4] },
    /// Erase and Program are ignored without a reply (B3.9; Annex X1 D12).
    IgnoredProgramming,
    /// From source port 1024: dropped and counted, never answered (B3.2 last item; 0022 §73 item 20).
    FromPort1024,
    /// Anything else on port 1024 that is not a General packet. The General packet is told apart
    /// by its length and is handled by the session layer (B5), not here.
    Other,
}

/// Classify a datagram to port 1024. `src_port` is the UDP source port.
pub fn classify(payload: &[u8], src_port: u16) -> Port1024 {
    if src_port == PORT {
        return Port1024::FromPort1024;
    }
    if payload.len() < 5 {
        return Port1024::Other;
    }
    match payload[4] {
        CMD_DISCOVERY => Port1024::Discovery,
        CMD_SET_IP if payload.len() >= 15 => {
            let mut mac = [0u8; 6];
            mac.copy_from_slice(&payload[5..11]);
            let mut ip = [0u8; 4];
            ip.copy_from_slice(&payload[11..15]);
            Port1024::SetIp { mac, ip }
        }
        CMD_ERASE | CMD_PROGRAM => Port1024::IgnoredProgramming,
        _ => Port1024::Other,
    }
}

/// Values the reply needs from the rest of the firmware.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub mac: [u8; 6],
    pub fw_major: u8,
    pub fw_minor: u8,
    pub beta: bool,
    /// Number of DDCs implemented, equal to record field `num_ddc`; 1 in this version (0004).
    pub num_ddc: u8,
}

/// B3.5: major × 10 + minor, capped at 255.
pub fn fw_code_version(major: u8, minor: u8) -> u8 {
    let v = major as u32 * 10 + minor as u32;
    if v > 255 { 255 } else { v as u8 }
}

/// Status byte 4 (B3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Not running, or running and the discovery comes from the owner.
    Available = 0x02,
    /// Running and the discovery comes from another host (also the owner's IP on another port),
    /// or level 2 active.
    InUse = 0x03,
}

/// Build the 60-byte reply of Table B3-1.
pub fn reply(id: &Identity, status: Status) -> [u8; REPLY_LEN] {
    let mut r = [0u8; REPLY_LEN];
    // Bytes 0-3: sequence number 0 (B2.7: the discovery reply always carries 0).
    r[4] = status as u8;
    r[5..11].copy_from_slice(&id.mac);
    r[11] = BOARD_TYPE;
    r[12] = P2_VERSION;
    r[13] = fw_code_version(id.fw_major, id.fw_minor);
    // Bytes 14-19: Mercury/Penny/Metis versions, 0 (Atlas only).
    r[20] = id.num_ddc;
    r[21] = 1; // phase word, fixed scale (B3.8)
    r[22] = 0x00; // Big-Endian, 3-byte I&Q only (B3.7)
    r[23] = id.beta as u8;
    // Bytes 24-59: 0.
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> Identity {
        Identity { mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55], fw_major: 1, fw_minor: 5, beta: false, num_ddc: 1 }
    }

    #[test]
    fn reply_matches_table_b3_1() {
        let r = reply(&id(), Status::Available);
        assert_eq!(r.len(), 60);
        assert_eq!(&r[0..4], &[0, 0, 0, 0]);
        assert_eq!(r[4], 0x02);
        assert_eq!(&r[5..11], &[0x02, 0x11, 0x22, 0x33, 0x44, 0x55]);
        assert_eq!(r[11], 200);
        assert_eq!(r[12], 44);
        assert_eq!(r[13], 15); // "15 = 1.5"
        assert!(r[14..20].iter().all(|&b| b == 0));
        assert_eq!(r[20], 1);
        assert_eq!(r[21], 1);
        assert_eq!(r[22], 0);
        assert_eq!(r[23], 0);
        assert!(r[24..].iter().all(|&b| b == 0));
        assert_eq!(reply(&id(), Status::InUse)[4], 0x03);
    }

    #[test]
    fn never_fe_ff_or_board_254_255() {
        for s in [Status::Available, Status::InUse] {
            let r = reply(&id(), s);
            assert!(r[4] != 0xFE && r[4] != 0xFF);
            assert!(r[11] != 254 && r[11] != 255);
        }
    }

    #[test]
    fn version_cap_and_beta() {
        assert_eq!(fw_code_version(0, 1), 1);
        assert_eq!(fw_code_version(25, 5), 255);
        assert_eq!(fw_code_version(30, 0), 255);
        let mut i = id();
        i.beta = true;
        assert_ne!(reply(&i, Status::Available)[23], 0);
    }

    #[test]
    fn classify_rules() {
        let mut req = [0u8; 60];
        req[4] = 0x02;
        assert_eq!(classify(&req, 50000), Port1024::Discovery);
        // Short requests and non-zero padding are still answered (B3.2).
        assert_eq!(classify(&req[..5], 50000), Port1024::Discovery);
        req[0] = 0x12;
        req[30] = 0x99;
        assert_eq!(classify(&req, 50000), Port1024::Discovery);
        // From port 1024: dropped (B3.2).
        assert_eq!(classify(&req, 1024), Port1024::FromPort1024);
        // Our own reply looks like a request (byte 4 = 0x02) and comes from 1024: dropped.
        let own = reply(&id(), Status::Available);
        assert_eq!(classify(&own, 1024), Port1024::FromPort1024);
        assert_eq!(classify(&req[..4], 50000), Port1024::Other);
        let mut e = [0u8; 60];
        e[4] = 0x04;
        assert_eq!(classify(&e, 50000), Port1024::IgnoredProgramming);
        e[4] = 0x05;
        assert_eq!(classify(&e, 50000), Port1024::IgnoredProgramming);
        let mut s = [0u8; 60];
        s[4] = 0x03;
        s[5..11].copy_from_slice(&[1, 2, 3, 4, 5, 6]);
        s[11..15].copy_from_slice(&[192, 168, 1, 50]);
        assert_eq!(classify(&s, 50000), Port1024::SetIp { mac: [1, 2, 3, 4, 5, 6], ip: [192, 168, 1, 50] });
    }
}
