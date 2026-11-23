//! UDP shell of the reference model: binds the level-1 ports of Table B2-1 on one address,
//! feeds the core, ticks it every millisecond, and serves the fixture control socket
//! (testbench/docs/testlist-format.md section 4) on TCP.
//!
//!   g7250-refmodel [--ip 127.0.0.1] [--netmask 255.0.0.0] [--mac 02:00:00:00:72:50]
//!                  [--tx locked|unlocked] [--cal none|rx|full] [--port-base 20000]
//!                  [--control 127.0.0.1:<base+7290>] [--endpoint-file PATH] [--f1 closed|open]
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
use g7250_refmodel::radio::{App, Config, Radio, Rx, MS};

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
            // the F1 gate of A1.8a item 3 (P-SYS-5): closed (default), or open with the streaming installer
            "--f1" => cfg.f1_open = match v.as_str() {
                "closed" => false,
                "open" => true,
                o => panic!("--f1 closed|open, not {o}"),
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

    let mut fixtures = vec!["net.hosts", "power.cycle", "reset.wdt", "link.set", "hw.observe", "hw.trace", "hw.input", "state.dump", "key.log", "atu.sim", "net.inject", "time.wait", "time.advance", "trial.sim", "slot.corrupt", "log.corrupt", "flash.delay", "store.full", "reset.flags", "stop.sim", "vendor.install", "c3.lose"];
    if cfg.f1_open {
        fixtures.push("installer.state");
    }
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
        ("model", J::obj(vec![("tx", J::Str(if cfg.calibrated { "unlocked" } else { "locked" }.into())),
                              ("f1_gate", J::Str(if cfg.f1_open { "open" } else { "closed" }.into()))])),
    ]);
    if let Some(p) = &endpoint_file {
        std::fs::write(p, ep.dump() + "\n").expect("endpoint file");
    }
    eprintln!("g7250-refmodel on {} (tx {}), control {}", cfg.ip, if cfg.calibrated { "unlocked" } else { "locked" }, control);

    let t0 = Instant::now();
    // time.advance (model only) moves the model clock forward
    let offset = std::cell::Cell::new(0u64);
    let now = || t0.elapsed().as_micros() as u64 + offset.get();
    let mut radio = Radio::new(cfg, now());
    let mut buf = vec![0u8; 2048];
    let mut next_tick = now();
    let host_ip = radio.cfg.ip;
    let mut inject: Vec<(u64, SocketAddrV4, u16, Vec<u8>)> = Vec::new();
    let mut waits: Vec<(u64, mpsc::Sender<String>)> = Vec::new();
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
            // v1.7: net.inject and time.wait need the loop (the model's time is the wall clock)
            let req = json::parse(&c.line).ok();
            let op = req.as_ref().and_then(|r| r.get("op")).and_then(|o| o.as_str()).unwrap_or("").to_string();
            let args = req.as_ref().and_then(|r| r.get("args")).cloned().unwrap_or(J::Null);
            let okv = |v: J| J::obj(vec![("ok", J::Bool(true)), ("value", v)]).dump();
            if op == "net.inject" {
                let t0 = now();
                let port = args.get("to_port").and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
                let from = args.get("from_port").and_then(|v| v.as_f64()).unwrap_or(0.0) as u16;
                let src = SocketAddrV4::new(host_ip, from);
                let at: Vec<u64> = args.get("at_us").and_then(|v| v.as_arr()).map_or(vec![], |a| {
                    a.iter().map(|x| x.as_f64().unwrap_or(0.0) as u64).collect()
                });
                let pl: Vec<Vec<u8>> = args.get("payloads").and_then(|v| v.as_arr()).map_or(vec![], |a| {
                    a.iter().map(|x| unhex(x.as_str().unwrap_or(""))).collect()
                });
                let mut times = Vec::new();
                for (off, data) in at.iter().zip(pl) {
                    inject.push((t0 + off, src, port, data));
                    times.push(J::Num((t0 + off) as f64));
                }
                inject.sort_by_key(|x| x.0);
                let _ = c.reply.send(okv(J::obj(vec![("t_us", J::Arr(times))])));
            } else if op == "time.advance" {
                // {us} as the emulator has it; {s} accepted too
                let us = args.get("us").and_then(|v| v.as_f64())
                    .or_else(|| args.get("s").and_then(|v| v.as_f64()).map(|x| x * 1e6)).unwrap_or(0.0);
                offset.set(offset.get() + us as u64);
                let _ = c.reply.send(okv(J::obj(vec![("t_us", J::Num(now() as f64))])));
            } else if op == "time.wait" {
                let us = args.get("us").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
                waits.push((now() + us, c.reply));
            } else {
                let reply = control_op(&mut radio, &c.line, now());
                let _ = c.reply.send(reply);
            }
        }
        // injected datagrams that are due, in their order (v1.7)
        while inject.first().map_or(false, |x| x.0 <= now()) {
            let (_, src, port, data) = inject.remove(0);
            let rx = Rx { src, local_port: port, to_ip: None, data: &data };
            let out = radio.on_datagram(now(), &rx);
            send(&socks, out);
        }
        let tnow = now();
        waits.retain(|(at, tx)| {
            if *at <= tnow {
                let _ = tx.send(J::obj(vec![("ok", J::Bool(true)), ("value", J::obj(vec![("t_us", J::Num(tnow as f64))]))]).dump());
                false
            } else {
                true
            }
        });
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

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
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
            if !r.not_this_firmware() {
                r.reset("power-on", None);
            }
            ok(J::Null)
        }
        "reset.wdt" => {
            if !r.not_this_firmware() {
                r.reset("watchdog", None);
            }
            ok(J::Null)
        }
        "installer.state" if r.cfg.f1_open => {
            // model only, --f1 open: what the application area holds (A1.8a item 3)
            let i = r.mgmt().inst.as_ref();
            let state = match r.app {
                App::Stock => "stock",
                App::Nothing => "nothing",
                App::Own if i.map_or(false, |i| i.erased()) => "writing",
                App::Own if i.is_some() => "before-erase",
                App::Own => "own",
            };
            ok(J::obj(vec![
                ("state", J::Str(state.into())),
                ("sectors", J::Num(i.map_or(0, |i| i.sectors) as f64)),
                ("bytes_written", J::Num(i.map_or(0, |i| i.flash.len()) as f64)),
                ("secret_stored", J::Bool(r.mgmt().secret.is_some())),
            ]))
        }
        "vendor.install" => {
            // model only: the vendor path through the controller puts this firmware back
            // (A1.8a item 9): a fresh radio without a secret, its first-secret window open
            let cfg = r.cfg.clone();
            *r = Radio::new(cfg, now);
            ok(J::Null)
        }
        "link.set" => {
            let sp = args.get("speed").and_then(|v| v.as_f64()).unwrap_or(100.0) as u16;
            r.link_mbit = sp;
            ok(J::Null)
        }
        "key.log" => {
            // format v1.5: key changes and High Priority arrivals, paged, capped
            const CAP: usize = 100_000;
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("read");
            if action == "start" {
                r.keylog = Some(Default::default());
                return ok(J::obj(vec![("t_us", J::Num(now as f64))]));
            }
            let Some(l) = r.keylog.as_ref() else { return err("key.log not started") };
            let from_k = args.get("from_key").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
            let from_h = args.get("from_hp").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
            const PAGE: usize = 2000;
            let key: Vec<J> = l.key.iter().skip(from_k).take(PAGE)
                .map(|(t, on)| {
                    // the model's other RF signals follow the key: T/R lines at TX levels and
                    // a non-zero bias while keyed (v1.5 "tr", "bias_nonzero")
                    let l = |b: bool| J::Num(b as u8 as f64);
                    J::obj(vec![("t_us", J::Num(*t as f64)), ("on", l(*on)),
                                ("tr", J::obj(vec![("PF4", l(!*on)), ("PF6", l(*on)), ("PC7", l(*on)), ("PH11", l(*on))])),
                                ("bias_nonzero", J::Bool(*on))])
                }).collect();
            let hp: Vec<J> = l.hp.iter().skip(from_h).take(PAGE)
                .map(|(t, seq, b4, b5)| J::obj(vec![("t_us", J::Num(*t as f64)), ("seq", J::Num(*seq as f64)),
                                                    ("b4", J::Num(*b4 as f64)), ("b5", J::Num(*b5 as f64))])).collect();
            let dropped = l.key.len().saturating_sub(CAP) + l.hp.len().saturating_sub(CAP);
            let v = J::obj(vec![
                ("t_us", J::Num(now as f64)),
                ("key", J::Arr(key)), ("hp", J::Arr(hp)),
                ("count_key", J::Num(l.key.len().min(CAP) as f64)), ("count_hp", J::Num(l.hp.len().min(CAP) as f64)),
                ("dropped", J::Num(dropped as f64)),
            ]);
            if action == "stop" {
                r.keylog = None;
            }
            ok(v)
        }
        "log.corrupt" => {
            // model only: the entry with this sequence number fails its check at the next start
            let n = args.get("seq").and_then(|v| v.as_f64()).unwrap_or(-1.0);
            if !r.log.iter().any(|e| e.0 as f64 == n) {
                return err("log.corrupt: no such entry");
            }
            r.log_corrupt = Some(n as u32);
            ok(J::Null)
        }
        "stop.sim" => {
            // model only: the firmware stops by itself (A3.6); `log` false: its line never reaches the log
            if !r.not_this_firmware() {
                let log = args.get("log").map_or(true, |v| *v != J::Bool(false));
                r.stop(if log { Some("restart: stop 1") } else { None });
            }
            ok(J::Null)
        }
        "c3.lose" => {
            // model only: the C-3 record cannot be read from the next start on, as after a double ECC
            // error in it (0072); C-3 is then lost until a SET_CONFIG of every C-3 item
            r.c3_unreadable = true;
            ok(J::Null)
        }
        "store.full" => {
            // model only: the next UPDATE_BLOCK or UPDATE_VERIFY finds no room in the store
            r.store_full = true;
            ok(J::Null)
        }
        "reset.flags" => {
            // model only: the reset flags of the next start, e.g. [] or ["software", "power-on"]
            let Some(J::Arr(a)) = args.get("flags") else { return err("reset.flags: flags missing") };
            r.next_flags = Some(a.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect());
            ok(J::Null)
        }
        "flash.delay" => {
            // model only: the flash work of the next `count` requests takes `us` (0069)
            let us = args.get("us").and_then(|v| v.as_f64()).unwrap_or(0.0) as u64;
            r.flash_delay = (us, args.get("count").and_then(|v| v.as_f64()).unwrap_or(1.0) as u32);
            ok(J::Null)
        }
        "slot.corrupt" => {
            // model only: one byte of the image in slot (1 A, 2 B) changed; the next judgement
            // of that slot fails (0067 item 4: at a start, a ROLLBACK, or the boot stage)
            let s = args.get("slot").and_then(|v| v.as_f64()).unwrap_or(0.0) as usize;
            match r.fw.slots.get_mut(s.wrapping_sub(1)).and_then(|i| i.as_mut()) {
                Some(i) => {
                    // restore: the byte back as it was (the next judgement passes again)
                    i.corrupt = !args.get("restore").map_or(false, |v| *v == J::Bool(true));
                    ok(J::Null)
                }
                None => err("slot.corrupt: no image in that slot"),
            }
        }
        "trial.sim" => {
            // model only: the next trial image keeps its network down (no self-confirmation)
            // the next trial image keeps its network down, or carries a class 2 fault: neither
            // confirms itself (0044)
            r.fw.sim_net_down = args.get("network_down").map_or(false, |v| *v == J::Bool(true))
                || args.get("class2_fault").map_or(false, |v| *v == J::Bool(true));
            ok(J::Null)
        }
        "atu.sim" => {
            // model only: how the next ATU tune cycle ends (result "tuned", "failed" or "none",
            // after_ms) and the ATU tune time (tune_time_ms; pending hardware, P-ATU-1)
            let res = args.get("result").and_then(|v| v.as_str()).unwrap_or("tuned");
            r.atu.sim_result = match res {
                "tuned" => Some(true),
                "failed" => Some(false),
                "none" => None,
                _ => return err(&format!("atu.sim: result {res}")),
            };
            if let Some(ms) = args.get("after_ms").and_then(|v| v.as_f64()) {
                r.atu.sim_after_us = (ms * 1000.0) as u64;
            }
            if let Some(ms) = args.get("tune_time_ms").and_then(|v| v.as_f64()) {
                r.atu.tune_time_us = (ms * 1000.0) as u64;
            }
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
