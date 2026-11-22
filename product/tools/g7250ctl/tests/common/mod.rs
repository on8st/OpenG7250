// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! A fake radio for the socket tests. Replies are built by hand from Table A1-1, not with the
//! protocol crate, so the tests do not share the crate's reading of the header.
#![allow(dead_code)]

use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use g7250ctl::transport::{Client, Policy};

/// Short timings so the tests run fast; same rules as the defaults.
pub fn fast() -> Policy {
    Policy { interval: Duration::from_millis(30), retransmits: 5, flash_window: Duration::from_millis(300) }
}

/// Table A1-1 reply header, by hand.
pub fn reply(id: [u8; 4], cmd: [u8; 2], status: u16, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![0x47, 0x37, 0x4D, 0x47, 1, 0, 2, 0];
    v.extend_from_slice(&id);
    v.extend_from_slice(&cmd);
    v.extend_from_slice(&status.to_be_bytes());
    v.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    v.extend_from_slice(payload);
    v
}

/// What the fake radio sends back for the n-th datagram it receives (0-based).
pub type Behaviour = Box<dyn FnMut(usize, &[u8]) -> Vec<Vec<u8>> + Send>;

/// Start a fake radio on 127.0.0.1; every received datagram is reported on the channel.
pub fn radio(mut behaviour: Behaviour) -> (SocketAddr, mpsc::Receiver<Vec<u8>>) {
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 2048];
        let mut n = 0;
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        while let Ok((len, from)) = sock.recv_from(&mut buf) {
            let d = buf[..len].to_vec();
            for r in behaviour(n, &d) {
                sock.send_to(&r, from).unwrap();
            }
            n += 1;
            // The receiver may be dropped by a test that does not look at the requests; the
            // radio goes on until 5 s without a datagram.
            let _ = tx.send(d);
        }
    });
    (addr, rx)
}

pub fn id_of(d: &[u8]) -> [u8; 4] {
    d[8..12].try_into().unwrap()
}
pub fn cmd_of(d: &[u8]) -> [u8; 2] {
    d[12..14].try_into().unwrap()
}
pub fn payload_of(d: &[u8]) -> &[u8] {
    &d[18..18 + u16::from_be_bytes([d[16], d[17]]) as usize]
}

pub fn client() -> Client {
    let mut c = Client::with_socket(UdpSocket::bind("127.0.0.1:0").unwrap()).unwrap();
    c.policy = fast();
    c
}

/// Table M-2, by hand: tag, length, value.
pub fn identify_payload(mac_last: u8, uptime_ms: u32) -> Vec<u8> {
    let mut p = Vec::new();
    let mut tlv = |t: u8, v: &[u8]| {
        p.push(t);
        p.push(v.len() as u8);
        p.extend_from_slice(v);
    };
    tlv(1, b"G7250");
    tlv(2, &[0x02, 0, 0, 0, 0, mac_last]);
    tlv(4, &[1, 2, 0x01, 0x2C]);
    tlv(5, &[0, 0, 0, 0]);
    tlv(9, &[1, 0]);
    tlv(12, &[1, 0x01, 0x2C]);
    tlv(13, &[1]);
    tlv(14, &[192, 168, 1, 10, 0xC3, 0x50]);
    tlv(16, &uptime_ms.to_be_bytes());
    tlv(18, &[1, 0, 44, 1, 1]);
    tlv(23, &[2]);
    tlv(25, &[1, 2, 3, 2, 6, 2]);
    tlv(200, &[0xEE; 3]); // unknown tag: skipped (Annex M)
    p
}

/// One `READ_LOG` entry of Table M-1, by hand.
pub fn log_entry(seq: u32, uptime: u32, text: &str) -> Vec<u8> {
    let mut v = seq.to_be_bytes().to_vec();
    v.extend_from_slice(&uptime.to_be_bytes());
    v.push(text.len() as u8);
    v.extend_from_slice(text.as_bytes());
    v
}
