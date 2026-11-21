// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Driver for the WCH CH394 Ethernet controller of the G7250 Main, in UDP mode.
//!
//! Sources: [HIS] HIS 40 (`9d10e72`): CH394 on SPI6, mode 0, 8-bit, MSB first, CS# PG15, RST#
//! PB5, INT# PC13; "W5500-register-compatible, per its datasheet"; the CH394 holds the MAC
//! address in its own register set. Register addresses, frame format and socket commands: the
//! public WIZnet W5500 datasheet (V1.0.x, sections 2.2 "SPI frame", 3 "Register and memory
//! organization", 4 "Register descriptions"), which the CH394 datasheet declares compatible.
//! Points where the CH394 may differ are marked `CH394?` and listed in the HIS questions.
//!
//! Generic over a blocking `embedded_hal::spi::SpiDevice` (the device owns CS#), so the protocol
//! is tested on the host against a register model (`tests/model.rs`).
#![cfg_attr(not(test), no_std)]

use embedded_hal::spi::{Operation, SpiDevice};

/// Block select bits of the control byte (W5500 section 2.2.2).
pub mod bsb {
    pub const COMMON: u8 = 0b00000;
    pub const fn sock_reg(n: u8) -> u8 {
        n * 4 + 1
    }
    pub const fn sock_tx(n: u8) -> u8 {
        n * 4 + 2
    }
    pub const fn sock_rx(n: u8) -> u8 {
        n * 4 + 3
    }
}

/// Common registers (W5500 section 4.1).
pub mod common {
    pub const MR: u16 = 0x0000;
    pub const GAR: u16 = 0x0001;
    pub const SUBR: u16 = 0x0005;
    pub const SHAR: u16 = 0x0009;
    pub const SIPR: u16 = 0x000F;
    pub const IR: u16 = 0x0015;
    /// GINTE ([CH394] 7.1.8): global interrupt enables.
    pub const GINTE: u16 = 0x0016;
    /// SINTE ([CH394] 7.1.10): socket interrupt enables, one bit per socket.
    pub const SINTE: u16 = 0x0018;
    pub const RTR: u16 = 0x0019;
    pub const RCR: u16 = 0x001B;
    pub const PHYCFGR: u16 = 0x002E;
    pub const VERSIONR: u16 = 0x0039;
    /// IR bit 7: IP conflict (spec A1.6 C-4, A1.11a: "GINT IP_CONFLI").
    pub const IR_CONFLICT: u8 = 0x80;
}

/// Socket registers (W5500 section 4.2).
pub mod sock {
    pub const MR: u16 = 0x0000;
    pub const CR: u16 = 0x0001;
    pub const IR: u16 = 0x0002;
    pub const SR: u16 = 0x0003;
    pub const PORT: u16 = 0x0004;
    pub const DIPR: u16 = 0x000C;
    pub const DPORT: u16 = 0x0010;
    pub const RXBUF_SIZE: u16 = 0x001E;
    /// Sn_INTE ([CH394] 7.2.20): which Sn_INT events raise INT#.
    pub const INTE: u16 = 0x002C;
    /// Sn_INT bit 2 INT_RECV ([CH394] 7.2.3).
    pub const IR_RECV: u8 = 0x04;
    pub const TXBUF_SIZE: u16 = 0x001F;
    pub const TX_FSR: u16 = 0x0020;
    pub const TX_WR: u16 = 0x0024;
    pub const RX_RSR: u16 = 0x0026;
    pub const RX_RD: u16 = 0x0028;

    pub const MR_UDP: u8 = 0x02;
    /// Sn_MODE PMD 0100: MACRAW, socket 0 only ([CH394] 7.2.1, p.15; 12.2.7, p.40-41).
    pub const MR_MACRAW: u8 = 0x04;
    pub const SR_MACRAW: u8 = 0x42;
    pub const CR_OPEN: u8 = 0x01;
    pub const CR_CLOSE: u8 = 0x10;
    pub const CR_SEND: u8 = 0x20;
    pub const CR_RECV: u8 = 0x40;
    pub const SR_CLOSED: u8 = 0x00;
    pub const SR_UDP: u8 = 0x22;
    pub const IR_SENDOK: u8 = 0x10;
    pub const IR_TIMEOUT: u8 = 0x08;
}

/// Number of hardware sockets (spec B2.3: "all 8 CH394Q sockets").
pub const SOCKETS: u8 = 8;

/// Whether the 2-byte length before a MACRAW frame counts its own 2 bytes. [CH394] 12.2.7 does
/// not say; read as "no" from its table. To confirm on the radio (emulator model to follow).
pub const MACRAW_LEN_INCLUDES_HEADER: bool = false;
/// UDP receive header in the socket RX buffer: source IP (4), source port (2), length (2).
pub const UDP_HDR: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    Spi(E),
    /// The chip did not answer as expected (version register, or a socket did not open).
    NoChip(u8),
    SocketState(u8),
    /// The datagram does not fit into the free TX buffer, or is larger than a TX buffer.
    TooLarge,
    /// SEND ended with a timeout (ARP for the destination failed).
    SendTimeout,
    /// The previous SEND on this socket had not ended within `send_wait_polls` (its ARP still
    /// under way): this datagram was not sent, and is counted in `dropped_busy`.
    Busy,
    /// Polling a command or flag gave up.
    Stuck,
}

/// Link state from PHYCFGR (W5500 section 4.1: bit 0 LNK, bit 1 SPD 100, bit 2 DPX full).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub up: bool,
    pub mbit_100: bool,
    pub full_duplex: bool,
}

impl Link {
    pub fn from_phycfgr(v: u8) -> Self {
        Self { up: v & 1 != 0, mbit_100: v & 2 != 0, full_duplex: v & 4 != 0 }
    }
    /// Speed in Mbit/s for `LINK_STATUS` [15:0] (B8, Table B8-2); 0 = link down.
    pub fn speed(&self) -> u16 {
        match (self.up, self.mbit_100) {
            (false, _) => 0,
            (true, true) => 100,
            (true, false) => 10,
        }
    }
}

/// A received datagram's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Datagram {
    pub src_ip: [u8; 4],
    pub src_port: u16,
    /// Bytes copied into the caller's buffer.
    pub len: usize,
    /// Bytes of the datagram that did not fit and were dropped.
    pub truncated: usize,
}

/// The value the version register holds on a W5500 (W5500 section 4.1, VERSIONR = 0x04).
/// On the CH394 it is CHIPV at 0x0039, read-only, value on the radio not known ([HIS] HIS 40,
/// `d199d33`); the driver accepts any value except the ones an absent or held-in-reset chip
/// reads (0x00, 0xFF).
pub const W5500_VERSION: u8 = 0x04;

pub struct Ch394<S> {
    spi: S,
    /// Socket buffer size in KiB per socket, RX and TX (default 2 KiB each, W5500 section 4.2).
    buf_kib: [u8; SOCKETS as usize],
    /// Bound on polling loops (each iteration one SPI transaction).
    pub poll_limit: u32,
    /// A SEND is under way on the socket; the next send waits for its end first.
    pending: [bool; SOCKETS as usize],
    /// Bound on that wait, in status reads (one SPI transaction each, about 10 us at 16 MHz).
    /// Never the full `poll_limit`: an ARP to a host that is gone takes RTIME x RCN (default
    /// 200 ms x 8, [CH394] 7.1.11-12), which would stall the caller (0054).
    pub send_wait_polls: u32,
    /// Datagrams not sent because the previous send on their socket had not ended.
    pub dropped_busy: [u32; SOCKETS as usize],
}

impl<S: SpiDevice> Ch394<S> {
    pub fn new(spi: S) -> Self {
        Self { spi, buf_kib: [2; SOCKETS as usize], poll_limit: 100_000, pending: [false; SOCKETS as usize], send_wait_polls: 100, dropped_busy: [0; SOCKETS as usize] }
    }

    pub fn release(self) -> S {
        self.spi
    }

    fn ctrl(block: u8, write: bool) -> u8 {
        (block << 3) | ((write as u8) << 2) // OM = 00, variable data length mode
    }

    pub fn read(&mut self, block: u8, addr: u16, buf: &mut [u8]) -> Result<(), Error<S::Error>> {
        let h = [(addr >> 8) as u8, addr as u8, Self::ctrl(block, false)];
        self.spi.transaction(&mut [Operation::Write(&h), Operation::Read(buf)]).map_err(Error::Spi)
    }

    pub fn write(&mut self, block: u8, addr: u16, data: &[u8]) -> Result<(), Error<S::Error>> {
        let h = [(addr >> 8) as u8, addr as u8, Self::ctrl(block, true)];
        self.spi.transaction(&mut [Operation::Write(&h), Operation::Write(data)]).map_err(Error::Spi)
    }

    pub fn read_u8(&mut self, block: u8, addr: u16) -> Result<u8, Error<S::Error>> {
        let mut b = [0u8; 1];
        self.read(block, addr, &mut b)?;
        Ok(b[0])
    }

    /// The sockets of `mask` that hold received data (Sn_RX_RS not 0), as a bit mask. The net
    /// loop asks this before it sleeps: whether the chip raises INT# again for a datagram left in
    /// a buffer after its RECV flag was cleared is not stated in CH394DS1, so the loop does not
    /// rely on it.
    pub fn rx_pending(&mut self, mask: u8) -> Result<u8, Error<S::Error>> {
        let mut out = 0;
        for n in 0..8u8 {
            if mask & (1 << n) != 0 && self.read_u16(bsb::sock_reg(n), sock::RX_RSR)? != 0 {
                out |= 1 << n;
            }
        }
        Ok(out)
    }

    /// 16-bit counters change while being read; read until two reads agree (W5500 section 4.2,
    /// Sn_TX_FSR and Sn_RX_RSR notes).
    pub fn read_u16_stable(&mut self, block: u8, addr: u16) -> Result<u16, Error<S::Error>> {
        let mut prev = self.read_u16(block, addr)?;
        for _ in 0..self.poll_limit {
            let v = self.read_u16(block, addr)?;
            if v == prev {
                return Ok(v);
            }
            prev = v;
        }
        Err(Error::Stuck)
    }

    pub fn read_u16(&mut self, block: u8, addr: u16) -> Result<u16, Error<S::Error>> {
        let mut b = [0u8; 2];
        self.read(block, addr, &mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    /// Check that a chip answers; returns the version register.
    pub fn probe(&mut self) -> Result<u8, Error<S::Error>> {
        let v = self.read_u8(bsb::COMMON, common::VERSIONR)?;
        if v == 0x00 || v == 0xFF { Err(Error::NoChip(v)) } else { Ok(v) }
    }

    /// The MAC address the chip holds (HIS 40: provided by the CH394 in its own register set;
    /// the firmware does not write it).
    pub fn mac(&mut self) -> Result<[u8; 6], Error<S::Error>> {
        let mut m = [0u8; 6];
        self.read(bsb::COMMON, common::SHAR, &mut m)?;
        Ok(m)
    }

    pub fn link(&mut self) -> Result<Link, Error<S::Error>> {
        Ok(Link::from_phycfgr(self.read_u8(bsb::COMMON, common::PHYCFGR)?))
    }

    /// Set the address, netmask and gateway (C-1, A1.6).
    pub fn set_ip(&mut self, ip: [u8; 4], mask: [u8; 4], gw: [u8; 4]) -> Result<(), Error<S::Error>> {
        self.write(bsb::COMMON, common::GAR, &gw)?;
        self.write(bsb::COMMON, common::SUBR, &mask)?;
        self.write(bsb::COMMON, common::SIPR, &ip)
    }

    /// INT# on received datagrams of the sockets in `mask` ([CH394] 7.1.8-10) and on an IP
    /// conflict; the other global sources stay polled through [`Self::take_irq`].
    pub fn enable_recv_interrupts(&mut self, mask: u8) -> Result<(), Error<S::Error>> {
        // IP conflict (GINT bit 7, [CH394] 7.1.7-8) raises INT# too, so the net loop need not
        // poll for it; take_irq clears it.
        self.write(bsb::COMMON, common::GINTE, &[common::IR_CONFLICT])?;
        self.write(bsb::COMMON, common::SINTE, &[mask])
    }

    /// Read and clear the common interrupt flags; returns them.
    pub fn take_irq(&mut self) -> Result<u8, Error<S::Error>> {
        let v = self.read_u8(bsb::COMMON, common::IR)?;
        if v != 0 {
            self.write(bsb::COMMON, common::IR, &[v])?;
        }
        Ok(v)
    }

    /// Set the receive and transmit buffer of every socket, in KiB: any value 0-16, each
    /// direction 16 KiB in total ([CH394] 7.2.12, 7.2.13, p.19: Sn_RXBUF_SIZE, Sn_TXBUF_SIZE,
    /// 1 KiB blocks given out in socket order). Set before the sockets open. Use powers of two:
    /// the 16-bit socket pointers wrap at 65536, and how a wrap maps onto a buffer whose size
    /// does not divide 65536 is not documented.
    pub fn set_buffers(&mut self, rx: [u8; SOCKETS as usize], tx: [u8; SOCKETS as usize]) -> Result<(), Error<S::Error>> {
        let sum = |k: &[u8; SOCKETS as usize]| k.iter().map(|&k| k as u32).sum::<u32>();
        if sum(&rx) > 16 || sum(&tx) > 16 || rx.iter().chain(tx.iter()).any(|&k| k > 16) {
            return Err(Error::TooLarge);
        }
        for n in 0..SOCKETS {
            self.write(bsb::sock_reg(n), sock::RXBUF_SIZE, &[rx[n as usize]])?;
            self.write(bsb::sock_reg(n), sock::TXBUF_SIZE, &[tx[n as usize]])?;
        }
        self.buf_kib = tx;
        Ok(())
    }

    fn command(&mut self, n: u8, cmd: u8) -> Result<(), Error<S::Error>> {
        self.write(bsb::sock_reg(n), sock::CR, &[cmd])?;
        // The command register reads 0 when the command has been taken (W5500 section 4.2).
        for _ in 0..self.poll_limit {
            if self.read_u8(bsb::sock_reg(n), sock::CR)? == 0 {
                return Ok(());
            }
        }
        Err(Error::Stuck)
    }

    /// Open socket `n` as UDP on local `port`. Broadcast reception stays enabled (needed for
    /// discovery to 255.255.255.255 and to the directed broadcast, B3.2).
    pub fn udp_open(&mut self, n: u8, port: u16) -> Result<(), Error<S::Error>> {
        self.udp_close(n)?;
        self.write(bsb::sock_reg(n), sock::MR, &[sock::MR_UDP])?;
        self.write(bsb::sock_reg(n), sock::PORT, &port.to_be_bytes())?;
        self.command(n, sock::CR_OPEN)?;
        let sr = self.read_u8(bsb::sock_reg(n), sock::SR)?;
        if sr != sock::SR_UDP {
            return Err(Error::SocketState(sr));
        }
        // Only a received datagram raises INT# for this socket: SEND_OK and TIMEOUT, cleared
        // lazily by the next send, must not hold INT# low.
        self.write(bsb::sock_reg(n), sock::INTE, &[sock::IR_RECV])
    }

    pub fn udp_close(&mut self, n: u8) -> Result<(), Error<S::Error>> {
        self.pending[n as usize] = false;
        self.command(n, sock::CR_CLOSE)?;
        self.write(bsb::sock_reg(n), sock::IR, &[0xFF])
    }

    /// Receive one datagram from socket `n` into `buf`, if one is waiting.
    pub fn udp_recv(&mut self, n: u8, buf: &mut [u8]) -> Result<Option<Datagram>, Error<S::Error>> {
        let avail = self.read_u16_stable(bsb::sock_reg(n), sock::RX_RSR)? as usize;
        if avail < UDP_HDR {
            return Ok(None);
        }
        // Clear INT_RECV before reading: a datagram that arrives while this one is read sets it
        // again, so INT# is never left high with an unread arrival behind it.
        self.write(bsb::sock_reg(n), sock::IR, &[sock::IR_RECV])?;
        let rd = self.read_u16(bsb::sock_reg(n), sock::RX_RD)?;
        let mut h = [0u8; UDP_HDR];
        self.read(bsb::sock_rx(n), rd, &mut h)?;
        let len = u16::from_be_bytes([h[6], h[7]]) as usize;
        let take = len.min(buf.len());
        // The RX buffer address wraps at 16 bits; the chip maps it onto the socket buffer.
        let data_at = rd.wrapping_add(UDP_HDR as u16);
        if take > 0 {
            self.read(bsb::sock_rx(n), data_at, &mut buf[..take])?;
        }
        let new_rd = data_at.wrapping_add(len as u16);
        self.write(bsb::sock_reg(n), sock::RX_RD, &new_rd.to_be_bytes())?;
        self.command(n, sock::CR_RECV)?;
        Ok(Some(Datagram {
            src_ip: [h[0], h[1], h[2], h[3]],
            src_port: u16::from_be_bytes([h[4], h[5]]),
            len: take,
            truncated: len - take,
        }))
    }

    /// Open socket 0 in MACRAW mode ([CH394] 12.2.7): every Ethernet frame, MAC filter off
    /// (Sn_MODE bit 7 = 0), broadcasts not masked (bit 6 = 0). Only socket 0 can do this.
    pub fn macraw_open(&mut self) -> Result<(), Error<S::Error>> {
        self.udp_close(0)?;
        self.write(bsb::sock_reg(0), sock::MR, &[sock::MR_MACRAW])?;
        self.command(0, sock::CR_OPEN)?;
        let sr = self.read_u8(bsb::sock_reg(0), sock::SR)?;
        if sr != sock::SR_MACRAW {
            return Err(Error::SocketState(sr));
        }
        Ok(())
    }

    /// One raw frame from socket 0 in MACRAW, if one is waiting: [CH394] 12.2.7 puts a 2-byte
    /// length before each frame. Read here as the length of the frame alone; the datasheet's
    /// table (p.41, "packet length 2 Byte, data N Byte") does not say whether it counts itself
    /// ([`MACRAW_LEN_INCLUDES_HEADER`]). Returns the frame length copied.
    pub fn macraw_recv(&mut self, buf: &mut [u8]) -> Result<Option<usize>, Error<S::Error>> {
        let avail = self.read_u16_stable(bsb::sock_reg(0), sock::RX_RSR)? as usize;
        if avail < 2 {
            return Ok(None);
        }
        let rd = self.read_u16(bsb::sock_reg(0), sock::RX_RD)?;
        let mut h = [0u8; 2];
        self.read(bsb::sock_rx(0), rd, &mut h)?;
        let raw = u16::from_be_bytes(h) as usize;
        let len = if MACRAW_LEN_INCLUDES_HEADER { raw.saturating_sub(2) } else { raw };
        let take = len.min(buf.len());
        let data_at = rd.wrapping_add(2);
        if take > 0 {
            self.read(bsb::sock_rx(0), data_at, &mut buf[..take])?;
        }
        self.write(bsb::sock_reg(0), sock::RX_RD, &data_at.wrapping_add(len as u16).to_be_bytes())?;
        self.command(0, sock::CR_RECV)?;
        Ok(Some(take))
    }

    /// Send one raw Ethernet frame (without FCS) from socket 0 in MACRAW.
    pub fn macraw_send(&mut self, frame: &[u8]) -> Result<(), Error<S::Error>> {
        if let Ok(false) = self.finish_send(0) {
            self.dropped_busy[0] = self.dropped_busy[0].saturating_add(1);
            return Err(Error::Busy);
        }
        let free = self.read_u16_stable(bsb::sock_reg(0), sock::TX_FSR)? as usize;
        if frame.len() > free {
            return Err(Error::TooLarge);
        }
        let wr = self.read_u16(bsb::sock_reg(0), sock::TX_WR)?;
        self.write(bsb::sock_tx(0), wr, frame)?;
        self.write(bsb::sock_reg(0), sock::TX_WR, &wr.wrapping_add(frame.len() as u16).to_be_bytes())?;
        self.command(0, sock::CR_SEND)?;
        self.pending[0] = true;
        Ok(())
    }

    /// Wait for the end of a SEND under way on socket `n` (SEND_OK or TIMEOUT).
    /// Wait, at most `send_wait_polls` status reads, for the end of the SEND under way on socket
    /// `n`. Ok(true): no send under way any more; Ok(false): still under way.
    fn finish_send(&mut self, n: u8) -> Result<bool, Error<S::Error>> {
        if !self.pending[n as usize] {
            return Ok(true);
        }
        for _ in 0..self.send_wait_polls.max(1) {
            let ir = self.read_u8(bsb::sock_reg(n), sock::IR)?;
            if ir & sock::IR_SENDOK != 0 {
                self.write(bsb::sock_reg(n), sock::IR, &[sock::IR_SENDOK])?;
                self.pending[n as usize] = false;
                return Ok(true);
            }
            if ir & sock::IR_TIMEOUT != 0 {
                self.write(bsb::sock_reg(n), sock::IR, &[sock::IR_TIMEOUT])?;
                self.pending[n as usize] = false;
                return Err(Error::SendTimeout);
            }
        }
        Ok(false)
    }

    /// Send one datagram from socket `n`. Does not wait for the datagram to leave. The chip takes
    /// one SEND at a time per socket, so a send first waits for the end of the previous one on
    /// the same socket, at most `send_wait_polls` reads (about 1 ms); if it has not ended (an ARP
    /// under way) the datagram is dropped, counted in `dropped_busy`, and `Busy` returned: a
    /// slow destination never stalls the caller. A timeout of the previous datagram is reported
    /// by this call and the new one is still sent.
    pub fn udp_send(&mut self, n: u8, ip: [u8; 4], port: u16, data: &[u8]) -> Result<(), Error<S::Error>> {
        // The previous send on this socket must have ended: wait a little, else drop this one
        // (counted) rather than stall the caller on an ARP (0054).
        let previous = self.finish_send(n);
        if let Ok(false) = previous {
            self.dropped_busy[n as usize] = self.dropped_busy[n as usize].saturating_add(1);
            return Err(Error::Busy);
        }
        let cap = self.buf_kib[n as usize] as usize * 1024;
        if data.len() > cap {
            return Err(Error::TooLarge);
        }
        let free = self.read_u16_stable(bsb::sock_reg(n), sock::TX_FSR)? as usize;
        if data.len() > free {
            return Err(Error::TooLarge);
        }
        self.write(bsb::sock_reg(n), sock::DIPR, &ip)?;
        self.write(bsb::sock_reg(n), sock::DPORT, &port.to_be_bytes())?;
        let wr = self.read_u16(bsb::sock_reg(n), sock::TX_WR)?;
        self.write(bsb::sock_tx(n), wr, data)?;
        self.write(bsb::sock_reg(n), sock::TX_WR, &wr.wrapping_add(data.len() as u16).to_be_bytes())?;
        self.command(n, sock::CR_SEND)?;
        self.pending[n as usize] = true;
        match previous {
            Err(Error::SendTimeout) => Err(Error::SendTimeout),
            _ => Ok(()),
        }
    }
}
