# HIS 90 — Open points that need the real radio

Questions from the product side that the sources and the emulator cannot
settle. Each needs a measurement or a test on the radio. Until then the
statements in the other sections stay as tagged there.

| Question | What is known now | What is needed |
| --- | --- | --- |
| HIS-02: the three NCO words in the FPGA 0x01 frame | The words are big-endian at byte offsets 1, 5 and 19 of the 24-byte frame (HIGH). Offsets 1 and 5 each set a receive-path tuning frequency (MEDIUM); offset 19 is most likely the transmit tuning word (HYPOTHESIS) (`80-rf-fpga.md`). | A capture on the radio including transmit, to confirm offset 19 and map offsets 1 and 5 to the physical receive paths. |
| HIS-07, HIS-11, HIS-12: timing and calibration values | Not in this spec on purpose: stock timing and stock calibration are not hardware facts. | Measurement on the radio. |
| HIS-17: power-up without the Control | Two hypotheses H1/H2 (`20-power.md`, Open). | Observation T0-P2: Main with the CTRL cable disconnected, DC on; does the Ethernet link come up? |
| HIS-19: does the vendor bootloader start an image it did not write; is there a bootloader entry without the Control | Unknown; the bootloaders are not analysed (`15-stock-recovery.md`). | The F1 drill, after the steps below. |
| IWDG start mode | Unknown: set by the IWDG1_SW option byte (`10-reset-clock.md`). | Read in step 2 below. |
| F1-Q1: Main part and package | STM32H725xG or H735xG (MEDIUM); package unknown (`10-reset-clock.md`). | DBGMCU_IDCODE in step 2; the part marking from a board photo. |
| F1-Q3, F1-Q4: supply wiring, state at application start | Supply configuration (MEDIUM, `10-reset-clock.md`); what the vendor bootloader leaves (clocks, VTOR, PRIMASK, caches, peripherals, IWDG, supply configuration) is unknown. | SMPS and LDO pins (VLXSMPS, VFBSMPS, VCAP) and any external use of the SMPS output, from board photos. After F1: the test image records PWR_CR3, RCC_CR/CFGR, VTOR, PRIMASK, CCR (caches), NVIC_ISER and the IWDG state at its first instruction and reports them on USART1. |
| HSE (decision D1) | Not recorded (`10-reset-clock.md`). | The crystal or oscillator marking near OSC_IN/OSC_OUT, from a board photo. |
| F1-Q5: ATU strobe pull resistors | Inactive level high (MEDIUM); board pulls unknown (`85-rf-control.md`). | Continuity or resistance check on PD13, PG2, PG5 to VDD/GND with power off, from board photos first. |
| F1-Q6: AXI SRAM size | 320 KiB (HIGH, `10-reset-clock.md`). | Option bytes (TCM/AXI sharing) in step 2. |
| F2-Q1: CH394 minimum RST# low time and maximum SPI clock | Not yet taken from the datasheet (`40-ethernet.md`). | From CH394DS1 (not a radio measurement). |
| F2-Q2, F2-Q3: CH394 version value, MAC uniqueness | CHIPV at 0x0039; MAC read from the chip (`40-ethernet.md`). | Read CHIPV and the MAC on two units. |
| F2-Q4: INT# board pull-up | Unknown; internal pull-up known to work (`40-ethernet.md`). | Resistance check on PC13 with power off. |
| F2-Q5: destination address of received UDP | The W5500-style header has the source only (`40-ethernet.md`). | The full CH394 manual, or a test: send to unicast, directed and limited broadcast and compare what the chip reports. |

## Safe order of the hardware steps for HIS-19 / F1

For every step: radio in RX, a dummy load on **both** ANT1 and ANT2 (which port
is selected, and the ATU latch state after reset, are unknown; `85-rf-control.md`).
Nothing below writes or erases unless the step says so.

1. **Locate SWD on the Main board.** The Main's SWD pins are PA13 (SWDIO) and
   PA14 (SWCLK) (ST datasheet). The factory application does not reconfigure
   them (HIGH: absent from the pin map), so SWD stays available while it runs.
   Whether pads or a header are reachable on the board is **not known**; look
   for them on board photos first (no wiring with power on).
2. **Read the Main's option bytes via SWD, read only.** ST-LINK in "hot plug"
   mode (no reset, no halt; halting the Main can freeze the PA state). Read RDP,
   BOOT_ADD0/1, WRP/PCROP, BOR, CPUFREQ_BOOST, the IWDG settings (IWDG1_SW and
   the freeze-in-debug bits), DBGMCU_IDCODE (Main expected DEV_ID 0x483) and the
   flash-size register. Never write, never press "Apply": changing RDP from
   level 1 to 0 erases the flash, level 2 is permanent. If the connection fails
   because of RDP level 2, record that and do not retry under reset.
3. **Only if the Main's RDP is level 0: back up the Main's full flash,**
   including the bootloader sector. Back up the Control only after its own
   steps 1–2: its SWD access point is unknown, and its option bytes must be read
   first (expected DEV_ID 0x450); otherwise limit the backup to the Main. The
   dumps are RE-side material: keep them locally, outside any repository, never
   give them to the product side, and analyse them (for the bootloader entry)
   only on the RE side under the clean-room rules. If RDP is level 1, stop here
   and decide with ON8ST: no backup is possible, so a failed F1 could only be
   recovered through the vendor path.
4. **Only then try F1** with a minimal test image, by SWD, which is allowed only
   at RDP level 0 (step 3).
   - **The test image** holds the safe state of `85-rf-control.md`, drives PH9
     high from reset (under H1 the unit otherwise drops power, `20-power.md`),
     and toggles no other pin. It reports that it runs through the Main debug
     console (USART1 TX on PB6, RX on PB7, AF7; VERIFIED) or through a RAM counter read
     over SWD.
   - **Programming:** no full-chip or mass erase. Sector-erase only the sectors
     the image needs (1…n) and write from 0x08020000. Read sector 0 back before
     and after, and compare it with the step-3 backup.
   - **Test:** check whether the vendor bootloader starts the test image. Then
     restore the factory firmware through the vendor update path
     (`15-stock-recovery.md`) and confirm stock works.
   - **Fallback:** if the vendor path fails, restore the application sectors
     1–7 from the step-3 backup by SWD.
