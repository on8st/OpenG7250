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
- **HSE:** a 25 MHz crystal on OSC_IN/OSC_OUT, not an external clock (HIGH:
  the factory image runs HSE on with HSEBYP clear and derives all PLLs from
  25 MHz; a wrong HSE would put every UART and timer off by the ratio).
- **Core supply (PWR_CR3):** the board is wired for the SMPS: the factory image
  selects "SMPS 2.5 V output supplying external circuits and the LDO"
  (PWR_CR3 = 0x2E: SMPSLEVEL 2.5 V, SMPSEXTHP, SMPSEN, LDOEN) (HIGH, firmware
  analysis). PWR_CR3 can be written once per power-on (ST RM0468); a value that
  does not match the board's supply wiring can stop the MCU. Whether the vendor
  bootloader writes PWR_CR3 before the application runs is not known
  (`90-needs-hardware.md`): read PWR_CR3 at application start.
- **AXI SRAM:** 320 KiB at 0x24000000 on the units as shipped (HIGH: the
  factory application's initial stack pointer is 0x24050000, the top of
  320 KiB, so the TCM/AXI sharing option bytes leave the full 320 KiB to the
  AXI SRAM). Read-only check in step 2 of `90-needs-hardware.md`.

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
