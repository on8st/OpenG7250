# Answers to the firmware's HIS questions F1-Q1..Q6, F2-Q1..Q5 (RE side)

RE background for the HIS changes on his/next (sections 10, 40, 85, 90).
Questions: `product/firmware/docs/questions.md` (g7250-fw tree). Sources:
g7250-base (former data pin, historical name; see
`re/emu/docs/REPO_LAYOUT.md`), `docs/mcu_identification.md`,
`docs/hardware_init.md`, `main_unit/docs/flow.md`,
`main_unit/docs/peripherals.md` (SPI6 section),
`main_unit/analysis/peripherals/gpio_map.csv`. Conclusions only, no code.

| Id | Answer (HIS) | Evidence (RE side) | Tag |
| --- | --- | --- | --- |
| F1-Q1 | H725xG or H735xG; package unknown, narrowed by the port H pins in use | mcu_identification.md: TIM23/24, UART9/USART10 and SMPS supply code only exist on H72x/H73x; H723/H733 excluded (no SMPS, LOW-confidence exclusion); H725 vs H735 not distinguishable (crypto unused). The image reads DBGMCU_IDCODE for a revision guard. | MEDIUM |
| F1-Q2 | 25 MHz crystal, not bypass | flow.md clock table: HSE on, crystal (HSEBYP clear), HSE_VALUE 25 MHz; PLL1 M=2 N=44 from HSE → 550/275/275 MHz; PLL2 100 MHz for the ADC. | HIGH |
| F1-Q3 | Board wired for the SMPS; factory value PWR_CR3 = 0x2E | flow.md: the supply call with 0x2E right after HAL_Init, "SMPS 2.5 V output feeding external loads and the LDO", then VOS0 in PWR_D3CR. 0x2E decodes per RM0468 to SMPSLEVEL = 2.5 V, SMPSEXTHP, SMPSEN, LDOEN. | HIGH |
| F1-Q4 | Unknown | The vendor bootloader is not in the images and not analysed (his/15). The factory application configures supply, clocks and VTOR itself (flow.md), which says nothing about what the bootloader leaves; that observation is stock behaviour and is not put into the HIS. | — |
| F1-Q5 | Idle level high; board pulls unknown | gpio_map.csv: PD13, PG2, PG5 push-pull outputs, no pull, initial level 1, roles "SPI1 ATU ... latch CS". The idle level is an interface fact; the pin set-up as such is stock configuration (for the gate to judge). | HIGH (idle level) |
| F1-Q6 | 320 KiB AXI SRAM | mcu_identification.md and the Main memory map: initial MSP 0x24050000 = 0x24000000 + 320 KiB; nothing above it is referenced. | HIGH |
| F2-Q1 | 12–19 ms no-access window (manual); 10 ms low / 100 ms wait known to work; SCK 17.19 MHz | peripherals.md SPI6: the WCH manual CH394DS1 V1.2 §10.4.3 window as cited there; the hard re-init path holds RST# low 10 ms and waits 100 ms; at boot RST# is low for more than 2 s; prescaler /8 from PCLK4 137.5 MHz. | VERIFIED (manual), HIGH |
| F2-Q2 | CHIPV at 0x0039 read-only, value unknown; PHY register at 0x002E as on the W5500 | peripherals.md: manual table 6-1 and §7.1.16; every offset the driver uses matches the W5500 map, including the PHY configuration register. The value the factory image logs on the radio has not been captured. | VERIFIED (address), MEDIUM (bit layout) |
| F2-Q3 | MAC read from the chip; uniqueness unknown | network.md: the MAC is read from the CH394, not from the EEPROM. | HIGH |
| F2-Q4 | Board pull-up unknown; internal pull-up known to work | gpio_map.csv: PC13 interrupt input, falling edge, pull-up. | HIGH |
| F2-Q5 | Not known | The W5500 receive header has source IP, port and length only; the CH394 sections cited in the RE docs say nothing about a destination field. | — |

Emulator note (no stock detail): in the emulator the CH394 model's version
register reads 0x04 (the W5500 value), a HYPOTHESIS of the model, not a
statement about the chip.
