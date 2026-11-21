// SPDX-License-Identifier: GPL-3.0-or-later
//! Host test of the CH394 driver against a small model of the W5500 register set.
use embedded_hal::spi::{ErrorKind, ErrorType, Operation, SpiDevice};
use g7250_ch394::*;

#[derive(Debug)]
struct E;
impl embedded_hal::spi::Error for E {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

/// Register model: common block, 8 socket register blocks, 8 RX and TX buffers of 2 KiB.
struct Chip {
    common: [u8; 0x40],
    sreg: [[u8; 0x30]; 8],
    rx: [[u8; 2048]; 8],
    tx: [[u8; 2048]; 8],
    sent: Vec<(u8, [u8; 4], u16, Vec<u8>)>,
    tx_rd: [u16; 8],
    /// RX read pointer at the last RECV, to compute what a RECV consumed.
    rx_base: [u16; 8],
}

impl Chip {
    fn new() -> Self {
        let mut c = Chip { common: [0; 0x40], sreg: [[0; 0x30]; 8], rx: [[0; 2048]; 8], tx: [[0; 2048]; 8], sent: vec![], tx_rd: [0; 8], rx_base: [0; 8] };
        c.common[0x39] = 0x04;
        c.common[0x09..0x0F].copy_from_slice(&[0x02, 0xAB, 0xCD, 0x00, 0x00, 0x01]);
        c.common[0x2E] = 0b011; // link up, 100 Mbit/s, half duplex
        for s in &mut c.sreg {
            s[0x20] = 0x08; // TX_FSR = 2048
        }
        c
    }

    /// The network delivers a datagram to socket n.
    fn deliver(&mut self, n: usize, ip: [u8; 4], port: u16, data: &[u8]) {
        let rsr = u16::from_be_bytes([self.sreg[n][0x26], self.sreg[n][0x27]]);
        let rd = u16::from_be_bytes([self.sreg[n][0x28], self.sreg[n][0x29]]);
        let mut at = rd.wrapping_add(rsr);
        let mut put = |b: u8, at: &mut u16| {
            self.rx[n][(*at as usize) % 2048] = b;
            *at = at.wrapping_add(1);
        };
        for b in ip.iter().copied().chain(port.to_be_bytes()).chain((data.len() as u16).to_be_bytes()).chain(data.iter().copied()) {
            put(b, &mut at);
        }
        let rsr = rsr + 8 + data.len() as u16;
        self.sreg[n][0x26..0x28].copy_from_slice(&rsr.to_be_bytes());
    }

    fn command(&mut self, n: usize, cmd: u8) {
        match cmd {
            0x01 => self.sreg[n][0x03] = if self.sreg[n][0] == 0x02 { 0x22 } else { 0x13 },
            0x10 => self.sreg[n][0x03] = 0x00,
            0x40 => {
                // RECV: the consumed bytes leave RX_RSR.
                let rd = u16::from_be_bytes([self.sreg[n][0x28], self.sreg[n][0x29]]);
                let rsr = u16::from_be_bytes([self.sreg[n][0x26], self.sreg[n][0x27]]);
                let used = rd.wrapping_sub(self.rx_base[n]);
                self.sreg[n][0x26..0x28].copy_from_slice(&(rsr - used).to_be_bytes());
                self.rx_base[n] = rd;
            }
            0x20 => {
                let wr = u16::from_be_bytes([self.sreg[n][0x24], self.sreg[n][0x25]]);
                let mut d = vec![];
                let mut a = self.tx_rd[n];
                while a != wr {
                    d.push(self.tx[n][(a as usize) % 2048]);
                    a = a.wrapping_add(1);
                }
                self.tx_rd[n] = wr;
                let ip = [self.sreg[n][0x0C], self.sreg[n][0x0D], self.sreg[n][0x0E], self.sreg[n][0x0F]];
                let port = u16::from_be_bytes([self.sreg[n][0x10], self.sreg[n][0x11]]);
                self.sent.push((n as u8, ip, port, d));
                self.sreg[n][0x02] |= 0x10;
            }
            _ => {}
        }
        self.sreg[n][0x01] = 0;
    }
}

struct Dev(Chip);

impl ErrorType for Dev {
    type Error = E;
}

impl SpiDevice for Dev {
    fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), E> {
        let (h, rest) = ops.split_first_mut().unwrap();
        let h = match h {
            Operation::Write(h) => [h[0], h[1], h[2]],
            _ => panic!("frame must start with the 3-byte header"),
        };
        let addr = u16::from_be_bytes([h[0], h[1]]);
        let block = h[2] >> 3;
        let write = h[2] & 0x04 != 0;
        assert_eq!(h[2] & 0x03, 0, "variable data length mode");
        let c = &mut self.0;
        for op in rest {
            match op {
                Operation::Read(buf) => {
                    assert!(!write);
                    for (i, b) in buf.iter_mut().enumerate() {
                        let a = addr.wrapping_add(i as u16) as usize;
                        *b = match block {
                            0 => c.common[a],
                            x if x % 4 == 1 => c.sreg[(x / 4) as usize][a],
                            x if x % 4 == 2 => c.tx[(x / 4) as usize][a % 2048],
                            x => c.rx[(x / 4) as usize][a % 2048],
                        };
                    }
                }
                Operation::Write(data) => {
                    assert!(write);
                    for (i, &b) in data.iter().enumerate() {
                        let a = addr.wrapping_add(i as u16) as usize;
                        match block {
                            0 if a == 0x15 => c.common[a] &= !b, // IR: write 1 to clear
                            0 => c.common[a] = b,
                            x if x % 4 == 1 => {
                                let n = (x / 4) as usize;
                                match a {
                                    0x01 => c.command(n, b),
                                    0x02 => c.sreg[n][a] &= !b,
                                    _ => c.sreg[n][a] = b,
                                }
                            }
                            x if x % 4 == 2 => c.tx[(x / 4) as usize][a % 2048] = b,
                            _ => panic!("RX buffer is read-only"),
                        }
                    }
                }
                _ => panic!("unexpected operation"),
            }
        }
        Ok(())
    }
}

fn chip() -> Ch394<Dev> {
    Ch394::new(Dev(Chip::new()))
}

#[test]
fn probe_mac_link() {
    let mut c = chip();
    assert_eq!(c.probe().unwrap(), 0x04);
    assert_eq!(c.mac().unwrap(), [0x02, 0xAB, 0xCD, 0x00, 0x00, 0x01]);
    let l = c.link().unwrap();
    assert!(l.up && l.mbit_100 && !l.full_duplex);
    assert_eq!(l.speed(), 100);
}

#[test]
fn no_chip_is_reported() {
    let mut d = Chip::new();
    d.common[0x39] = 0xFF;
    let mut c = Ch394::new(Dev(d));
    assert!(matches!(c.probe(), Err(Error::NoChip(0xFF))));
}

#[test]
fn set_ip_writes_registers() {
    let mut c = chip();
    c.set_ip([192, 168, 1, 20], [255, 255, 255, 0], [192, 168, 1, 1]).unwrap();
    let d = c.release().0;
    assert_eq!(&d.common[0x01..0x05], &[192, 168, 1, 1]);
    assert_eq!(&d.common[0x05..0x09], &[255, 255, 255, 0]);
    assert_eq!(&d.common[0x0F..0x13], &[192, 168, 1, 20]);
}

#[test]
fn udp_open_recv_send() {
    let mut c = chip();
    c.udp_open(0, 1024).unwrap();
    let mut buf = [0u8; 64];
    assert_eq!(c.udp_recv(0, &mut buf).unwrap(), None);
    let mut d = c.release().0;
    assert_eq!(d.sreg[0][0x03], 0x22);
    assert_eq!(&d.sreg[0][0x04..0x06], &1024u16.to_be_bytes());
    let mut req = [0u8; 60];
    req[4] = 2;
    d.deliver(0, [192, 168, 1, 5], 50123, &req);
    d.deliver(0, [192, 168, 1, 6], 50124, &[9, 9, 9]);
    let mut c = Ch394::new(Dev(d));
    let g = c.udp_recv(0, &mut buf).unwrap().unwrap();
    assert_eq!((g.src_ip, g.src_port, g.len, g.truncated), ([192, 168, 1, 5], 50123, 60, 0));
    assert_eq!(buf[4], 2);
    // A datagram larger than the buffer is truncated and fully consumed.
    let mut small = [0u8; 2];
    let g = c.udp_recv(0, &mut small).unwrap().unwrap();
    assert_eq!((g.src_port, g.len, g.truncated), (50124, 2, 1));
    assert_eq!(c.udp_recv(0, &mut buf).unwrap(), None);
    c.udp_send(0, [192, 168, 1, 5], 50123, &[1, 2, 3, 4]).unwrap();
    let d = c.release().0;
    assert_eq!(d.sent, vec![(0, [192, 168, 1, 5], 50123, vec![1, 2, 3, 4])]);
}

#[test]
fn buffers_are_checked() {
    let mut c = chip();
    // More than 16 KiB in a direction, or a socket above 16: refused ([CH394] 7.2.12-13).
    assert!(matches!(c.set_buffers([4, 4, 4, 4, 1, 0, 0, 0], [2; 8]), Err(Error::TooLarge)));
    assert!(matches!(c.set_buffers([2; 8], [17, 0, 0, 0, 0, 0, 0, 0]), Err(Error::TooLarge)));
    // Any size 0-16 is allowed, powers of two or not (3 KiB, 6 KiB).
    c.set_buffers([1, 2, 1, 3, 6, 0, 2, 1], [1, 1, 1, 0, 0, 8, 2, 3]).unwrap();
    // A datagram larger than its socket's transmit buffer is refused.
    assert!(matches!(c.udp_send(0, [1, 2, 3, 4], 1, &[0u8; 1500]), Err(Error::TooLarge)));
}
