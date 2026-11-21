// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Gate on the FPGA bus (GPIOE PE0-PE10, [HIS] HIS 80), like the TX lock.
//!
//! HIS 80 (`6e2208a`): strobe polarity and timing and the bus turnaround (who drives D0-D7 when)
//! are not measured, bus contention is a hardware risk, and the meaning and safe values of the
//! 0x01 config frame bytes 0 and 9-18 (per-rate calibration among them) are unknown. Coordinator,
//! 2026-11-21: the image must not drive PE0-PE7 or any FPGA bus strobe unless the bus parameters
//! are set from a cleared HIS; until then the pins stay undriven and DDC0 reports reason 2.
//!
//! HIS 80 at `5292ba8` (gate review 8) answers part of it: STB and CMD are active-high pulses;
//! the MCU drives D0-D7 for commands and writes, switches them to inputs for reads, the FPGA
//! drives while STB is high, and the MCU may drive again only after STB is low; samples are
//! big-endian 24-bit, one strobe per byte, six per sample; DRDY most likely rising; f_clk
//! 122.88 MHz as the word scale (all MEDIUM). Still not known: set-up, strobe width and turnaround
//! times, I/Q order, which tuning word is DDC0, and the meaning and safe values of 0x01 byte 0 and
//! bytes 9-18. The spec names the frame P-RX-2 (0042). So the parameter set stays incomplete and
//! the gate stays closed. Safety rule of HIS 80: any 0x01 write while byte 0 is not understood
//! only with the RF section in the HIS 85 safe state (byte 0 may hold a TX enable).
//!
//! The gate is in the types: every bus access takes a [`BusPermit`], and the only way to get
//! one is [`permit`], which needs a complete [`BusParams`] set from [`CLEARED`] (the radio) or,
//! with the build feature `emulator-fpga-bus`, from [`EMULATOR`] (emulator runs only; values
//! that came through the HIS as emulator observations). Both are compile-time constants:
//! nothing received over the network can open the gate. Both are `None` today.

/// Bus parameters, each from the HIS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BusParams {
    /// Level of the STB (PE9) strobe while active.
    pub strobe_active_high: bool,
    /// Level of CMD (PE8) while a command byte is latched.
    pub cmd_active_high: bool,
    /// Data set-up before the strobe edge, ns.
    pub setup_ns: u32,
    /// Strobe active width, ns.
    pub strobe_ns: u32,
    /// Time from releasing D0-D7 (MCU inputs) to the FPGA driving them, and back, ns.
    pub turnaround_ns: u32,
    /// The DRDY (PE10) edge that marks new samples: rising (HIS 80) if true.
    pub drdy_rising: bool,
    /// 0x01 config frame: the safe value of byte 0 (status flags).
    pub frame_byte0: u8,
    /// 0x01 config frame: safe values of bytes 9-18 for each rate index 0-3 (384, 192, 96, 48).
    pub frame_bytes_9_18: [[u8; 10]; 4],
    /// Spectral-inversion flag: byte and bit in the frame (HIS 80 "Open").
    pub inversion_flag: (u8, u8),
}

/// Parameters for the radio, from a cleared HIS. Set only by a commit that cites the HIS.
pub const CLEARED: Option<BusParams> = None;

/// Parameters for emulator runs (feature `emulator-fpga-bus`), from the HIS as emulator
/// observations. Set only by a commit that cites the HIS.
pub const EMULATOR: Option<BusParams> = None;

/// Permission to drive the FPGA bus. Made only by [`permit`].
#[derive(Debug)]
pub struct BusPermit {
    pub params: BusParams,
    _sealed: (),
}

/// The bus permit of this build, if any.
pub fn permit() -> Option<BusPermit> {
    let p = if cfg!(feature = "emulator-fpga-bus") { EMULATOR } else { CLEARED };
    p.map(|params| BusPermit { params, _sealed: () })
}

/// Commands that MUST NOT be sent ([HIS] HIS 80).
pub const FORBIDDEN_COMMANDS: [u8; 4] = [0x05, 0x06, 0x09, 0x0A];

/// The commands this firmware may send, with a permit ([HIS] HIS 80 command table).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Command {
    Loopback = 0x00,
    Config = 0x01,
    Status = 0x02,
    RxIq = 0x04,
    Version = 0x08,
}

impl Command {
    pub const fn code(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_holds_by_default() {
        assert_eq!(CLEARED, None);
        assert_eq!(EMULATOR, None);
        assert!(permit().is_none(), "no FPGA bus permit may exist in this build");
    }

    #[test]
    fn forbidden_commands_are_not_sendable() {
        for c in [Command::Loopback, Command::Config, Command::Status, Command::RxIq, Command::Version] {
            assert!(!FORBIDDEN_COMMANDS.contains(&c.code()), "{c:?}");
        }
        // TX I&Q (0x03) is not in the list either: no TX path in this build.
        for code in 0u8..=0xFF {
            if FORBIDDEN_COMMANDS.contains(&code) {
                assert!(![0x00, 0x01, 0x02, 0x04, 0x08].contains(&code));
            }
        }
    }
}
