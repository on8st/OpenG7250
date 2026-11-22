//! The streaming installer of A1.8a item 3 (option --f1 open) against spec statements (no network).

use std::net::{Ipv4Addr, SocketAddrV4};

use g7250_refmodel::codes::mgmt::st;
use g7250_refmodel::codes::port;
use g7250_refmodel::crypto::{crc32, hmac_sha256, sha256};
use g7250_refmodel::radio::{App, Config, Radio, Rx, S};

const SECRET: [u8; 32] = [7; 32];
const SECTOR: usize = 128 * 1024;
use g7250_refmodel::codes::mgmt::{
    CHANGE_SECRET as CHANGE, GET_CHALLENGE as CHALLENGE, GET_STATS, IDENTIFY, RESTART, SET_FIRST_SECRET as FIRST,
    UPDATE_ABORT as ABORT, UPDATE_ACTIVATE as ACTIVATE, UPDATE_BEGIN as BEGIN, UPDATE_BLOCK as BLOCK,
    UPDATE_STATUS as STATUS, UPDATE_VERIFY as VERIFY,
};

struct Host {
    r: Radio,
    t: u64,
    id: u32,
}

impl Host {
    fn new(f1_open: bool) -> Host {
        let mut h = Host { r: Radio::new(Config { f1_open, ..Config::default() }, 0), t: S, id: 1 };
        assert_eq!(h.plain(FIRST, &SECRET).unwrap().0, st::OK);
        h
    }
    fn raw(&mut self, cmd: u16, p: &[u8], auth: Option<&[u8]>) -> Option<(u16, Vec<u8>)> {
        self.id += 1;
        let mut d = 0x4737_4D47u32.to_be_bytes().to_vec();
        d.extend_from_slice(&[1, 0, 1, auth.is_some() as u8]);
        d.extend_from_slice(&self.id.to_be_bytes());
        d.extend_from_slice(&cmd.to_be_bytes());
        d.extend_from_slice(&[0, 0]);
        d.extend_from_slice(&(p.len() as u16).to_be_bytes());
        d.extend_from_slice(p);
        if let Some(c) = auth {
            let mut m = c.to_vec();
            m.extend_from_slice(&d);
            d.extend_from_slice(&hmac_sha256(&SECRET, &m));
        }
        self.send(&d)
    }
    fn send(&mut self, d: &[u8]) -> Option<(u16, Vec<u8>)> {
        self.t += S / 10; // under the reply caps
        let src = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 40_000);
        let out = self.r.on_datagram(self.t, &Rx { src, local_port: port::MGMT, to_ip: None, data: d });
        out.iter().find(|t| t.src_port == port::MGMT).map(|t| {
            (u16::from_be_bytes([t.data[14], t.data[15]]), t.data[18..].to_vec())
        })
    }
    fn plain(&mut self, cmd: u16, p: &[u8]) -> Option<(u16, Vec<u8>)> {
        self.raw(cmd, p, None)
    }
    fn req(&mut self, cmd: u16, p: &[u8]) -> (u16, Vec<u8>) {
        let c = self.plain(CHALLENGE, &[]).expect("challenge").1;
        self.raw(cmd, p, Some(&c)).expect("reply")
    }
    fn begin(&mut self, img: &[u8], hw: u16, len: u32) -> (u16, Vec<u8>) {
        let mut h = vec![1u8]; // foreign image
        h.extend_from_slice(b"G7PK");
        h.extend_from_slice(&[1, 1]);
        h.extend_from_slice(&hw.to_be_bytes());
        h.extend_from_slice(&[0; 6]);
        h.extend_from_slice(&len.to_be_bytes());
        h.extend_from_slice(&sha256(img));
        h.extend_from_slice(&[0, 0, 0]);
        self.req(BEGIN, &h)
    }
    fn block(&mut self, n: usize, img: &[u8], bad: bool) -> u16 {
        let d = &img[n * 512..img.len().min((n + 1) * 512)];
        let mut p = (n as u32).to_be_bytes().to_vec();
        p.extend_from_slice(&(crc32(d) ^ bad as u32).to_be_bytes());
        p.extend_from_slice(d);
        self.req(BLOCK, &p).0
    }
    fn status(&mut self) -> (u8, u8) {
        let p = self.plain(STATUS, &[]).unwrap().1;
        (p[0], p[9])
    }
    fn tag(&mut self, t: u8) -> Vec<u8> {
        let p = self.plain(IDENTIFY, &[]).unwrap().1;
        let mut i = 0;
        while i + 2 <= p.len() {
            if p[i] == t {
                return p[i + 2..i + 2 + p[i + 1] as usize].to_vec();
            }
            i += 2 + p[i + 1] as usize;
        }
        vec![]
    }
}

fn image(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 31 + i / 251) as u8).collect()
}

#[test]
fn gate_closed_is_the_default() {
    assert!(!Config::default().f1_open);
    let mut h = Host::new(false);
    let img = image(1000);
    assert_eq!(h.begin(&img, 0x00C8, 1000).0, st::NOT_SUPPORTED);
    assert_eq!(h.begin(&img, 0x00C8, 8 * SECTOR as u32).0, st::NOT_SUPPORTED); // 56 precedes 61 (T-A1-61)
    assert_eq!(h.begin(&img, 0x00C9, 1000).0, st::WRONG_HARDWARE);
    assert!(h.r.mgmt().inst.is_none() && !h.r.update_in_progress);
}

#[test]
fn begin_checks_and_hand_over() {
    let mut h = Host::new(true);
    let img = image(1000);
    assert_eq!(h.begin(&img, 0x00C9, 1000).0, st::WRONG_HARDWARE);
    assert_eq!(h.begin(&img, 0x00C8, 7 * SECTOR as u32 + 1).0, st::TOO_LARGE_IMAGE);
    assert!(h.r.mgmt().inst.is_none());
    let (s, p) = h.begin(&img, 0x00C8, 1000);
    assert_eq!((s, p), (st::OK, vec![2, 0, 0, 0, 0, 2, 1])); // 512, two blocks, one-way
    assert!(h.r.log.iter().any(|e| e.2.contains("foreign image by 127.0.0.1:40000, 1000 bytes, SHA-256")));
    assert_eq!((h.status(), h.tag(10), h.tag(22)), ((3, 0), vec![3], vec![0]));
    assert_eq!(h.r.mgmt().secret, Some(SECRET)); // in storage until the first erase
    for cmd in [GET_STATS] {
        assert_eq!(h.plain(cmd, &[]).unwrap().0, st::UPDATE_IN_PROGRESS);
    }
    assert_eq!(h.req(RESTART, &[]).0, st::UPDATE_IN_PROGRESS);
    assert_eq!(h.req(CHANGE, &[0; 32]).0, st::UPDATE_IN_PROGRESS);
    let src = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 40_001);
    let mut disc = vec![0u8; 60];
    disc[4] = 2;
    assert!(h.r.on_datagram(h.t, &Rx { src, local_port: port::DISCOVERY, to_ip: None, data: &disc }).is_empty());
}

#[test]
fn whole_installation() {
    let mut h = Host::new(true);
    let img = image(SECTOR + 700);
    assert_eq!(h.begin(&img, 0x00C8, img.len() as u32).0, st::OK);
    assert_eq!(h.req(ACTIVATE, &[0]).0, st::MALFORMED);
    assert_eq!(h.req(ACTIVATE, &[1]).0, st::VERIFY_FAILED);
    assert_eq!(h.block(0, &img, true), st::BAD_CRC);
    assert_eq!(h.block(1, &img, false), st::OUT_OF_ORDER);
    for n in 0..255 {
        assert_eq!(h.block(n, &img, false), st::OK);
    }
    assert_eq!((h.status(), h.r.mgmt().secret.is_some()), ((3, 0), true)); // no sector before all its blocks
    assert_eq!(h.block(255, &img, true), st::BAD_CRC);
    assert_eq!((h.status(), h.r.mgmt().secret.is_some()), ((3, 0), true));
    assert_eq!(h.block(255, &img, false), st::OK);
    assert_eq!((h.status(), h.r.mgmt().secret.is_some(), h.tag(22)), ((4, 1), false, vec![1]));
    assert_eq!(h.tag(12)[0], 0); // no first-secret window
    assert_eq!(h.req(ABORT, &[]).0, st::ONE_WAY);
    assert_eq!(h.req(VERIFY, &[]).0, st::VERIFY_FAILED);
    // a stop of more than 10 min does not restart, and the transfer resumes
    h.t += 1200 * S;
    h.r.tick(h.t);
    assert_eq!(h.status(), (4, 1));
    assert_eq!(h.block(256, &img, false), st::OK);
    assert_eq!(h.block(257, &img, false), st::OK);
    assert_eq!(h.status(), (4, 2));
    let (s, p) = h.req(VERIFY, &[]);
    assert_eq!((s, p), (st::OK, sha256(&img).to_vec()));
    assert_eq!(h.req(ACTIVATE, &[0]).0, st::MALFORMED);
    assert_eq!(h.req(ACTIVATE, &[1]), (st::OK, vec![1]));
    assert_eq!(h.r.app, App::Stock);
    assert!(h.plain(IDENTIFY, &[]).is_none());
    h.t += 1200 * S;
    h.r.tick(h.t);
    assert!(h.r.not_this_firmware() && h.plain(IDENTIFY, &[]).is_none());
}

#[test]
fn wrong_hash_is_55_and_the_blocks_may_be_sent_again() {
    let mut h = Host::new(true);
    let img = image(1000);
    let mut other = img.clone();
    other[3] ^= 1;
    assert_eq!(h.begin(&img, 0x00C8, 1000).0, st::OK);
    for n in 0..2 {
        assert_eq!(h.block(n, &other, false), st::OK);
    }
    assert_eq!(h.req(VERIFY, &[]).0, st::VERIFY_FAILED);
    assert_eq!(h.req(ACTIVATE, &[1]).0, st::VERIFY_FAILED);
    for n in 0..2 {
        assert_eq!(h.block(n, &img, false), st::OK);
    }
    assert_eq!(h.req(VERIFY, &[]).0, st::OK);
    assert_eq!(h.block(0, &img, false), st::OK); // a block after it cancels the good verify
    assert_eq!(h.req(ACTIVATE, &[1]).0, st::VERIFY_FAILED);
}

#[test]
fn back_before_the_first_erase() {
    let img = image(SECTOR + 700);
    for how in ["abort", "10 min", "power"] {
        let mut h = Host::new(true);
        assert_eq!(h.begin(&img, 0x00C8, img.len() as u32).0, st::OK);
        assert_eq!(h.block(0, &img, false), st::OK);
        match how {
            "abort" => assert_eq!(h.req(ABORT, &[]), (st::OK, vec![1])),
            "10 min" => {
                h.t += 601 * S;
                h.r.tick(h.t);
            }
            _ => {
                assert!(!h.r.not_this_firmware());
                h.r.reset("power-on", None);
            }
        }
        assert!(h.r.mgmt().inst.is_none() && h.r.app == App::Own, "{how}");
        assert_eq!((h.status(), h.tag(10), h.r.mgmt().secret), ((0, 0), vec![0], Some(SECRET)), "{how}");
        if how != "power" {
            assert!(h.r.log.iter().any(|e| e.2 == "restart: installer-abandoned"), "{how}");
        }
    }
}

#[test]
fn power_loss_while_writing_leaves_nothing_valid() {
    let mut h = Host::new(true);
    let img = image(SECTOR + 700);
    assert_eq!(h.begin(&img, 0x00C8, img.len() as u32).0, st::OK);
    for n in 0..256 {
        assert_eq!(h.block(n, &img, false), st::OK);
    }
    assert!(h.r.not_this_firmware());
    assert_eq!(h.r.app, App::Nothing);
    assert!(h.plain(IDENTIFY, &[]).is_none());
    assert!(h.r.not_this_firmware()); // also at the next power cycle
}
