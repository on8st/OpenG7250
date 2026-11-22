// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! M6 against a local fake register port: the capability record (B4.4 items 1 and 4, B8.1,
//! B8.5). Records and packets are built here by hand from Table B4-1 and B8.1.

use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use g7250_proto::caprec_host::RecordError;
use g7250ctl::caps::{self, CapsError, Skipped};

/// A record of `len` registers: magic, version 1.`minor`, length, then zeros except where set.
fn record(len: usize, minor: u32) -> Vec<u32> {
    let mut r = vec![0u32; len];
    r[0] = 0x4737_3235;
    r[1] = 0x0001_0000 | minor;
    r[2] = len as u32;
    r[4] = 0x4737_3235; // "G725"
    r[5] = 0x302D_5032; // "0-P2"
    r
}

fn packet(regs: &[u32], echo: bool) -> Vec<u8> {
    let mut p = vec![0, 0, 0, 1];
    if echo {
        p.extend([0x01, 0x00, 0, 0, 0, 1]);
    }
    for (i, v) in regs.iter().enumerate() {
        // The pair (0x0000, 0) would be padding; register 0 is the magic, never 0.
        p.extend((i as u16).to_be_bytes());
        p.extend(v.to_be_bytes());
    }
    p
}

#[derive(Default)]
struct Fake {
    /// Requests to leave unanswered first.
    ignore: u32,
    /// What to send for an answered request (the default: a whole v1.1 record).
    answer: Option<Vec<u8>>,
    /// Sent before the answer, from the register port: a push without the echo.
    push_first: bool,
    requests: u32,
    sources: Vec<SocketAddr>,
}

fn start(f: Fake) -> (SocketAddr, Arc<Mutex<Fake>>) {
    let st = Arc::new(Mutex::new(f));
    let st2 = st.clone();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 1500];
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        while let Ok((n, from)) = sock.recv_from(&mut buf) {
            let mut f = st2.lock().unwrap();
            assert_eq!(&buf[..n], &[0, 0, 0, 0, 0x01, 0x00, 0, 0, 0, 1], "CAP_REQUEST = 1, one pair");
            f.requests += 1;
            f.sources.push(from);
            if f.ignore > 0 {
                f.ignore -= 1;
                continue;
            }
            if f.push_first {
                sock.send_to(&packet(&record(0xCF, 1), false), from).unwrap();
            }
            let a = f.answer.clone().unwrap_or_else(|| packet(&record(0xCF, 1), true));
            sock.send_to(&a, from).unwrap();
        }
    });
    (addr, st)
}

fn client() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").unwrap()
}

const FAST: Duration = Duration::from_millis(80);

#[test]
fn the_answer_with_the_echo_is_the_record() {
    let (addr, st) = start(Fake { push_first: true, ..Default::default() });
    let mut sk = Skipped::default();
    let r = caps::request(&client(), addr, FAST, 5, &mut sk).unwrap();
    assert_eq!((r.len(), r.version()), (0xCF, (1, 1)));
    assert_eq!(sk, Skipped { other_source: 0, not_an_answer: 1 }, "the push without the echo is skipped");
    assert_eq!(st.lock().unwrap().requests, 1);
}

#[test]
fn a_record_from_another_address_or_port_is_not_taken() {
    let (addr, _st) = start(Fake { ignore: 1, ..Default::default() });
    let c = client();
    // Another socket sends a whole answer to the tool first: wrong source port.
    let intruder = UdpSocket::bind("127.0.0.1:0").unwrap();
    let me = c.local_addr().unwrap();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        intruder.send_to(&packet(&record(0xCF, 1), true), me).unwrap();
    });
    let mut sk = Skipped::default();
    caps::request(&c, addr, FAST, 5, &mut sk).unwrap();
    assert_eq!(sk.other_source, 1);
}

#[test]
fn no_answer_is_asked_again_and_reported_after_five() {
    let (addr, st) = start(Fake { ignore: 2, ..Default::default() });
    caps::request(&client(), addr, FAST, 5, &mut Skipped::default()).unwrap();
    assert_eq!(st.lock().unwrap().requests, 3, "two unanswered, the third answered");

    let (addr, st) = start(Fake { ignore: 100, ..Default::default() });
    let e = caps::request(&client(), addr, FAST, 5, &mut Skipped::default()).unwrap_err();
    assert!(matches!(e, CapsError::NoAnswer { tries: 5, .. }), "{e}");
    assert_eq!(st.lock().unwrap().requests, 5);
    assert!(e.to_string().contains("5 requests"));
}

#[test]
fn an_answer_without_the_whole_record_is_incomplete_never_a_record() {
    let mut r = record(0xCF, 1);
    r.truncate(100); // cap_length still says 207
    let (addr, _st) = start(Fake { answer: Some(packet(&r, true)), ..Default::default() });
    let e = caps::request(&client(), addr, FAST, 5, &mut Skipped::default()).unwrap_err();
    assert!(matches!(e, CapsError::Bad { why: RecordError::Incomplete { have: 100, need: 0xCF }, .. }), "{e}");
    assert!(e.to_string().contains("incomplete"));
}

#[test]
fn an_unknown_major_version_is_not_read() {
    let mut r = record(0xCF, 1);
    r[1] = 0x0002_0000;
    let (addr, _st) = start(Fake { answer: Some(packet(&r, true)), ..Default::default() });
    let e = caps::request(&client(), addr, FAST, 5, &mut Skipped::default()).unwrap_err();
    assert!(e.to_string().contains("newer than this tool"), "{e}");
}

#[test]
fn the_tool_never_sends_from_port_7250() {
    // B8.5: the radio drops a packet from source port 7250. If the port is free here, a socket
    // bound to it is refused before anything is sent.
    if let Ok(s) = UdpSocket::bind("127.0.0.1:7250") {
        let (addr, st) = start(Fake::default());
        assert!(caps::request(&s, addr, FAST, 1, &mut Skipped::default()).is_err());
        assert_eq!(st.lock().unwrap().requests, 0);
    }
    assert_ne!(caps::socket().unwrap().local_addr().unwrap().port(), 7250);
}

fn parse(regs: &[u32]) -> g7250_proto::caprec_host::CapRecord {
    g7250_proto::caprec_host::parse_answer(&packet(regs, true)).unwrap()
}

#[test]
fn rendering_names_absent_fields_and_bits_not_defined() {
    // v1.0: the v1.1 fields are absent (B4.10 rule 2).
    let mut r = record(0xB9, 0);
    r[0x0C] = 0x0000_0101; // streams: RX IQ, and bit 8, which is not defined
    r[0x13] = 1;
    r[0x14] = 500_000;
    r[0x15] = 30_000_000;
    r[0x21] = 1;
    r[0x22] = 7_000_000;
    r[0x23] = 7_200_000;
    r[0x24] = (1000 << 16) | 250;
    let h = caps::human(&parse(&r), false);
    assert!(h.contains("product_name    G7250-P2"), "{h}");
    assert!(h.contains("RX IQ; bits not defined: 0x00000100"), "{h}");
    assert!(h.contains("rx_range[0]     0.500000 MHz to 30.000000 MHz"), "{h}");
    assert!(h.contains("max 100.0 W SSB/CW/FM, 25.0 W AM"), "{h}");
    assert!(h.contains("status_fill     absent (beyond cap_length)"), "{h}");
    assert!(h.contains("coupler         absent"), "{h}");

    // A coupler value the spec does not define, and a later minor version with more registers.
    let mut r = record(0xD1, 3);
    r[0xBB] = 5;
    r[0xD0] = 0xABCD;
    let h = caps::human(&parse(&r), false);
    assert!(h.contains("5 (not defined)"), "{h}");
    assert!(h.contains("later minor version, not defined here"), "{h}");
    assert!(h.contains("0xd0  0x0000abcd"), "{h}");
    let j = caps::json(&parse(&r));
    assert!(j.starts_with("{\"product_name\":\"G7250-P2\",\"version\":\"1.3\",\"length\":209"), "{j}");
    assert!(j.contains("\"coupler\":5"), "{j}");
}
