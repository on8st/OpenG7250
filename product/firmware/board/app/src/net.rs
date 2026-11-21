// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Milestone F2: CH394 on SPI6, IP configuration, G7250-P2 discovery on port 1024.
//!
//! Pins and bus: [HIS] HIS 40 (SCK PG13, MISO PG12, MOSI PG14, AF5, mode 0; CS# PG15; RST# PB5;
//! INT# PC13, not used yet: the driver polls). Sockets: spec Table B2-1; socket 0 is port 1024.
//! Socket 7 (the management socket, port 7251) is borrowed for DHCP at power-on, before the P2
//! sockets are opened (A1.11, provisional).

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

// DMA rule for later (RM0468): SPI6 is in the D3 domain and can use only the BDMA, which reaches
// only D3 memory. When this driver moves from blocking transfers to DMA, its buffers MUST be in
// SRAM4 (0x38000000, section `.sram4` of memory-slot-*.x), never in AXI or D2 SRAM (transfer
// error) and never in the Backup SRAM at 0x38800000 ([HIS] HIS 15 rule 4).

/// SPI clock. [HIS] HIS 40 (`6e2208a`): the CH394 takes up to 62 MHz in mode 0 or 3 (datasheet,
/// VERIFIED), but about 17 MHz seen on the board link (MEDIUM) is the safe bound until the link
/// is measured (P-NET-1). SPI6 kernel clock is PCLK4 = 64 MHz here; 64 / 4 = 16 MHz.
const SPI_HZ: u32 = 16_000_000;
const SOCK_DISCOVERY: u8 = 0;
const SOCK_MGMT: u8 = 7;

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

fn ip_str(ip: Ip) -> heapless::String<16> {
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
async fn dhcp(chip: &mut Chip, mac: [u8; 6], xid: u32) -> Option<(NetConfig, Option<u32>)> {
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
                        break 'outer Some((NetConfig { mode: Mode::Dhcp, ip: ack.yiaddr, mask, gateway: gw }, ack.lease_s));
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
    while Instant::now() - start < Duration::from_secs(3) {
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

async fn configure(chip: &mut Chip, mac: [u8; 6], want: NetConfig) -> (NetConfig, Source) {
    if want.mode == Mode::Static && netcfg::valid_host(want.ip, want.mask) {
        let _ = chip.set_ip(want.ip, want.mask, want.gateway);
        return (want, Source::Static);
    }
    let xid = u32::from_be_bytes([mac[2], mac[3], mac[4], mac[5]]) ^ Instant::now().as_ticks() as u32;
    if let Some((c, lease)) = dhcp(chip, mac, xid).await {
        let _ = chip.set_ip(c.ip, c.mask, c.gateway);
        match lease {
            Some(l) => println!("net: DHCP lease {} s (renewal not implemented yet)", l),
            None => println!("net: DHCP lease without a lease time"),
        }
        return (c, Source::Dhcp);
    }
    let ll = netcfg::link_local(mac, 0);
    let _ = chip.set_ip(ll.ip, ll.mask, ll.gateway);
    (ll, Source::LinkLocal)
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
pub async fn run(pins: NetPins<'static>, id: Identity) {
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

    let (mut cfg, src) = configure(&mut chip, mac, NetConfig::FACTORY).await;
    print_cfg(&cfg, src);

    if let Err(e) = chip.udp_open(SOCK_DISCOVERY, discovery::PORT) {
        println!("net: FAULT cannot open port 1024 ({:?})", e);
        return;
    }
    println!("net: discovery listening on UDP 1024 (board G7250-P2, type {})", discovery::BOARD_TYPE);

    let id = Identity { mac, ..id };
    let mut limit: RateLimit<16> = RateLimit::new(10, 50);
    let mut rx = [0u8; 1444];
    let mut dropped_1024: u32 = 0;
    let mut answered: u32 = 0;
    let mut last_link = link;
    loop {
        match chip.udp_recv(SOCK_DISCOVERY, &mut rx) {
            Ok(Some(d)) => {
                let now = Instant::now().as_millis();
                match discovery::classify(&rx[..d.len], d.src_port) {
                    Port1024::Discovery => {
                        if limit.allow(d.src_ip, d.src_port, now) {
                            // No P2 session exists in F2, so the radio is never "running" (B3.4).
                            let r = discovery::reply(&id, Status::Available);
                            if chip.udp_send(SOCK_DISCOVERY, d.src_ip, d.src_port, &r).is_ok() {
                                answered = answered.wrapping_add(1);
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
                        } else if limit.allow(d.src_ip, d.src_port, now) {
                            println!("net: Set IP {} from {} (kept in RAM until restart: no configuration store yet)", ip_str(ip).as_str(), ip_str(d.src_ip).as_str());
                            let r = discovery::reply(&id, Status::Available);
                            let _ = chip.udp_send(SOCK_DISCOVERY, d.src_ip, d.src_port, &r);
                            let want = if ip == [0; 4] {
                                NetConfig::FACTORY
                            } else {
                                netcfg::after_set_ip(&cfg, ip)
                            };
                            let _ = chip.udp_close(SOCK_DISCOVERY);
                            let (c, s) = configure(&mut chip, mac, want).await;
                            cfg = c;
                            print_cfg(&cfg, if s == Source::Static { Source::SetIp } else { s });
                            if chip.udp_open(SOCK_DISCOVERY, discovery::PORT).is_err() {
                                println!("net: FAULT cannot reopen port 1024");
                                return;
                            }
                        }
                    }
                    Port1024::SetIp { .. } | Port1024::IgnoredProgramming | Port1024::Other => {}
                    Port1024::FromPort1024 => dropped_1024 = dropped_1024.wrapping_add(1),
                }
            }
            Ok(None) => {
                if let Ok(irq) = chip.take_irq() {
                    if irq & g7250_ch394::common::IR_CONFLICT != 0 {
                        println!("net: address conflict on {} (fall-back of C-4 not implemented yet)", ip_str(cfg.ip).as_str());
                    }
                }
                if let Ok(l) = chip.link() {
                    if l != last_link {
                        println!("net: link {} Mbit/s", l.speed());
                        last_link = l;
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
