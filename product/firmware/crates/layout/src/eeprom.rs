// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Read-only access to the 24Cxx EEPROM at I²C1 0x50 ([HIS] HIS 15 rule 3; HIS 30).
//!
//! [`GuardedI2c`] wraps the I²C bus. Towards address 0x50 it lets through exactly one shape of
//! transaction: a write of the two address bytes followed by a read (the 24Cxx random/sequential
//! read, 16-bit addressing per HIS 30). Every other transaction to 0x50, and every write-only
//! transaction, is refused before it reaches the bus. Other addresses pass unchanged.

use embedded_hal::i2c::{self, ErrorKind, ErrorType, I2c, Operation, SevenBitAddress};

/// 7-bit address of the factory EEPROM ([HIS] HIS 30).
pub const EEPROM_ADDR: u8 = 0x50;

/// Error of the guarded bus.
#[derive(Debug, PartialEq, Eq)]
pub enum GuardError<E> {
    /// The transaction would write the EEPROM; it was not sent.
    EepromWriteRefused,
    /// Error of the underlying bus.
    Bus(E),
}

impl<E: i2c::Error> i2c::Error for GuardError<E> {
    fn kind(&self) -> ErrorKind {
        match self {
            GuardError::EepromWriteRefused => ErrorKind::Other,
            GuardError::Bus(e) => e.kind(),
        }
    }
}

/// The only transaction shape allowed towards the EEPROM.
pub fn is_eeprom_read(ops: &[Operation<'_>]) -> bool {
    matches!(ops, [Operation::Write(a), Operation::Read(_)] if a.len() == 2)
}

pub struct GuardedI2c<I> {
    inner: I,
}

impl<I> GuardedI2c<I> {
    pub fn new(inner: I) -> Self {
        Self { inner }
    }
}

impl<I: I2c> ErrorType for GuardedI2c<I> {
    type Error = GuardError<I::Error>;
}

impl<I: I2c> I2c<SevenBitAddress> for GuardedI2c<I> {
    fn transaction(&mut self, address: u8, operations: &mut [Operation<'_>]) -> Result<(), Self::Error> {
        if address == EEPROM_ADDR && !is_eeprom_read(operations) {
            return Err(GuardError::EepromWriteRefused);
        }
        self.inner.transaction(address, operations).map_err(GuardError::Bus)
    }
}

/// Convenience: read `buf.len()` bytes from EEPROM offset `offset`.
pub fn read_eeprom<I: I2c>(bus: &mut GuardedI2c<I>, offset: u16, buf: &mut [u8]) -> Result<(), GuardError<I::Error>> {
    bus.write_read(EEPROM_ADDR, &offset.to_be_bytes(), buf)
}
