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
use g7250ctl::reads::{Failure, LogShown, Observed};
use g7250ctl::secret::Secret;
use g7250ctl::writes::{self, ResetEvidence, ResetStart, Whereabouts};
use g7250_proto::mgmt::Command;
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
    /// The log (one `READ_LOG` page, flag bit 1 clear).
    log: Vec<String>,
    /// Tag 17 now, and after a commanded reset (None: 2, software).
    reset_cause: u8,
    reset_cause_after: Option<u8>,
    /// The log after the reset instead of the log so far with the request's reason line and
    /// "start after software: log kept".
    log_after: Option<Vec<&'static str>>,
    /// The reset keeps the secret: it was not the factory reset's own.
    secret_kept: bool,
    /// READ_LOG gets no reply.
    log_silent: bool,
    /// Status of an authenticated request while no secret is set (None: 32, as A1.10 gives it
    /// since 0080; set to 33 to test a radio built before it).
    no_secret_status: Option<u16>,
    /// RESTART clears the secret as well (another host's factory reset in the window).
    restart_clears_secret: bool,
    /// C-3 is lost: a SET_CONFIG with some but not every C-3 item gets 48 (A1.6 "Lost"; 0072).
    c3_lost: bool,
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
    if cmd == 0x30 && st.log_silent {
        return vec![];
    }
    let needs_auth = !matches!(cmd, 0x01 | 0x02 | 0x10 | 0x11 | 0x20 | 0x30 | 0x45 | 0x60);
    let r = |status: u16, p: &[u8]| reply(id, cmd.to_be_bytes(), status, p);
    let mut did_reset = false;
    let out = if needs_auth && flags & 1 == 0 {
        r(32, &[])
    } else if needs_auth && st.secret.is_none() {
        r(st.no_secret_status.unwrap_or(32), &[])
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
                p.extend([17, 1, st.reset_cause]);
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
                let mut given = Vec::new();
                let mut p = &payload[..];
                while p.len() >= 2 {
                    let (t, l) = (p[0], p[1] as usize);
                    let v = p[2..2 + l].to_vec();
                    given.push((t, if t == 8 { v[0] } else { 0 }));
                    next.insert((t, if t == 8 { v[0] } else { 0 }), v);
                    p = &p[2 + l..];
                }
                // While C-3 is lost: tag 7, tag 8 for every band the radio has, tags 9 and 10.
                let mut every: Vec<(u8, u8)> = st.config.keys().filter(|k| k.0 == 8).copied().collect();
                every.extend([(7, 0), (9, 0), (10, 0)]);
                let some = given.iter().any(|k| (7..=10).contains(&k.0));
                if st.c3_lost && some && !every.iter().all(|k| given.contains(k)) {
                    r(48, &[])
                } else {
                    let out = r(0, &config_payload(&next));
                    // A change of address (tag 2) or netmask (tag 3) ends the write session and
                    // discards the challenges, after the reply (A1.10; 0074 item 9).
                    let moved = [2, 3].iter().any(|t| next.get(&(*t, 0)) != st.config.get(&(*t, 0)));
                    if !st.config_lies {
                        st.config = next;
                    }
                    if moved {
                        st.holder = None;
                        st.challenges.clear();
                    }
                    out
                }
            }
            0x22 | 0x24 => {
                if st.reset_status != 0 {
                    r(st.reset_status, &[])
                } else {
                    st.uptime = 0;
                    st.holder = None;
                    // The reply is sent, then lost with the reset, as are the challenges
                    // (Table A1-4); the reason, then the start entry (A1.7; 0068).
                    did_reset = true;
                    st.challenges.clear();
                    st.log.push(if cmd == 0x22 { "restart: factory-reset" } else { "restart: restart" }.into());
                    st.log.push("start after software: log kept".into());
                    if let Some(l) = &st.log_after {
                        st.log = l.iter().map(|t| t.to_string()).collect();
                    }
                    st.reset_cause = st.reset_cause_after.unwrap_or(2);
                    if (cmd == 0x22 && !st.secret_kept) || st.restart_clears_secret {
                        st.secret = None;
                        st.window_open = true;
                    }
                    r(0, &[])
                }
            }
            0x23 | 0x31 => r(0, &[]),
            0x30 => {
                let from = u32::from_be_bytes(payload[..4].try_into().unwrap()) as usize;
                let mut p = Vec::new();
                for (i, t) in st.log.iter().enumerate().skip(from) {
                    p.extend(log_entry(i as u32, 0, t));
                }
                r(0, &p)
            }
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
    if did_reset {
        stored.clear();
    } else {
        stored.insert((from, id), out.clone());
    }
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
    let writes::ResetStart::Answered(before) = writes::reset(&mut c, addr, &secret(OLD), g7250_proto::mgmt::Command::Restart).unwrap() else {
        panic!("answered")
    };
    assert_eq!(cmds(&st), vec![0x01, 0x10, 0x24], "IDENTIFY for the uptime, then the signed RESTART");
    assert_eq!((before.uptime_ms, before.slot_changes), (50_100, false), "a RESTART keeps the slot: only the uptime counts");
    let up = writes::wait_for_restart(&mut c, addr, before, std::time::Duration::from_secs(2)).unwrap();
    assert!(up.uptime_ms < 50_000);
    assert_eq!(up.reset_cause, 2);
}

#[test]
fn a_restart_with_its_reply_lost_is_told_from_identify_and_the_log_and_not_sent_again() {
    // A1.4 "Requests that end in a reset" (0074 item 10): the retransmission after the reset
    // meets a spent challenge (33); retransmitting stops there, IDENTIFY shows the reset
    // (uptime, cause software) and the log names it. No new challenge, no new RESTART.
    let (addr, st) = start(State { secret: Some(OLD), uptime: 50_000, reset_cause: 1, drop_first: Some(0x24), ..Default::default() });
    let r = writes::reset(&mut client(), addr, &secret(OLD), g7250_proto::mgmt::Command::Restart).unwrap();
    assert_eq!(r, writes::ResetStart::Seen { evidence: ResetEvidence::Log("restart: restart".into()) });
    let c = cmds(&st);
    assert_eq!(c.iter().filter(|x| **x == 0x10).count(), 1, "one challenge: {c:x?}");
    let restarts: Vec<Vec<u8>> = st.lock().unwrap().seen.iter().filter(|s| s.cmd == 0x24).map(|s| s.raw.clone()).collect();
    assert_eq!(restarts.len(), 2, "the first send and one retransmission, stopped at the 33: {c:x?}");
    assert_eq!(restarts[0], restarts[1], "the same datagram");
    assert_eq!(c[c.len() - 2..], [0x01, 0x30], "IDENTIFY, then the log: {c:x?}");
}

#[test]
fn a_lost_reply_counts_only_with_cause_software_and_for_the_slot_commands_a_changed_slot() {
    // A1.4 (0074 item 10): uptime AND tag 17 = 2, and for UPDATE_ACTIVATE and ROLLBACK also a
    // changed tag 23. Any restart still ends the wait after an answered request.
    let before = writes::Before { uptime_ms: 50_000, running_slot: 1, slot_changes: false, secret_set: true, sent: std::time::Instant::now() };
    std::thread::sleep(std::time::Duration::from_millis(50));
    let o = |uptime_ms, reset_cause, running_slot| Observed { uptime_ms, reset_cause, running_slot, secret_set: true };
    assert!(before.ran(&o(10, 2, 1)), "software, started after the request");
    for cause in [0, 1, 3, 4, 5] {
        assert!(!before.ran(&o(10, cause, 1)), "cause {cause} is not evidence that the request ran");
        assert!(before.restarted(&o(10, cause, 1)), "but it is a restart (cause {cause})");
    }
    assert!(!before.ran(&o(50_200, 2, 1)) && !before.restarted(&o(50_200, 2, 1)), "still the same run");
    let slot = writes::Before { slot_changes: true, ..before };
    assert!(slot.ran(&o(10, 2, 2)));
    assert!(!slot.ran(&o(10, 2, 1)), "a software reset with the same slot running: not the activation");
    assert!(slot.restarted(&o(10, 2, 1)));
    assert!(!slot.ran(&o(60_000, 2, 2)), "a changed slot without a start after the request");
    // An uptime below the time since the request, even above the one before (a request just
    // after a start): the radio started after it.
    let early = writes::Before { uptime_ms: 5, running_slot: 1, slot_changes: false, secret_set: true, sent: std::time::Instant::now() - std::time::Duration::from_secs(2) };
    assert!(early.ran(&o(1_000, 2, 1)));
}

#[test]
fn a_lost_restart_reply_with_cause_unknown_is_not_told_as_done() {
    let (addr, st) = start(State { secret: Some(OLD), uptime: 50_000, reset_cause: 1, reset_cause_after: Some(0), drop_first: Some(0x24), ..Default::default() });
    let f = writes::reset(&mut client(), addr, &secret(OLD), g7250_proto::mgmt::Command::Restart).unwrap_err();
    assert!(matches!(&f, Failure::ResetNotShown { seen: Some(Observed { reset_cause: 0, .. }), .. }), "{f:?}");
    let t = f.to_string();
    assert!(t.contains("status 33") && t.contains("the radio restarted, cause unknown") && t.contains("not sent again"), "{t}");
    assert_eq!(f.exit_code(), g7250ctl::exit::STATUS);
    let c = cmds(&st);
    assert_eq!((c.iter().filter(|x| **x == 0x10).count(), c.iter().filter(|x| **x == 0x24).count()), (1, 2), "{c:x?}");
}

/// `cmd` with its reply lost, and a software reset after it that leaves `log` (None: the
/// request's own lines): the result, and the radio's state. Nothing is sent again.
fn lost(cmd: Command, st: State) -> (Result<ResetStart, Failure>, Arc<Mutex<State>>) {
    let code = if cmd == Command::FactoryReset { 0x22 } else { 0x24 };
    let (addr, st) = start(State { secret: Some(OLD), uptime: 50_000, reset_cause: 1, drop_first: Some(code), ..st });
    let r = writes::reset(&mut client(), addr, &secret(OLD), cmd);
    let c = cmds(&st);
    assert_eq!(c.iter().filter(|x| **x == 0x10).count(), 1, "one challenge, no session end: {c:x?}");
    let sent: Vec<Vec<u8>> = st.lock().unwrap().seen.iter().filter(|s| s.cmd == code).map(|s| s.raw.clone()).collect();
    assert_eq!(sent.len(), 2, "the first send and one retransmission: {c:x?}");
    assert_eq!(sent[0], sent[1], "the same datagram");
    (r, st)
}

/// The log of a `ResetNotShown`, and its text.
fn not_shown(r: Result<ResetStart, Failure>) -> (LogShown, String) {
    let f = r.unwrap_err();
    assert_eq!(f.exit_code(), g7250ctl::exit::STATUS, "{f}");
    let t = f.to_string();
    assert!((t.contains("status 33") || t.contains("status 32")) && t.contains("Not confirmed") && t.contains("not sent again"), "{t}");
    match f {
        Failure::ResetNotShown { log: Some(l), seen: Some(_), .. } => (l, t),
        f => panic!("{f:?}"),
    }
}

#[test]
fn an_unexpected_stop_in_the_window_is_not_told_as_done() {
    // The request did not run; the firmware stopped by itself (0075, 0078): a software reset,
    // and "start: unexpected stop" after the start entry, whatever the entry before it names.
    for cmd in [Command::Restart, Command::FactoryReset] {
        for before in ["restart: stop 7", "restart: restart", "restart: factory-reset"] {
            let log = vec![before, "start after software: log kept", "boot: slot B refused: hash", "start: unexpected stop"];
            let (r, _) = lost(cmd, State { log_after: Some(log), secret_kept: true, ..Default::default() });
            let (l, t) = not_shown(r);
            assert_eq!(l, LogShown::Unexpected { before: Some(before.into()) });
            assert!(t.contains("start: unexpected stop"), "{t}");
        }
    }
    // Also a factory reset that the secret seems to confirm: tag 11 counts only with the log lost.
    let log = vec!["restart: stop 7", "start after software: log kept", "start: unexpected stop"];
    let (r, _) = lost(Command::FactoryReset, State { log_after: Some(log), ..Default::default() });
    assert!(not_shown(r).1.contains("no secret set, tag 11 = 0"));
}

#[test]
fn a_log_lost_or_not_readable_confirms_no_restart_and_says_why() {
    // With the log lost, a software reset is also an unexpected stop until P-SYS-9 is set.
    for log in [vec!["start after software: log lost"], vec!["start after software: log lost", "start: unexpected stop"]] {
        let (r, _) = lost(Command::Restart, State { log_after: Some(log), ..Default::default() });
        let (l, t) = not_shown(r);
        assert_eq!(l, LogShown::Lost);
        assert!(t.contains("the log was lost at the reset") && t.contains("g7250ctl info") && t.contains("g7250ctl logs"), "{t}");
    }
    let (r, _) = lost(Command::Restart, State { log_silent: true, ..Default::default() });
    let (l, t) = not_shown(r);
    assert!(matches!(l, LogShown::Unreadable(_)) && t.contains("the log could not be read"), "{t}");
    for (log, want) in [(vec![], LogShown::NoStart), (vec!["start after software: log kept"], LogShown::NoReason)] {
        let (r, _) = lost(Command::Restart, State { log_after: Some(log), ..Default::default() });
        assert_eq!(not_shown(r).0, want);
    }
}

#[test]
fn a_log_naming_another_request_is_not_told_as_done() {
    for (cmd, line) in [
        (Command::Restart, "restart: factory-reset"),
        (Command::Restart, "restart: update-abandoned"),
        (Command::FactoryReset, "restart: restart"),
        (Command::Restart, "dhcp: address 192.0.2.1"),
    ] {
        // The request's own line earlier in the log is not the last entry before the start.
        let own = if cmd == Command::Restart { "restart: restart" } else { "restart: factory-reset" };
        let log = vec![own, "start after software: log kept", line, "start after software: log kept"];
        let (r, _) = lost(cmd, State { log_after: Some(log), secret_kept: true, ..Default::default() });
        let (l, t) = not_shown(r);
        assert_eq!(l, LogShown::Other(line.into()));
        assert!(t.contains(line) && t.contains("does not name this request"), "{t}");
    }
}

#[test]
fn a_lost_factory_reset_reply_is_judged_by_the_log_and_by_tag_11_when_the_log_is_lost() {
    // The log names it, and the secret is gone.
    let (r, _) = lost(Command::FactoryReset, State::default());
    assert_eq!(r.unwrap(), ResetStart::Seen { evidence: ResetEvidence::Log("restart: factory-reset".into()) });
    let lost_log = || Some(vec!["start after software: log lost", "start: unexpected stop"]);
    // Log lost, no secret set any more: done, and the evidence is named.
    let (r, _) = lost(Command::FactoryReset, State { log_after: lost_log(), ..Default::default() });
    let ResetStart::Seen { evidence } = r.unwrap() else { panic!() };
    assert_eq!(evidence, ResetEvidence::SecretCleared);
    let t = evidence.to_string();
    assert!(t.contains("the log was lost") && t.contains("the evidence used") && t.contains("tag 11 = 0"), "{t}");
    // Log lost, a secret still set: it did not run.
    let (r, _) = lost(Command::FactoryReset, State { log_after: lost_log(), secret_kept: true, ..Default::default() });
    let (l, t) = not_shown(r);
    assert_eq!(l, LogShown::Lost);
    assert!(t.contains("a secret is set (tag 11 = 1)") && t.contains("it did not run") && t.contains("the log was lost"), "{t}");
    // A secret still set is never a confirmed factory reset, even with its line in the log.
    let (r, _) = lost(Command::FactoryReset, State { secret_kept: true, ..Default::default() });
    let (l, t) = not_shown(r);
    assert_eq!(l, LogShown::Names("restart: factory-reset".into()));
    assert!(t.contains("a secret is set (tag 11 = 1)"), "{t}");
    // Tag 11 confirms with the log lost, not with the log unreadable.
    let (r, _) = lost(Command::FactoryReset, State { log_silent: true, ..Default::default() });
    assert!(matches!(not_shown(r).0, LogShown::Unreadable(_)));
}

#[test]
fn a_factory_reset_retransmission_may_get_32_or_33_and_a_first_32_is_a_refusal() {
    // Found against the reference model: after a factory reset no secret is set, and the
    // retransmission gets 32, not 33. Both are judged by IDENTIFY and the log.
    for code in [32, 33] {
        let (r, _) = lost(Command::FactoryReset, State { no_secret_status: Some(code), ..Default::default() });
        assert_eq!(r.unwrap(), ResetStart::Seen { evidence: ResetEvidence::Log("restart: factory-reset".into()) }, "{code}");
        let log = vec!["restart: stop 1", "start after software: log kept", "start: unexpected stop"];
        let (r, _) = lost(Command::FactoryReset, State { no_secret_status: Some(code), log_after: Some(log), ..Default::default() });
        let (l, t) = not_shown(r);
        assert!(matches!(l, LogShown::Unexpected { .. }) && t.contains(&format!("status {code}")), "{t}");
    }
    // A 32 to the first send: refused, nothing more is looked at.
    let (addr, st) = start(State { uptime: 50_000, ..Default::default() });
    let f = writes::reset(&mut client(), addr, &secret(OLD), Command::FactoryReset).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, Command::FactoryReset) if a.status_code == 32 && a.sends == 1), "{f:?}");
    assert_eq!(cmds(&st), vec![0x01, 0x10, 0x22]);
    // For RESTART a 32 to the retransmission stays a refusal.
    let (r, _) = lost(Command::Restart, State { restart_clears_secret: true, ..Default::default() });
    assert!(matches!(r.unwrap_err(), Failure::Status(a, Command::Restart) if a.status_code == 32 && a.sends == 2));
}

#[test]
fn an_answered_restart_followed_by_a_watchdog_reset_ends_the_wait_and_names_the_cause() {
    let (addr, _st) = start(State { secret: Some(OLD), uptime: 50_000, reset_cause: 1, reset_cause_after: Some(3), ..Default::default() });
    let dir = std::env::temp_dir().join(format!("g7250ctl-writes-wdt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret");
    std::fs::write(&file, "11".repeat(32)).unwrap();
    let t0 = std::time::Instant::now();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_g7250ctl"))
        .args(["--interval", "30", "restart", &addr.to_string(), "--wait", "--secret-file", file.to_str().unwrap()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(g7250ctl::exit::OK as i32), "{text}{}", String::from_utf8_lossy(&out.stderr));
    assert!(text.contains("restart done") && text.contains("reset cause watchdog (tag 17), not software"), "{text}");
    assert!(t0.elapsed() < std::time::Duration::from_secs(10), "the wait ended at the restart, not at its limit");
}

#[test]
fn a_32_with_no_secret_points_to_the_first_secret() {
    // No secret set (A1.10 "No secret, no HMAC"; 0080): a write gets 32 once, is not sent again,
    // and the text says how to set the first secret.
    let (addr, st) = start(State { window_open: true, ..Default::default() });
    let mut c = client();
    let change = Config { default_level: Some(2), ..Default::default() };
    let f = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap_err();
    let t = f.to_string();
    assert!(t.contains("status 32: authentication required") && t.contains("no secret is set") && t.contains("g7250ctl set-secret"), "{t}");
    assert_eq!(st.lock().unwrap().seen.iter().filter(|s| s.cmd == 0x21).count(), 1, "one SET_CONFIG, no retransmission");
    // A 33 carries no such hint.
    let (addr, _st) = start(State { secret: Some(NEW), ..Default::default() });
    let t = writes::set_config(&mut client(), addr, &secret(OLD), &change).unwrap_err().to_string();
    assert!(t.contains("status 33") && !t.contains("set-secret"), "{t}");
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
fn a_48_to_a_c3_write_names_the_lost_state() {
    // C-3 lost (A1.6 "Lost"; 0072): some but not every C-3 item gets 48, and the text says how
    // the lost state ends; every C-3 item at once is accepted; a 48 to a write with no C-3 item
    // carries no such hint.
    let mut config = BTreeMap::new();
    for band in 0..2u8 {
        config.insert((8, band), vec![band, 0, 0]);
    }
    let (addr, st) = start(State { secret: Some(OLD), config, c3_lost: true, ..Default::default() });
    let dir = std::env::temp_dir().join(format!("g7250ctl-writes-c3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret");
    std::fs::write(&file, "11".repeat(32)).unwrap();
    let run = |items: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_g7250ctl"))
            .args(["config", "set", &addr.to_string(), "--secret-file", file.to_str().unwrap()])
            .args(items)
            .output()
            .unwrap();
        (out.status.code(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
    };
    let (code, text) = run(&["--band-limit", "0=50"]);
    assert_ne!(code, Some(g7250ctl::exit::OK as i32), "{text}");
    assert!(text.contains("status 48") && text.contains("config: C-3 lost, TX closed until rewritten"), "{text}");
    assert!(text.contains("every C-3 item"), "{text}");
    let (code, text) = run(&["--max-tx-time", "60000", "--band-limit", "0=50", "--band-limit", "1=0", "--swr-limit", "2.5", "--pa-temp-limit", "70"]);
    assert_eq!(code, Some(g7250ctl::exit::OK as i32), "{text}");
    assert!(text.contains("power tx_band[0]   50.0 W"), "{text}");
    // A write with no C-3 item is not refused for the lost state.
    let (code, text) = run(&["--default-level", "2"]);
    assert_eq!(code, Some(g7250ctl::exit::OK as i32), "{text}");
    assert_eq!(st.lock().unwrap().config.get(&(5, 0)), Some(&vec![2]));
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
fn an_address_or_netmask_change_sends_no_session_end() {
    // The fake stays at 127.0.0.1; a new netmask is a change of A1.10 all the same.
    let mut config = BTreeMap::new();
    config.insert((1, 0), vec![1]);
    config.insert((2, 0), vec![127, 0, 0, 1]);
    config.insert((3, 0), vec![255, 0, 0, 0]);
    let (addr, st) = start(State { secret: Some(OLD), config, ..Default::default() });
    let mut c = client();
    let change = Config { netmask: Some([255, 255, 0, 0]), ..Default::default() };
    let r = writes::set_config(&mut c, addr, &secret(OLD), &change).unwrap();
    assert_eq!(r.whereabouts, Whereabouts::At(addr));
    assert_eq!(cmds(&st), vec![0x20, 0x10, 0x21, 0x20], "GET_CONFIG, SET_CONFIG, read-back; no END_WRITE_SESSION");
    // The next write takes a new challenge and is accepted.
    writes::clear_log(&mut c, addr, &secret(OLD)).unwrap();
    let g = st.lock().unwrap();
    assert!(g.seen.iter().rfind(|s| s.cmd == 0x31).unwrap().auth_ok);
    assert_eq!(g.holder, None);
}

#[test]
fn dhcp_to_a_static_address_equal_to_the_lease_ends_the_session() {
    // GET_CONFIG's address changes, the address in use does not: not certain, so the session
    // is ended (the radio would answer 0 if it had ended it already).
    let mut config = BTreeMap::new();
    config.insert((1, 0), vec![0]);
    config.insert((2, 0), vec![0, 0, 0, 0]);
    config.insert((3, 0), vec![0, 0, 0, 0]);
    let (addr, st) = start(State { secret: Some(OLD), config, ..Default::default() });
    let change = Config { address_mode: Some(1), address: Some([127, 0, 0, 1]), netmask: Some([255, 0, 0, 0]), ..Default::default() };
    let r = writes::set_config(&mut client(), addr, &secret(OLD), &change).unwrap();
    assert_eq!(r.whereabouts, Whereabouts::At(addr));
    let c = cmds(&st);
    assert_eq!(c[c.len() - 2..], [0x10, 0x13], "{c:x?}");
}

#[test]
fn a_network_change_that_keeps_address_and_netmask_ends_the_session() {
    let mut config = BTreeMap::new();
    config.insert((1, 0), vec![1]);
    config.insert((2, 0), vec![127, 0, 0, 1]);
    config.insert((3, 0), vec![255, 0, 0, 0]);
    let (addr, st) = start(State { secret: Some(OLD), config, ..Default::default() });
    let change = Config { gateway: Some([127, 0, 0, 254]), ..Default::default() };
    writes::set_config(&mut client(), addr, &secret(OLD), &change).unwrap();
    assert_eq!(cmds(&st), vec![0x20, 0x10, 0x21, 0x20, 0x10, 0x13]);
    assert_eq!(st.lock().unwrap().holder, None);
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
