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
| 0x080C0000–0x080DFFFF | 6 | store, used in turn with sector 7 | application, boot stage; the stock installer |
| 0x080E0000–0x080FFFFF | 7 | store, used in turn with sector 6 | application, boot stage; the stock installer |

- Sectors 6 and 7 hold the store in turn, so that a compaction never erases the only copy
  (coordinator review of 25cd204). HIS 15 rule 5 has an open check whether the vendor bootloader
  uses sector 6 as a staging area (HYPOTHESIS). If it does, a vendor update overwrites the store
  there; that is harmless, since a vendor update replaces this firmware anyway (and our secret
  should not survive a return to stock, 0024 R5). If F1 showed that the bootloader *acts on*
  sector 6 at every start, this layout comes back for review.
- Option bytes are not memory-mapped flash. They are protected by never using the option-byte
  registers (`FLASH_OPTCR`, `OPTSR_PRG`, `OPTKEYR` and the other `*_PRG` registers): a source scan
  test fails on any use.
- The EEPROM at I²C1 0x50 is read-only (HIS 15 rule 3): `g7250_layout::eeprom::GuardedI2c` lets
  only the address-then-read transaction through to 0x50.
- Backup SRAM and RTC backup registers are never used (HIS 15 rule 4). The vendored Embassy HAL
  is patched so that its init does not touch the backup domain (`board/vendor/README.md`).

## Store (sectors 6 and 7)

`crates/store`: append-only records, each a whole number of 32-byte flash words, each word
programmed once; header (magic "G7ST", kind, payload version, length, sequence number, CRC-32)
and payload, padded with 0xFF. The newest valid record of a kind wins; a torn record fails its CRC
and is skipped. Kinds: boot record (1), boot counter (2), configuration (3, Table M-3 TLVs),
calibration set (4, the `SET_CALIBRATION` payload), management secret (5), commit (6).

**Generations.** A commit record (payload: generation) marks a sector as complete. The live sector
is the one whose newest commit has the highest generation; with no commit anywhere (a fresh unit,
or a store written by the step-2 image) sector 7 is live as generation 0, and its records are
read as they are.

**Compaction**, when the live sector is full (about every 4000 starts at one boot-counter record
per start): (0) copy the newest record of every kind to RAM; (1) erase the *other* sector;
(2) copy the records into it; (3) write a commit with the next generation; (4) switch to it. The
old sector is never touched by this; it is erased by the next compaction. A reset or power loss at
any step leaves the old committed sector live (steps 0–3, until the commit word is complete) or
the new one (after), with every value present. Host tests cut power at every flash word of a
compaction and halfway through the erase, and check that a half-erased sector's old commit does
not win (`crates/store`, 12 tests).

**Erase from RAM.** A 128 KiB sector erase can outlast the 2 s IWDG, and the CPU cannot fetch
from the bank being erased (RM0468). `board/app/src/nv.rs` erases with `erase_sector_from_ram`:
placed in `.data` (AXI SRAM), interrupts masked for its duration (vector table and handlers are
in that bank), own FLASH register access, IWDG refreshed on every poll of BSY/QW, poll count
bounded (far above the datasheet maximum); a timeout or an error flag is an error, the store
stays on the old sector. `board/check-ramfunc.sh` (run by `build.sh`) fails the build if the
function is not in RAM or branches out of it.

**Flash ECC (R0).** On the radio, a word left half-programmed or half-erased by a power loss may
carry an invalid ECC; a CPU read of it returns a bus error (RM0468, flash ECC: double error,
DBECCERR1 in FLASH_SR1), which the CRC scheme alone cannot catch. Every flash read of the store
and of the stager goes through `g7250_board::flash::read_tolerant`: FAULTMASK set and SCB
CCR.BFHFNMIGN for the copy only (a precise data bus fault at priority -1 is ignored, ARMv7-M),
then FLASH_SR1 says whether a word was unreadable (flags and the BusFault status cleared). The
store skips such a word like a torn record and never programs it (`FlashIo::unreadable`; host
tests cut power at every word of a compaction and during the erase with the cut word left
unreadable, and put unreadable words in a header, a payload and a commit). A slot is read
through its mapped slice only after `flash::probe` read every word of it: the boot stage, the
rollback check and IDENTIFY's integrity code treat a slot with an unreadable word as not
bootable / bad (code 2), and the stager's VERIFY answers 55. `board/check-faultmask.py` (run by
`build.sh`) checks in each ELF that nothing between `cpsid f` and `cpsie f` calls or branches
out. Not covered: an unreadable word in the image that is running, or in the boot stage itself
(both are only written by the vendor bootloader or a completed, verified update).

**Test build.** `out/test/g7250-compact.elf` (feature `test-compact-at-start`) compacts at every
start, for the emulator run (`docs/emulator-run.md`, "Store").

## RAM

| Range | Use |
|---|---|
| 0x24000000–0x2401FFDF | AXI SRAM: `.data`, `.bss`, stack (128 KiB less 32 bytes, present in every TCM/AXI option setting); the same for the boot stage |
| 0x2401FFE0–0x2401FFFF | AXI SRAM: `.persist` (application, NOLOAD), kept over a reset without power loss: `TRIP_COUNT` (A3.5, Table B8-2). Two slots `[magic, sequence, value, check]`, written alternately; read only when the reset flags show no power-on or brown-out reset (after a power loss it is written first, so no never-written SRAM word with a bad ECC is read, RM0468); a block overwritten by a previous stage fails the check and the count starts at 0. Not the Backup SRAM ([HIS] HIS 15 rule 4). **Pending hardware parameter:** whether this range survives the stage that runs before the boot stage at a reset is not known (open HIS item; first-radio check RD-01) |
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
2. Boot stage: **safe state at reset** from `__pre_init`, right after the reset handler has set
   the stack pointer and the vector table and before the RAM initialisation (A3.6; 0045: within
   20 us of the first instruction at 0x08020000, before any other initialisation): GPIO clocks
   on, then one straight-line step per port with masks computed at build time
   (`g7250_safety::safe_state::RESET_PLAN`). `board/check-safe-margin.py` (run by `build.sh`)
   checks the path in the built ELF and bounds it with a worst-case model (64 MHz, 16 cycles per
   flash word fetched and per load or store): 17.6 us today. Then, in `main`: the start state
   the previous stage left; mask interrupts; the full safe state (HIS 85, with PH9 high, 0029;
   also output speed and pulls); clear every NVIC
   enable and pending bit and SysTick that a previous stage left; refresh the IWDG; clocks on HSI
   64 MHz with all prescalers /1; console on USART1 (PB6, 115200 8N1); start the IWDG (2 s);
   banner and reset cause; self-check of the safe state.
3. Slot choice (`g7250_update::boot::boot_stage`): the newest boot record in the store (sectors
   6-7, kind 1), else slot A;
   a slot is started only if its header, length, CRC, load address and vectors check out; with
   neither slot valid the boot stage stays, holds the safe state and refreshes the watchdog.
4. Jump: D-cache off, VTOR to the slot, stack pointer and reset vector from the slot. Interrupts
   stay masked.
5. Application: the same early steps (safe state again, NVIC cleared), then the Embassy HAL init
   (HSI 64 MHz, no PLL, no HSE, no HSI48, `SupplyConfig::Default` = the H72x power-on value),
   console, IWDG at 2 s refreshed from its own task every 250 ms, banner.

**Update and trial boot** (A1.8; `g7250-update`). The application stages a new image in the
other slot (Writer::Application: only that slot and the store) and, after `UPDATE_VERIFY`,
writes one boot record: active = the running slot, trial = the other, one attempt
(`UPDATE_ACTIVATE`). The boot stage (Writer::BootStage: only the store) records the attempt
before it jumps to the trial; if it cannot, it starts the active slot. The trial confirms itself
(0044) after at least 10 s with the network up (CH394 initialised, management socket open), the
main loop running, the watchdog serviced and no class 2 fault (safe state found not held,
watchdog reset at start-up); until then arms get `MAINTENANCE`. Not confirmed 60 s after start:
the main task (independent of the network) makes a commanded reset. After that, or after any
other reset before the confirmation (crash, watchdog, power loss), the boot stage finds the
one attempt used and no confirmation, records a rollback (byte 12 bit 0) and starts the
previous image; an unconfirmed image is never tried twice (T-A1-68). The application logs it. `ROLLBACK` makes the other slot active. Each of these is one store record, so a power
loss leaves the old or the new record (host tests cut power at every flash word and erase).
Both the boot stage and the application write flash only through `g7250_board::flash` (erase
and program from RAM, interrupts masked, IWDG refreshed, FlashGuard per writer).

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
