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
use g7250_proto::ipv4ll;
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
/// Socket receive and transmit buffers, KiB, sockets 0-7 = ports 1024, 1025, 1026, 1027, 1029,
/// 1035, 7250, 7251 (from 0004 "CH394Q buffer split", B2.3; the normative split goes to part B).
/// - 1027 (High Priority) receives 4 KiB: two packets back to back (a stop and a new run,
///   2 x 1452 bytes) both fit (T-B10-L13).
/// - 1029 (DUC I&Q) receives 4 KiB, two packets: the FIFO of B10.7 and B11.7 is the firmware's
///   (20 packets, P-TIM-2); the chip only holds packets until the next read.
/// - 1024 receives 1 KiB (discovery and General packets are 60 bytes); 7251 receives 2 KiB
///   (requests up to 1000 bytes) and sends 2 KiB (replies up to 1400 bytes).
///
/// Every size is a power of two: the socket pointers are 16 bit and wrap at 65536, and the
/// datasheet does not say how a wrap maps onto a buffer whose size does not divide 65536
/// ([CH394] 7.2.12-13 only bound the sizes). Checked at compile time below.
const BUF_RX_KIB: [u8; 8] = [1, 2, 1, 4, 4, 0, 2, 2];
const BUF_TX_KIB: [u8; 8] = [1, 1, 1, 0, 0, 8, 2, 2];
const _: () = {
    let mut i = 0;
    let (mut rx, mut tx) = (0u32, 0u32);
    while i < 8 {
        assert!(BUF_RX_KIB[i] == 0 || BUF_RX_KIB[i].is_power_of_two(), "socket buffers: powers of two only");
        assert!(BUF_TX_KIB[i] == 0 || BUF_TX_KIB[i].is_power_of_two(), "socket buffers: powers of two only");
        rx += BUF_RX_KIB[i] as u32;
        tx += BUF_TX_KIB[i] as u32;
        i += 1;
    }
    assert!(rx <= 16 && tx <= 16, "socket buffers: 16 KiB per direction");
};
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
    // The socket buffers ([BUF_RX_KIB], [BUF_TX_KIB]); the CH394 has no flow control in UDP mode,
    // so a packet that does not fit is lost (T-B10-L13).
    chip.set_buffers(BUF_RX_KIB, BUF_TX_KIB)?;
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
/// Claim a link-local address as RFC 3927 section 2 (A1.11; 0049): the chip's own address at
/// 0.0.0.0 (it then answers no ARP for the candidate), socket 0 in MACRAW (the only socket that
/// can, [CH394] 7.2.1), probes and announcements from [`ipv4ll::Claim`], every conflict logged.
/// The caller has closed the other sockets. Ends with the address set and socket 0 closed.
async fn claim_link_local(chip: &mut Chip, claim: &mut ipv4ll::Claim) -> NetConfig {
    let _ = chip.set_ip([0; 4], [0; 4], [0; 4]);
    if let Err(e) = chip.macraw_open() {
        println!("net: FAULT cannot open socket 0 in MACRAW for the link-local probe ({:?})", e);
    }
    println!("net: probing link-local {} (RFC 3927)", ip_str(claim.candidate().ip).as_str());
    let mut f = [0u8; 1514];
    loop {
        let now = Instant::now().as_millis();
        while let Ok(Some(n)) = chip.macraw_recv(&mut f) {
            let (before, cand) = (claim.conflicts(), claim.candidate().ip);
            claim.on_frame(&f[..n], now);
            if claim.conflicts() != before {
                println!(
                    "net: link-local address conflict on {} while probing; next candidate {}",
                    ip_str(cand).as_str(),
                    ip_str(claim.candidate().ip).as_str()
                );
            }
        }
        match claim.poll(now) {
            ipv4ll::Action::Send(frame) => {
                let _ = chip.macraw_send(&frame);
            }
            ipv4ll::Action::Claimed(c) => {
                let _ = chip.udp_close(0);
                let _ = chip.set_ip(c.ip, c.mask, c.gateway);
                println!("net: link-local {} probed and announced", ip_str(c.ip).as_str());
                return c;
            }
            ipv4ll::Action::None => {}
        }
        Timer::after_millis(2).await;
    }
}

/// Every socket closed: before a probe on socket 0 and a new configuration (0049: discovery and
/// the other sockets are closed during the probe).
fn close_all(chip: &mut Chip) {
    for s in 0..g7250_ch394::SOCKETS {
        let _ = chip.udp_close(s);
    }
}

async fn configure(chip: &mut Chip, mac: [u8; 6], want: NetConfig, claim: &mut ipv4ll::Claim) -> (NetConfig, Source, Option<DhcpClient>) {
    if want.mode == Mode::Static && netcfg::valid_host(want.ip, want.mask) {
        let _ = chip.set_ip(want.ip, want.mask, want.gateway);
        return (want, Source::Static, None);
    }
    let xid = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ Instant::now().as_ticks() as u32;
    let found = dhcp(chip, mac, xid).await;
    // The client's timers start now, after the power-on exchange: started before it, the first
    // retry fell due at once and borrowed socket 7 just as the management port opened.
    let now = Instant::now().as_millis();
    if let Some((c, lease, server)) = found {
        // Not probed yet: it is probed when the client falls back to it (A1.11; 0049).
        let ll = claim.candidate();
        let _ = chip.set_ip(c.ip, c.mask, c.gateway);
        let l = lease.unwrap_or(3600).max(60);
        println!("net: DHCP lease {} s from {}; renewal at {} s", l, ip_str(server).as_str(), l / 2);
        return (c, Source::Dhcp, Some(DhcpClient::new(mac, xid.wrapping_add(16), ll, Some((c, server, l)), now)));
    }
    // DHCP attempts first, then the probe and announcements, before any socket opens (0048, 0049).
    let _ = chip.udp_close(SOCK_MGMT);
    claim.restart(now);
    let ll = claim_link_local(chip, claim).await;
    println!("net: no DHCP lease; link-local, DHCP keeps trying (A1.11)");
    let now = Instant::now().as_millis();
    (ll, Source::LinkLocal, Some(DhcpClient::new(mac, xid.wrapping_add(16), ll, None, now)))
}

/// One pass of the DHCP client after the power-on exchange (A1.11; 0041): retries while
/// link-local, renewal, rebinding. Socket 7 is borrowed (port 68) from a send until the client
/// stops listening, at most [`g7250_radio::dhcp_client::WAIT_MS`], and then always given back to
/// the management port, for a finished exchange as for an abandoned one.
/// Returns true when the client falls back to link-local (lease end, DHCPNAK): the caller then
/// probes it again with every socket closed ([`relink`]; 0049).
fn dhcp_step(chip: &mut Chip, c: &mut DhcpClient, borrowed: &mut bool, cfg: &mut NetConfig, p2: &mut crate::p2::P2, buf: &mut [u8], now: u64) -> bool {
    let mut relink = false;
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
            DhcpAction::Use(new) if new == c.link_local() && new != *cfg => relink = true,
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
    relink
}

/// A link-local address chosen while running (lease end, DHCPNAK, a conflict): an address change
/// of A1.11a (the session ends), every socket closed, the probe on socket 0, at most about 10 s
/// (after 10 conflicts the claim waits 60 s first: no address, the sockets stay closed), then
/// the sockets open again (A1.11; 0049). False if they cannot be opened.
async fn relink(chip: &mut Chip, claim: &mut ipv4ll::Claim, p2: &mut crate::p2::P2, dhcpc: &mut Option<DhcpClient>, cfg: &mut NetConfig, borrowed: &mut bool) -> bool {
    p2.address_changed(chip, Instant::now().as_millis());
    close_all(chip);
    *borrowed = false;
    let c = claim_link_local(chip, claim).await;
    if let Some(d) = dhcpc.as_mut() {
        d.set_link_local(c);
    }
    *cfg = c;
    print_cfg(cfg, Source::LinkLocal);
    if crate::p2::P2::open(chip).is_err() || chip.udp_open(SOCK_MGMT, m::PORT).is_err() {
        println!("net: FAULT cannot reopen the P2 and management ports");
        return false;
    }
    true
}

/// DHCPDECLINE for a leased address in conflict (RFC 2131 section 3.1 item 5; 0049), broadcast
/// from port 68 on socket 7; the caller closes the sockets after it.
async fn send_decline(chip: &mut Chip, mac: [u8; 6], xid: u32, declined: Ip, server: Ip) {
    let _ = chip.udp_close(SOCK_MGMT);
    if chip.udp_open(SOCK_MGMT, dhcp::CLIENT_PORT).is_ok() {
        let mut b = [0u8; dhcp::MSG_LEN];
        let n = dhcp::decline(xid, mac, declined, server, &mut b);
        let _ = chip.udp_send(SOCK_MGMT, [255; 4], dhcp::SERVER_PORT, &b[..n]);
        Timer::after_millis(20).await; // let it leave the CH394
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

    // The RFC 3927 claim lives as long as the radio: its conflict count drives the rate limit.
    let mut claim = ipv4ll::Claim::new(mac, Instant::now().as_millis());
    // The configuration before the last change, for the fall-back of A1.6 C-4 (0049).
    let mut prev_cfg: Option<NetConfig> = None;
    let (mut cfg, src, mut dhcpc) = configure(&mut chip, mac, NetConfig::FACTORY, &mut claim).await;
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
    // TRIP_COUNT kept over a reset without power loss (A3.5; persist.rs).
    p2.key.trips = g7250_board::persist::trip_count();
    p2.regs.trip_count = p2.key.trips;
    // The transmit ranges of the configured region (A1.6 C-6).
    p2.key.set_region(crate::mgmt_init::region_from_config(&mgmt.config));
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
            if dhcp_step(&mut chip, c, &mut dhcp_borrowed, &mut cfg, &mut p2, &mut mrx, Instant::now().as_millis()) {
                // Lease end or DHCPNAK: the link-local address is probed again (0049).
                claim.restart(Instant::now().as_millis());
                if !relink(&mut chip, &mut claim, &mut p2, &mut dhcpc, &mut cfg, &mut dhcp_borrowed).await {
                    return;
                }
                continue;
            }
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
                            close_all(&mut chip);
                            prev_cfg = Some(cfg);
                            let (c, s, d) = configure(&mut chip, mac, want, &mut claim).await;
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
                            // DUC I&Q (B10.7): the FIFO, or discarded whole and counted.
                            (crate::p2::S1029, None) => p2.duc_iq(&mut chip, from, &rx[..d.len], now),
                            _ => {}
                        }
                        handled = true;
                    }
                }
                p2.tick(&mut chip, Instant::now().as_millis());
                if handled {
                    continue;
                }
                // Counters of the key path for GET_STATS (Table B10-1, B10.5a item 1).
                server.stats.cw_hang_clamped = p2.key.cw_hang_clamped;
                server.stats.cw_speed_clamped = p2.key.cw_speed_clamped;
                server.stats.takeovers = p2.takeovers;
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
                    // A SET_CONFIG or FACTORY_RESET may have changed the region (A1.6 C-6).
                    p2.key.set_region(crate::mgmt_init::region_from_config(&mgmt.config));
                    for e in after {
                        match e {
                            g7250_radio::mgmt::Effect::ApplyNetwork => {
                                let want = crate::mgmt_init::network_from_config(&mgmt.config);
                                let now = Instant::now().as_millis();
                                p2.address_changed(&mut chip, now);
                                close_all(&mut chip);
                                prev_cfg = Some(cfg);
                                let (c, s, dc) = configure(&mut chip, mac, want, &mut claim).await;
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
                        // Another host uses our address ([CH394] GINT IP_CONFLI): give it up at once,
                        // no defending ARP (RFC 3927 2.5 option (a)); each case an address change of
                        // A1.11a; every conflict logged (A1.11; 0049).
                        let now = Instant::now().as_millis();
                        println!("net: address conflict on {}", ip_str(cfg.ip).as_str());
                        server.log.push(now, format_args!("address conflict on {}", ip_str(cfg.ip).as_str()));
                        let leased = dhcpc.as_ref().is_some_and(|d| d.lease().is_some_and(|l| l.0.ip == cfg.ip));
                        let link_local = cfg.ip[..2] == [169, 254];
                        if leased {
                            // DHCPDECLINE, link-local, DHCP again (RFC 2131 3.1 item 5).
                            if let Some(d) = dhcpc.as_mut() {
                                let xid = d.xid;
                                if let Some((ip, srv)) = d.decline(now) {
                                    p2.address_changed(&mut chip, now);
                                    send_decline(&mut chip, mac, xid, ip, srv).await;
                                    println!("net: DHCPDECLINE for {} sent; link-local, DHCP again in 10 s", ip_str(ip).as_str());
                                }
                            }
                            claim.restart(now);
                        } else if link_local {
                            claim.conflict(now);
                        } else if let Some(p) = prev_cfg.take().filter(|p| p.ip != cfg.ip && p.ip[..2] != [169, 254]) {
                            // A static address: the previous configuration (A1.6 C-4).
                            println!("net: static address in conflict; back to the previous configuration");
                            server.log.push(now, format_args!("static address in conflict: previous configuration restored"));
                            p2.address_changed(&mut chip, now);
                            close_all(&mut chip);
                            let (c, s, dc) = configure(&mut chip, mac, p, &mut claim).await;
                            cfg = c;
                            dhcpc = dc;
                            dhcp_borrowed = false;
                            print_cfg(&cfg, s);
                            if crate::p2::P2::open(&mut chip).is_err() || chip.udp_open(SOCK_MGMT, m::PORT).is_err() {
                                println!("net: FAULT cannot reopen the P2 and management ports");
                                return;
                            }
                            continue;
                        } else {
                            // A static address with no previous configuration: link-local.
                            claim.restart(now);
                        }
                        if !relink(&mut chip, &mut claim, &mut p2, &mut dhcpc, &mut cfg, &mut dhcp_borrowed).await {
                            return;
                        }
                        continue;
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
