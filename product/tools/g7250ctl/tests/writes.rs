// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The M3 authenticated commands against a local fake radio (0070 point 1.2; T-A1-93 host
//! side). The fake radio checks HMAC trailers, unmasks `CHANGE_SECRET` and keeps the write
//! session with its own code (hmac, sha2), not with the protocol crate.

mod common;

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use common::*;
use g7250_proto::mgmt::payload::Config;
use g7250ctl::reads::Failure;
use g7250ctl::secret::Secret;
use g7250ctl::writes::{self, Whereabouts};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

const OLD: [u8; 32] = [0x11; 32];
const NEW: [u8; 32] = [0x22; 32];

/// One request as the fake radio saw it.
#[derive(Clone, Debug)]
struct Seen {
    from: SocketAddr,
    cmd: u16,
    /// Whether the HMAC trailer verified.
    auth_ok: bool,
    raw: Vec<u8>,
}

#[derive(Default)]
struct State {
    secret: Option<[u8; 32]>,
    window_open: bool,
    challenges: HashMap<SocketAddr, [u8; 16]>,
    next_challenge: u8,
    holder: Option<SocketAddr>,
    /// Table M-3 items as raw TLVs, keyed by (tag, band index for tag 8).
    config: BTreeMap<(u8, u8), Vec<u8>>,
    /// Apply SET_CONFIG to the reply only, not to the store (a radio that lies).
    config_lies: bool,
    calibration: Option<Vec<u8>>,
    uptime: u32,
    /// Status for RESTART / FACTORY_RESET instead of 0.
    reset_status: u16,
    /// Drop the reply to the first datagram of this command.
    drop_first: Option<u16>,
    seen: Vec<Seen>,
}

fn config_payload(c: &BTreeMap<(u8, u8), Vec<u8>>) -> Vec<u8> {
    let mut p = Vec::new();
    for ((tag, _), v) in c {
        p.push(*tag);
        p.push(v.len() as u8);
        p.extend_from_slice(v);
    }
    p
}

/// Stored replies by (from, request id), as A1.4 asks of the radio.
type Stored = HashMap<(SocketAddr, [u8; 4]), Vec<u8>>;

fn handle(st: &mut State, stored: &mut Stored, from: SocketAddr, d: &[u8]) -> Vec<Vec<u8>> {
    let id = id_of(d);
    let cmd = u16::from_be_bytes(cmd_of(d));
    let flags = d[7];
    let n = u16::from_be_bytes([d[16], d[17]]) as usize;
    let payload = d[18..18 + n].to_vec();
    let mut auth_ok = false;
    if flags & 1 != 0 {
        let trailer = &d[18 + n..18 + n + 32];
        if let (Some(secret), Some(ch)) = (st.secret, st.challenges.get(&from).copied()) {
            auth_ok = hmac(&secret, &[&ch, &d[..18 + n]]) == trailer;
        }
    }
    st.seen.push(Seen { from, cmd, auth_ok, raw: d.to_vec() });
    if let Some(r) = stored.get(&(from, id)) {
        return vec![r.clone()];
    }
    let needs_auth = !matches!(cmd, 0x01 | 0x02 | 0x10 | 0x11 | 0x20 | 0x30 | 0x45 | 0x60);
    let r = |status: u16, p: &[u8]| reply(id, cmd.to_be_bytes(), status, p);
    let out = if needs_auth && flags & 1 == 0 {
        r(32, &[])
    } else if needs_auth && !auth_ok {
        st.challenges.remove(&from);
        r(33, &[])
    } else if needs_auth && cmd != 0x24 && st.holder.is_some_and(|h| h != from) {
        st.challenges.remove(&from);
        r(35, &[])
    } else {
        if needs_auth {
            st.challenges.remove(&from);
            if !matches!(cmd, 0x13 | 0x24) {
                st.holder = Some(from);
            }
        }
        match cmd {
            0x01 => {
                st.uptime += 100;
                let mut p = identify_payload(1, st.uptime);
                p.extend([11, 1, st.secret.is_some() as u8]);
                r(0, &p)
            }
            0x10 => {
                st.next_challenge += 1;
                let ch = [st.next_challenge; 16];
                st.challenges.insert(from, ch);
                r(0, &ch)
            }
            0x11 => {
                if !st.window_open || st.secret.is_some() {
                    r(36, &[])
                } else {
                    st.secret = Some(payload.clone().try_into().unwrap());
                    st.window_open = false;
                    r(0, &[])
                }
            }
            0x12 => {
                let ch = [st.next_challenge; 16];
                let pad = hmac(&st.secret.unwrap(), &[b"G7NS", &ch]);
                let new: Vec<u8> = payload.iter().zip(pad).map(|(a, b)| a ^ b).collect();
                st.secret = Some(new.try_into().unwrap());
                r(0, &[])
            }
            0x13 => {
                st.holder = None;
                r(0, &[])
            }
            0x20 => r(0, &config_payload(&st.config)),
            0x21 => {
                let mut next = st.config.clone();
                let mut p = &payload[..];
                while p.len() >= 2 {
                    let (t, l) = (p[0], p[1] as usize);
                    let v = p[2..2 + l].to_vec();
                    next.insert((t, if t == 8 { v[0] } else { 0 }), v);
                    p = &p[2 + l..];
                }
                let out = r(0, &config_payload(&next));
                if !st.config_lies {
                    st.config = next;
                }
                out
            }
            0x22 | 0x24 => {
                if st.reset_status != 0 {
                    r(st.reset_status, &[])
                } else {
                    st.uptime = 0;
                    st.holder = None;
                    if cmd == 0x22 {
                        st.secret = None;
                        st.window_open = true;
                    }
                    r(0, &[])
                }
            }
            0x23 | 0x31 => r(0, &[]),
            0x60 => match &st.calibration {
                None => r(0, &[0; 32]),
                Some(set) => {
                    let mut p = Sha256::digest(set).to_vec();
                    p.extend_from_slice(set);
                    r(0, &p)
                }
            },
            0x61 => {
                st.calibration = Some(payload.clone());
                let mut p = Sha256::digest(&payload).to_vec();
                p.push(1);
                r(0, &p)
            }
            0x62 => {
                st.calibration = None;
                r(0, &[])
            }
            _ => r(16, &[]),
        }
    };
    stored.insert((from, id), out.clone());
    if st.drop_first == Some(cmd) {
        st.drop_first = None;
        return vec![];
    }
    vec![out]
}

fn start(state: State) -> (SocketAddr, Arc<Mutex<State>>) {
    let st = Arc::new(Mutex::new(state));
    let st2 = st.clone();
    let mut stored = Stored::new();
    // Its own socket loop: the handler needs the sender's address (challenges, the session and
    // stored replies are per address and port).
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    let addr = sock.local_addr().unwrap();
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        sock.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        while let Ok((len, from)) = sock.recv_from(&mut buf) {
            let out = handle(&mut st2.lock().unwrap(), &mut stored, from, &buf[..len]);
            for o in out {
                sock.send_to(&o, from).unwrap();
            }
        }
    });
    (addr, st)
}

fn secret(b: [u8; 32]) -> Secret {
    Secret(b)
}

fn cmds(st: &Arc<Mutex<State>>) -> Vec<u16> {
    st.lock().unwrap().seen.iter().map(|s| s.cmd).collect()
}

#[test]
fn authenticated_write_layout_session_end_and_retransmission() {
    let (addr, st) = start(State { secret: Some(OLD), drop_first: Some(0x21), ..Default::default() });
    let mut c = client();
    let change = Config { default_level: Some(1), region: Some(0), ..Default::default() };
    let r = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap();
    assert_eq!(r.whereabouts, Whereabouts::At(addr));
    let g = st.lock().unwrap();
    let seq: Vec<u16> = g.seen.iter().map(|s| s.cmd).collect();
    // challenge, SET_CONFIG (reply lost), SET_CONFIG again, GET_CONFIG read-back, then the
    // END_WRITE_SESSION with its own challenge.
    assert_eq!(seq, vec![0x10, 0x21, 0x21, 0x10, 0x13, 0x20], "{seq:x?}");
    let sets: Vec<&Seen> = g.seen.iter().filter(|s| s.cmd == 0x21).collect();
    assert_eq!(sets[0].raw, sets[1].raw, "the retransmission is the identical signed datagram");
    assert!(sets[0].auth_ok);
    assert_eq!(sets[0].raw[7], 1, "flag bit 0: authenticated");
    assert_eq!(sets[0].raw.len(), 18 + u16::from_be_bytes([sets[0].raw[16], sets[0].raw[17]]) as usize + 32);
    assert!(g.seen.iter().all(|s| s.from == g.seen[0].from), "challenge and request from one address and port");
    assert_eq!(g.holder, None, "the write session was ended");
}

#[test]
fn change_secret_is_masked_and_checked_with_the_new_secret() {
    let (addr, st) = start(State { secret: Some(OLD), ..Default::default() });
    let mut c = client();
    writes::change_secret(&mut c, addr, &secret(OLD), &secret(NEW)).unwrap();
    let g = st.lock().unwrap();
    assert_eq!(g.secret, Some(NEW));
    let change = g.seen.iter().find(|s| s.cmd == 0x12).unwrap();
    assert!(change.auth_ok, "signed with the old secret");
    let end = g.seen.iter().rfind(|s| s.cmd == 0x13).unwrap();
    assert!(end.auth_ok, "END_WRITE_SESSION verified with the new secret");
    for s in &g.seen {
        assert!(!s.raw.windows(32).any(|w| w == OLD || w == NEW), "no secret in the clear on the wire");
    }
    assert_eq!(g.holder, None);
}

#[test]
fn wrong_secret_gets_33_once_and_is_not_retried() {
    let (addr, st) = start(State { secret: Some(OLD), ..Default::default() });
    let mut c = client();
    let f = writes::clear_log(&mut c, addr, &secret(NEW)).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 33), "{f:?}");
    assert_eq!(f.exit_code(), g7250ctl::exit::STATUS);
    assert_eq!(cmds(&st), vec![0x10, 0x31], "one challenge, one request, no END_WRITE_SESSION");
}

#[test]
fn another_writer_gets_35_and_no_session_end_is_sent() {
    let other: SocketAddr = "127.0.0.1:9".parse().unwrap();
    let (addr, st) = start(State { secret: Some(OLD), holder: Some(other), ..Default::default() });
    let mut c = client();
    let f = writes::switch_level(&mut c, addr, &secret(OLD), 1).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 35));
    assert_eq!(cmds(&st), vec![0x10, 0x23]);
    assert_eq!(st.lock().unwrap().holder, Some(other));
}

#[test]
fn restart_is_allowed_while_another_host_writes_and_sends_no_session_end() {
    let other: SocketAddr = "127.0.0.1:9".parse().unwrap();
    let (addr, st) = start(State { secret: Some(OLD), holder: Some(other), uptime: 50_000, ..Default::default() });
    let mut c = client();
    let before = writes::reset(&mut c, addr, &secret(OLD), g7250_proto::mgmt::Command::Restart).unwrap().expect("answered");
    assert_eq!(cmds(&st), vec![0x01, 0x10, 0x24], "IDENTIFY for the uptime, then the signed RESTART");
    assert_eq!((before.uptime_ms, before.slot_changes), (50_100, false), "a RESTART keeps the slot: only the uptime counts");
    let up = writes::wait_for_restart(&mut c, addr, before, std::time::Duration::from_secs(2)).unwrap();
    assert!(up < 50_000);
}

#[test]
fn refused_factory_reset_is_reported() {
    let (addr, st) = start(State { secret: Some(OLD), reset_status: 59, ..Default::default() });
    let mut c = client();
    let f = writes::reset(&mut c, addr, &secret(OLD), g7250_proto::mgmt::Command::FactoryReset).unwrap_err();
    assert!(f.to_string().contains("status 59: reset refused: RF present"), "{f}");
    // A refused FACTORY_RESET may have opened the session: it is ended.
    assert_eq!(cmds(&st), vec![0x01, 0x10, 0x22, 0x10, 0x13]);
}

#[test]
fn first_secret_in_the_window_and_after_it() {
    let (addr, st) = start(State { window_open: true, ..Default::default() });
    let mut c = client();
    writes::set_first_secret(&mut c, addr, &secret(OLD)).unwrap();
    assert_eq!(st.lock().unwrap().secret, Some(OLD));
    let f = writes::set_first_secret(&mut c, addr, &secret(NEW)).unwrap_err();
    assert!(f.to_string().contains("status 36"), "{f}");
    assert_eq!(st.lock().unwrap().secret, Some(OLD));
}

#[test]
fn config_set_is_read_back() {
    let (addr, st) = start(State { secret: Some(OLD), config_lies: true, ..Default::default() });
    let mut c = client();
    let mut change = Config::default();
    change.band_limit[2] = Some(500);
    let f = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap_err();
    assert!(f.to_string().contains("does not show the change"), "{f}");
    st.lock().unwrap().config_lies = false;
    let r = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap();
    assert_eq!(r.read_back.unwrap().band_limit[2], Some(500));
}

#[test]
fn network_change_that_does_not_come_up_is_reported() {
    // A new static address the radio never answers at: the read-back goes there first, then
    // to the old address, where the radio still is (the fallback of A1.6 C-4).
    let (addr, st) = start(State { secret: Some(OLD), ..Default::default() });
    let mut c = client();
    let change = Config { address_mode: Some(1), address: Some([127, 0, 0, 1]), ..Default::default() };
    // Same IP as the fake radio but the change leaves it where it is: not a move.
    let r = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap();
    assert_eq!(r.whereabouts, Whereabouts::At(addr));
    let change = Config { address_mode: Some(1), address: Some([192, 0, 2, 1]), ..Default::default() };
    let f = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap_err();
    assert!(f.to_string().contains("kept its old address"), "{f}");
    assert_eq!(st.lock().unwrap().holder, None, "the session was ended at the address that answered");
}

#[test]
fn calibration_set_and_clear_are_read_back() {
    let (addr, st) = start(State { secret: Some(OLD), ..Default::default() });
    let mut c = client();
    let e = vec![("P-RF-2".to_string(), vec![0, 10])];
    let (id, tx) = writes::set_calibration(&mut c, addr, &secret(OLD), 1_795_305_600, "bench", &e).unwrap();
    assert!(tx);
    let stored = st.lock().unwrap().calibration.clone().unwrap();
    assert_eq!(id.to_vec(), Sha256::digest(&stored).to_vec());
    writes::clear_calibration(&mut c, addr, &secret(OLD)).unwrap();
    assert!(st.lock().unwrap().calibration.is_none());
    assert_eq!(st.lock().unwrap().holder, None);
}

#[test]
fn a_secret_file_is_never_needed_on_the_command_line() {
    // The binary refuses to run a write without a secret source, and names only the options.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_g7250ctl"))
        .args(["clear-log", "127.0.0.1:9"])
        .env_remove(g7250ctl::secret::ENV)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--secret-file"));
}
