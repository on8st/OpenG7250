# HIS 40 — Ethernet / LAN

**Evidence:** pins and SPI settings from g7250-base `main_unit/docs/peripherals.md`
(SPI6 section, VERIFIED in the firmware analysis, **rests on disassembly**);
register compatibility per the CH394 datasheet (WCH CH394DS1). Emulator (re/emu
B2): a W5500-register-compatible model on SPI6 with CS on PG15 is accepted by the
factory image; this shows consistency with a model built from the disassembly,
not hardware evidence.

- The Main's LAN controller is a **CH394** (W5500-register-compatible, per its
  datasheet) on **SPI6**, used by nothing else on that bus (HIGH). The LAN
  connector is on the Main unit (HIGH).

| Signal | Pin | Notes | Tag |
| --- | --- | --- | --- |
| SCK / MISO / MOSI | PG13 / PG12 / PG14 (AF5) | SPI mode 0, 8-bit, MSB first | VERIFIED |
| CS# | PG15 | GPIO, active low | VERIFIED |
| RST# | PB5 | active low | VERIFIED |
| INT# | PC13 | active low, needs a pull-up | VERIFIED |

- **MAC address (F2-Q3):** the CH394 provides the MAC address in its own
  register set after reset; the factory firmware does not store one in the
  EEPROM (HIGH, firmware analysis). Whether the value is unique per unit is
  **not known**: compare two units or check the CH394 datasheet
  (`90-needs-hardware.md`).
- **Reset timing (F2-Q1):** the CH394 manual (WCH CH394DS1 V1.2, §10.4.3)
  gives a window of 12–19 ms after RST# goes high in which the chip must not be
  accessed (VERIFIED, datasheet). The minimum RST# low time is open: to be
  taken from CH394DS1 (`90-needs-hardware.md`).
- **SCK:** the CH394 SPI device interface runs at up to 62 MHz in SPI mode 0
  or 3 (WCH CH394 manual CH394DS1 V1.2, features list; VERIFIED, datasheet).
  About 17 MHz is the upper value seen in use on the board link, not a
  datasheet limit (MEDIUM).
- **Version register (F2-Q2):** CHIPV at 0x0039, read-only (CH394DS1 V1.2,
  table 6-1 and §7.1.16; VERIFIED). Its value on the radio is **not known**
  (W5500 VERSIONR reads 0x04; do not rely on it). The documented software reset
  is MODE bit 7 (0x0000, §7.1.1); writing 0x0039 has no documented effect.
  The CH394 adds CHIP_CONFIG at 0x003E (IPv4/IPv6 select).
- **PHY status:** the register offsets, including the PHY configuration
  register at 0x002E, match the W5500 map (HIGH, firmware analysis; the check
  against CH394DS1 table 6-1 is open); the bit layout (link, speed, duplex) as
  on the W5500 is MEDIUM.
- **INT# pull-up (F2-Q4):** a board pull-up on PC13 is not known; use the
  MCU's internal pull-up unless one is found (`90-needs-hardware.md`).
- **Destination address of a received UDP datagram (F2-Q5):** the
  W5500-compatible receive header carries only the source address, source
  port and length. Whether the CH394 reports the destination (unicast,
  directed or limited broadcast) in any register is **not known** from the
  sources here; it needs the full CH394 manual or a test on the radio
  (`90-needs-hardware.md`).
- Network behaviour (addressing, ports, protocols) is defined by the product
  spec, not here.
