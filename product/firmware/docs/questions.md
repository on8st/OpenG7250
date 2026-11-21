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

## Spec questions

| Id | Question | Interim in the firmware |
|---|---|---|
| SQ-F2-1 | Factory default of C-1 (Table A1-4: "TBD (HIS-14)") and the fall-back when DHCP gets no lease; DHCP renewal (A1.11 is provisional). | DHCP at power-on on the management socket before the P2 sockets open (A1.11); no lease within 3 tries → RFC 3927 link-local address from the MAC; the lease is kept until restart (no renewal yet). |
| SQ-F2-2 | B3.9 distinguishes Set IP to the limited broadcast (any source) from Set IP to the unicast or directed broadcast address (own subnet only). If the CH394 cannot tell the destination apart (F2-Q5), which rule applies? | Accept from inside the subnet, or from any source while on a link-local address; otherwise ignore and log. T-B3-32 (second half) fails with this rule. |
| SQ-F2-3 | Set IP carries no netmask ([P2]). Which netmask applies after a Set IP? | Keep the netmask in use if the new address is a valid host in it, else 255.255.255.0. |
| SQ-F3-1 | Arm reject reason for missing calibration. | **Answered** by the spec at 0e075f2: arm reject 12 `NOT_CALIBRATED`, record `tx` bit 4, checked after 11 `TX_NOT_IMPLEMENTED` (B10.2 item 3). This build: bit 1 = 0 (no P2 key path yet), so an arm gets 11; bit 4 = 0. |
| SQ-F3-2 | `ATU_TUNE` = 1 while not armed and not calibrated: `NOT_ARMED` (9) or `NOT_CALIBRATED` (11) first? | `NOT_CALIBRATED` first. |
| SQ-F3-3 | B10.2 item 5 needs a boot counter in non-volatile memory for the token. The configuration store is not built yet. | Boot counter 0; harmless while TX cannot be armed; the store comes with the management channel. |
