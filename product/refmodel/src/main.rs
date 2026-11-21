//! UDP shell of the reference model: binds the level-1 ports of Table B2-1 on one address,
//! feeds the core, ticks it every millisecond, and serves the fixture control socket
//! (testbench/docs/testlist-format.md section 4) on TCP.
//!
//!   g7250-refmodel [--ip 127.0.0.1] [--netmask 255.0.0.0] [--mac 02:00:00:00:72:50]
//!                  [--tx locked|unlocked] [--cal none|rx|full] [--port-base 20000]
//!                  [--control 127.0.0.1:<base+7290>] [--endpoint-file PATH]
//!
//! Port base: the model binds each spec port p (Table B2-1) at local port base + p and writes
//! the mapping into the endpoint file as `radio.connect` (testlist-format v1.3 section 3). The
//! default base 20000 keeps the model off the spec ports 1024-7251, which the emulator runner
//! uses (its hosts[0] binds the fixed source ports 1024 and 7250), off 30000-38299, and below the
//! ephemeral port range (49152 and up on macOS), where client sockets take ports. The model
//! and the emulator runner must never share local ports. `--port-base 0` binds the spec ports
//! themselves; use it only when no emulator run can be going on this machine.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use g7250_refmodel::codes::port;
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
    let mut control: Option<String> = None;
    let mut base: u16 = 20_000;
    let mut endpoint_file: Option<String> = None;
    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        let v = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "--ip" => cfg.ip = v.parse().expect("ip"),
            "--netmask" => cfg.netmask = v.parse().expect("netmask"),
            "--mac" => cfg.mac = parse_mac(&v),
            // locked: no calibration set (r1.0 before measurement); unlocked: a complete simulated
            // calibration set at start (Annex P "Tests")
            "--tx" => cfg.calibrated = v == "unlocked",
            // calibration set at start: none (DDC0 not offered, TX frozen), rx (P-RX-2 only: DDC0
            // offered, TX frozen; the default), full (complete simulated set, as --tx unlocked)
            "--cal" => match v.as_str() {
                "none" => { cfg.calibrated = false; cfg.rx_params = false; }
                "rx" => { cfg.calibrated = false; cfg.rx_params = true; }
                "full" => cfg.calibrated = true,
                o => panic!("--cal none|rx|full, not {o}"),
            },
            "--control" => control = Some(v),
            "--port-base" => base = v.parse().expect("port base"),
            "--endpoint-file" => endpoint_file = Some(v),
            "-h" | "--help" => {
                println!("see the header of src/main.rs and README.md");
                return;
            }
            o => panic!("unknown option {o}"),
        }
        i += 2;
    }

    if base != 0 {
        // every local port the model or the bench uses: base+1024 .. base+9199
        let (lo, hi) = (base as u32 + 1024, base as u32 + 9_199);
        let overlaps = |a: u32, b: u32| lo <= b && a <= hi;
        if hi > 65_535 || overlaps(1024, 7251) || overlaps(30_000, 38_299) {
            panic!("port base {base}: local ports {lo}..{hi} must fit and avoid 1024-7251 and 30000-38299");
        }
    }
    let control = control.unwrap_or_else(|| format!("{}:{}", cfg.ip, base as u32 + 7290));
    let mut socks = Vec::new();
    for p in port::LEVEL1 {
        let lp = base + p;
        let s = UdpSocket::bind(SocketAddrV4::new(cfg.ip, lp)).unwrap_or_else(|e| panic!("bind {}:{lp}: {e}", cfg.ip));
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

    let fixtures = ["net.hosts", "power.cycle", "reset.wdt", "link.set", "hw.observe", "hw.trace", "hw.input", "state.dump"];
    let ep = J::obj(vec![
        ("format", J::Str("g7250-endpoint".into())),
        ("version", J::Num(1.0)),
        ("kind", J::Str("refmodel".into())),
        ("radio", J::obj(vec![
            ("ip", J::Str(cfg.ip.to_string())),
            ("netmask", J::Str(cfg.netmask.to_string())),
            ("mac", J::Str(cfg.mac.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":"))),
            ("connect", if base == 0 { J::obj(vec![]) } else {
                // the open ports of Table B2-1, plus closed ports (nothing bound at base + p) so
                // that T-B3-21 can probe them (B11.9)
                J::Obj(port::LEVEL1.iter().chain([1028u16, 1030, 1036, 1100, 7249, 7252].iter())
                    .map(|p| (p.to_string(), J::Str(format!("{}:{}", cfg.ip, base + p)))).collect())
            }),
        ])),
        // with a port base, the bench binds its host sockets in base+9000 .. base+9199 (two hosts
        // on one address, different ports: different hosts, A2.2)
        ("hosts", if base == 0 {
            J::Arr(vec![J::obj(vec![("ip", J::Str(cfg.ip.to_string())), ("in_subnet", J::Bool(true))])])
        } else {
            let h = |lo: u32| J::obj(vec![("ip", J::Str(cfg.ip.to_string())), ("in_subnet", J::Bool(true)),
                                         ("bind", J::Str(cfg.ip.to_string())),
                                         ("ports", J::Arr(vec![J::Num(lo as f64), J::Num((lo + 99) as f64)]))]);
            J::Arr(vec![h(base as u32 + 9_000), h(base as u32 + 9_100)])
        }),
        ("broadcast", J::obj(vec![("limited", J::Bool(false)), ("directed", J::Null)])),
        ("control", J::obj(vec![("transport", J::Str("tcp".into())), ("address", J::Str(control.clone()))])),
        ("fixtures", J::Arr(fixtures.iter().map(|f| J::Str(f.to_string())).collect())),
        ("time_scale", J::Num(1.0)),
        ("model", J::obj(vec![("tx", J::Str(if cfg.calibrated { "unlocked" } else { "locked" }.into()))])),
    ]);
    if let Some(p) = &endpoint_file {
        std::fs::write(p, ep.dump() + "\n").expect("endpoint file");
    }
    eprintln!("g7250-refmodel on {} (tx {}), control {}", cfg.ip, if cfg.calibrated { "unlocked" } else { "locked" }, control);

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
                // v1.4: mode and level per watched pin; the model drives every HIS 85 item as the
                // HIS asks and leaves PD5 high-impedance (allowed by HIS 70)
                ("pins", {
                    let out = |l: J| J::obj(vec![("mode", J::Str("output".into())), ("level", l)]);
                    J::obj(vec![
                        ("PF4", out(lvl(!tx))), ("PF6", out(lvl(tx))), ("PC7", out(lvl(tx))), ("PH11", out(lvl(tx))),
                        ("PA10", out(J::Num(0.0))), ("PH10", out(J::Num(1.0))),
                        ("PD13", out(J::Num(1.0))), ("PG2", out(J::Num(1.0))), ("PG5", out(J::Num(1.0))),
                        ("PD12", out(J::Num(0.0))), ("PB10", out(J::Num(0.0))), ("PH9", out(J::Num(1.0))),
                        ("PD5", J::obj(vec![("mode", J::Str("analog".into())), ("level", J::Str("z".into()))])),
                    ])
                }),
                ("model_note", J::Str("notional state of the reference model, not pins".into())),
            ]))
        }
        "hw.trace" => {
            // start: the latched "ever" flags count from now (the model has no raw event list;
            // stop returns an empty list)
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("start");
            if action == "start" {
                r.hw.chain_a_bit7_ever = r.hw.tx_levels;
                r.hw.bias_nonzero_ever = r.hw.tx_levels;
                return ok(J::Null);
            }
            // stop: v1.2 shape; the model has no raw events and writes no trace file
            ok(J::obj(vec![("count", J::Num(0.0)), ("dropped", J::Num(0.0))]))
        }
        "hw.input" => {
            // local PTT (PH6) and KEY contacts (PH3, PH4): not used in v1.0 (0030), so the model
            // records the level and nothing follows from it
            let pin = args.get("pin").and_then(|v| v.as_str()).unwrap_or("");
            if !["PH3", "PH4", "PH6"].contains(&pin) {
                return err(&format!("pin not offered: {pin}"));
            }
            let level = args.get("level").and_then(|v| v.as_f64()).unwrap_or(0.0) as u8;
            r.count(&format!("hw.input.{pin}.{level}"));
            ok(J::Null)
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
