// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Minimal DHCP client messages (RFC 2131, RFC 2132): DISCOVER, REQUEST and the parsing of
//! OFFER, ACK and NAK. The exchange runs at power-on before the P2 sockets are opened (spec
//! A1.11, provisional). Renewal is not implemented yet: the lease time is reported, and the
//! address is kept until a restart (spec question SQ-F2-1; A1.11 final rule TBD, HIS-14).

use crate::netcfg::Ip;

pub const CLIENT_PORT: u16 = 68;
pub const SERVER_PORT: u16 = 67;
const MAGIC: [u8; 4] = [99, 130, 83, 99];

pub const DISCOVER: u8 = 1;
pub const OFFER: u8 = 2;
pub const REQUEST: u8 = 3;
pub const DECLINE: u8 = 4;
pub const ACK: u8 = 5;
pub const NAK: u8 = 6;

/// Length of the messages we send: 236 fixed bytes, the magic cookie and our options, padded to
/// the 300-byte BOOTP minimum.
pub const MSG_LEN: usize = 300;

fn base(xid: u32, mac: [u8; 6], buf: &mut [u8; MSG_LEN]) {
    buf.fill(0);
    buf[0] = 1; // BOOTREQUEST
    buf[1] = 1; // Ethernet
    buf[2] = 6;
    buf[4..8].copy_from_slice(&xid.to_be_bytes());
    buf[10] = 0x80; // broadcast flag: we cannot receive unicast before the address is set
    buf[28..34].copy_from_slice(&mac);
    buf[236..240].copy_from_slice(&MAGIC);
}

/// DHCPDISCOVER with options 53, 55 (subnet, router), 61 (client id) and end.
pub fn discover(xid: u32, mac: [u8; 6], buf: &mut [u8; MSG_LEN]) -> usize {
    base(xid, mac, buf);
    let o = [53, 1, DISCOVER, 55, 2, 1, 3, 61, 7, 1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 255];
    buf[240..240 + o.len()].copy_from_slice(&o);
    MSG_LEN
}

/// DHCPREQUEST in the SELECTING state: options 50 (requested address) and 54 (server id).
pub fn request(xid: u32, mac: [u8; 6], offered: Ip, server: Ip, buf: &mut [u8; MSG_LEN]) -> usize {
    base(xid, mac, buf);
    let o = [
        53, 1, REQUEST, 50, 4, offered[0], offered[1], offered[2], offered[3], 54, 4, server[0], server[1], server[2],
        server[3], 55, 2, 1, 3, 61, 7, 1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 255,
    ];
    buf[240..240 + o.len()].copy_from_slice(&o);
    MSG_LEN
}

/// DHCPDECLINE (RFC 2131 section 3.1 item 5; section 4.4.1, table 5): the leased address is in
/// use by another host. `ciaddr` 0, requested address and server identifier, broadcast; the
/// client then starts again (spec A1.11, 0049).
pub fn decline(xid: u32, mac: [u8; 6], declined: Ip, server: Ip, buf: &mut [u8; MSG_LEN]) -> usize {
    base(xid, mac, buf);
    let o = [
        53, 1, DECLINE, 50, 4, declined[0], declined[1], declined[2], declined[3], 54, 4, server[0], server[1], server[2],
        server[3], 61, 7, 1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 255,
    ];
    buf[240..240 + o.len()].copy_from_slice(&o);
    MSG_LEN
}

/// DHCPREQUEST in the RENEWING or REBINDING state (RFC 2131 sections 4.3.2, 4.4.5): `ciaddr` is
/// the address in use; no options 50 and 54; no broadcast flag (the client has an address).
pub fn renew(xid: u32, mac: [u8; 6], ciaddr: Ip, buf: &mut [u8; MSG_LEN]) -> usize {
    base(xid, mac, buf);
    buf[10] = 0;
    buf[12..16].copy_from_slice(&ciaddr);
    let o = [53, 1, REQUEST, 55, 2, 1, 3, 61, 7, 1, mac[0], mac[1], mac[2], mac[3], mac[4], mac[5], 255];
    buf[240..240 + o.len()].copy_from_slice(&o);
    MSG_LEN
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reply {
    pub msg_type: u8,
    pub yiaddr: Ip,
    pub server: Ip,
    pub mask: Option<Ip>,
    pub router: Option<Ip>,
    pub lease_s: Option<u32>,
}

/// Parse a server reply for our `xid` and `mac`. Anything malformed or not for us is `None`.
pub fn parse(b: &[u8], xid: u32, mac: [u8; 6]) -> Option<Reply> {
    if b.len() < 241 || b[0] != 2 || b[1] != 1 || b[2] != 6 {
        return None;
    }
    if b[4..8] != xid.to_be_bytes() || b[28..34] != mac || b[236..240] != MAGIC {
        return None;
    }
    let ip4 = |s: &[u8]| -> Option<Ip> { s.try_into().ok() };
    let mut r = Reply { msg_type: 0, yiaddr: ip4(&b[16..20])?, server: [0; 4], mask: None, router: None, lease_s: None };
    let mut i = 240;
    while i < b.len() {
        let code = b[i];
        if code == 255 {
            break;
        }
        if code == 0 {
            i += 1;
            continue;
        }
        let len = *b.get(i + 1)? as usize;
        let v = b.get(i + 2..i + 2 + len)?;
        match (code, len) {
            (53, 1) => r.msg_type = v[0],
            (1, 4) => r.mask = ip4(v),
            (3, l) if l >= 4 => r.router = ip4(&v[..4]),
            (51, 4) => r.lease_s = Some(u32::from_be_bytes([v[0], v[1], v[2], v[3]])),
            (54, 4) => r.server = ip4(v)?,
            _ => {}
        }
        i += 2 + len;
    }
    if r.msg_type == 0 { None } else { Some(r) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decline_carries_the_declined_address_and_the_server() {
        let mac = [2, 0, 0, 0, 0x72, 0x50];
        let mut b = [0u8; MSG_LEN];
        decline(7, mac, [192, 168, 1, 77], [192, 168, 1, 1], &mut b);
        assert_eq!((b[0], &b[12..16], b[10]), (1, &[0u8; 4][..], 0x80), "request, ciaddr 0, broadcast");
        assert_eq!(&b[240..252], &[53, 1, DECLINE, 50, 4, 192, 168, 1, 77, 54, 4, 192]);
    }

    fn server_reply(t: u8, xid: u32, mac: [u8; 6]) -> Vec<u8> {
        let mut b = vec![0u8; 240];
        b[0] = 2;
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&xid.to_be_bytes());
        b[16..20].copy_from_slice(&[192, 168, 1, 77]);
        b[28..34].copy_from_slice(&mac);
        b[236..240].copy_from_slice(&MAGIC);
        b.extend_from_slice(&[53, 1, t, 1, 4, 255, 255, 255, 0, 3, 4, 192, 168, 1, 1, 51, 4, 0, 0, 0x0E, 0x10, 54, 4, 192, 168, 1, 1, 255]);
        b
    }

    #[test]
    fn discover_layout() {
        let mut b = [0u8; MSG_LEN];
        let mac = [2, 1, 2, 3, 4, 5];
        assert_eq!(discover(0x1234_5678, mac, &mut b), 300);
        assert_eq!(&b[0..3], &[1, 1, 6]);
        assert_eq!(&b[4..8], &[0x12, 0x34, 0x56, 0x78]);
        assert_eq!(b[10], 0x80);
        assert_eq!(&b[28..34], &mac);
        assert_eq!(&b[236..243], &[99, 130, 83, 99, 53, 1, 1]);
    }

    #[test]
    fn parse_offer_and_reject_foreign() {
        let mac = [2, 1, 2, 3, 4, 5];
        let r = parse(&server_reply(OFFER, 7, mac), 7, mac).unwrap();
        assert_eq!(r.msg_type, OFFER);
        assert_eq!(r.yiaddr, [192, 168, 1, 77]);
        assert_eq!(r.mask, Some([255, 255, 255, 0]));
        assert_eq!(r.router, Some([192, 168, 1, 1]));
        assert_eq!(r.lease_s, Some(3600));
        assert_eq!(r.server, [192, 168, 1, 1]);
        assert_eq!(parse(&server_reply(OFFER, 8, mac), 7, mac), None);
        assert_eq!(parse(&server_reply(OFFER, 7, [9; 6]), 7, mac), None);
        let mut t = server_reply(ACK, 7, mac);
        t.truncate(245); // option cut in the middle
        assert_eq!(parse(&t, 7, mac), None);
    }
}
