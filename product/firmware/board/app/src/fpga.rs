// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! FPGA bus on GPIOE ([HIS] HIS 80): D0-D7 PE0-PE7 (bidirectional), CMD PE8 (pulsed with PE9 to
//! latch a command byte), STB PE9 (per-byte strobe), DRDY PE10 (input, edge = samples ready).
//!
//! The only code in the firmware that touches GPIOE (checked by crates/layout/tests/forbidden.rs).
//! Every function takes a `BusPermit` (`g7250_safety::fpga_gate`), which cannot exist until the
//! bus parameters are cleared through the HIS: until then nothing here runs and PE0-PE10 keep
//! their reset (undriven) state. The bus turnaround is explicit: the MCU drives D0-D7 only
//! between `drive` and `release`, and waits `turnaround_ns` at both changes of direction.

#![allow(dead_code)] // no permit exists in this build; the module is compiled, not called

use embassy_stm32::pac;
use embassy_stm32::pac::gpio::vals::{Moder, Ot, Ospeedr, Pupdr};
use g7250_safety::fpga_gate::{BusPermit, Command, FORBIDDEN_COMMANDS};

const CMD: usize = 8;
const STB: usize = 9;
const DRDY: usize = 10;
const CPU_HZ: u32 = 64_000_000;

fn wait_ns(ns: u32) {
    cortex_m::asm::delay(((ns as u64 * CPU_HZ as u64) / 1_000_000_000 + 1) as u32);
}

pub struct Bus<'p> {
    p: &'p BusPermit,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BusError {
    /// A command of HIS 80's "MUST NOT be sent" list.
    Forbidden(u8),
}

impl<'p> Bus<'p> {
    /// Take the bus: CMD and STB to their inactive levels as outputs (level written first), D0-D7
    /// as inputs (the FPGA side's direction is unknown until measured: the MCU does not drive).
    pub fn open(p: &'p BusPermit) -> Self {
        let g = pac::GPIOE;
        let b = Bus { p };
        b.set(CMD, !p.params.cmd_active_high);
        b.set(STB, !p.params.strobe_active_high);
        for n in [CMD, STB] {
            g.otyper().modify(|w| w.set_ot(n, Ot::PUSH_PULL));
            g.ospeedr().modify(|w| w.set_ospeedr(n, Ospeedr::LOW_SPEED));
            g.moder().modify(|w| w.set_moder(n, Moder::OUTPUT));
        }
        for n in 0..8 {
            g.pupdr().modify(|w| w.set_pupdr(n, Pupdr::FLOATING));
            g.moder().modify(|w| w.set_moder(n, Moder::INPUT));
        }
        g.moder().modify(|w| w.set_moder(DRDY, Moder::INPUT));
        b
    }

    fn set(&self, n: usize, high: bool) {
        pac::GPIOE.bsrr().write(|w| if high { w.set_bs(n, true) } else { w.set_br(n, true) });
    }

    /// Drive D0-D7 with `byte` (after the turnaround from FPGA-driven).
    fn drive(&self, byte: u8) {
        let g = pac::GPIOE;
        g.bsrr().write(|w| {
            for n in 0..8 {
                if byte & (1 << n) != 0 { w.set_bs(n, true) } else { w.set_br(n, true) }
            }
        });
        wait_ns(self.p.params.turnaround_ns);
        for n in 0..8 {
            g.moder().modify(|w| w.set_moder(n, Moder::OUTPUT));
        }
        wait_ns(self.p.params.setup_ns);
    }

    /// Stop driving D0-D7 and wait the turnaround before the FPGA may drive them.
    fn release(&self) {
        let g = pac::GPIOE;
        for n in 0..8 {
            g.moder().modify(|w| w.set_moder(n, Moder::INPUT));
        }
        wait_ns(self.p.params.turnaround_ns);
    }

    fn pulse(&self, with_cmd: bool) {
        let p = self.p.params;
        if with_cmd {
            self.set(CMD, p.cmd_active_high);
        }
        self.set(STB, p.strobe_active_high);
        wait_ns(p.strobe_ns);
        self.set(STB, !p.strobe_active_high);
        if with_cmd {
            self.set(CMD, !p.cmd_active_high);
        }
    }

    /// Latch a command byte. The HIS 80 "MUST NOT be sent" codes are refused by the type
    /// (`Command` has none of them) and again here.
    pub fn command(&self, c: Command) -> Result<(), BusError> {
        if FORBIDDEN_COMMANDS.contains(&c.code()) {
            return Err(BusError::Forbidden(c.code()));
        }
        self.drive(c.code());
        self.pulse(true);
        self.release();
        Ok(())
    }

    /// Write data bytes after a command (for example the 0x01 config frame).
    pub fn write(&self, bytes: &[u8]) {
        for &b in bytes {
            self.drive(b);
            self.pulse(false);
        }
        self.release();
    }

    /// Read data bytes after a command; the FPGA drives D0-D7 (the MCU has released them).
    pub fn read(&self, out: &mut [u8]) {
        let g = pac::GPIOE;
        let p = self.p.params;
        for b in out.iter_mut() {
            self.set(STB, p.strobe_active_high);
            wait_ns(p.strobe_ns);
            *b = g.idr().read().0 as u8;
            self.set(STB, !p.strobe_active_high);
            wait_ns(p.setup_ns);
        }
    }

    /// DRDY level.
    pub fn drdy(&self) -> bool {
        pac::GPIOE.idr().read().0 & (1 << DRDY) != 0
    }

    /// Give the bus back: everything to analog (undriven), as at reset.
    pub fn close(self) {
        let g = pac::GPIOE;
        for n in 0..=DRDY {
            g.moder().modify(|w| w.set_moder(n, Moder::ANALOG));
        }
    }
}
