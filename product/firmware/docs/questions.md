# Open questions from the firmware session

Questions go to the coordinator (openg7250): HIS questions for the gate, spec questions for the
spec session (openg7250-spec). Until answered, the firmware uses the interim behaviour named in
each row, chosen to stay on the safe side.

## HIS questions

| Id | Question | Why it matters | Interim in the firmware |
|---|---|---|---|
| F1-Q1 | Exact Main part and package (STM32H725 or H735; LQFP176, BGA…)? DBGMCU_IDCODE and the marking (HIS 10 says MEDIUM). | HAL chip selection; pin availability on port H (HIS 85 uses PH3–PH12). | `stm32h725ig`; the register set is the same on the H735. |
| F1-Q2 | Is there an HSE crystal or clock on the Main, and at what frequency? | Clock tree; accuracy of UART and timers. | HSI 64 MHz, no PLL, no HSE. |
| F1-Q3 | Power supply configuration of the Main's core (LDO, SMPS, SMPS feeding LDO, external)? PWR_CR3 can be written once per power-on, and a wrong value can stop the MCU. | Safe power-up of our image. | Write only the H72x power-on value (`SupplyConfig::Default`); if the vendor bootloader already wrote PWR_CR3, our write has no effect. |
| F1-Q4 | In what state does the vendor bootloader start the application: clocks, PLL, VTOR, PRIMASK, caches, peripherals or interrupts left on, IWDG running? | Our early start-up must not assume reset values. | Defensive: mask interrupts, clear NVIC/SysTick, switch to HSI, set VTOR, start the IWDG ourselves. |
| F1-Q5 | ATU latch strobes PD13, PG2, PG5 are left untouched (HIS 85). Is there a board pull resistor on them, or can a floating strobe clock noise into a latch between reset and our start? | 0032 bypass relies on the latches staying unwritten. | Untouched, as HIS 85 says. |
| F1-Q6 | AXI SRAM size as configured (TCM/AXI sharing option bytes), read-only in HIS 90 step 2. | Buffer sizes for Ethernet and streaming. | 128 KiB of AXI SRAM, the part present in every option setting. |
| F2-Q1 | CH394 reset: minimum RST# low time and time to first SPI access after RST# goes high; maximum SCK. | Bring-up timing (HIS 40 gives "about 17 MHz known to work"). | 10 ms low, 200 ms wait, 16 MHz SCK. |
| F2-Q2 | CH394 VERSIONR value, and is PHYCFGR (link, speed, duplex) as on the W5500? | Probing the chip; LINK_DOWN drop event 8 (HIS-14). | Accept any version except 0x00/0xFF; PHYCFGR read as on the W5500. |
| F2-Q3 | Does the CH394 hold a MAC address after reset, and is it unique per unit (HIS 40, HIS-15)? | Discovery bytes 5–10 must be unique per unit (B3.6). | Use the CH394's value; if it reads all zero, all ones or multicast, derive a locally administered MAC from the MCU unique ID and write it to the chip. |
| F2-Q4 | Is INT# (PC13) pulled up on the board, or must the MCU pull it up (HIS 40: "needs a pull-up")? | Interrupt-driven receive. | INT# not used yet; the driver polls every 2 ms. |
| F2-Q5 | Does the CH394 report the destination address of a received UDP datagram (unicast, directed broadcast, limited broadcast)? The W5500 UDP header carries only the source. | B3.9 / T-B3-32: Set IP from outside the subnet is accepted only to 255.255.255.255. | See SQ-F2-2. |

## HIS questions for the DDC0 stream (FPGA bus, step 4)

The bus is gated in the code (`g7250_safety::fpga_gate`): no permit exists until every value
below comes through the HIS, for the radio (`CLEARED`, measured, HIS 90) or for emulator runs
(`EMULATOR`, an emulator observation the gate clears as such). Until then PE0–PE10 stay
undriven and DDC0 reports reason 2.

| Id | Question | Why |
|---|---|---|
| F4-Q1 | STB (PE9) and CMD (PE8) active levels; data set-up time, strobe width, and the bus turnaround: who drives D0–D7 after a command and when the FPGA releases them. | Bus contention is a hardware risk (HIS 80 "Open"). |
| F4-Q2 | 0x01 config frame: meaning and safe value of byte 0 (status flags) and of bytes 9–18 for each rate index (per-rate calibration), and the position of the spectral-inversion flag. | Tuning DDC0 or setting its rate needs a full frame; writing unknown values to the FPGA is not acceptable. |
| F4-Q3 | Which NCO word (offset 1 or 5) is the receive path for ADC0/DDC0, and f_clk (122.88 MHz, MEDIUM). | Frequency of DDC0 (B6.8; P-RX-1). |
| F4-Q4 | 0x04 RX I&Q read: byte order of the two signed 24-bit values, I before Q?, number of samples per DRDY per rate index (8/4/2/1, MEDIUM), and how many strobes one read takes. | Packing DDC0 (B6.3). |
| F4-Q5 | DRDY (PE10): active edge (rising, HIS 80) and what happens if a read is late (overrun flag in 0x02?). | Pacing and the DDC FIFO overflow bit (B9.2 byte 30). |

### Answers (cleared HIS `5292ba8`, gate review 8)

| Id | Answer | Firmware |
|---|---|---|
| F4-Q1 | STB, CMD active-high pulses (MEDIUM); MCU drives D0–D7 for commands and writes, inputs for reads, FPGA drives while STB is high, MCU drives again only after STB is low. Set-up, strobe width, turnaround: not known (radio). | Matches `fpga.rs` (release before a read, drive again only after STB low). Times still missing: gate closed. |
| F4-Q2 | Byte 0 and bytes 9–18: not known; DDC0 cannot run until understood; any 0x01 write only with the RF section in the HIS 85 safe state. Spec: P-RX-2 (0042). | Gate closed; record `streams` bit 0 = 0; DDC0 reason 2. |
| F4-Q3 | Which word (offset 1 or 5) is DDC0: not known; f_clk 122.88 MHz as the scale (MEDIUM). | Waits for the capture (HIS 90). |
| F4-Q4 | Big-endian 24-bit, one strobe per byte, six per sample (MEDIUM); I/Q order not known. | Waits. |
| F4-Q5 | DRDY most likely rising (MEDIUM); late-read behaviour not known. | Waits. |

## HIS answers (cleared HIS `d199d33`, gate review 6)

| Id | Answer | Firmware change |
|---|---|---|
| F1-Q1 | STM32H725xG or H735xG (MEDIUM), DEV_ID 0x483; package not known. | None: `stm32h725ig` stays; the pins used (PH3–PH12) need a package that bonds them out. |
| F1-Q2 | HSE not recorded until read from the board. | None: HSI 64 MHz only, no HSE, no PLL. |
| F1-Q3 | Board wired for "SMPS supplies the LDO, 2.5 V SMPS output also available to external circuits" (MEDIUM); select it and no other; PWR_CR3 write-once. | App: `SupplyConfig::SMPSExternalLDO(V2_5)` instead of the power-on default. Boot stage reports PWR_CR3 as found. |
| F1-Q4 | State at application start not known. | Boot stage captures PWR_CR3, RCC_CR/CFGR/CSR, PRIMASK, CCR, NVIC_ISER, IWDG PR/RLR at its first instruction and prints them (`boot: start …`). VTOR is already ours by then (cortex-m-rt `set-vtor`). |
| F1-Q5 | ATU strobes inactive high (MEDIUM): drive to the inactive level, never pulse. Board pulls not known. | Safe state drives PD13, PG2, PG5 high (level first, then output). |
| F1-Q6 | AXI SRAM 320 KiB as shipped (HIGH), may differ per unit. | None yet: the images keep using 128 KiB, valid for every option setting. |
| F2-Q1 | No access 12–19 ms after RST# high (VERIFIED); minimum low time and maximum SCK open; ~17 MHz seen in use. | None: RST# low since start + 10 ms, then 200 ms before the first access; 16 MHz. |
| F2-Q2 | CHIPV at 0x0039, value not known; PHY register 0x002E as W5500 (HIGH), bits MEDIUM. | None: any CHIPV except 0x00/0xFF accepted. |
| F2-Q3 | MAC from the chip; uniqueness open. | None. |
| F2-Q4 | INT# board pull-up unknown: use the MCU's internal pull-up. | INT# not used yet (polling); when used, input with internal pull-up. |
| F2-Q5 | Destination address not known from the sources. | None: B3.9 rule of 56197e4 applies (own subnet, or any source while link-local). |

## Spec answers (spec/draft 56197e4)

| Id | Answer | Firmware |
|---|---|---|
| tx bit 1 | r1.0 reports tx bit 1 = 1 and bit 4 from the calibration set; an uncalibrated arm gets 12 `NOT_CALIBRATED`. Bit 1 = 0 with 11 only before M4 without a key path (B4.8, B10 intro, T-B4-18). | Done: `TX_IMPLEMENTED = true`; an uncalibrated arm gets 12 (T-B4-18, T-B10-03, T-B10-41). |
| SQ-F2-1 | DHCP default; while no lease, RFC 3927 link-local **and keep retrying DHCP**; renewals borrow the management socket; an address change at renewal is A1.11a. No renewal is not conformant for r1.0. | **Open work:** DHCP retry while link-local, and renewal. |
| SQ-F2-2 | Without destination-address info every Set IP counts as unicast/directed: own subnet only, any source while link-local (B3.9; T-B3-32). | As implemented. |
| SQ-F2-3 | Keep the netmask if the new address is in that subnet, else /24; keep the gateway if inside the resulting subnet, else clear it; 0.0.0.0 selects DHCP; log the values (B3.9; T-B3-34). | Implemented (`netcfg::after_set_ip`). |
| SQ-F3-2 | A3.9b item 1: `BUSY`, `NOT_OWNER`, `BAD_VALUE`, `NOT_CALIBRATED`, `NOT_ARMED`. | As implemented. |
| SQ-F3-3 | The boot counter lives in the non-volatile store with configuration and calibration; without that store tx bit 4 MUST stay 0. | Bit 4 = 0; store not built yet. |

## Spec questions (as asked)

| Id | Question | Interim in the firmware |
|---|---|---|
| SQ-F2-1 | Factory default of C-1 (Table A1-4: "TBD (HIS-14)") and the fall-back when DHCP gets no lease; DHCP renewal (A1.11 is provisional). | DHCP at power-on on the management socket before the P2 sockets open (A1.11); no lease within 3 tries → RFC 3927 link-local address from the MAC; the lease is kept until restart (no renewal yet). |
| SQ-F2-2 | B3.9 distinguishes Set IP to the limited broadcast (any source) from Set IP to the unicast or directed broadcast address (own subnet only). If the CH394 cannot tell the destination apart (F2-Q5), which rule applies? | Accept from inside the subnet, or from any source while on a link-local address; otherwise ignore and log. T-B3-32 (second half) fails with this rule. |
| SQ-F2-3 | Set IP carries no netmask ([P2]). Which netmask applies after a Set IP? | Keep the netmask in use if the new address is a valid host in it, else 255.255.255.0. |
| SQ-F3-1 | Arm reject reason for missing calibration. | **Answered** by the spec at 0e075f2: arm reject 12 `NOT_CALIBRATED`, record `tx` bit 4, checked after 11 `TX_NOT_IMPLEMENTED` (B10.2 item 3). This build: bit 1 = 1 (r1.0, 0036), bit 4 = 0, so an arm gets 12. |
| SQ-F3-2 | `ATU_TUNE` = 1 while not armed and not calibrated: `NOT_ARMED` (9) or `NOT_CALIBRATED` (11) first? | `NOT_CALIBRATED` first. |
| SQ-F3-3 | B10.2 item 5 needs a boot counter in non-volatile memory for the token. The configuration store is not built yet. | Boot counter 0; harmless while TX cannot be armed; the store comes with the management channel. |
