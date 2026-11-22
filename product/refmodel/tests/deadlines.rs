//! Resets at a deadline that lies before the tick's time (time.advance jumps past it). In the debug
//! build a time that runs backwards into the management state is an overflow panic.

use std::net::{Ipv4Addr, SocketAddrV4};

use g7250_refmodel::codes::mgmt::{
    st, GET_CHALLENGE as CHALLENGE, IDENTIFY, SET_FIRST_SECRET as FIRST, UPDATE_BEGIN as BEGIN,
};
use g7250_refmodel::codes::port;
use g7250_refmodel::crypto::hmac_sha256;
use g7250_refmodel::radio::{Config, Radio, Rx, S};

const SECRET: [u8; 32] = [9; 32];

fn ask(r: &mut Radio, t: u64, id: u32, cmd: u16, p: &[u8], auth: Option<&[u8]>) -> (u16, Vec<u8>) {
    let mut d = 0x4737_4D47u32.to_be_bytes().to_vec();
    d.extend_from_slice(&[1, 0, 1, auth.is_some() as u8]);
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(&cmd.to_be_bytes());
    d.extend_from_slice(&[0, 0]);
    d.extend_from_slice(&(p.len() as u16).to_be_bytes());
    d.extend_from_slice(p);
    if let Some(c) = auth {
        let mut m = c.to_vec();
        m.extend_from_slice(&d);
        d.extend_from_slice(&hmac_sha256(&SECRET, &m));
    }
    let src = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 40_000);
    let out = r.on_datagram(t, &Rx { src, local_port: port::MGMT, to_ip: None, data: &d });
    let t = out.iter().find(|t| t.src_port == port::MGMT).expect("a reply");
    (u16::from_be_bytes([t.data[14], t.data[15]]), t.data[18..].to_vec())
}

/// IDENTIFY tag 12: window open, seconds left.
fn window(r: &mut Radio, t: u64, id: u32) -> (u8, u16) {
    let p = ask(r, t, id, IDENTIFY, &[], None).1;
    let mut i = 0;
    while p[i] != 12 {
        i += 2 + p[i + 1] as usize;
    }
    (p[i + 2], u16::from_be_bytes([p[i + 3], p[i + 4]]))
}

#[test]
fn an_abandoned_update_restarts_at_its_deadline() {
    let mut r = Radio::new(Config::default(), 0);
    assert_eq!(ask(&mut r, S, 1, FIRST, &SECRET, None).0, st::OK);
    let c = ask(&mut r, 2 * S, 2, CHALLENGE, &[], None).1;
    let mut h = vec![0u8];
    h.extend_from_slice(b"G7PK");
    h.extend_from_slice(&[1, 1, 0, 0xC8, 0, 0, 9, 0, 0, 1]);
    h.extend_from_slice(&1000u32.to_be_bytes());
    h.extend_from_slice(&[0; 32]);
    h.extend_from_slice(&[0, 0, 2]);
    assert_eq!(ask(&mut r, 2 * S, 3, BEGIN, &h, Some(&c)).0, st::OK);
    assert!(r.update_in_progress);
    r.tick(2 * S + 900 * S); // the clock jumped 300 s past the deadline
    assert!(!r.update_in_progress);
    assert_eq!(r.booted_at, 602 * S);
    assert_eq!(r.now, 902 * S);
    assert!(r.log.iter().any(|e| e.2 == "restart: update-abandoned"));
    assert_eq!(window(&mut r, 903 * S, 4), (0, 0)); // a secret is set: no window
}

#[test]
fn an_unconfirmed_trial_resets_at_its_deadline() {
    let mut r = Radio::new(Config::default(), 0);
    r.fw.slots[1] = r.fw.slots[0].clone();
    r.fw.running = 2;
    r.fw.trial_net_down = true; // does not confirm itself
    r.fw.trial = Some(10 * S);
    r.tick(100 * S); // 30 s past its 60 s
    assert!(r.fw.trial.is_none());
    assert_eq!((r.fw.running, r.booted_at, r.now), (1, 70 * S, 100 * S));
    // no secret: the window is fresh from that reset, and never above 300 s
    assert_eq!(window(&mut r, 100 * S, 1), (1, 270));
}

#[test]
fn the_window_after_a_power_cycle() {
    let mut r = Radio::new(Config::default(), 0);
    r.now = 100 * S;
    r.reset("power-on", None);
    assert_eq!(window(&mut r, 100 * S, 1), (1, 300));
    assert_eq!(window(&mut r, 399 * S, 2), (1, 1));
    assert_eq!(window(&mut r, 400 * S, 3), (0, 0));
    r.now = 500 * S;
    r.reset("power-on", None);
    assert_eq!(window(&mut r, 500 * S, 4), (0, 0)); // used up: not reopened (A1.10)
}
