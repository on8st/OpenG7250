// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! M5 against a local fake radio: the reserved RETURN_TO_STOCK and ENTER_VENDOR_RECOVERY, and
//! the streaming installation of a stock image (A1.8a items 2 to 5). The fake checks HMACs and
//! the image hash with its own code (hmac, sha2), and reads the package header by hand from
//! Table M-4.

mod common;

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use g7250ctl::reads::Failure;
use g7250ctl::secret::Secret;
use g7250ctl::stock::{self, StockLeft, StockOptions, StockOutcome};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

const SECRET: [u8; 32] = [0x3C; 32];

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}

#[derive(Default)]
struct Radio {
    challenges: HashMap<SocketAddr, [u8; 16]>,
    next_ch: u8,
    /// The F1 gate (A1.8a item 3): closed, a foreign UPDATE_BEGIN gets 56.
    gate_open: bool,
    /// 0 this firmware, 3 installer before the first erase, 4 installer writing.
    state: u8,
    header: Vec<u8>,
    blocks: Vec<Option<Vec<u8>>>,
    /// The first erase comes once this many blocks have arrived.
    erase_at: usize,
    sectors: u8,
    verified: bool,
    activated: bool,
    /// Status 53 to this block, this many times.
    crc_fail: Option<(u32, u32)>,
    drop_activate_reply: bool,
    /// The activation ran with its reply lost: the next request (the retransmission) meets the
    /// spent challenge and gets 33, then the stock firmware runs and nothing answers.
    switching: bool,
    restarted_into_this_firmware: bool,
    seen: Vec<(u16, Vec<u8>)>,
}

type Stored = HashMap<(SocketAddr, [u8; 4]), Vec<u8>>;

fn tlv(p: &mut Vec<u8>, t: u8, v: &[u8]) {
    p.push(t);
    p.push(v.len() as u8);
    p.extend_from_slice(v);
}

fn handle(r: &mut Radio, stored: &mut Stored, from: SocketAddr, d: &[u8]) -> Option<Vec<u8>> {
    if r.activated {
        return None; // the stock firmware does not speak this channel
    }
    if r.switching {
        r.switching = false;
        r.activated = true;
        return Some(reply(id_of(d), cmd_of(d), 33, &[]));
    }
    let id = id_of(d);
    let cmd = u16::from_be_bytes(cmd_of(d));
    let n = u16::from_be_bytes([d[16], d[17]]) as usize;
    let payload = d[18..18 + n].to_vec();
    let auth_ok = d[7] & 1 != 0 && r.challenges.get(&from).is_some_and(|ch| hmac(&SECRET, &[ch, &d[..18 + n]]) == d[18 + n..18 + n + 32]);
    r.seen.push((cmd, payload.clone()));
    if let Some(s) = stored.get(&(from, id)) {
        return Some(s.clone());
    }
    let reply_of = |status: u16, p: &[u8]| reply(id, cmd.to_be_bytes(), status, p);
    if !matches!(cmd, 0x01 | 0x10 | 0x45) {
        if !auth_ok {
            return Some(reply_of(33, &[]));
        }
        r.challenges.remove(&from);
    }
    let received = |r: &Radio| r.blocks.iter().filter(|b| b.is_some()).count() as u32;
    let out = match cmd {
        0x01 => {
            let mut p = Vec::new();
            tlv(&mut p, 1, b"G7250");
            tlv(&mut p, 4, &[1, 0, 0, 5]);
            tlv(&mut p, 10, &[if r.state >= 3 { 3 } else { 0 }]);
            tlv(&mut p, 16, &100_000u32.to_be_bytes());
            tlv(&mut p, 23, &[1]);
            reply_of(0, &p)
        }
        0x10 => {
            r.next_ch = r.next_ch.wrapping_add(1);
            let ch = [r.next_ch; 16];
            r.challenges.insert(from, ch);
            reply_of(0, &ch)
        }
        0x13 => reply_of(0, &[]),
        0x50 | 0x51 => reply_of(56, &[]),
        0x40 => {
            let h = &payload[1..];
            if payload[0] & 1 == 0 {
                reply_of(17, &[])
            } else if !r.gate_open {
                reply_of(56, &[])
            } else if h[0..4] != *b"G7PK" || h[5] != 1 || h[6..8] != [0x00, 0xC8] || h[10..14] != [0; 4] || h[52] != 0 {
                reply_of(17, &[])
            } else {
                let len = u32::from_be_bytes(h[14..18].try_into().unwrap()) as usize;
                let count = len.div_ceil(512);
                r.header = h.to_vec();
                r.blocks = vec![None; count];
                r.state = 3;
                let mut p = 512u16.to_be_bytes().to_vec();
                p.extend((count as u32).to_be_bytes());
                p.push(1); // one-way
                reply_of(0, &p)
            }
        }
        0x41 => {
            let num = u32::from_be_bytes(payload[0..4].try_into().unwrap());
            if let Some((b, times)) = r.crc_fail
                && b == num
                && times > 0
            {
                r.crc_fail = Some((b, times - 1));
                reply_of(53, &[])
            } else if r.state < 3 {
                reply_of(54, &[])
            } else {
                r.blocks[num as usize] = Some(payload[8..].to_vec());
                if r.state == 3 && received(r) as usize >= r.erase_at {
                    r.state = 4;
                    r.sectors = 1;
                }
                let mut p = num.to_be_bytes().to_vec();
                p.extend(received(r).to_be_bytes());
                reply_of(0, &p)
            }
        }
        0x42 => {
            if r.blocks.iter().any(|b| b.is_none()) {
                reply_of(55, &[])
            } else {
                let img: Vec<u8> = r.blocks.iter().flat_map(|b| b.clone().unwrap()).collect();
                let h = Sha256::digest(&img);
                if h[..] == r.header[18..50] {
                    r.verified = true;
                    reply_of(0, &h)
                } else {
                    reply_of(55, &[])
                }
            }
        }
        0x43 => {
            if payload != [1] {
                reply_of(17, &[])
            } else if !r.verified {
                reply_of(55, &[])
            } else {
                if r.drop_activate_reply {
                    r.switching = true;
                    return None;
                }
                r.activated = true;
                reply_of(0, &[1])
            }
        }
        0x44 => match r.state {
            4 => reply_of(60, &[]),
            3 => {
                r.state = 0;
                r.restarted_into_this_firmware = true;
                reply_of(0, &[0])
            }
            _ => reply_of(0, &[0]),
        },
        0x45 => {
            let mut p = vec![r.state];
            p.extend(received(r).to_be_bytes());
            p.extend((r.blocks.len() as u32).to_be_bytes());
            p.push(r.sectors);
            reply_of(0, &p)
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

fn open_gate() -> Radio {
    Radio { gate_open: true, erase_at: 2, ..Default::default() }
}

fn image(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 13 + 1) as u8).collect()
}

fn package(img: &[u8]) -> g7250ctl::update::PackageFile {
    stock::stock_package("stock.bin", img, Sha256::digest(img).into()).unwrap()
}

fn cmds(st: &Arc<Mutex<Radio>>) -> Vec<u16> {
    st.lock().unwrap().seen.iter().map(|s| s.0).collect()
}

fn opts() -> StockOptions {
    StockOptions { resume: false, verify_retries: 1 }
}

#[test]
fn reserved_commands_get_56_and_the_session_is_ended() {
    for cmd in [g7250_proto::mgmt::Command::ReturnToStock, g7250_proto::mgmt::Command::EnterVendorRecovery] {
        let (addr, st) = start(Radio::default());
        let f = stock::reserved(&mut client(), addr, &Secret(SECRET), cmd).unwrap_err();
        assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 56), "{f}");
        assert_eq!(cmds(&st), vec![0x10, cmd as u16, 0x10, 0x13], "signed, then END_WRITE_SESSION (refused write, Q9 reading)");
    }
}

#[test]
fn the_host_checks_hash_and_length_before_anything_is_sent() {
    let img = image(1000);
    let mut wrong: [u8; 32] = Sha256::digest(&img).into();
    wrong[0] ^= 1;
    assert!(stock::stock_package("s", &img, wrong).unwrap_err().contains("SHA-256"));
    let big = vec![0u8; stock::MAX_STOCK_LEN + 1];
    assert!(stock::stock_package("s", &big, Sha256::digest(&big).into()).unwrap_err().contains("sectors 1-7"));
    assert!(stock::stock_package("s", &[], Sha256::digest([]).into()).is_err());
    assert!(stock::parse_sha256("ab").is_err());
    assert_eq!(stock::parse_sha256(&"0f".repeat(32)).unwrap(), [0x0f; 32]);
}

#[test]
fn the_package_header_is_that_of_a_foreign_image() {
    let img = image(1300);
    let p = package(&img);
    let h = p.header_bytes();
    assert_eq!(h.len(), 53);
    assert_eq!(&h[0..4], b"G7PK");
    assert_eq!((h[4], h[5], &h[6..8], &h[8..10]), (1, 1, &[0x00, 0xC8][..], &[0, 0][..]), "header version, component, hardware id, any revision");
    assert_eq!(&h[10..14], &[0; 4], "version 0 for a foreign image");
    assert_eq!(u32::from_be_bytes(h[14..18].try_into().unwrap()), 1300);
    assert_eq!(&h[18..50], &Sha256::digest(&img)[..]);
    assert_eq!((&h[50..52], h[52]), (&[0, 0][..], 0), "no signature, slot 0");
}

#[test]
fn a_full_stock_install_is_permanent_and_nothing_follows_the_activation() {
    let img = image(512 * 3 + 100);
    let (addr, st) = start(open_gate());
    let mut left = None;
    let o = stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), opts(), &mut |_| {}, &mut left).unwrap();
    assert_eq!(o, StockOutcome::Activated);
    let r = st.lock().unwrap();
    let begin = &r.seen.iter().find(|s| s.0 == 0x40).unwrap().1;
    assert_eq!(begin[0], 0x01, "foreign flag, no downgrade flag");
    let activate = &r.seen.iter().find(|s| s.0 == 0x43).unwrap().1;
    assert_eq!(activate, &vec![1], "UPDATE_ACTIVATE repeats the foreign flag");
    assert_eq!(r.seen.last().unwrap().0, 0x43, "no END_WRITE_SESSION or IDENTIFY after the switch");
    assert!(r.activated && left.is_none());
}

#[test]
fn a_closed_gate_gets_56_and_nothing_is_sent_after_it_but_the_session_end() {
    let img = image(600);
    let (addr, st) = start(Radio { gate_open: false, ..Default::default() });
    let mut left = None;
    let f = stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), opts(), &mut |_| {}, &mut left).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 56), "{f}");
    assert_eq!(cmds(&st), vec![0x10, 0x40, 0x10, 0x13]);
    assert_eq!(left, Some(StockLeft::Nothing));
}

#[test]
fn a_failure_before_the_first_erase_is_aborted() {
    // Block 0 gets 53 three times: the tool gives up before anything is erased.
    let img = image(512 * 3);
    let (addr, st) = start(Radio { crc_fail: Some((0, 3)), ..open_gate() });
    let mut left = None;
    let f = stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), opts(), &mut |_| {}, &mut left).unwrap_err();
    assert!(matches!(&f, Failure::Status(a, _) if a.status_code == 53), "{f}");
    let c = cmds(&st);
    let at = c.iter().position(|x| *x == 0x44).expect("aborted");
    assert_eq!(c[at - 2], 0x45, "UPDATE_STATUS first: {c:x?}");
    assert_eq!(left, Some(StockLeft::AbortedBeforeErase));
    assert!(st.lock().unwrap().restarted_into_this_firmware);
}

#[test]
fn a_failure_after_the_first_erase_is_never_aborted_and_resume_completes_it() {
    // The first erase comes after two blocks; block 2 then fails.
    let img = image(512 * 4);
    let (addr, st) = start(Radio { crc_fail: Some((2, 3)), ..open_gate() });
    let mut left = None;
    stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), opts(), &mut |_| {}, &mut left).unwrap_err();
    assert!(!cmds(&st).contains(&0x44), "no abort once writing");
    assert_eq!(left, Some(StockLeft::Writing { sectors: 1 }));
    let mut left = None;
    let mut ev = Vec::new();
    let o = stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), StockOptions { resume: true, ..opts() }, &mut |e| ev.push(e), &mut left).unwrap();
    assert_eq!(o, StockOutcome::Activated);
    assert!(ev.contains(&g7250ctl::update::Event::Resumed { from: 2, blocks: 4 }), "{ev:?}");
    assert_eq!(cmds(&st).iter().filter(|c| **c == 0x40).count(), 1, "no second UPDATE_BEGIN");
}

#[test]
fn a_lost_activation_reply_and_a_silent_radio_mean_the_stock_firmware_runs() {
    let img = image(700);
    let (addr, st) = start(Radio { drop_activate_reply: true, ..open_gate() });
    let mut left = None;
    let o = stock::install_stock(&mut client(), addr, &Secret(SECRET), &package(&img), opts(), &mut |_| {}, &mut left).unwrap();
    assert_eq!(o, StockOutcome::ActivatedUnconfirmed);
    assert!(!cmds(&st).contains(&0x44));
}

// The binary: exit codes and confirmations (cli_stock.rs).

fn files(tag: &str, img: &[u8]) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("g7250ctl-stock-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let secret = dir.join("secret");
    std::fs::write(&secret, "3c".repeat(32)).unwrap();
    let image = dir.join("stock.bin");
    std::fs::write(&image, img).unwrap();
    (secret, image, stock::hex(&Sha256::digest(img)))
}

fn run_binary(addr: SocketAddr, args: &[&str]) -> std::process::Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_g7250ctl"))
        .args(["--interval", "30", args[0], &addr.to_string()])
        .args(&args[1..])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn binary_a_lost_activation_reply_with_silence_exits_5_not_confirmed() {
    let img = image(900);
    let (secret, image, sha) = files("exit5", &img);
    let (addr, st) = start(Radio { drop_activate_reply: true, ..open_gate() });
    let out = run_binary(addr, &["install-stock", image.to_str().unwrap(), "--sha256", &sha, "--yes", "--secret-file", secret.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(g7250ctl::exit::NOT_CONFIRMED as i32), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("most likely started"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("activated"), "no activation is claimed: {}", String::from_utf8_lossy(&out.stderr));
    assert!(st.lock().unwrap().activated);
}

#[test]
fn binary_abort_before_the_first_erase_names_the_installer_restart() {
    // Found on the reference model: the installer restarts the radio itself, so `abort` must
    // not ask for a restart, and `--restart` sends none.
    let img = image(512 * 3);
    let (secret, _image, _sha) = files("abort3", &img);
    let p = package(&img);
    let (addr, st) = start(Radio { state: 3, header: p.header_bytes().to_vec(), blocks: vec![None; 3], ..open_gate() });
    let out = run_binary(addr, &["abort", "--restart", "--secret-file", secret.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(g7250ctl::exit::OK as i32), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("restarts into this firmware") && !text.contains("g7250ctl restart"), "{text}");
    assert!(st.lock().unwrap().restarted_into_this_firmware);
    let c = cmds(&st);
    assert_eq!((c.first(), c.contains(&0x44), c.contains(&0x24)),(Some(&0x45), true, false), "{c:x?}");
}

#[test]
fn binary_resume_before_the_first_erase_asks_for_confirmation() {
    // State 3: the installer has erased nothing yet. Without a terminal and without --yes the
    // question cannot be asked, so nothing is sent but UPDATE_STATUS.
    let img = image(512 * 3);
    let (secret, image, sha) = files("resume3", &img);
    let p = package(&img);
    let (addr, st) = start(Radio { state: 3, header: p.header_bytes().to_vec(), blocks: vec![None; 3], ..open_gate() });
    let out = run_binary(addr, &["install-stock", image.to_str().unwrap(), "--sha256", &sha, "--resume", "--secret-file", secret.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(g7250ctl::exit::USAGE as i32), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--yes"));
    assert_eq!(cmds(&st), vec![0x45]);
}

#[test]
fn binary_resume_while_writing_does_not_ask() {
    let img = image(512 * 3);
    let (secret, image, sha) = files("resume4", &img);
    let p = package(&img);
    let mut blocks = vec![None; 3];
    blocks[0] = Some(img[..512].to_vec());
    let (addr, st) = start(Radio { state: 4, sectors: 1, header: p.header_bytes().to_vec(), blocks, ..open_gate() });
    let out = run_binary(addr, &["install-stock", image.to_str().unwrap(), "--sha256", &sha, "--resume", "--secret-file", secret.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(g7250ctl::exit::OK as i32), "{}", String::from_utf8_lossy(&out.stderr));
    let c = cmds(&st);
    assert_eq!(c.iter().filter(|x| **x == 0x41).count(), 2, "blocks 1 and 2 only: {c:x?}");
    assert!(st.lock().unwrap().activated);
}
