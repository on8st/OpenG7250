// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Host end of P2 discovery (B3.2, B3.3, Table B3-1), for `g7250ctl` (0037). The radio end is
//! [`crate::discovery`]; nothing here is used by the firmware.

use crate::discovery::{BOARD_TYPE, CMD_DISCOVERY, REPLY_LEN};

/// Length of the discovery request the host sends ([P2] Discovery packet: 60 bytes).
pub const REQUEST_LEN: usize = 60;

/// Byte 4 of a reply: not running, or running and asked by the owner (B3.4).
pub const STATUS_AVAILABLE: u8 = 0x02;
/// Byte 4 of a reply: running for another host, or level 2 active (B3.4).
pub const STATUS_IN_USE: u8 = 0x03;

/// The discovery request of B3.2: sequence number 0, byte 4 = 0x02, bytes 5-59 zero. A host
/// sends it from any local port other than 1024 (B3.2, last item).
pub fn request() -> [u8; REQUEST_LEN] {
    let mut r = [0u8; REQUEST_LEN];
    r[4] = CMD_DISCOVERY;
    r
}

/// A parsed discovery reply (Table B3-1). Fields are kept raw so that a host can show a reply
/// from any P2 radio, not only a G7250.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    /// Byte 4: 0x02 available, 0x03 in use (B3.4).
    pub status: u8,
    /// Bytes 5-10 (B3.6).
    pub mac: [u8; 6],
    /// Byte 11: 200 for a G7250 (0002).
    pub board_type: u8,
    /// Byte 12: P2 version × 10, 44 for V4.4 (B3.5).
    pub p2_version: u8,
    /// Byte 13: firmware major × 10 + minor, capped at 255 (B3.5).
    pub fw_code: u8,
    /// Byte 20: number of DDCs implemented.
    pub num_ddc: u8,
    /// Byte 21: 1 = phase word (B3.8).
    pub phase_word: u8,
    /// Byte 22: endian modes, 0 = Big-Endian, 3-byte I&Q only (B3.7).
    pub endian_modes: u8,
    /// Byte 23: non-zero for a beta build.
    pub beta: u8,
}

impl Reply {
    pub fn is_g7250(&self) -> bool {
        self.board_type == BOARD_TYPE
    }
}

/// Parse a reply received on the host's discovery socket. A host request has the same shape
/// (byte 4 = 0x02, sequence 0), so the caller must also check that the datagram came from port
/// 1024 (B2.4a); a broadcast request can loop back to the sender's own socket.
/// `None` for anything that is not a
/// [P2] discovery reply: shorter than the 60-byte short reply, a non-zero sequence number
/// (B2.7), or byte 4 not 0x02 or 0x03. A longer datagram (another radio's long reply) is read
/// by its first 60 bytes.
pub fn parse_reply(d: &[u8]) -> Option<Reply> {
    if d.len() < REPLY_LEN || d[0..4] != [0, 0, 0, 0] || !(d[4] == STATUS_AVAILABLE || d[4] == STATUS_IN_USE) {
        return None;
    }
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&d[5..11]);
    Some(Reply {
        status: d[4],
        mac,
        board_type: d[11],
        p2_version: d[12],
        fw_code: d[13],
        num_ddc: d[20],
        phase_word: d[21],
        endian_modes: d[22],
        beta: d[23],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::{self, Identity, Status};

    #[test]
    fn request_layout() {
        let r = request();
        assert_eq!(r.len(), 60);
        assert_eq!(r[4], 0x02);
        assert!(r.iter().enumerate().all(|(i, b)| i == 4 || *b == 0));
        // The radio end classifies it as a discovery from a non-1024 port.
        assert!(matches!(discovery::classify(&r, 50000), discovery::Port1024::Discovery));
    }

    #[test]
    fn reads_the_radio_reply() {
        let id = Identity { mac: [0x02, 0x11, 0x22, 0x33, 0x44, 0x55], fw_major: 1, fw_minor: 5, beta: false, num_ddc: 1 };
        let r = parse_reply(&discovery::reply(&id, Status::InUse)).unwrap();
        assert_eq!(r.status, STATUS_IN_USE);
        assert_eq!(r.mac, id.mac);
        assert!(r.is_g7250());
        assert_eq!((r.p2_version, r.fw_code, r.num_ddc, r.phase_word, r.endian_modes, r.beta), (44, 15, 1, 1, 0, 0));
    }

    #[test]
    fn rejects_non_replies() {
        let ok = {
            let mut b = [0u8; 60];
            b[4] = 0x02;
            b
        };
        assert!(parse_reply(&ok).is_some());
        assert!(parse_reply(&ok[..59]).is_none());
        let mut seq = ok;
        seq[3] = 1;
        assert!(parse_reply(&seq).is_none());
        let mut busy = ok;
        busy[4] = 0xFE; // 0xFE/0xFF are never sent by a G7250 (B3.4) and are not a short reply
        assert!(parse_reply(&busy).is_none());
        let mut long = [0u8; 80];
        long[4] = 0x03;
        assert!(parse_reply(&long).is_some());
    }
}
