//! UDP shell of the reference model: binds the level-1 ports of Table B2-1 on one address,
//! feeds the core, ticks it every millisecond, and serves the fixture control socket
//! (testbench/docs/testlist-format.md section 4) on TCP.
//!
//!   g7250-refmodel [--ip 127.0.0.1] [--netmask 255.0.0.0] [--mac 02:00:00:00:72:50]
//!                  [--tx locked|unlocked] [--lock-reason spec|not-calibrated]
//!                  [--control 127.0.0.1:7290] [--endpoint-file PATH]

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use g7250_refmodel::codes::{arm, port};
use g7250_refmodel::json::{self, J};
use g7250_refmodel::radio::{Config, Radio, Rx, MS};

fn parse_mac(s: &str) -> [u8; 6] {
    let mut m = [0u8; 6];
    for (i, p) in s.split(':').enumerate().take(6) {
        m[i] = u8::from_str_radix(p, 16).expect("mac");
    }
    m
}

struct Ctl {
    line: String,
    reply: mpsc::Sender<String>,
}

fn main() {
    let mut cfg = Config::default();
    let mut control = "127.0.0.1:7290".to_string();
    let mut endpoint_file: Option<String> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--ip" => cfg.ip = v.parse().expect("ip"),
            "--netmask" => cfg.netmask = v.parse().expect("netmask"),
            "--mac" => cfg.mac = parse_mac(&v),
            "--tx" => cfg.tx_locked = v != "unlocked",
            "--lock-reason" => {
                cfg.lock_reason = if v == "not-calibrated" { arm::PROPOSED_NOT_CALIBRATED } else { arm::TX_NOT_IMPLEMENTED }
            }
            "--control" => control = v,
            "--endpoint-file" => endpoint_file = Some(v),
            "-h" | "--help" => {
                println!("see the header of src/main.rs and README.md");
                return;
            }
            o => panic!("unknown option {o}"),
        }
        i += 2;
    }

    let mut socks = Vec::new();
    for p in port::LEVEL1 {
        let s = UdpSocket::bind(SocketAddrV4::new(cfg.ip, p)).unwrap_or_else(|e| panic!("bind {}:{p}: {e}", cfg.ip));
        s.set_nonblocking(true).unwrap();
        s.set_broadcast(true).unwrap();
        socks.push((p, s));
    }
    let (tx, rxc) = mpsc::channel::<Ctl>();
    let listener = TcpListener::bind(&control).expect("control socket");
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut w = conn.try_clone().unwrap();
                for line in BufReader::new(conn).lines().map_while(Result::ok) {
                    let (rtx, rrx) = mpsc::channel();
                    if tx.send(Ctl { line, reply: rtx }).is_err() {
                        return;
                    }
                    let r = rrx.recv().unwrap_or_else(|_| "{\"ok\":false}".into());
                    if writeln!(w, "{r}").is_err() {
                        return;
                    }
                }
            });
        }
    });

    let fixtures = ["net.hosts", "power.cycle", "reset.wdt", "link.set", "hw.observe", "state.dump"];
    let ep = J::obj(vec![
        ("format", J::Str("g7250-endpoint".into())),
        ("version", J::Num(1.0)),
        ("kind", J::Str("refmodel".into())),
        ("radio", J::obj(vec![
            ("ip", J::Str(cfg.ip.to_string())),
            ("netmask", J::Str(cfg.netmask.to_string())),
            ("mac", J::Str(cfg.mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"))),
        ])),
        ("hosts", J::Arr(vec![J::obj(vec![("ip", J::Str(cfg.ip.to_string())), ("in_subnet", J::Bool(true))])])),
        ("broadcast", J::obj(vec![("limited", J::Bool(false)), ("directed", J::Null)])),
        ("control", J::obj(vec![("transport", J::Str("tcp".into())), ("address", J::Str(control.clone()))])),
        ("fixtures", J::Arr(fixtures.iter().map(|f| J::Str(f.to_string())).collect())),
        ("time_scale", J::Num(1.0)),
        ("model", J::obj(vec![("tx", J::Str(if cfg.tx_locked { "locked" } else { "unlocked" }.into())),
                             ("lock_reason", J::Num(cfg.lock_reason as f64))])),
    ]);
    if let Some(p) = &endpoint_file {
        std::fs::write(p, ep.dump() + "\n").expect("endpoint file");
    }
    eprintln!("g7250-refmodel on {} (tx {}), control {}", cfg.ip, if cfg.tx_locked { "locked" } else { "unlocked" }, control);

    let t0 = Instant::now();
    let now = || t0.elapsed().as_micros() as u64;
    let mut radio = Radio::new(cfg, now());
    let mut buf = vec![0u8; 2048];
    let mut next_tick = now();
    loop {
        let mut busy = false;
        // Round robin, one datagram per socket per pass: closer to arrival order than draining
        // each socket in port order (the sockets give no order between ports).
        loop {
            let mut any = false;
            for (p, s) in &socks {
                if let Ok((n, from)) = s.recv_from(&mut buf) {
                    any = true;
                    let SocketAddr::V4(src) = from else { continue };
                    let rx = Rx { src, local_port: *p, to_ip: None, data: &buf[..n] };
                    let out = radio.on_datagram(now(), &rx);
                    send(&socks, out);
                }
            }
            if !any {
                break;
            }
            busy = true;
        }
        while let Ok(c) = rxc.try_recv() {
            let reply = control_op(&mut radio, &c.line, now());
            let _ = c.reply.send(reply);
        }
        let t = now();
        if t >= next_tick {
            let out = radio.tick(t);
            send(&socks, out);
            next_tick = t + MS;
        }
        if !busy {
            std::thread::sleep(Duration::from_micros(200));
        }
    }
}

fn send(socks: &[(u16, UdpSocket)], out: Vec<g7250_refmodel::radio::Tx>) {
    for t in out {
        if let Some((_, s)) = socks.iter().find(|(p, _)| *p == t.src_port) {
            let _ = s.send_to(&t.data, t.dst);
        }
    }
}

fn control_op(r: &mut Radio, line: &str, now: u64) -> String {
    let ok = |v: J| J::obj(vec![("ok", J::Bool(true)), ("value", v)]).dump();
    let err = |e: &str| J::obj(vec![("ok", J::Bool(false)), ("error", J::Str(e.into()))]).dump();
    let Ok(req) = json::parse(line) else { return err("bad json") };
    let op = req.get("op").and_then(|o| o.as_str()).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(J::Null);
    r.now = now;
    match op {
        "hw.observe" => {
            let tx = r.hw.tx_levels;
            let lvl = |b: bool| J::Num(b as u8 as f64);
            ok(J::obj(vec![
                ("t_us", J::Num(now as f64)),
                ("tr_lines", J::obj(vec![("PF4", lvl(!tx)), ("PF6", lvl(tx)), ("PC7", lvl(tx)), ("PH11", lvl(tx))])),
                ("spi1_a", J::obj(vec![("oe_PA10", J::Num(0.0)), ("last_frame", J::Num(if tx { 0x80 } else { 0x00 } as f64)),
                                       ("bit7_ever", J::Bool(r.hw.chain_a_bit7_ever || tx))])),
                ("spi1_b", J::obj(vec![("oe_PH10", J::Num(0.0)), ("last_frame", J::Null)])),
                ("atu_latch_writes", J::obj(vec![("PD13", J::Num(0.0)), ("PG2", J::Num(0.0)), ("PG5", J::Num(0.0))])),
                ("pa_dac", J::obj(vec![("reset_PB14", J::Num(0.0)), ("bias_nonzero_ever", J::Bool(r.hw.bias_nonzero_ever || tx))])),
                ("codec_reset_PB10", J::Num(0.0)),
                ("PD12", J::Num(0.0)),
                ("model_note", J::Str("notional state of the reference model, not pins".into())),
            ]))
        }
        "power.cycle" => {
            r.reset("power-on");
            ok(J::Null)
        }
        "reset.wdt" => {
            r.reset("processor watchdog");
            ok(J::Null)
        }
        "link.set" => {
            let sp = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(100.0) as u16;
            r.link_mbit = sp;
            ok(J::Null)
        }
        "state.dump" => ok(J::obj(vec![
            ("session_owner", r.session.as_ref().map_or(J::Null, |s| J::Str(s.owner.to_string()))),
            ("tx_arm_state", J::Num(r.arm.word() as f64)),
            ("token", J::Num(r.token as f64)),
            ("keyed", J::Bool(r.tx.keyed)),
            ("resets", J::Num(r.resets as f64)),
            ("ip", J::Str(Ipv4Addr::from(r.cfg.ip).to_string())),
        ])),
        _ => err(&format!("fixture:{op} not offered")),
    }
}
