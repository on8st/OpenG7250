# Flash layout and boot (firmware design, milestone F1)

Decision 0028 Decision 3 leaves the slot sizes, the place of configuration and log, and the boot
record format to firmware design. This is that design. The code is `crates/layout`
(`g7250-layout`); this page explains it.

## Main flash

Single bank, 1 MiB, eight sectors of 128 KiB ([HIS] HIS 15, `9d10e72`, part MEDIUM).

| Range | Sector | Content | Who may write |
|---|---|---|---|
| 0x08000000–0x0801FFFF | 0 | vendor bootloader | **nobody, ever** (HIS 15 rule 1) |
| 0x08020000–0x0803FFFF | 1 | our boot stage; the vendor bootloader starts it (HIS 10) | the stock installer only (0028 Decision 2) |
| 0x08040000–0x0807FFFF | 2–3 | slot A, 256 KiB | the application running from B; the stock installer |
| 0x08080000–0x080BFFFF | 4–5 | slot B, 256 KiB | the application running from A; the stock installer |
| 0x080C0000–0x080DFFFF | 6 | kept free | the stock installer only |
| 0x080E0000–0x080FFFFF | 7 | configuration, boot record, log | application, boot stage; the stock installer |

- Sector 6 is kept free because HIS 15 rule 5 has an open check whether the vendor bootloader
  uses sector 6 as a staging area (HYPOTHESIS). Once the F1 drill settles it, it can hold the log.
- Option bytes are not memory-mapped flash. They are protected by never using the option-byte
  registers (`FLASH_OPTCR`, `OPTSR_PRG`, `OPTKEYR` and the other `*_PRG` registers): a source scan
  test fails on any use.
- The EEPROM at I²C1 0x50 is read-only (HIS 15 rule 3): `g7250_layout::eeprom::GuardedI2c` lets
  only the address-then-read transaction through to 0x50.
- Backup SRAM and RTC backup registers are never used (HIS 15 rule 4). The vendored Embassy HAL
  is patched so that its init does not touch the backup domain (`board/vendor/README.md`).

## RAM

| Range | Use |
|---|---|
| 0x24000000–0x2401FFFF | AXI SRAM: `.data`, `.bss`, stack (128 KiB, present in every TCM/AXI option setting) |
| 0x38000000–0x38003FFF | SRAM4 (D3): section `.sram4`, the **only** place for SPI6 DMA buffers. SPI6 is in D3 and can use only the BDMA, which reaches only D3 memory (RM0468); a BDMA buffer in AXI or D2 SRAM gives a transfer error. Today SPI6 runs without DMA. |
| 0x38800000–0x38800FFF | Backup SRAM: never used ([HIS] HIS 15 rule 4); `g7250-imgtool` refuses any segment there |

## The guard

`FlashGuard::check(writer, op, addr, len)` is the one gate for every flash program and erase.
It refuses, in this order:

1. any span that shares a byte with sector 0, for every writer (`VendorBootloader`);
2. zero length, or a span outside sectors 1–7, or one that wraps around (`OutsideApplicationArea`);
3. a span not aligned to the flash word (32 bytes) or, for erase, to the sector (`Misaligned`);
4. a span outside the writer's own regions (`NotAllowedForWriter`).

Tests (`crates/layout/tests/guard.rs`) try every flash word of sector 0 with four lengths, for
every writer and both operations, plus spans that start below sector 0, wrap around, or run
from an allowed region into a forbidden one. In milestone F1 no code writes flash at all.

## Images

An image is linked for one place: the boot stage for 0x08020000, the application for slot A or
for slot B (cargo feature `slot-b`). It is not position-independent, so an update sends the
build for the other slot.

| Offset | Content |
|---|---|
| 0x000 | vector table (initial SP, reset vector, …; 0x2CC bytes on the H72x) |
| 0x400 | image header, 64 bytes |
| 0x440 | code, then read-only data, then the initial values of `.data` |

The image is one gap-free run of bytes (the linker scripts fill alignment gaps with 0xFF;
`g7250-imgtool` refuses an image with a gap), so its CRC does not depend on what a loader puts in
gaps.

Header (little-endian): magic `G7FW`, header version 1, header length 64, image length, CRC-32
(IEEE, over the image with the CRC field taken as zero), `fw_version` ([31:24] major, [23:16]
minor, [15:0] build, as record field `fw_version`, B4.8), hardware id `G725`, load address, kind
(1 boot stage, 2 application), beta flag, 16 bytes of source revision. The linker places it with
length and CRC zero; `g7250-imgtool seal` fills them after the link.

## Boot

1. The vendor bootloader starts our boot stage at 0x08020000 (HIS 10). Whether it does so for an
   image it did not write is HIS-19, proven only by the F1 drill (HIS 90).
2. Boot stage: mask interrupts; **safe state** (HIS 85, with PH9 high, 0029); clear every NVIC
   enable and pending bit and SysTick that a previous stage left; refresh the IWDG; clocks on HSI
   64 MHz with all prescalers /1; console on USART1 (PB6, 115200 8N1); start the IWDG (2 s);
   banner and reset cause; self-check of the safe state.
3. Slot choice (`g7250_layout::select`): the last valid boot record in sector 7, else slot A;
   a slot is started only if its header, length, CRC, load address and vectors check out; with
   neither slot valid the boot stage stays, holds the safe state and refreshes the watchdog.
4. Jump: D-cache off, VTOR to the slot, stack pointer and reset vector from the slot. Interrupts
   stay masked.
5. Application: the same early steps (safe state again, NVIC cleared), then the Embassy HAL init
   (HSI 64 MHz, no PLL, no HSE, no HSI48, `SupplyConfig::Default` = the H72x power-on value),
   console, IWDG at 2 s refreshed from its own task every 250 ms, banner.

**Trial boot.** The boot record format allows a trial slot with an attempt count (U-5, U-6). The
F1 boot stage does not write flash, so it cannot count attempts; it ignores a trial entry and
starts the active slot. Writing the record comes with the update path (A1.8).

**Watchdog.** Started by the boot stage, so a hung boot stage, a hung trial image or a hung
application resets the MCU, and the reset brings the safe state back. A panic re-applies the
safe state and stops refreshing.

## Outputs of `build.sh`

| File | What |
|---|---|
| `out/g7250.elf` | boot stage and slot A in one ELF (program headers only), for a single-file loader such as the emulator run (0034) |
| `out/g7250-boot.elf`, `out/g7250-app-a.elf`, `out/g7250-app-b.elf` | the sealed images with symbols |
| `out/*.bin` | flat sealed images, one per region |
| `out/SHA256SUMS` | checksums |
