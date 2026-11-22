// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Host-side behaviour against a local socket (0070 point 1.1; part of T-A1-93 before the
//! reference-model run). The fake radio is in `common`.

mod common;

use std::net::UdpSocket;
use std::thread;
use std::time::Duration;

use common::*;

use g7250_proto::mgmt::payload::Identify;
use g7250_proto::mgmt::Command;
use g7250ctl::discover;
use g7250ctl::transport::Error;

#[test]
fn request_layout_is_table_a1_1() {
    let (addr, rx) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &[])]));
    let mut c = client();
    let first = c.peek_id();
    c.request(addr, Command::GetStats, &[], None).unwrap();
    let d = rx.recv().unwrap();
    assert_eq!(&d[0..4], b"G7MG");
    assert_eq!(&d[4..8], &[1, 0, 1, 0], "version 1.0, type request, no flags");
    assert_eq!(u32::from_be_bytes(id_of(&d)), first);
    assert_eq!(cmd_of(&d), [0x00, 0x02]);
    assert_eq!(&d[14..18], &[0, 0, 0, 0], "status 0 in requests, empty payload");
    assert_eq!(d.len(), 18);
}

#[test]
fn lost_replies_are_retransmitted_identically() {
    // The first three requests get no reply.
    let (addr, rx) = radio(Box::new(|n, d| if n < 3 { vec![] } else { vec![reply(id_of(d), cmd_of(d), 0, &[])] }));
    let mut c = client();
    let a = c.request(addr, Command::Identify, &[], None).unwrap();
    assert_eq!(a.sends, 4);
    let sent: Vec<_> = rx.try_iter().collect();
    assert_eq!(sent.len(), 4);
    assert!(sent.iter().all(|d| *d == sent[0]), "same id, command and payload each time");
}

#[test]
fn new_requests_get_new_ids() {
    let (addr, rx) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &[])]));
    let mut c = client();
    c.request(addr, Command::Identify, &[], None).unwrap();
    c.request(addr, Command::Identify, &[], None).unwrap();
    c.request(addr, Command::GetStats, &[], None).unwrap();
    let ids: Vec<u32> = rx.try_iter().map(|d| u32::from_be_bytes(id_of(&d))).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[1], ids[0].wrapping_add(1));
    assert_eq!(ids[2], ids[1].wrapping_add(1));
}

#[test]
fn ordinary_request_gives_up_after_five_retransmissions() {
    let (addr, rx) = radio(Box::new(|_, _| vec![]));
    let mut c = client();
    match c.request(addr, Command::Identify, &[], None) {
        Err(Error::NoReply { sends, .. }) => assert_eq!(sends, 6),
        other => panic!("{other:?}"),
    }
    thread::sleep(Duration::from_millis(50));
    assert_eq!(rx.try_iter().count(), 6);
}

#[test]
fn flash_work_request_keeps_retransmitting_for_its_window() {
    let (addr, rx) = radio(Box::new(|_, _| vec![]));
    let mut c = client();
    // 300 ms window at 30 ms: sends at 0, 30, ..., 300 ms = 11.
    let t = std::time::Instant::now();
    match c.request(addr, Command::UpdateBlock, &[0; 8], None) {
        Err(Error::NoReply { sends, .. }) => assert_eq!(sends, 11),
        other => panic!("{other:?}"),
    }
    assert!(t.elapsed() >= Duration::from_millis(300));
    thread::sleep(Duration::from_millis(50));
    let sent: Vec<_> = rx.try_iter().collect();
    assert_eq!(sent.len(), 11);
    assert!(sent.iter().all(|d| *d == sent[0]));
}

#[test]
fn slow_flash_reply_is_taken_and_extra_copies_ignored() {
    // As 0069 item 4: the reply comes late, then once more for each retransmission that arrived
    // during the work. The host uses the first; the copies must not answer the next request.
    let (addr, _rx) = radio(Box::new(|n, d| match n {
        0..=5 => vec![],
        6 => vec![reply(id_of(d), cmd_of(d), 0, &[]), reply(id_of(d), cmd_of(d), 0, &[])],
        _ => vec![reply(id_of(d), cmd_of(d), 0, &[0xAA])],
    }));
    let mut c = client();
    let a = c.request(addr, Command::SetConfig, &[], None).unwrap();
    assert_eq!(a.sends, 7, "past the 6 sends of an ordinary request");
    let b = c.request(addr, Command::GetConfig, &[], None).unwrap();
    assert_eq!(b.payload, vec![0xAA], "the stale copy of the SET_CONFIG reply was not taken");
}

#[test]
fn non_matching_datagrams_are_ignored() {
    let (addr, _rx) = radio(Box::new(|_, d| {
        let id = id_of(d);
        let mut other_id = id;
        other_id[3] ^= 1;
        let mut as_request = reply(id, cmd_of(d), 0, &[1]);
        as_request[6] = 1;
        let mut bad_magic = reply(id, cmd_of(d), 0, &[2]);
        bad_magic[0] = 0;
        let mut short = reply(id, cmd_of(d), 0, &[3, 3]);
        short.pop();
        vec![
            reply(other_id, cmd_of(d), 0, &[4]),
            reply(id, [0x00, 0x20], 0, &[5]),
            as_request,
            bad_magic,
            short,
            reply(id, cmd_of(d), 0, &[9]),
        ]
    }));
    let mut c = client();
    let a = c.request(addr, Command::GetStats, &[], None).unwrap();
    assert_eq!(a.payload, vec![9]);
    assert_eq!(a.sends, 1);
}

#[test]
fn reply_from_another_address_is_ignored() {
    let (addr, _rx) = radio(Box::new(|_, _| vec![]));
    let impostor = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut c = client();
    let me = c.socket().local_addr().unwrap();
    let id = c.peek_id().to_be_bytes();
    impostor.send_to(&reply(id, [0, 1], 0, &[]), me).unwrap();
    assert!(matches!(c.request(addr, Command::Identify, &[], None), Err(Error::NoReply { .. })));
}

#[test]
fn status_and_version_are_reported() {
    let (addr, _rx) = radio(Box::new(|_, d| {
        let mut r = reply(id_of(d), cmd_of(d), 18, &[]);
        r[4] = 2;
        r[5] = 3;
        vec![r]
    }));
    let mut c = client();
    let a = c.request(addr, Command::Identify, &[], None).unwrap();
    assert!(!a.ok());
    assert_eq!(a.status_code, 18);
    assert_eq!(a.version, (2, 3));
}

#[test]
fn identify_reply_decodes() {
    let (addr, _rx) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &identify_payload(0x55, 3_723_456))]));
    let mut c = client();
    let a = c.request(addr, Command::Identify, &[], None).unwrap();
    let i = Identify::decode(&a.payload).unwrap();
    assert_eq!(i.model, b"G7250");
    assert_eq!(i.mac[5], 0x55);
    assert_eq!((i.running.major, i.running.minor, i.running.build), (1, 2, 300));
    assert_eq!(i.running_slot, 2);
    assert_eq!(i.window, (true, 300));
    assert_eq!(i.owner, ([192, 168, 1, 10], 50000));
    let text = g7250ctl::render::identify_human(&i);
    assert!(text.contains("1.2.300 in slot B (good)"), "{text}");
    assert!(text.contains("first-secret window open, 300 s left"), "{text}");
    assert!(text.contains("192.168.1.10:50000"), "{text}");
    assert!(text.contains("slot A sectors 1-2, slot B sectors 3-4, store sectors 6-7"), "{text}");
    let json = g7250ctl::render::identify_json(&i);
    assert!(json.contains(r#""running_slot":"B""#), "{json}");
    assert!(json.contains(r#""owner":"192.168.1.10:50000""#), "{json}");
}

#[test]
fn stats_reply_decodes() {
    let mut p = Vec::new();
    for (n, v) in [("hp_rx", 7u64), ("mgmt_rx", 1 << 40)] {
        p.push(n.len() as u8);
        p.extend_from_slice(n.as_bytes());
        p.extend_from_slice(&v.to_be_bytes());
    }
    let (addr, _rx) = radio(Box::new(move |_, d| vec![reply(id_of(d), cmd_of(d), 0, &p)]));
    let mut c = client();
    let a = c.request(addr, Command::GetStats, &[], None).unwrap();
    let e: Vec<_> = g7250_proto::mgmt::payload::stats(&a.payload).map(|r| r.unwrap()).collect();
    assert_eq!(e, vec![(&b"hp_rx"[..], 7), (&b"mgmt_rx"[..], 1 << 40)]);
}

#[test]
fn discovery_collects_every_radio_once() {
    let (a1, rx1) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &identify_payload(1, 0))]));
    // The second radio answers every copy; it must still be listed once.
    let (a2, _rx2) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &identify_payload(2, 0)); 2]));
    let (_a3, _) = radio(Box::new(|_, _| vec![]));
    let mut c = client();
    let found = discover::identify_all(&mut c, &[a1, a2], Duration::from_millis(250)).unwrap();
    let mut from: Vec<_> = found.iter().map(|a| a.from).collect();
    from.sort();
    let mut want = vec![a1, a2];
    want.sort();
    assert_eq!(from, want);
    // One request id for the whole discovery, retransmitted at most 5 times (A1.4).
    thread::sleep(Duration::from_millis(50));
    let sent: Vec<_> = rx1.try_iter().collect();
    assert_eq!(sent.len(), 6);
    assert!(sent.iter().all(|d| *d == sent[0]));
}

#[test]
fn p2_discovery_reads_replies_from_the_radio_port_only() {
    // A fake P2 radio: 60-byte reply of Table B3-1, by hand.
    let (addr, rx) = radio(Box::new(|_, _| {
        let mut r = vec![0u8; 60];
        r[4] = 0x02;
        r[5..11].copy_from_slice(&[0x02, 1, 2, 3, 4, 5]);
        r[11] = 200;
        r[12] = 44;
        r[13] = 12;
        r[20] = 1;
        r[21] = 1;
        vec![r]
    }));
    let found = discover::p2_all(&[addr], addr.port(), Duration::from_millis(150), Duration::from_millis(30), 5).unwrap();
    assert_eq!(found.len(), 1);
    assert!(found[0].reply.is_g7250());
    assert_eq!(found[0].reply.fw_code, 12);
    let req = rx.recv().unwrap();
    assert_eq!(req.len(), 60);
    assert_eq!(req[4], 0x02);
    // Replies from any other port are not taken.
    let none = discover::p2_all(&[addr], addr.port().wrapping_add(1), Duration::from_millis(100), Duration::from_millis(30), 5).unwrap();
    assert!(none.is_empty());
}
