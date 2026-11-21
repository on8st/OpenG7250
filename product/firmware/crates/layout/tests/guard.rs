// SPDX-License-Identifier: GPL-3.0-or-later
//! The hard guard: nothing ever writes or erases 0x08000000-0x0801FFFF ([HIS] HIS 15 rule 1).
use g7250_layout::*;

const WRITERS: [Writer; 4] = [
    Writer::Application { running: Slot::A },
    Writer::Application { running: Slot::B },
    Writer::BootStage,
    Writer::StockInstaller,
];

#[test]
fn sector0_refused_for_every_writer_every_op_every_span() {
    for w in WRITERS {
        for op in [Op::Program, Op::Erase] {
            // Every flash word and every sector-sized span that starts in or reaches into sector 0.
            let mut a = VENDOR_BOOTLOADER.start;
            while a < VENDOR_BOOTLOADER.end {
                for len in [FLASH_WORD, SECTOR_SIZE, 2 * SECTOR_SIZE, 0x10_0000] {
                    assert_eq!(FlashGuard::check(w, op, a, len), Err(Refusal::VendorBootloader), "{w:?} {op:?} {a:#x}+{len:#x}");
                }
                a += FLASH_WORD;
            }
            // Spans that start below sector 0 and run into it, and wrap-around spans.
            assert_eq!(FlashGuard::check(w, op, 0x07FF_FFE0, 0x40), Err(Refusal::VendorBootloader));
            assert_eq!(FlashGuard::check(w, op, 0xFFFF_FFE0, 0x40), Err(Refusal::VendorBootloader));
            assert_eq!(FlashGuard::check(w, op, 0, u32::MAX), Err(Refusal::VendorBootloader));
            // Offsets relative to the flash base, as drivers pass them.
            assert_eq!(FlashGuard::check_offset(w, op, 0, SECTOR_SIZE), Err(Refusal::VendorBootloader));
            assert_eq!(FlashGuard::check_offset(w, op, 0x1_FFE0, 32), Err(Refusal::VendorBootloader));
        }
    }
}

#[test]
fn outside_flash_refused() {
    for w in WRITERS {
        assert_eq!(FlashGuard::check(w, Op::Program, FLASH_END, 32), Err(Refusal::OutsideApplicationArea));
        assert_eq!(FlashGuard::check(w, Op::Program, FLASH_END - 32, 64), Err(Refusal::OutsideApplicationArea));
        // System memory, option-byte area of other STM32 families, RAM: never flash writes.
        for a in [0x1FF0_0000u32, 0x1FFF_C000, 0x2000_0000, 0x2400_0000, 0x5200_2000] {
            assert!(FlashGuard::check(w, Op::Program, a, 32).is_err(), "{a:#x}");
        }
    }
}

#[test]
fn application_may_write_only_other_slot_and_config() {
    let w = Writer::Application { running: Slot::A };
    assert_eq!(FlashGuard::check(w, Op::Erase, SLOT_B.start, SECTOR_SIZE), Ok(()));
    assert_eq!(FlashGuard::check(w, Op::Program, CONFIG.start, 32), Ok(()));
    assert_eq!(FlashGuard::check(w, Op::Erase, SLOT_A.start, SECTOR_SIZE), Err(Refusal::NotAllowedForWriter));
    assert_eq!(FlashGuard::check(w, Op::Erase, BOOT_STAGE.start, SECTOR_SIZE), Err(Refusal::NotAllowedForWriter));
    assert_eq!(FlashGuard::check(w, Op::Erase, RESERVED_S6.start, SECTOR_SIZE), Err(Refusal::NotAllowedForWriter));
    // A span that runs from slot B into sector 6 is refused as a whole.
    assert_eq!(FlashGuard::check(w, Op::Erase, SLOT_B.start, 3 * SECTOR_SIZE), Err(Refusal::NotAllowedForWriter));
}

#[test]
fn boot_stage_writes_only_config() {
    assert_eq!(FlashGuard::check(Writer::BootStage, Op::Program, CONFIG.start + 64, 32), Ok(()));
    for r in [BOOT_STAGE, SLOT_A, SLOT_B, RESERVED_S6] {
        assert_eq!(FlashGuard::check(Writer::BootStage, Op::Program, r.start, 32), Err(Refusal::NotAllowedForWriter));
    }
}

#[test]
fn installer_writes_sectors_1_to_7_only() {
    assert_eq!(FlashGuard::check(Writer::StockInstaller, Op::Erase, APPLICATION_AREA.start, 7 * SECTOR_SIZE), Ok(()));
    assert_eq!(FlashGuard::check(Writer::StockInstaller, Op::Erase, 0x0800_0000, 8 * SECTOR_SIZE), Err(Refusal::VendorBootloader));
}

#[test]
fn alignment() {
    let w = Writer::StockInstaller;
    assert_eq!(FlashGuard::check(w, Op::Program, SLOT_A.start + 4, 32), Err(Refusal::Misaligned));
    assert_eq!(FlashGuard::check(w, Op::Erase, SLOT_A.start, FLASH_WORD), Err(Refusal::Misaligned));
    assert_eq!(FlashGuard::check(w, Op::Program, SLOT_A.start, 0), Err(Refusal::Misaligned));
}
