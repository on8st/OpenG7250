// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Milestone F2: CH394 on SPI6, IP configuration, G7250-P2 discovery on port 1024.
//!
//! Pins and bus: [HIS] HIS 40 (SCK PG13, MISO PG12, MOSI PG14, AF5, mode 0; CS# PG15; RST# PB5;
//! INT# PC13, not used yet: the driver polls). Sockets: spec Table B2-1; socket 0 is port 1024.
//! Socket 7 (the management socket, port 7251) is used for DHCP at power-on, before the P2
//! sockets are opened, and borrowed for each later exchange, at most 2 s (A1.11; 0041).

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::mode::Blocking;
use embassy_stm32::spi::{self, Spi};
use embassy_stm32::time::Hertz;
use embassy_time::{Duration, Instant, Timer};
use embedded_hal::spi::{ErrorType, Operation, SpiBus, SpiDevice};
use g7250_board::println;
use g7250_ch394::{Ch394, Error as ChError};
use g7250_proto::discovery::{self, Identity, Port1024, Status};
use g7250_proto::netcfg::{self, Ip, Mode, NetConfig, Source};
use g7250_proto::ratelimit::RateLimit;
use g7250_proto::dhcp;
use g7250_proto::mgmt as m;
use g7250_radio::dhcp_client::{Action as DhcpAction, Client as DhcpClient};
use g7250_radio::mgmt::{Ctx, Server};

// DMA rule for later (RM0468): SPI6 is in the D3 domain and can use only the BDMA, which reaches
// only D3 memory. When this driver moves from blocking transfers to DMA, its buffers MUST be in
// SRAM4 (0x38000000, section `.sram4` of memory-slot-*.x), never in AXI or D2 SRAM (transfer
// error) and never in the Backup SRAM at 0x38800000 ([HIS] HIS 15 rule 4).

/// SPI clock. [HIS] HIS 40 (`6e2208a`): the CH394 takes up to 62 MHz in mode 0 or 3 (datasheet,
/// VERIFIED), but about 17 MHz seen on the board link (MEDIUM) is the safe bound until the link
/// is measured (P-NET-1). SPI6 kernel clock is PCLK4 = 64 MHz here; 64 / 4 = 16 MHz.
const SPI_HZ: u32 = 16_000_000;
const SOCK_DISCOVERY: u8 = 0;
const SOCK_MGMT: u8 = crate::p2::S7251;

/// SPI device over the HAL bus with a GPIO chip select.
///
/// Every operation runs full duplex: one frame received for every frame sent. RM0468: the SPI6
/// receive FIFO holds only a few frames, and in full-duplex mode a frame that arrives while it is
/// full sets OVR. The HAL's `write` sends without reading back (it leaves OVR set on purpose),
/// so a write followed by a read in one chip-select failed with `Overrun` (first emulator run,
/// 2026-11-21: "cannot open port 1024 (Spi(Overrun))"). Writes therefore go through
/// `transfer_in_place` on a copy, reads send zeros, and OVR is cleared before each transaction
/// and after an error.
pub struct CsDevice<'d> {
    bus: Spi<'d, Blocking, spi::mode::Master>,
    cs: Output<'d>,
}

/// Clear OVR (and the other error flags) of SPI6: SPI_IFCR, RM0468.
fn spi6_clear_errors() {
    embassy_stm32::pac::SPI6.ifcr().write(|w| {
        w.set_ovrc(true);
        w.set_udrc(true);
        w.set_modfc(true);
        w.set_eotc(true);
        w.set_txtfc(true);
    });
}

impl ErrorType for CsDevice<'_> {
    type Error = spi::Error;
}

impl SpiDevice for CsDevice<'_> {
    fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), spi::Error> {
        spi6_clear_errors();
        self.cs.set_low();
        let mut r = Ok(());
        for op in ops.iter_mut() {
            r = match op {
                Operation::Read(b) => {
                    b.fill(0);
                    SpiBus::transfer_in_place(&mut self.bus, b)
                }
                Operation::Write(b) => {
                    let mut tmp = [0u8; 64];
                    let mut res = Ok(());
                    for chunk in b.chunks(tmp.len()) {
                        let t = &mut tmp[..chunk.len()];
                        t.copy_from_slice(chunk);
                        res = SpiBus::transfer_in_place(&mut self.bus, t);
                        if res.is_err() {
                            break;
                        }
                    }
                    res
                }
                Operation::Transfer(rd, wr) => SpiBus::transfer(&mut self.bus, rd, wr),
                Operation::TransferInPlace(b) => SpiBus::transfer_in_place(&mut self.bus, b),
                Operation::DelayNs(_) => Ok(()),
            };
            if r.is_err() {
                break;
            }
        }
        let f = SpiBus::<u8>::flush(&mut self.bus);
        self.cs.set_high();
        if r.is_err() || f.is_err() {
            spi6_clear_errors();
        }
        r.and(f)
    }
}

pub struct NetPins<'d> {
    pub spi: embassy_stm32::Peri<'d, embassy_stm32::peripherals::SPI6>,
    pub sck: embassy_stm32::Peri<'d, embassy_stm32::peripherals::PG13>,
    pub mosi: embassy_stm32::Peri<'d, embassy_stm32::peripherals::PG14>,
    pub miso: embassy_stm32::Peri<'d, embassy_stm32::peripherals::PG12>,
    pub cs: embassy_stm32::Peri<'d, embassy_stm32::peripherals::PG15>,
    pub rst: embassy_stm32::Peri<'d, embassy_stm32::peripherals::PB5>,
}

type Chip = Ch394<CsDevice<'static>>;

pub fn ip_str(ip: Ip) -> heapless::String<16> {
    use core::fmt::Write;
    let mut s = heapless::String::new();
    let _ = write!(s, "{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]);
    s
}

/// A MAC address for the link. HIS 40: the CH394 provides one in its own register set; whether
/// it is unique per unit is not verified (HIS-15). If the chip holds none (all zero or all
/// ones), derive a locally administered one from the MCU's 96-bit unique ID (RM0468, "Unique
/// device ID register", 0x1FF1E800) and write it to the chip.
fn mac_or_derived(chip: &mut Chip) -> Result<([u8; 6], bool), ChError<spi::Error>> {
    let m = chip.mac()?;
    if m != [0; 6] && m != [0xFF; 6] && m[0] & 1 == 0 {
        return Ok((m, false));
    }
    // SAFETY: the unique ID is a read-only system memory area, always mapped.
    let uid: [u32; 3] = unsafe { core::ptr::read_volatile(0x1FF1_E800 as *const [u32; 3]) };
    let mut h: u32 = 0x811C_9DC5;
    for w in uid {
        for b in w.to_le_bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(0x0100_0193);
        }
    }
    let x = (uid[0] ^ uid[1].rotate_left(11) ^ uid[2].rotate_left(23)).to_be_bytes();
    let d = [0x02, x[1] ^ (h >> 24) as u8, x[2], x[3], (h >> 8) as u8, h as u8];
    chip.write(g7250_ch394::bsb::COMMON, g7250_ch394::common::SHAR, &d)?;
    Ok((d, true))
}

async fn bring_up(pins: NetPins<'static>) -> Result<Chip, ChError<spi::Error>> {
    // RST# has been low since the safe state at start (HIS 85: held until the product brings it
    // up). Keep it low a while longer, then release. [HIS] HIS 40 (`d199d33`): the chip must not
    // be accessed 12-19 ms after RST# goes high (VERIFIED, CH394DS1 10.4.3); the minimum low
    // time is still open. 10 ms low (after the time since start) and 200 ms before the first
    // access are well outside both.
    let mut rst = Output::new(pins.rst, Level::Low, Speed::Low);
    Timer::after_millis(10).await;
    rst.set_high();
    core::mem::forget(rst); // keep the pin driven high for the rest of the run
    Timer::after_millis(200).await;

    let mut cfg = spi::Config::default();
    cfg.frequency = Hertz(SPI_HZ);
    cfg.mode = spi::MODE_0;
    let bus = Spi::new_blocking(pins.spi, pins.sck, pins.mosi, pins.miso, cfg);
    let cs = Output::new(pins.cs, Level::High, Speed::VeryHigh);
    let mut chip = Ch394::new(CsDevice { bus, cs });
    let mut last = Err(ChError::NoChip(0));
    for _ in 0..10 {
        last = chip.probe();
        if last.is_ok() {
            break;
        }
        Timer::after_millis(100).await;
    }
    let v = last?;
    println!("net: CH394 answers, version register {:#04x}", v);
    Ok(chip)
}

async fn wait_link(chip: &mut Chip) -> g7250_ch394::Link {
    let start = Instant::now();
    loop {
        match chip.link() {
            Ok(l) if l.up => return l,
            Ok(l) if Instant::now() - start > Duration::from_secs(5) => return l,
            _ => Timer::after_millis(100).await,
        }
    }
}

/// DHCP at power-on (A1.11): DISCOVER, OFFER, REQUEST, ACK on socket 7 port 68.
async fn dhcp(chip: &mut Chip, mac: [u8; 6], xid: u32) -> Option<(NetConfig, Option<u32>, Ip)> {
    chip.set_ip([0; 4], [0; 4], [0; 4]).ok()?;
    chip.udp_open(SOCK_MGMT, dhcp::CLIENT_PORT).ok()?;
    let mut msg = [0u8; dhcp::MSG_LEN];
    let mut rx = [0u8; 600];
    let result = 'outer: {
        for attempt in 0..3u32 {
            let x = xid.wrapping_add(attempt);
            let n = dhcp::discover(x, mac, &mut msg);
            if chip.udp_send(SOCK_MGMT, [255; 4], dhcp::SERVER_PORT, &msg[..n]).is_err() {
                continue;
            }
            let Some(offer) = wait_reply(chip, &mut rx, x, mac, dhcp::OFFER).await else { continue };
            let n = dhcp::request(x, mac, offer.yiaddr, offer.server, &mut msg);
            if chip.udp_send(SOCK_MGMT, [255; 4], dhcp::SERVER_PORT, &msg[..n]).is_err() {
                continue;
            }
            match wait_reply(chip, &mut rx, x, mac, dhcp::ACK).await {
                Some(ack) => {
                    let mask = ack.mask.or(offer.mask).unwrap_or([255, 255, 255, 0]);
                    let gw = ack.router.or(offer.router).unwrap_or([0; 4]);
                    if netcfg::valid_host(ack.yiaddr, mask) {
                        let server = if ack.server == [0; 4] { offer.server } else { ack.server };
                        break 'outer Some((NetConfig { mode: Mode::Dhcp, ip: ack.yiaddr, mask, gateway: gw }, ack.lease_s, server));
                    }
                }
                None => continue,
            }
        }
        None
    };
    let _ = chip.udp_close(SOCK_MGMT);
    result
}

async fn wait_reply(chip: &mut Chip, rx: &mut [u8], xid: u32, mac: [u8; 6], want: u8) -> Option<dhcp::Reply> {
    let start = Instant::now();
    while Instant::now() - start < Duration::from_millis(g7250_radio::dhcp_client::WAIT_MS) {
        if let Ok(Some(d)) = chip.udp_recv(SOCK_MGMT, rx) {
            if let Some(r) = dhcp::parse(&rx[..d.len], xid, mac) {
                if r.msg_type == want {
                    return Some(r);
                }
                if r.msg_type == dhcp::NAK {
                    return None;
                }
            }
        } else {
            Timer::after_millis(5).await;
        }
    }
    None
}

/// Bring up the address: static, or DHCP with link-local fall-back. In DHCP mode the returned
/// client goes on in the main loop: renewal, rebinding, and retries while link-local (A1.11).
async fn configure(chip: &mut Chip, mac: [u8; 6], want: NetConfig) -> (NetConfig, Source, Option<DhcpClient>) {
    if want.mode == Mode::Static && netcfg::valid_host(want.ip, want.mask) {
        let _ = chip.set_ip(want.ip, want.mask, want.gateway);
        return (want, Source::Static, None);
    }
    let xid = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ Instant::now().as_ticks() as u32;
    let ll = netcfg::link_local(mac, 0);
    let found = dhcp(chip, mac, xid).await;
    // The client's timers start now, after the power-on exchange: started before it, the first
    // retry fell due at once and borrowed socket 7 just as the management port opened.
    let now = Instant::now().as_millis();
    if let Some((c, lease, server)) = found {
        let _ = chip.set_ip(c.ip, c.mask, c.gateway);
        let l = lease.unwrap_or(3600).max(60);
        println!("net: DHCP lease {} s from {}; renewal at {} s", l, ip_str(server).as_str(), l / 2);
        return (c, Source::Dhcp, Some(DhcpClient::new(mac, xid.wrapping_add(16), ll, Some((c, server, l)), now)));
    }
    let _ = chip.set_ip(ll.ip, ll.mask, ll.gateway);
    println!("net: no DHCP lease; link-local, DHCP keeps trying (A1.11)");
    (ll, Source::LinkLocal, Some(DhcpClient::new(mac, xid.wrapping_add(16), ll, None, now)))
}

/// One pass of the DHCP client after the power-on exchange (A1.11; 0041): retries while
/// link-local, renewal, rebinding. Socket 7 is borrowed (port 68) from a send until the client
/// stops listening, at most [`g7250_radio::dhcp_client::WAIT_MS`], and then always given back to
/// the management port, for a finished exchange as for an abandoned one.
fn dhcp_step(chip: &mut Chip, c: &mut DhcpClient, borrowed: &mut bool, cfg: &mut NetConfig, p2: &mut crate::p2::P2, buf: &mut [u8], now: u64) {
    let mut actions: heapless::Vec<DhcpAction, 2> = heapless::Vec::new();
    if *borrowed {
        if let Ok(Some(d)) = chip.udp_recv(SOCK_MGMT, buf) {
            let _ = actions.push(c.on_reply(&buf[..d.len], now));
        }
    }
    let _ = actions.push(c.poll(now));
    for a in actions {
        match a {
            DhcpAction::None => {}
            DhcpAction::Send { to, msg } => {
                if !*borrowed {
                    let _ = chip.udp_close(SOCK_MGMT);
                    *borrowed = chip.udp_open(SOCK_MGMT, dhcp::CLIENT_PORT).is_ok();
                }
                let mut b = [0u8; dhcp::MSG_LEN];
                let n = msg.encode(c.xid, c.mac(), &mut b);
                let _ = chip.udp_send(SOCK_MGMT, to, dhcp::SERVER_PORT, &b[..n]);
            }
            DhcpAction::Use(new) => {
                if new.ip != cfg.ip || new.mask != cfg.mask {
                    // Another address: a C-1 change under A1.11a: the session ends
                    // (SESSION_CHANGED while armed), nothing is carried over.
                    p2.address_changed(chip, now);
                    let _ = chip.set_ip(new.ip, new.mask, new.gateway);
                    print_cfg(&new, if new.ip[..2] == [169, 254] { Source::LinkLocal } else { Source::Dhcp });
                } else if new.gateway != cfg.gateway {
                    let _ = chip.set_ip(new.ip, new.mask, new.gateway);
                }
                *cfg = new;
            }
        }
    }
    if *borrowed && !c.listening() {
        *borrowed = false;
        // udp_open closes first; a failed open is tried again on the next pass.
        if chip.udp_open(SOCK_MGMT, m::PORT).is_err() {
            *borrowed = true;
            println!("net: FAULT cannot give socket 7 back to the management port; retrying");
        }
    }
}

fn print_cfg(c: &NetConfig, s: Source) {
    println!(
        "net: address {} mask {} gateway {} ({:?})",
        ip_str(c.ip).as_str(),
        ip_str(c.mask).as_str(),
        ip_str(c.gateway).as_str(),
        s
    );
}

#[embassy_executor::task]
pub async fn run(pins: NetPins<'static>, id: Identity, mut mgmt: crate::mgmt_init::MgmtInit, tx: g7250_safety::tx::TxCore) {
    let mut chip = match bring_up(pins).await {
        Ok(c) => c,
        Err(e) => {
            println!("net: FAULT no answer from the CH394 ({:?}); network stays down", e);
            return;
        }
    };
    let (mac, derived) = match mac_or_derived(&mut chip) {
        Ok(m) => m,
        Err(e) => {
            println!("net: FAULT reading the MAC address ({:?})", e);
            return;
        }
    };
    println!(
        "net: MAC {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}{}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
        if derived { " (derived from the MCU unique ID: the CH394 held none)" } else { " (from the CH394)" }
    );
    let link = wait_link(&mut chip).await;
    println!("net: link {} Mbit/s {}", link.speed(), if link.full_duplex { "full duplex" } else { "half duplex" });

    let (mut cfg, src, mut dhcpc) = configure(&mut chip, mac, NetConfig::FACTORY).await;
    // Socket 7 lent to DHCP (port 68) for a running exchange (A1.11).
    let mut dhcp_borrowed = false;
    print_cfg(&cfg, src);

    // All P2 sockets of Table B2-1 (1024-1027, 1029, 1035, 7250); 1028 is never opened (0017).
    if let Err(e) = crate::p2::P2::open(&mut chip) {
        println!("net: FAULT cannot open the P2 ports ({:?})", e);
        return;
    }
    println!("net: discovery listening on UDP 1024 (board G7250-P2, type {})", discovery::BOARD_TYPE);
    println!("net: P2 ports 1025 1026 1027 1029 1035 7250 open");
    let rec = g7250_radio::record::build(&g7250_radio::record::Params {
        fw_version: g7250_board::FW_VERSION,
        tx_implemented: g7250_safety::tx::TX_IMPLEMENTED,
        tx_calibrated: tx.calibrated(),
        rate_384: false, // P-NET-1: offered only once >= 50 MHz SPI is shown
        // DDC0 offered only with the FPGA receive parameters (P-RX-2, through the bus gate).
        rx_offered: g7250_safety::fpga_gate::permit().is_some(),
    });
    let mut p2 = crate::p2::P2::new(tx, g7250_radio::regmap::RegMap::new(rec));
    p2.set_link(link);
    // Management channel on socket 7 (A1.2, A1.3), after DHCP has given the socket back.
    if let Err(e) = chip.udp_open(SOCK_MGMT, m::PORT) {
        println!("net: FAULT cannot open the management port {} ({:?})", m::PORT, e);
        return;
    }
    let mut server = Server::new();
    server.network_up(Instant::now().as_millis());
    mgmt.identify.mac = mac;
    // Boot record at start (A1.8 U-6): a rollback is logged; a trial cannot arm until confirmed.
    let slot_name = if mgmt.update.run.slot == g7250_layout::Slot::A { "A" } else { "B" };
    match mgmt.update.state {
        g7250_update::boot::UpdateState::RolledBack => {
            server.log.push(Instant::now().as_millis(), format_args!("rollback: the trial image was not confirmed; running slot {}", slot_name));
            println!("net: running slot {} after a rollback (trial not confirmed)", slot_name);
        }
        g7250_update::boot::UpdateState::Trial => {
            server.log.push(Instant::now().as_millis(), format_args!("trial boot of slot {}", slot_name));
            println!("net: trial boot of slot {}: arm refused (MAINTENANCE) until confirmed", slot_name);
        }
        g7250_update::boot::UpdateState::Normal => {}
    }
    p2.trial = mgmt.update.run.trial;
    let loop_start = Instant::now().as_millis();
    let mut last_confirm_try: u64 = 0;
    println!("net: management listening on UDP {} (protocol {}.{})", m::PORT, m::VERSION_MAJOR, m::VERSION_MINOR);
    let mut mrx = [0u8; 1100];
    let mut mtx = [0u8; m::MAX_REPLY];

    let id = Identity { mac, ..id };
    let mut limit: RateLimit<16> = RateLimit::new(10, 50);
    let mut rx = [0u8; 1444];
    let mut dropped_1024: u32 = 0;
    let mut answered: u32 = 0;
    let mut last_link = link;
    loop {
        // Session watchdog and status first, on every pass, so that no branch below can delay
        // the standby beyond B5.6's 1100 ms (T-B5-10).
        p2.tick(&mut chip, Instant::now().as_millis());
        // DHCP after the power-on exchange (A1.11), also on every pass: no traffic on the other
        // sockets can hold a borrow of socket 7 past its 2 s (0041).
        if let Some(c) = dhcpc.as_mut() {
            dhcp_step(&mut chip, c, &mut dhcp_borrowed, &mut cfg, &mut p2, &mut mrx, Instant::now().as_millis());
        }
        // Trial boot: self-confirmation by the criterion of 0044 (network up: this loop runs only
        // then). The 60 s limit is in the main task. A failed write is tried again once a second.
        if mgmt.update.run.trial {
            let now = Instant::now().as_millis();
            if crate::update::may_confirm(now, loop_start) && now.saturating_sub(last_confirm_try) >= 1000 {
                last_confirm_try = now;
                if mgmt.update.confirm(mgmt.store.as_mut()) {
                    p2.trial = false;
                    mgmt.identify.update_state = 0;
                    server.log.push(now, format_args!("trial image confirmed itself"));
                    println!("net: trial image confirmed (slot {})", slot_name);
                }
            }
        }
        // An update without an update command for 10 min is abandoned (A1.8; T-A1-34, T-A1-46):
        // staging discarded, logged, restart into the running image (no RF in this build).
        if mgmt.update.stager.abandoned(Instant::now().as_millis()) {
            mgmt.update.stager.abort(Instant::now().as_millis());
            server.log.push(Instant::now().as_millis(), format_args!("update abandoned after 10 min; restart into the running image"));
            println!("net: update abandoned (10 min); restarting into the running image");
            crate::safe_reset(&mut mgmt, false);
        }
        match chip.udp_recv(SOCK_DISCOVERY, &mut rx) {
            // General packet on 1024: byte 4 = 0x00 (B11.4; Table B11-1).
            Ok(Some(d)) if d.len >= 5 && rx[4] == 0x00 && d.src_port != discovery::PORT => {
                let now = Instant::now().as_millis();
                p2.cnc(&mut chip, (d.src_ip, d.src_port), g7250_radio::session::CnC::General, &rx[..d.len], now);
            }
            Ok(Some(d)) => {
                let now = Instant::now().as_millis();
                match discovery::classify(&rx[..d.len], d.src_port) {
                    Port1024::Discovery => {
                        if limit.allow(d.src_ip, d.src_port, now) {
                            // Byte 4: 0x03 while running for another host (B3.4).
                            let st = if p2.discovery_from((d.src_ip, d.src_port)) { Status::InUse } else { Status::Available };
                            let r = discovery::reply(&id, st);
                            if chip.udp_send(SOCK_DISCOVERY, d.src_ip, d.src_port, &r).is_ok() {
                                answered = answered.wrapping_add(1);
                                server.stats.discovery_answered += 1;
                                println!("net: discovery from {}:{} answered ({})", ip_str(d.src_ip).as_str(), d.src_port, answered);
                            }
                        }
                    }
                    Port1024::SetIp { mac: target, ip } if target == mac => {
                        // B3.9. The CH394 does not report the destination address of a datagram,
                        // so a limited broadcast cannot be told from a unicast or directed one
                        // (spec question SQ-F2-2). Interim rule: accept from inside the subnet,
                        // or from anywhere while on a link-local address.
                        let ok_src = netcfg::in_subnet(cfg.ip, cfg.mask, d.src_ip) || cfg.ip[..2] == [169, 254];
                        if !ok_src {
                            println!("net: Set IP from {} outside the subnet ignored", ip_str(d.src_ip).as_str());
                            server.stats.set_ip_ignored += 1;
                        } else if limit.allow(d.src_ip, d.src_port, now) {
                            println!("net: Set IP {} from {} (kept in RAM until restart: no configuration store yet)", ip_str(ip).as_str(), ip_str(d.src_ip).as_str());
                            let r = discovery::reply(&id, Status::Available);
                            let _ = chip.udp_send(SOCK_DISCOVERY, d.src_ip, d.src_port, &r);
                            let want = if ip == [0; 4] {
                                NetConfig::FACTORY
                            } else {
                                netcfg::after_set_ip(&cfg, ip)
                            };
                            // A new address ends a running session (A1.6; B3.9;
                            // SESSION_CHANGED, Table A3-1 event 9) and nothing is carried over.
                            p2.address_changed(&mut chip, now);
                            let _ = chip.udp_close(SOCK_DISCOVERY);
                            let (c, s, d) = configure(&mut chip, mac, want).await;
                            cfg = c;
                            dhcpc = d;
                            dhcp_borrowed = false;
                            print_cfg(&cfg, if s == Source::Static { Source::SetIp } else { s });
                            // DHCP may have borrowed socket 7 (A1.11): reopen everything.
                            if crate::p2::P2::open(&mut chip).is_err() || chip.udp_open(SOCK_MGMT, m::PORT).is_err() {
                                println!("net: FAULT cannot reopen the P2 and management ports");
                                return;
                            }
                        }
                    }
                    Port1024::SetIp { .. } | Port1024::IgnoredProgramming | Port1024::Other => {}
                    Port1024::FromPort1024 => {
                        dropped_1024 = dropped_1024.wrapping_add(1);
                        server.stats.discovery_from_port_1024 += 1;
                    }
                }
                server.stats.discovery_dropped_rate = limit.dropped as u64;
            }
            Ok(None) => {
                // P2 sockets: DDC-specific, DUC-specific, High Priority, DUC I&Q, registers.
                let mut handled = false;
                for (sock, kind) in [
                    (crate::p2::S1025, Some(g7250_radio::session::CnC::DdcSpecific)),
                    (crate::p2::S1026, Some(g7250_radio::session::CnC::DucSpecific)),
                    (crate::p2::S1027, Some(g7250_radio::session::CnC::HighPriority)),
                    (crate::p2::S1029, None),
                    (crate::p2::S7250, None),
                ] {
                    if let Ok(Some(d)) = chip.udp_recv(sock, &mut rx) {
                        let now = Instant::now().as_millis();
                        let from = (d.src_ip, d.src_port);
                        match (sock, kind) {
                            (_, Some(k)) => p2.cnc(&mut chip, from, k, &rx[..d.len], now),
                            (crate::p2::S7250, None) => p2.register_packet(&mut chip, from, &rx[..d.len], now),
                            // DUC I&Q: discarded whole and counted while record tx bit 4 is 0, and
                            // never enters a FIFO while not the owner's (B10.7; 0039).
                            _ => p2.duc_iq_ignored += 1,
                        }
                        handled = true;
                    }
                }
                p2.tick(&mut chip, Instant::now().as_millis());
                if handled {
                    continue;
                }
                // Management channel, socket 7 (not while lent to DHCP).
                if dhcp_borrowed {
                    Timer::after_millis(2).await;
                    continue;
                }
                if let Ok(Some(d)) = chip.udp_recv(SOCK_MGMT, &mut mrx) {
                    let now = Instant::now().as_millis();
                    let mut ident = mgmt.identify;
                    ident.uptime_ms = now as u32;
                    ident.tx_arm_state = p2.tx.arm_state_reg();
                    ident.owner = p2.session.owner().unwrap_or(([0; 4], 0));
                    let mut rng = |c: &mut [u8; 16]| crate::nv::rng_fill(c);
                    let mut ctx = Ctx {
                        now_ms: now,
                        own_ip: cfg.ip,
                        own_mask: cfg.mask,
                        identify: &ident,
                        config: &mgmt.config,
                        calibration: &mgmt.calibration,
                        rng: &mut rng,
                        secret: mgmt.secret.as_ref(),
                        p2_session: p2.session.running(),
                        armed: p2.tx.state() != g7250_safety::tx::ArmState::Disarmed,
                        update: &mut crate::update::Port { u: &mut mgmt.update, store: mgmt.store.as_mut() },
                    };
                    // A datagram longer than the buffer arrives cut off; the header still decides
                    // (A1.4: 19 for a declared length above 1000, else 17 for the length mismatch).
                    let reply = server.handle((d.src_ip, d.src_port), &mrx[..d.len], &mut ctx, &mut mtx);
                    let effects = server.take_effects();
                    // Store writes before the reply (A1.6: stored with a check value before it
                    // counts); network changes and resets after it (A1.6, A1.9).
                    let mut after: heapless::Vec<g7250_radio::mgmt::Effect, 4> = heapless::Vec::new();
                    for e in effects {
                        match crate::mgmt_init::apply_stored(&mut mgmt, e) {
                            Ok(None) => {}
                            Ok(Some(later)) => {
                                let _ = after.push(later);
                            }
                            Err(msg) => println!("net: FAULT store write failed ({})", msg),
                        }
                    }
                    if let Some(n) = reply {
                        let _ = chip.udp_send(SOCK_MGMT, d.src_ip, d.src_port, &mtx[..n]);
                    }
                    for e in after {
                        match e {
                            g7250_radio::mgmt::Effect::ApplyNetwork => {
                                let want = crate::mgmt_init::network_from_config(&mgmt.config);
                                let now = Instant::now().as_millis();
                                p2.address_changed(&mut chip, now);
                                let _ = chip.udp_close(SOCK_DISCOVERY);
                                let (c, s, dc) = configure(&mut chip, mac, want).await;
                                cfg = c;
                                dhcpc = dc;
                                dhcp_borrowed = false;
                                print_cfg(&cfg, s);
                                if crate::p2::P2::open(&mut chip).is_err() || chip.udp_open(SOCK_MGMT, m::PORT).is_err() {
                                    println!("net: FAULT cannot reopen the P2 and management ports");
                                    return;
                                }
                            }
                            g7250_radio::mgmt::Effect::Restart | g7250_radio::mgmt::Effect::FactoryReset => {
                                // Commanded reset (A1.9): unkey and disarm (never armed in this
                                // build), RF gone (never keyed), reply sent (above), relays to the
                                // A3.6 state, then the reset. A factory reset writes the items of
                                // Table A1-4 only now, between steps 4 and 5 (A1.6a).
                                let factory = e == g7250_radio::mgmt::Effect::FactoryReset;
                                Timer::after_millis(20).await; // let the reply leave the CH394
                                crate::safe_reset(&mut mgmt, factory);
                            }
                            g7250_radio::mgmt::Effect::UpdateStarted => {
                                // A1.8: the session ends, streams stop, arms get MAINTENANCE,
                                // until a restart.
                                p2.enter_update(Instant::now().as_millis());
                            }
                            _ => {}
                        }
                    }
                    continue;
                }
                if let Ok(irq) = chip.take_irq() {
                    if irq & g7250_ch394::common::IR_CONFLICT != 0 {
                        println!("net: address conflict on {} (fall-back of C-4 not implemented yet)", ip_str(cfg.ip).as_str());
                    }
                }
                if let Ok(l) = chip.link() {
                    if l != last_link {
                        println!("net: link {} Mbit/s", l.speed());
                        last_link = l;
                        p2.set_link(l); // a session ends on link loss or < 100 Mbit/s (B5.8)
                    }
                }
                Timer::after_millis(2).await;
            }
            Err(e) => {
                println!("net: receive error {:?}", e);
                Timer::after_millis(100).await;
            }
        }
    }
}
