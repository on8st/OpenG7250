// SPDX-License-Identifier: GPL-3.0-or-later
//! The EEPROM at I²C1 0x50 is read-only ([HIS] HIS 15 rule 3).
use embedded_hal::i2c::{ErrorKind, ErrorType, I2c, Operation};
use g7250_layout::eeprom::*;

#[derive(Default)]
struct Bus {
    sent: Vec<(u8, usize)>,
}
#[derive(Debug, PartialEq)]
struct E;
impl embedded_hal::i2c::Error for E {
    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}
impl ErrorType for Bus {
    type Error = E;
}
impl I2c for Bus {
    fn transaction(&mut self, a: u8, ops: &mut [Operation<'_>]) -> Result<(), E> {
        self.sent.push((a, ops.len()));
        Ok(())
    }
}

#[test]
fn eeprom_writes_never_reach_the_bus() {
    let mut g = GuardedI2c::new(Bus::default());
    assert_eq!(g.write(EEPROM_ADDR, &[0x00, 0x10, 0xAA]), Err(GuardError::EepromWriteRefused));
    assert_eq!(g.write(EEPROM_ADDR, &[0x00, 0x10]), Err(GuardError::EepromWriteRefused));
    assert_eq!(g.write(EEPROM_ADDR, &[]), Err(GuardError::EepromWriteRefused));
    // An address pointer followed by data and then a read is still a write: refused.
    let mut buf = [0u8; 4];
    assert_eq!(g.write_read(EEPROM_ADDR, &[0, 0x10, 0xAA], &mut buf), Err(GuardError::EepromWriteRefused));
    assert_eq!(
        g.transaction(EEPROM_ADDR, &mut [Operation::Write(&[0, 0]), Operation::Read(&mut buf), Operation::Write(&[1])]),
        Err(GuardError::EepromWriteRefused)
    );
    let mut b2 = [0u8; 2];
    assert_eq!(g.read(EEPROM_ADDR, &mut b2), Err(GuardError::EepromWriteRefused));
}

#[test]
fn eeprom_read_and_other_devices_pass() {
    let mut g = GuardedI2c::new(Bus::default());
    let mut buf = [0u8; 8];
    assert_eq!(read_eeprom(&mut g, 0x0100, &mut buf), Ok(()));
    assert_eq!(g.write(0x34, &[0x10, 0x00]), Ok(()));
}
