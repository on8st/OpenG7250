// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The M4 update against a local fake radio (0070 points 1.3, 1.4; T-A1-93 host side). The
//! fake radio checks HMACs, block CRCs and the image hash with its own code (hmac, sha2, a
//! CRC-32 written here), and package files are built here by hand from Table M-4.

mod common;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use g7250ctl::reads::Failure;
use g7250ctl::secret::Secret;
use g7250ctl::update::{self, Event, Options, PackageFile, Waits};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

const SECRET: [u8; 32] = [0x5A; 32];

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

/// CRC-32 [IEEE802.3], bitwise, written here.
fn crc32(d: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in d {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// A package file of Table M-4 by hand: version, slot, image.
fn package(version: [u8; 4], slot: u8, image: &[u8]) -> Vec<u8> {
    let mut f = b"G7PK".to_vec();
    f.extend([1, 1, 0x00, 0xC8, 0, 0]);
    f.extend(version);
    f.extend((image.len() as u32).to_be_bytes());
    f.extend(Sha256::digest(image));
    f.extend([0, 0, slot]);
    assert_eq!(f.len(), 53);
    f.extend_from_slice(image);
    f
}

fn image(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + 3) as u8).collect()
}

fn pf(name: &str, bytes: Vec<u8>) -> PackageFile {
    PackageFile::from_bytes(name, bytes).unwrap()
}

#[derive(Clone, Debug)]
struct Seen {
    cmd: u16,
    id: [u8; 4],
    auth_ok: bool,
    payload: Vec<u8>,
    raw: Vec<u8>,
}

struct Transfer {
    header: Vec<u8>,
    blocks: Vec<Option<Vec<u8>>>,
}

#[derive(Default)]
struct Radio {
    challenges: HashMap<SocketAddr, [u8; 16]>,
    next_ch: u8,
    running_slot: u8,
    running: [u8; 4],
    update_state: u8,
    uptime: u32,
    transfer: Option<Transfer>,
    /// The state stays "update in progress" after an abort until a restart.
    aborted: bool,
    // Behaviour.
    begin_status: u16,
    room_on_block: Vec<u32>,
    crc_on_block: Vec<u32>,
    write_fail_on_block: Option<u32>,
    drop_reply_on_block: Option<u32>,
    silent_from_block: Option<u32>,
    silent: bool,
    verify_fails: u32,
    /// After activation: IDENTIFYs in trial before the confirmation, or a rollback.
    trial_identifies: u32,
    trial_rolls_back: bool,
    /// Lose the reply to UPDATE_ACTIVATE; the reset then discards the stored replies.
    drop_activate_reply: bool,
    /// Restart while handling this block: the transfer, challenges and stored replies are
    /// lost, and so is the reply (the block was not taken).
    restart_on_block: Option<u32>,
    /// The uptime a reset starts from (0 by default). Above the uptime before the request, it
    /// models a request made a moment after a start: only the running slot shows the reset.
    reset_uptime: u32,
    old: ([u8; 4], u8),
    seen: Vec<Seen>,
}

type Stored = HashMap<(SocketAddr, [u8; 4]), Vec<u8>>;

fn identify(r: &mut Radio) -> Vec<u8> {
    r.uptime += 100;
    if r.update_state == 1 {
        if r.trial_identifies == 0 {
            if r.trial_rolls_back {
                r.running = r.old.0;
                r.running_slot = r.old.1;
                r.update_state = 2;
                r.uptime = 0;
            } else {
                r.update_state = 0;
            }
        } else {
            r.trial_identifies -= 1;
        }
    }
    let mut p = Vec::new();
    for (t, v) in [(1u8, b"G7250".to_vec()), (4, r.running.to_vec()), (10, vec![r.update_state]), (16, r.uptime.to_be_bytes().to_vec()), (23, vec![r.running_slot])] {
        p.push(t);
        p.push(v.len() as u8);
        p.extend(v);
    }
    p
}

fn handle(r: &mut Radio, stored: &mut Stored, from: SocketAddr, d: &[u8]) -> Option<Vec<u8>> {
    if r.silent {
        return None;
    }
    let id = id_of(d);
    let cmd = u16::from_be_bytes(cmd_of(d));
    let n = u16::from_be_bytes([d[16], d[17]]) as usize;
    let payload = d[18..18 + n].to_vec();
    let auth_ok = d[7] & 1 != 0 && r.challenges.get(&from).is_some_and(|ch| hmac(&SECRET, &[ch, &d[..18 + n]]) == d[18 + n..18 + n + 32]);
    r.seen.push(Seen { cmd, id, auth_ok, payload: payload.clone(), raw: d.to_vec() });
    if let Some(s) = stored.get(&(from, id)) {
        return Some(s.clone());
    }
    let reply_of = |status: u16, p: &[u8]| reply(id, cmd.to_be_bytes(), status, p);
    let needs_auth = !matches!(cmd, 0x01 | 0x10 | 0x45);
    if needs_auth {
        if !auth_ok {
            return Some(reply_of(33, &[]));
        }
        r.challenges.remove(&from);
    }
    let out = match cmd {
        0x01 => reply_of(0, &identify(r)),
        0x10 => {
            r.next_ch = r.next_ch.wrapping_add(1);
            let ch = [r.next_ch; 16];
            r.challenges.insert(from, ch);
            reply_of(0, &ch)
        }
        0x13 => reply_of(0, &[]),
        0x40 => {
            if r.begin_status != 0 {
                reply_of(r.begin_status, &[])
            } else if r.aborted || r.update_state == 1 {
                reply_of(58, &[])
            } else {
                let h = payload[1..].to_vec();
                let len = u32::from_be_bytes(h[14..18].try_into().unwrap()) as usize;
                let count = len.div_ceil(512);
                r.transfer = Some(Transfer { header: h, blocks: vec![None; count] });
                let mut p = 512u16.to_be_bytes().to_vec();
                p.extend((count as u32).to_be_bytes());
                p.push(0);
                reply_of(0, &p)
            }
        }
        0x41 => {
            let num = u32::from_be_bytes(payload[0..4].try_into().unwrap());
            if r.silent_from_block == Some(num) {
                r.silent = true;
                r.silent_from_block = None;
                return None;
            }
            if r.restart_on_block == Some(num) {
                r.restart_on_block = None;
                r.transfer = None;
                r.challenges.clear();
                stored.clear();
                r.uptime = r.reset_uptime;
                return None;
            }
            let Some(t) = r.transfer.as_mut() else { return Some(reply_of(54, &[])) };
            let crc = u32::from_be_bytes(payload[4..8].try_into().unwrap());
            let data = &payload[8..];
            if let Some(i) = r.room_on_block.iter().position(|b| *b == num) {
                r.room_on_block.remove(i);
                reply_of(64, &[])
            } else if let Some(i) = r.crc_on_block.iter().position(|b| *b == num) {
                r.crc_on_block.remove(i);
                reply_of(53, &[])
            } else if r.write_fail_on_block == Some(num) {
                reply_of(55, &[])
            } else if crc != crc32(data) || num as usize >= t.blocks.len() {
                reply_of(53, &[])
            } else {
                t.blocks[num as usize] = Some(data.to_vec());
                let received = t.blocks.iter().filter(|b| b.is_some()).count() as u32;
                let mut p = num.to_be_bytes().to_vec();
                p.extend(received.to_be_bytes());
                let out = reply_of(0, &p);
                if r.drop_reply_on_block == Some(num) {
                    r.drop_reply_on_block = None;
                    stored.insert((from, id), out);
                    return None;
                }
                out
            }
        }
        0x42 => match &r.transfer {
            None => reply_of(55, &[]),
            Some(t) => {
                if r.verify_fails > 0 {
                    r.verify_fails -= 1;
                    reply_of(55, &[])
                } else if t.blocks.iter().any(|b| b.is_none()) {
                    reply_of(55, &[])
                } else {
                    let img: Vec<u8> = t.blocks.iter().flat_map(|b| b.clone().unwrap()).collect();
                    let h = Sha256::digest(&img);
                    if h[..] != t.header[18..50] { reply_of(55, &[]) } else { reply_of(0, &h) }
                }
            }
        },
        0x43 => match r.transfer.take() {
            None => reply_of(55, &[]),
            Some(t) => {
                r.old = (r.running, r.running_slot);
                r.running = t.header[10..14].try_into().unwrap();
                r.running_slot = 3 - r.running_slot;
                r.update_state = 1;
                r.uptime = r.reset_uptime;
                if r.drop_activate_reply {
                    r.drop_activate_reply = false;
                    stored.clear();
                    r.challenges.clear();
                    return None;
                }
                reply_of(0, &[0])
            }
        },
        0x44 if r.update_state == 1 && r.transfer.is_none() => {
            // Recorded so that a test can see it; a host must not send it here.
            r.aborted = true;
            reply_of(0, &[1])
        }
        0x44 => {
            r.transfer = None;
            r.aborted = true;
            reply_of(0, &[1])
        }
        0x45 => {
            let (state, rec, count) = match &r.transfer {
                None => (0, 0, 0),
                Some(t) => (1u8, t.blocks.iter().filter(|b| b.is_some()).count() as u32, t.blocks.len() as u32),
            };
            let mut p = vec![state];
            p.extend(rec.to_be_bytes());
            p.extend(count.to_be_bytes());
            p.push(0);
            reply_of(0, &p)
        }
        0x46 => {
            r.old = (r.running, r.running_slot);
            r.running_slot = 3 - r.running_slot;
            r.running = [1, 0, 0, 1];
            r.update_state = 1;
            r.uptime = r.reset_uptime;
            reply_of(0, &[])
        }
        _ => reply_of(16, &[]),
    };
    stored.insert((from, id), out.clone());
    Some(out)
}

fn start(r: Radio) -> (SocketAddr, Arc<Mutex<Radio>>) {
    let st = Arc::new(Mutex::new(r));
    let st2 = st.clone();
    let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut stored = Stored::new();
        let mut buf = [0u8; 2048];
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        while let Ok((len, from)) = sock.recv_from(&mut buf) {
            if let Some(o) = handle(&mut st2.lock().unwrap(), &mut stored, from, &buf[..len]) {
                sock.send_to(&o, from).unwrap();
            }
        }
    });
    (addr, st)
}

fn radio(slot: u8) -> Radio {
    Radio { running_slot: slot, running: [1, 0, 0, 5], uptime: 100_000, ..Default::default() }
}

fn waits() -> Option<Waits> {
    Some(Waits { restart: Duration::from_secs(3), confirm: Duration::from_secs(3) })
}

fn opts() -> Options {
    Options { verify_retries: 1, wait: waits(), ..Default::default() }
}

/// Block numbers of the UPDATE_BLOCK requests seen, with their request ids.
fn blocks(st: &Arc<Mutex<Radio>>) -> Vec<(u32, [u8; 4])> {
    st.lock().unwrap().seen.iter().filter(|s| s.cmd == 0x41).map(|s| (u32::from_be_bytes(s.payload[0..4].try_into().unwrap()), s.id)).collect()
}

fn cmds(st: &Arc<Mutex<Radio>>) -> Vec<u16> {
    st.lock().unwrap().seen.iter().map(|s| s.cmd).collect()
}

fn pair(img: &[u8]) -> Vec<PackageFile> {
    vec![pf("a.g7pk", package([1, 1, 0, 9], 1, img)), pf("b.g7pk", package([1, 1, 0, 9], 2, img))]
}

#[test]
fn bad_package_files_are_refused_before_anything_is_sent() {
    let img = image(1300);
    let good = package([1, 1, 0, 9], 2, &img);
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    cases.push(("short", good[..40].to_vec()));
    cases.push(("length", good[..good.len() - 1].to_vec()));
    let mut m = good.clone();
    m[1] = b'X';
    cases.push(("magic", m));
    let mut v = good.clone();
    v[4] = 2;
    cases.push(("header version", v));
    let mut s = good.clone();
    s[51] = 1;
    cases.push(("signature", s));
    let mut h = good.clone();
    h[60] ^= 0xFF;
    cases.push(("hash", h));
    assert!(PackageFile::from_bytes("good", good).is_ok());
    let (addr, st) = start(radio(1));
    let dir = std::env::temp_dir().join(format!("g7250ctl-pkg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (what, bytes) in cases {
        let e = PackageFile::from_bytes(what, bytes.clone()).unwrap_err();
        assert!(e.contains("refused, nothing sent"), "{what}: {e}");
        // The binary sends nothing either.
        let p = dir.join(format!("{}.g7pk", what.replace(' ', "-")));
        std::fs::write(&p, &bytes).unwrap();
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_g7250ctl"))
            .args(["install", &addr.to_string()])
            .arg(&p)
            .env(g7250ctl::secret::ENV, "5a".repeat(32))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(g7250ctl::exit::ERROR as i32), "{what}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(st.lock().unwrap().seen.is_empty(), "nothing reached the radio");
}

#[test]
fn the_package_for_the_other_slot_is_sent() {
    let img = image(1300);
    for (running, want) in [(1u8, 2u8), (2, 1)] {
        for order in [pair(&img), pair(&img).into_iter().rev().collect()] {
            let (addr, st) = start(radio(running));
            let mut c = client();
            update::install(&mut c, addr, &Secret(SECRET), &order, Options { wait: None, ..opts() }, &mut |_| {}).unwrap();
            let g = st.lock().unwrap();
            let begin = g.seen.iter().find(|s| s.cmd == 0x40).unwrap();
            assert_eq!(begin.payload[0], 0, "no flags");
            assert_eq!(begin.payload[1 + 52], want, "running {running}: package for slot {want}");
        }
    }
    // Only the package for the running slot: refused, nothing but IDENTIFY sent.
    let (addr, st) = start(radio(2));
    let mut c = client();
    let f = update::install(&mut c, addr, &Secret(SECRET), &[pf("b", package([1, 1, 0, 9], 2, &img))], opts(), &mut |_| {}).unwrap_err();
    assert!(f.to_string().contains("give the package for slot A"), "{f}");
    assert_eq!(cmds(&st), vec![0x01]);
}

#[test]
fn full_install_blocks_in_order_each_signed_then_confirmed() {
    let img = image(512 * 3 + 100);
    let (addr, st) = start(Radio { drop_reply_on_block: Some(1), trial_identifies: 2, ..radio(1) });
    let mut c = client();
    let mut ev = Vec::new();
    let out = update::install(&mut c, addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap();
    assert_eq!(out, update::Outcome::Confirmed);
    let b = blocks(&st);
    assert_eq!(b.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 1, 2, 3], "block 1 retransmitted after the lost reply");
    assert_eq!(b[1].1, b[2].1, "the retransmission has the same id");
    let g = st.lock().unwrap();
    let ones: Vec<&Seen> = g.seen.iter().filter(|s| s.cmd == 0x41 && u32::from_be_bytes(s.payload[0..4].try_into().unwrap()) == 1).collect();
    assert_eq!(ones[0].raw, ones[1].raw, "identical datagram");
    // Block 3 is the short last one; block n = image bytes 512n..512n+511.
    let last = g.seen.iter().rfind(|s| s.cmd == 0x41).unwrap();
    assert_eq!(&last.payload[8..], &img[1536..]);
    // Every authenticated request was preceded by its own challenge.
    let seq: Vec<u16> = g.seen.iter().map(|s| s.cmd).filter(|c| *c != 0x01).collect();
    for (i, c) in seq.iter().enumerate() {
        if [0x40, 0x42, 0x43].contains(c) || (*c == 0x41 && seq[i - 1] != 0x41) {
            assert_eq!(seq[i - 1], 0x10, "challenge before {c:#x} at {i}: {seq:x?}");
        }
    }
    // The first copy of every authenticated request verifies; a repeat comes after its
    // challenge is spent and is answered from the stored reply (A1.4).
    let mut ids = std::collections::HashSet::new();
    assert!(g.seen.iter().filter(|s| s.cmd != 0x01 && s.cmd != 0x10 && ids.insert(s.id)).all(|s| s.auth_ok));
    assert_eq!((g.running_slot, g.running, g.update_state), (2, [1, 1, 0, 9], 0));
    assert!(ev.contains(&Event::Trial) && ev.contains(&Event::Confirmed));
}

#[test]
fn status_64_sends_the_same_block_again_as_a_new_request() {
    let img = image(512 * 4);
    let (addr, st) = start(Radio { room_on_block: vec![2], ..radio(1) });
    let mut c = client();
    let mut ev = Vec::new();
    update::install(&mut c, addr, &Secret(SECRET), &pair(&img), Options { wait: None, ..opts() }, &mut |e| ev.push(e)).unwrap();
    let b = blocks(&st);
    assert_eq!(b.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 2, 2, 3]);
    assert_ne!(b[2].1, b[3].1, "a new request id");
    assert!(ev.contains(&Event::RoomMade { n: 2 }));
    assert!(!cmds(&st).contains(&0x45), "no UPDATE_STATUS needed");
}

#[test]
fn a_second_64_for_the_same_block_ends_the_install_with_an_abort() {
    let img = image(512 * 4);
    let (addr, st) = start(Radio { room_on_block: vec![1, 1], ..radio(1) });
    let mut c = client();
    let mut ev = Vec::new();
    let f = update::install(&mut c, addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap_err();
    assert!(f.to_string().contains("status 64"), "{f}");
    assert!(cmds(&st).contains(&0x44), "UPDATE_ABORT sent");
    assert!(ev.contains(&Event::Aborted { restart_needed: true }));
}

#[test]
fn bad_crc_report_resends_the_block() {
    let img = image(512 * 2);
    let (addr, st) = start(Radio { crc_on_block: vec![0], ..radio(1) });
    let mut c = client();
    update::install(&mut c, addr, &Secret(SECRET), &pair(&img), Options { wait: None, ..opts() }, &mut |_| {}).unwrap();
    assert_eq!(blocks(&st).iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 0, 1]);
}

#[test]
fn failed_verify_sends_every_block_again_in_order_with_new_ids() {
    let img = image(512 * 3);
    let (addr, st) = start(Radio { verify_fails: 1, ..radio(2) });
    let mut c = client();
    let mut ev = Vec::new();
    update::install(&mut c, addr, &Secret(SECRET), &pair(&img), Options { wait: None, ..opts() }, &mut |e| ev.push(e)).unwrap();
    let b = blocks(&st);
    assert_eq!(b.iter().map(|x| x.0).collect::<Vec<_>>(), vec![0, 1, 2, 0, 1, 2]);
    let ids: std::collections::HashSet<_> = b.iter().map(|x| x.1).collect();
    assert_eq!(ids.len(), 6, "every block sent again as a new request");
    assert_eq!(cmds(&st).iter().filter(|c| **c == 0x42).count(), 2);
    assert!(ev.contains(&Event::VerifyFailed { round: 1 }));
    // Without retries left: reported and aborted.
    let (addr, st) = start(Radio { verify_fails: 5, ..radio(2) });
    let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), Options { wait: None, ..opts() }, &mut |_| {}).unwrap_err();
    assert!(f.to_string().contains("UpdateVerify with status 55"), "{f}");
    assert!(cmds(&st).contains(&0x44));
}

#[test]
fn refused_begin_is_reported_without_an_abort() {
    let img = image(600);
    for status in [52u16, 63, 51, 58] {
        let (addr, st) = start(Radio { begin_status: status, ..radio(1) });
        let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |_| {}).unwrap_err();
        assert!(matches!(&f, Failure::Status(a, _) if a.status_code == status), "{f}");
        let c = cmds(&st);
        assert!(!c.contains(&0x41) && !c.contains(&0x44), "{status}: {c:x?}");
        assert_eq!(c.last(), Some(&0x13), "{status}: write session ended");
    }
}

#[test]
fn downgrade_flag_is_sent_when_asked() {
    let img = image(600);
    let (addr, st) = start(radio(1));
    update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), Options { downgrade: true, wait: None, ..opts() }, &mut |_| {}).unwrap();
    assert_eq!(st.lock().unwrap().seen.iter().find(|s| s.cmd == 0x40).unwrap().payload[0], 0x02);
}

#[test]
fn write_failure_on_a_block_aborts() {
    let img = image(512 * 3);
    let (addr, st) = start(Radio { write_fail_on_block: Some(1), ..radio(1) });
    let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |_| {}).unwrap_err();
    assert!(f.to_string().contains("status 55"), "{f}");
    assert!(cmds(&st).contains(&0x44));
}

#[test]
fn a_radio_that_restarts_during_the_transfer_is_not_sent_an_abort() {
    // The reply to block 2 is lost in the restart; the retransmission meets a spent challenge
    // and gets 33. UPDATE_STATUS then shows no transfer, so no UPDATE_ABORT is sent.
    let img = image(512 * 4);
    let (addr, st) = start(Radio { restart_on_block: Some(2), ..radio(1) });
    let mut ev = Vec::new();
    let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 33), "{f}");
    let c = cmds(&st);
    assert!(!c.contains(&0x44), "no abort: {c:x?}");
    assert_eq!(c.last(), Some(&0x45), "UPDATE_STATUS decided it: {c:x?}");
    assert!(ev.contains(&Event::NoTransfer { state: 0 }), "{ev:?}");
    assert!(!st.lock().unwrap().aborted);
}

#[test]
fn a_failure_with_a_transfer_in_progress_reads_update_status_then_aborts() {
    let img = image(512 * 3);
    let (addr, st) = start(Radio { write_fail_on_block: Some(1), ..radio(1) });
    let mut ev = Vec::new();
    update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap_err();
    let c = cmds(&st);
    let at = c.iter().position(|x| *x == 0x44).expect("aborted");
    assert_eq!(c[at - 2..at], [0x45, 0x10], "UPDATE_STATUS, then the challenge for the abort: {c:x?}");
    assert!(ev.contains(&Event::Aborted { restart_needed: true }), "{ev:?}");
}

#[test]
fn an_activation_just_after_a_start_is_seen_by_the_running_slot() {
    // The radio started a moment ago (uptime 0) and its reset starts from a higher uptime, so
    // the uptime never falls below the one read before; the running slot changed.
    let img = image(512 * 2);
    let (addr, st) = start(Radio { uptime: 0, reset_uptime: 5_000, drop_activate_reply: true, ..radio(1) });
    let t = std::time::Instant::now();
    update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |_| {}).unwrap();
    assert!(t.elapsed() < Duration::from_secs(10), "no wait for the 30 s probe: {:?}", t.elapsed());
    let r = st.lock().unwrap();
    assert_eq!((r.running_slot, r.update_state, r.aborted), (2, 0, false));
}

#[test]
fn a_rollback_just_after_a_start_is_seen_by_the_running_slot() {
    let (addr, st) = start(Radio { uptime: 0, reset_uptime: 5_000, ..radio(2) });
    assert_eq!(update::rollback(&mut client(), addr, &Secret(SECRET), waits(), &mut |_| {}).unwrap(), 1);
    assert_eq!(st.lock().unwrap().update_state, 0);
}

#[test]
fn a_silent_radio_is_left_for_resume_and_resume_continues() {
    let img = image(512 * 5);
    let (addr, st) = start(Radio { silent_from_block: Some(3), ..radio(1) });
    let mut ev = Vec::new();
    let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap_err();
    assert_eq!(f.exit_code(), g7250ctl::exit::NO_REPLY);
    assert!(ev.contains(&Event::LeftForResume));
    assert!(!cmds(&st).contains(&0x44), "no abort");
    st.lock().unwrap().silent = false;
    let mut ev = Vec::new();
    update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), Options { resume: true, wait: None, ..opts() }, &mut |e| ev.push(e)).unwrap();
    assert!(ev.contains(&Event::Resumed { from: 3, blocks: 5 }), "{ev:?}");
    let b: Vec<u32> = blocks(&st).iter().map(|x| x.0).collect();
    assert_eq!(b, vec![0, 1, 2, 3, 3, 4], "blocks 3 and 4 after the resume");
    assert_eq!(cmds(&st).iter().filter(|c| **c == 0x40).count(), 1, "no second UPDATE_BEGIN");
}

#[test]
fn a_lost_activation_reply_is_resolved_with_identify_and_never_aborted() {
    // Found against the reference model: the reply to UPDATE_ACTIVATE is lost, the reset
    // discards the stored reply, the retransmission gets 33 (challenge spent). The radio did
    // restart into the new image; the host must see that and must not send UPDATE_ABORT.
    let img = image(900);
    // A radio that has just restarted (small uptime), as in the reference-model run: the
    // restart is judged against the uptime read just before the activation, not at the start.
    let (addr, st) = start(Radio { drop_activate_reply: true, trial_identifies: 1, uptime: 200, ..radio(1) });
    let mut ev = Vec::new();
    let out = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |e| ev.push(e)).unwrap();
    assert_eq!(out, update::Outcome::Confirmed);
    let c = cmds(&st);
    assert!(!c.contains(&0x44), "no UPDATE_ABORT: {c:x?}");
    assert_eq!(c.iter().filter(|x| **x == 0x43).count(), 2, "UPDATE_ACTIVATE sent and retransmitted once");
    let g = st.lock().unwrap();
    assert_eq!((g.running_slot, g.update_state, g.aborted), (2, 0, false));
}

#[test]
fn a_trial_that_rolls_back_is_reported() {
    let img = image(700);
    let (addr, _st) = start(Radio { trial_identifies: 1, trial_rolls_back: true, ..radio(1) });
    let f = update::install(&mut client(), addr, &Secret(SECRET), &pair(&img), opts(), &mut |_| {}).unwrap_err();
    assert!(f.to_string().contains("rolled back"), "{f}");
}

#[test]
fn rollback_starts_the_other_slot_and_waits_for_the_confirmation() {
    let (addr, st) = start(Radio { trial_identifies: 1, ..radio(1) });
    let mut ev = Vec::new();
    let slot = update::rollback(&mut client(), addr, &Secret(SECRET), waits(), &mut |e| ev.push(e)).unwrap();
    assert_eq!(slot, 2);
    assert_eq!(st.lock().unwrap().running_slot, 2);
    assert!(ev.contains(&Event::Confirmed));
    assert!(!cmds(&st).contains(&0x13), "the reset ends the session; no END_WRITE_SESSION");
}

#[test]
fn abort_reports_restart_needed() {
    let (addr, st) = start(radio(1));
    assert!(update::abort(&mut client(), addr, &Secret(SECRET)).unwrap());
    assert_eq!(cmds(&st), vec![0x10, 0x44, 0x10, 0x13]);
}

#[test]
fn update_status_is_read_without_authentication() {
    let (addr, st) = start(radio(1));
    let s = update::update_status(&mut client(), addr).unwrap();
    assert_eq!((s.state, s.received, s.count), (0, 0, 0));
    assert_eq!(cmds(&st), vec![0x45]);
    assert!(st.lock().unwrap().seen[0].raw[7] & 1 == 0);
}
