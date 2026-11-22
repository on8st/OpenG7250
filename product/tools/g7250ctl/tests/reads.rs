// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! The M2 read commands against a local fake radio: `READ_LOG` paging (A1.7 L-3, 0068),
//! `GET_CONFIG` (Table M-3), `GET_CALIBRATION` (Tables M-1, M-5).

mod common;

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use common::*;
use g7250ctl::reads::{self, Failure, LogEvent, LogLine};

/// A radio with a log ring; at most `page` entries per `READ_LOG` reply.
struct Radio {
    log: Vec<(u32, u32, String)>,
    page: usize,
    uptime: u32,
    /// Answer nothing (a reset in progress).
    silent: bool,
    /// Reply to `READ_LOG` with this status instead.
    log_status: u16,
}

fn log_radio(r: Arc<Mutex<Radio>>) -> std::net::SocketAddr {
    let (addr, _rx) = radio(Box::new(move |_, d| {
        let mut r = r.lock().unwrap();
        if r.silent {
            return vec![];
        }
        match cmd_of(d) {
            [0x00, 0x01] => {
                r.uptime += 10;
                vec![reply(id_of(d), cmd_of(d), 0, &identify_payload(1, r.uptime))]
            }
            [0x00, 0x30] => {
                if r.log_status != 0 {
                    return vec![reply(id_of(d), cmd_of(d), r.log_status, &[])];
                }
                let from = u32::from_be_bytes(payload_of(d).try_into().unwrap());
                let mut p = Vec::new();
                for (s, u, t) in r.log.iter().filter(|e| e.0 >= from).take(r.page) {
                    p.extend(log_entry(*s, *u, t));
                }
                vec![reply(id_of(d), cmd_of(d), 0, &p)]
            }
            _ => vec![reply(id_of(d), cmd_of(d), 16, &[])],
        }
    }));
    addr
}

fn entries(n: std::ops::Range<u32>) -> Vec<(u32, u32, String)> {
    n.map(|s| (s, 1000 * s, format!("entry {s}"))).collect()
}

fn lines(ev: &[LogEvent]) -> Vec<u32> {
    ev.iter().filter_map(|e| if let LogEvent::Entry(l) = e { Some(l.seq) } else { None }).collect()
}

#[test]
fn log_is_read_in_pages_to_the_end() {
    let r = Arc::new(Mutex::new(Radio { log: entries(0..7), page: 3, uptime: 0, silent: false, log_status: 0 }));
    let addr = log_radio(r);
    let mut c = client();
    let mut ev = Vec::new();
    let next = reads::read_log(&mut c, addr, 0, &mut |e| ev.push(e)).unwrap();
    assert_eq!(lines(&ev), (0..7).collect::<Vec<_>>());
    assert_eq!(next, 7);
    assert_eq!(ev[2], LogEvent::Entry(LogLine { seq: 2, uptime_ms: 2000, text: "entry 2".into() }));
}

#[test]
fn log_from_a_sequence_and_a_gap() {
    // The ring has dropped 0..10; asking for 4 gives 10 onwards and reports the gap.
    let r = Arc::new(Mutex::new(Radio { log: entries(10..13), page: 2, uptime: 0, silent: false, log_status: 0 }));
    let addr = log_radio(r);
    let mut c = client();
    let mut ev = Vec::new();
    let next = reads::read_log(&mut c, addr, 4, &mut |e| ev.push(e)).unwrap();
    assert_eq!(ev[0], LogEvent::Gap { wanted: 4, got: 10 });
    assert_eq!(lines(&ev), vec![10, 11, 12]);
    assert_eq!(next, 13);
    // Nothing new: an empty page, the same next.
    let mut ev = Vec::new();
    assert_eq!(reads::read_log(&mut c, addr, 13, &mut |e| ev.push(e)).unwrap(), 13);
    assert!(ev.is_empty());
}

#[test]
fn log_status_is_reported() {
    let r = Arc::new(Mutex::new(Radio { log: vec![], page: 2, uptime: 0, silent: false, log_status: 34 }));
    let addr = log_radio(r);
    let mut c = client();
    match reads::read_log(&mut c, addr, 0, &mut |_| {}) {
        Err(f @ Failure::Status(..)) => {
            assert_eq!(f.exit_code(), g7250ctl::exit::STATUS);
            assert!(f.to_string().contains("status 34: rate-limited"), "{f}");
        }
        other => panic!("{other:?}"),
    }
}

/// Run follow_log in a thread for `run`, while `script` changes the radio.
fn follow(r: Arc<Mutex<Radio>>, run: Duration, script: impl FnOnce(&Mutex<Radio>) + Send + 'static) -> (Vec<LogEvent>, usize) {
    let addr = log_radio(r.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    let silences = Arc::new(Mutex::new(0usize));
    let (e2, s2) = (events.clone(), silences.clone());
    let t0 = std::time::Instant::now();
    let h = thread::spawn(move || {
        let mut c = client();
        reads::follow_log(
            &mut c,
            addr,
            0,
            Duration::from_millis(40),
            &mut |e| e2.lock().unwrap().push(e),
            &mut |_| *s2.lock().unwrap() += 1,
            &|| t0.elapsed() > run,
        )
        .unwrap();
    });
    script(&r);
    h.join().unwrap();
    let ev = events.lock().unwrap().clone();
    let s = *silences.lock().unwrap();
    (ev, s)
}

#[test]
fn follow_reads_new_entries_and_survives_a_silent_reset_with_the_log_kept() {
    let r = Arc::new(Mutex::new(Radio { log: entries(0..3), page: 2, uptime: 5000, silent: false, log_status: 0 }));
    let (ev, silences) = follow(r, Duration::from_millis(1500), |r| {
        thread::sleep(Duration::from_millis(150));
        r.lock().unwrap().log.push((3, 9000, "new".into()));
        thread::sleep(Duration::from_millis(150));
        r.lock().unwrap().silent = true;
        thread::sleep(Duration::from_millis(400));
        // Back after a reset with the log kept: uptime from 0, sequence continues (0068).
        let mut g = r.lock().unwrap();
        g.silent = false;
        g.uptime = 0;
        g.log.push((4, 1, "start after restart: log kept".into()));
    });
    assert_eq!(lines(&ev), vec![0, 1, 2, 3, 4]);
    assert!(!ev.contains(&LogEvent::Restarted));
    assert_eq!(silences, 1, "one notice per silence");
}

#[test]
fn follow_starts_again_at_0_when_the_log_was_lost() {
    let r = Arc::new(Mutex::new(Radio { log: entries(0..5), page: 3, uptime: 5000, silent: false, log_status: 0 }));
    let (ev, _) = follow(r, Duration::from_millis(800), |r| {
        thread::sleep(Duration::from_millis(200));
        // Power loss: the ring starts empty and numbering at 0 (0068).
        let mut g = r.lock().unwrap();
        g.uptime = 0;
        g.log = vec![(0, 1, "start after power-on: log lost".into()), (1, 50, "dhcp".into())];
    });
    let pos = ev.iter().position(|e| *e == LogEvent::Restarted).expect("restart reported");
    assert_eq!(lines(&ev[..pos]), vec![0, 1, 2, 3, 4]);
    assert_eq!(lines(&ev[pos..]), vec![0, 1]);
}

#[test]
fn config_reply_decodes() {
    // Table M-3, by hand: DHCP, address, netmask, gateway, level 1, region 0, band 2 at 50 W,
    // allow-list 192.168.1.0/24.
    let mut p = vec![1, 1, 0];
    p.extend([2, 4, 169, 254, 7, 9, 3, 4, 255, 255, 0, 0, 4, 4, 0, 0, 0, 0, 5, 1, 1, 6, 1, 0]);
    p.extend([7, 4, 0, 0, 0x75, 0x30, 8, 3, 2, 0x01, 0xF4, 11, 5, 192, 168, 1, 0, 24]);
    let (addr, _rx) = radio(Box::new(move |_, d| vec![reply(id_of(d), cmd_of(d), 0, &p)]));
    let mut c = client();
    let cfg = reads::get_config(&mut c, addr).unwrap();
    assert_eq!(cfg.address_mode, Some(0));
    assert_eq!(cfg.address, Some([169, 254, 7, 9]));
    assert_eq!(cfg.max_tx_time_ms, Some(30_000));
    assert_eq!(cfg.band_limit[2], Some(500));
    assert_eq!((cfg.allow_count, cfg.allow[0]), (Some(1), ([192, 168, 1, 0], 24)));
    let h = g7250ctl::render::config_human(&cfg);
    assert!(h.contains("DHCP") && h.contains("169.254.7.9") && h.contains("30000 ms") && h.contains("50.0 W") && h.contains("192.168.1.0/24"), "{h}");
}

#[test]
fn bad_config_reply_is_a_decode_failure() {
    let (addr, _rx) = radio(Box::new(|_, d| vec![reply(id_of(d), cmd_of(d), 0, &[2, 4, 1, 2])])); // TLV cut off
    let mut c = client();
    let f = reads::get_config(&mut c, addr).unwrap_err();
    assert!(matches!(f, Failure::Decode { .. }));
    assert_eq!(f.exit_code(), g7250ctl::exit::ERROR);
}

#[test]
fn calibration_reply_decodes() {
    // id, date, source "bench", entries P-RF-2 = 00 0a, P-PWR-2 = 3 bytes.
    let mut p = vec![0xAB; 32];
    p.extend(1_795_305_600u32.to_be_bytes());
    p.push(5);
    p.extend(b"bench");
    for (k, v) in [("P-RF-2", vec![0x00, 0x0A]), ("P-PWR-2", vec![1, 2, 3])] {
        p.push(k.len() as u8);
        p.extend(k.as_bytes());
        p.extend((v.len() as u16).to_be_bytes());
        p.extend(v);
    }
    let (addr, _rx) = radio(Box::new(move |_, d| vec![reply(id_of(d), cmd_of(d), 0, &p)]));
    let mut c = client();
    let cal = reads::get_calibration(&mut c, addr).unwrap();
    assert!(!cal.is_none());
    assert_eq!(cal.source, "bench");
    assert_eq!(cal.entries, vec![("P-RF-2".into(), vec![0, 10]), ("P-PWR-2".into(), vec![1, 2, 3])]);
    let h = g7250ctl::render::calibration_human(&cal);
    assert!(h.contains("2026-11-22 00:00:00 UTC") && h.contains("P-RF-2     2 bytes  000a"), "{h}");
}

#[test]
fn no_calibration_set() {
    // Both forms: the zero id alone, and the zero id with an empty head.
    for p in [vec![0u8; 32], vec![0u8; 37]] {
        let (addr, _rx) = radio(Box::new(move |_, d| vec![reply(id_of(d), cmd_of(d), 0, &p)]));
        let mut c = client();
        let cal = reads::get_calibration(&mut c, addr).unwrap();
        assert!(cal.is_none() && cal.entries.is_empty());
        assert_eq!(g7250ctl::render::calibration_human(&cal), "no calibration set\n");
    }
}
