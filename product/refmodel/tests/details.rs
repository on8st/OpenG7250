//! 0073, 0074, 0075 (the log line) and 0077 against spec statements (no network).

use std::net::{Ipv4Addr, SocketAddrV4};

use g7250_refmodel::codes::mgmt::{
    st, CLEAR_LOG, END_WRITE_SESSION as END, GET_CALIBRATION, GET_CHALLENGE as CHALLENGE, GET_CONFIG, GET_STATS,
    IDENTIFY, READ_LOG, RESTART, SET_CONFIG, SET_FIRST_SECRET as FIRST, UPDATE_ABORT as ABORT, UPDATE_BEGIN as BEGIN,
    UPDATE_BLOCK as BLOCK, UPDATE_STATUS as STATUS,
};
use g7250_refmodel::codes::{port, COUNTERS};
use g7250_refmodel::crypto::{crc32, hmac_sha256, sha256};
use g7250_refmodel::radio::{Config, Radio, Rx, S};

const SECRET: [u8; 32] = [7; 32];
const A: u16 = 40_000;
const B: u16 = 40_001;

struct Bench {
    r: Radio,
    t: u64,
    id: u32,
}

#[derive(Debug)]
struct Reply {
    status: u16,
    flags: u8,
    p: Vec<u8>,
}

impl Bench {
    fn new(cfg: Config) -> Bench {
        let mut b = Bench { r: Radio::new(cfg, 0), t: S, id: 1 };
        assert_eq!(b.plain(A, FIRST, &SECRET).status, st::OK);
        b
    }
    fn to(&mut self, from: u16, p: u16, data: &[u8]) -> Vec<g7250_refmodel::radio::Tx> {
        let src = SocketAddrV4::new(Ipv4Addr::LOCALHOST, from);
        self.r.on_datagram(self.t, &Rx { src, local_port: p, to_ip: None, data })
    }
    fn datagram(&mut self, cmd: u16, p: &[u8], ch: Option<&[u8]>) -> Vec<u8> {
        self.id += 1;
        let mut d = 0x4737_4D47u32.to_be_bytes().to_vec();
        d.extend_from_slice(&[1, 0, 1, ch.is_some() as u8]);
        d.extend_from_slice(&self.id.to_be_bytes());
        d.extend_from_slice(&cmd.to_be_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&(p.len() as u16).to_be_bytes());
        d.extend_from_slice(p);
        if let Some(c) = ch {
            let mut m = c.to_vec();
            m.extend_from_slice(&d);
            d.extend_from_slice(&hmac_sha256(&SECRET, &m));
        }
        d
    }
    fn send(&mut self, from: u16, d: &[u8]) -> Option<Reply> {
        self.t += S / 10;
        let out = self.to(from, port::MGMT, d);
        out.iter().find(|t| t.src_port == port::MGMT).map(|t| Reply {
            status: u16::from_be_bytes([t.data[14], t.data[15]]),
            flags: t.data[7],
            p: t.data[18..].to_vec(),
        })
    }
    fn plain(&mut self, from: u16, cmd: u16, p: &[u8]) -> Reply {
        let d = self.datagram(cmd, p, None);
        self.send(from, &d).expect("reply")
    }
    fn signed(&mut self, from: u16, cmd: u16, p: &[u8], ch: &[u8]) -> Reply {
        let d = self.datagram(cmd, p, Some(ch));
        self.send(from, &d).expect("reply")
    }
    fn req(&mut self, from: u16, cmd: u16, p: &[u8]) -> Reply {
        let c = self.plain(from, CHALLENGE, &[]).p;
        self.signed(from, cmd, p, &c)
    }
    fn stats(&mut self) -> Vec<(String, u64)> {
        let p = self.plain(B, GET_STATS, &[]).p;
        assert!(18 + p.len() <= 1400);
        let (mut v, mut i) = (Vec::new(), 0);
        while i < p.len() {
            let n = p[i] as usize;
            let name = String::from_utf8(p[i + 1..i + 1 + n].to_vec()).unwrap();
            v.push((name, u64::from_be_bytes(p[i + 1 + n..i + 9 + n].try_into().unwrap())));
            i += 9 + n;
        }
        v
    }
    fn stat(&mut self, name: &str) -> u64 {
        self.stats().iter().find(|(n, _)| n == name).expect("listed").1
    }
    fn tags(&mut self, cmd: u16) -> Vec<u8> {
        let p = self.plain(A, cmd, &[]).p;
        let (mut v, mut i) = (Vec::new(), 0);
        while i < p.len() {
            v.push(p[i]);
            i += 2 + p[i + 1] as usize;
        }
        v
    }
    fn logged(&self, s: &str) -> bool {
        self.r.log.iter().any(|e| e.2 == s)
    }
    fn begin_own(&mut self) -> u16 {
        let mut h = vec![0u8];
        h.extend_from_slice(b"G7PK");
        h.extend_from_slice(&[1, 1, 0, 0xC8, 0, 0, 9, 0, 0, 1]);
        h.extend_from_slice(&1000u32.to_be_bytes());
        h.extend_from_slice(&[0; 32]);
        h.extend_from_slice(&[0, 0, 2]);
        self.req(A, BEGIN, &h).status
    }
}

fn hp(seq: u32, run: bool) -> Vec<u8> {
    let mut d = vec![0u8; 1444];
    d[..4].copy_from_slice(&seq.to_be_bytes());
    d[4] = run as u8;
    d
}
const TX_TIME: [u8; 6] = [7, 4, 0, 1, 0, 0];

#[test]
fn table_m_6() {
    let mut b = Bench::new(Config::default());
    let s = b.stats();
    let names: Vec<&str> = s.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(names[..45], COUNTERS);
    assert!(names.iter().all(|n| n.len() <= 16 && n.is_ascii()));
    assert!(s[..45].iter().all(|x| x.1 == 0), "{s:?}");
    b.to(A, port::HIGH_PRIORITY, &hp(1, true));
    assert_eq!(b.stat("hp_rx"), 1);
    // B11.6: 1, 2, 4 is one gap; a repeat and an older number are none
    for n in [2, 4, 4, 3, 4] {
        b.to(A, port::HIGH_PRIORITY, &hp(n, true));
    }
    assert_eq!((b.stat("gap_1027"), b.stat("gap_1029")), (1, 0));
    b.to(A, port::HIGH_PRIORITY, &[0; 10]);
    b.to(A, port::HIGH_PRIORITY, &[0; 10]);
    assert_eq!(b.stat("drop_1027"), 2);
    assert_eq!(b.r.log.iter().filter(|e| e.2 == "drop on port 1027: short").count(), 1); // once per second
    b.to(B, port::DUC_IQ, &[0; 1444]);
    assert_eq!((b.stat("duc_foreign"), b.stat("drop_1029")), (1, 1));
    b.to(B, port::HIGH_PRIORITY, &hp(0, true));
    assert_eq!(b.stat("takeovers"), 1);
    // a new session counts from 0 again
    b.to(B, port::HIGH_PRIORITY, &hp(1, false));
    assert_eq!(b.stat("gap_1027"), 1);
    b.to(B, port::HIGH_PRIORITY, &hp(2, true));
    assert_eq!(b.stat("gap_1027"), 0);
}

#[test]
fn counters_over_resets() {
    let mut b = Bench::new(Config::default());
    b.to(A, port::HIGH_PRIORITY, &hp(1, true));
    assert_eq!(b.req(A, RESTART, &[]).status, st::OK);
    assert_eq!((b.stat("hp_rx"), b.stat("reset_software"), b.stat("resets")), (0, 1, 1));
    b.r.reset("watchdog", None);
    assert_eq!((b.stat("reset_software"), b.stat("reset_watchdog")), (1, 1));
    b.r.next_flags = Some(vec![]);
    b.r.reset("software", None);
    assert_eq!((b.stat("reset_software"), b.stat("reset_watchdog"), b.stat("reset_unknown")), (0, 0, 0));
}

#[test]
fn rate_drop_is_the_excess() {
    let mut b = Bench::new(Config::default());
    let n0 = b.stat("mg_rate_drop");
    b.t += 2 * S;
    let mut answered = 0;
    for _ in 0..25 {
        let d = b.datagram(GET_CONFIG, &[], None);
        answered += b.to(A, port::MGMT, &d).len() as u64; // 25 at one time
    }
    assert_eq!(answered, 20);
    b.t += 2 * S;
    assert_eq!((b.stat("mg_rate_drop") - n0, b.stat("drop_7251") >= 5), (5, true));
}

#[test]
fn read_log_flag() {
    let mut b = Bench::new(Config::default());
    assert_eq!(b.req(A, CLEAR_LOG, &[]).status, st::OK);
    for i in 0..100 {
        b.r.logf(&format!("filler {i:03} {}", "x".repeat(40)));
    }
    let last = b.r.log.last().unwrap().0;
    let (mut from, mut pages, mut seen) = (0u32, 0, Vec::new());
    loop {
        let r = b.plain(A, READ_LOG, &from.to_be_bytes());
        let mut i = 0;
        while i < r.p.len() {
            seen.push(u32::from_be_bytes(r.p[i..i + 4].try_into().unwrap()));
            i += 9 + r.p[i + 8] as usize;
        }
        pages += 1;
        from = seen.last().unwrap() + 1;
        assert_eq!(r.flags, if from <= last { 2 } else { 0 });
        if r.flags == 0 {
            break;
        }
    }
    assert!(pages > 2 && seen.windows(2).all(|w| w[1] == w[0] + 1) && *seen.last().unwrap() == last);
    let r = b.plain(A, READ_LOG, &from.to_be_bytes());
    assert_eq!((r.flags, r.p.len()), (0, 0)); // an empty page
    assert_eq!(b.plain(A, IDENTIFY, &[]).flags, 0);
    b.r.logf("one more");
    let r = b.plain(A, READ_LOG, &from.to_be_bytes());
    assert_eq!((r.flags, r.p.ends_with(b"one more")), (0, true));
}

#[test]
fn write_session() {
    let mut b = Bench::new(Config::default());
    // none held: 0, opens none
    assert_eq!(b.req(A, END, &[]).status, st::OK);
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::OK);
    assert_eq!(b.req(A, END, &[]).status, st::BUSY);
    assert_eq!(b.req(B, END, &[]).status, st::OK);
    // a refused write opens it: 48
    assert_eq!(b.req(A, SET_CONFIG, &[7, 4, 0, 0, 0, 0]).status, st::INVALID_CONFIG);
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::BUSY);
    assert_eq!(b.req(A, END, &[]).status, st::OK);
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::OK);
}

#[test]
fn the_idle_time_does_not_run_during_the_holders_request() {
    let mut b = Bench::new(Config::default());
    assert_eq!(b.req(A, SET_CONFIG, &TX_TIME).status, st::OK);
    b.t += 50 * S;
    // the model sleeps for a stall, so it is short here; what counts is that the 60 s start at the reply
    b.r.flash_delay = (S / 2, 1);
    assert_eq!(b.req(A, SET_CONFIG, &TX_TIME).status, st::OK);
    let replied = b.r.now;
    assert!(replied >= b.t + S / 2);
    b.t = replied + 60 * S - S / 2; // 59.5 s after the reply, more than 60 s after the request came
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::BUSY);
    b.t = replied + 60 * S;
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::OK);
}

#[test]
fn abort_with_no_update_state() {
    let mut b = Bench::new(Config::default());
    b.to(A, port::HIGH_PRIORITY, &hp(1, true));
    b.r.fw.trial = Some(b.t);
    let r = b.req(A, ABORT, &[]);
    assert_eq!((r.status, r.p), (st::OK, vec![0]));
    assert!(b.r.session.is_some() && !b.r.update_in_progress && b.r.mgmt().last_update.is_none());
    assert_eq!(b.plain(A, STATUS, &[]).p[0], 0);
    assert!(b.r.log.iter().any(|e| e.2.contains("no update state")));
    b.r.tick(b.t + 11 * S);
    assert!(b.r.fw.trial.is_none() && b.logged("trial image confirmed"));
    b.r.tick(b.t + 700 * S);
    assert_eq!(b.r.resets, 0); // no 10-minute timer
    // with an update state 1, also the second time
    b.t += 701 * S;
    assert_eq!(b.begin_own(), st::OK);
    for _ in 0..2 {
        assert_eq!(b.req(A, ABORT, &[]).p, vec![1]);
    }
}

#[test]
fn limits_absent_before_the_calibration_set() {
    let mut b = Bench::new(Config { rx_params: false, ..Config::default() });
    assert!(b.tags(GET_CONFIG).iter().all(|t| *t != 9 && *t != 10));
    let p = b.plain(A, GET_CALIBRATION, &[]).p;
    assert_eq!(p, vec![0; 37]);
    assert!(b.req(A, SET_CONFIG, &TX_TIME).p.len() > 0);
    let mut b = Bench::new(Config { calibrated: true, ..Config::default() });
    let t = b.tags(GET_CONFIG);
    assert!(t.contains(&9) && t.contains(&10));
}

#[test]
fn address_change() {
    let mut b = Bench::new(Config::default());
    let net = [1, 1, 1, 2, 4, 127, 0, 0, 9, 3, 4, 255, 0, 0, 0];
    assert_eq!(b.req(A, SET_CONFIG, &TX_TIME).status, st::OK);
    let old = b.plain(A, CHALLENGE, &[]).p;
    let d = b.datagram(SET_CONFIG, &net, Some(&old));
    assert_eq!(b.send(A, &d).unwrap().status, st::OK); // answered first
    assert!(b.r.log.iter().any(|e| e.2.starts_with("address 127.0.0.1 255.0.0.0 -> 127.0.0.9 255.0.0.0")));
    assert_eq!(b.send(A, &d).unwrap().status, st::AUTH_FAILED); // no stored reply, no challenge
    assert_eq!(b.req(B, SET_CONFIG, &TX_TIME).status, st::OK); // no session
    assert_eq!(b.req(B, SET_CONFIG, &net).status, st::OK); // the same address: no change
    assert_eq!(b.req(A, SET_CONFIG, &TX_TIME).status, st::BUSY); // B's session goes on
    // Set IP is a change of the address too
    let kept = b.plain(A, CHALLENGE, &[]).p;
    let mut s = vec![0u8; 60];
    s[4] = 3;
    s[5..11].copy_from_slice(&b.r.cfg.mac.clone());
    s[11..15].copy_from_slice(&[127, 0, 0, 10]);
    b.to(A, port::DISCOVERY, &s);
    assert_eq!(b.signed(A, SET_CONFIG, &TX_TIME, &kept).status, st::AUTH_FAILED);
    assert_eq!(b.req(A, SET_CONFIG, &TX_TIME).status, st::OK);
}

/// 0073: a limit that is due acts before a request that waited through the work is handled.
#[test]
fn due_limits_before_waiting_requests() {
    let mut b = Bench::new(Config::default());
    b.r.fw.slots[1] = b.r.fw.slots[0].clone();
    b.r.fw.running = 2;
    b.r.fw.trial_net_down = true;
    b.r.fw.trial = Some(b.t);
    b.t += 61 * S; // no tick in between
    assert_eq!(b.plain(A, STATUS, &[]).status, st::OK);
    assert!(b.logged("restart: trial-unconfirmed") && b.r.fw.running == 1);
    assert_eq!(b.begin_own(), st::OK);
    b.t += 601 * S;
    assert_eq!(b.plain(A, STATUS, &[]).p[0], 0);
    assert!(b.logged("restart: update-abandoned"));
}

/// 0075: a software reset without a reason line as the last kept entry.
#[test]
fn unexpected_stop_line() {
    let mut b = Bench::new(Config::default());
    let line = "start: unexpected stop";
    assert_eq!(b.req(A, RESTART, &[]).status, st::OK);
    b.r.reset("watchdog", None);
    b.r.reset("power-on", None);
    assert!(!b.logged(line));
    b.r.reset("software", None); // no reason line
    assert!(b.logged(line));
    b.r.reset("power-on", None);
    b.r.log_corrupt = Some(0); // the whole ring
    b.r.reset("software", Some("restart: restart"));
    assert_eq!(b.r.log.iter().map(|e| e.2.as_str()).collect::<Vec<_>>(), ["start after software: log lost", line]);
}

/// 0077: the write session goes on in the installer.
#[test]
fn installer_session() {
    let mut b = Bench::new(Config { f1_open: true, ..Config::default() });
    let img = vec![5u8; 2000];
    let mut h = vec![1u8];
    h.extend_from_slice(b"G7PK");
    h.extend_from_slice(&[1, 1, 0, 0xC8, 0, 0, 0, 0, 0, 0]);
    h.extend_from_slice(&2000u32.to_be_bytes());
    h.extend_from_slice(&sha256(&img));
    h.extend_from_slice(&[0, 0, 0]);
    assert_eq!(b.req(A, BEGIN, &h).status, st::OK);
    let block = |n: usize| {
        let d = &img[n * 512..img.len().min((n + 1) * 512)];
        let mut p = (n as u32).to_be_bytes().to_vec();
        p.extend_from_slice(&crc32(d).to_be_bytes());
        p.extend_from_slice(d);
        p
    };
    assert_eq!(b.plain(B, FIRST, &[1; 32]).status, st::WINDOW_CLOSED);
    assert_eq!(b.plain(B, GET_CONFIG, &[]).status, st::UPDATE_IN_PROGRESS);
    assert_eq!(b.req(A, BLOCK, &block(0)).status, st::OK);
    assert_eq!(b.req(B, BLOCK, &block(1)).status, st::BUSY);
    assert_eq!(b.req(B, END, &[]).status, st::BUSY);
    assert_eq!(b.req(A, END, &[]).status, st::OK);
    assert_eq!(b.req(B, BLOCK, &block(1)).status, st::OK);
    b.t += 60 * S; // B idle
    assert_eq!(b.req(A, BLOCK, &block(2)).status, st::OK);
    assert_eq!(b.req(A, BLOCK, &block(3)).status, st::OK); // erased
    assert_eq!(b.plain(A, STATUS, &[]).p[0], 4);
    assert_eq!(b.plain(B, FIRST, &[1; 32]).status, st::WINDOW_CLOSED);
    assert_eq!(b.req(A, BLOCK, &block(3)).status, st::OK); // the old secret still authenticates
}
