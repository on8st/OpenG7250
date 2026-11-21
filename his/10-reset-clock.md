# HIS 10 — Reset and clock

**Evidence:** the application load base and the bootloader region come from the
vendor image layout as analysed in g7250-base (**rests on image analysis /
disassembly**). Emulator observation (re/emu M1/M2): both stock images run when
loaded at 0x08020000 with the vector table there.

## Units
| Unit | MCU class |
| --- | --- |
| Main | STM32H72x |
| Control | STM32H743 |

## Debug and random-number hardware

- **SWD:** PA13 (SWDIO) and PA14 (SWCLK), per the ST datasheet. The factory
  application does not reconfigure these pins (HIGH: absent from the pin map),
  so SWD stays available. Whether pads or a header are reachable on the Main
  board is not known (`90-needs-hardware.md`).
- **RNG:** both candidate parts for the Main (STM32H725xG and H735xG) have a
  true random-number generator, per the ST datasheet (HIGH for "present", since
  the exact part is MEDIUM; confirm with DBGMCU_IDCODE and the part marking).

- **IWDG (open):** whether the independent watchdog starts from reset depends
  on the IWDG1_SW option byte, which is not yet read. If it is hardware-started,
  new firmware must service it from reset. Read it in step 2 of
  `90-needs-hardware.md`.

## Main MCU, clock, supply and memory (F1-Q1, F1-Q2, F1-Q3, F1-Q6)

- **Part:** STM32H725xG or STM32H735xG (MEDIUM: the firmware analysis shows a
  device with TIM23/TIM24, UART9/USART10 and the SMPS, which only the H72x/H73x
  have; H723/H733 have no SMPS). H725 and H735 cannot be told apart from the
  firmware. Expected DEV_ID 0x483. **Package: not known.** The board uses
  PH3, PH4, PH6 and PH9–PH12 (`85-rf-control.md`, `20-power.md`), so the
  package must bond out those pins; check the ST datasheet pinout tables.
  Confirm part and package with DBGMCU_IDCODE and the marking
  (`90-needs-hardware.md`).
- **HSE:** frequency, and crystal or external clock, are not recorded until
  read from the board (decision D1); see `90-needs-hardware.md`.
- **Core supply:** the board is wired for the RM0468 supply configuration
  *SMPS supplies the LDO, SMPS output 2.5 V also available to external
  circuits* (MEDIUM: inferred from the configuration the factory image selects;
  tied to the part, since only parts with an SMPS can be wired so). Whether any
  board circuit uses the 2.5 V SMPS output is not known. The supply
  configuration (PWR_CR3) can be written only once per power-on (RM0468, PWR
  chapter, "Power supply configuration"); a configuration that does not match
  the wiring can stop the MCU, so new firmware selects this configuration and
  no other until the wiring is checked. Whether the vendor bootloader has
  already written it is not known: read it at application start
  (`90-needs-hardware.md`).
- **AXI SRAM:** 320 KiB at 0x24000000 on the units as shipped (HIGH: the
  factory application's initial stack pointer is 0x24050000, the top of
  320 KiB, so the TCM/AXI sharing option bytes leave the full 320 KiB to the
  AXI SRAM). A unit whose option bytes were changed may differ; read them in
  step 2 of `90-needs-hardware.md` (read only).

## State at application start (F1-Q4, open)

The vendor bootloader at 0x08000000 is not analysed and not available
(`15-stock-recovery.md`). In which state it starts the application (clocks and
PLL, VTOR, PRIMASK, caches, peripherals or interrupts left on, whether the IWDG
already runs) is **not known**. New firmware must not assume reset values; the
facts can only come from the radio (`90-needs-hardware.md`).

## Image placement (platform constraint)
- The bootloader occupies 0x08000000–0x0801FFFF and is not part of the
  application image.
- The application image is loaded at **0x08020000**, with its vector table at
  that address. Any replacement image must follow this placement.
