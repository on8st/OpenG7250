"""Run-wide flash rules on a flash.log (format v1.8, proposed by the emulator side): in every
test, no erase or program of sector 0 (the vendor bootloader, [HIS] HIS 15; A1.8a item 1), no
option-byte write, and no address outside the main flash. Sector 1 (the boot stage) and the
store sectors 6 and 7 (0061) are judged by the tests that are about them, not here.

Layout ([HIS] HIS 15, as the emulator models it): one bank, eight 128 KiB sectors from
0x08000000; sectors 6 and 7 are 0x080C0000-0x080FFFFF.

GPL-3.0-or-later, (c) 2026 ON8ST.
"""
from __future__ import annotations

FLASH_BASE, SECTOR, N_SECTORS = 0x0800_0000, 128 * 1024, 8
FLASH_END = FLASH_BASE + SECTOR * N_SECTORS  # exclusive


def sector_of(address: int) -> int | None:
    if FLASH_BASE <= address < FLASH_END:
        return (address - FLASH_BASE) // SECTOR
    return None


def check_flash(entries: list[dict]):
    """(check name, ok, detail) for the run-wide rules."""
    sec0, option, outside = [], [], []
    for e in entries:
        op = e.get("op")
        if op == "option_write":
            option.append(e)
            continue
        if op == "bank_erase":
            sec0.append(e)  # a bank erase includes sector 0
            continue
        if op not in ("erase", "program"):
            continue
        addr = e.get("address")
        if addr is None:
            continue
        addr = int(addr, 0) if isinstance(addr, str) else int(addr)
        end = addr + int(e.get("len", 0) or 0)
        s = sector_of(addr)
        if s is None or end > FLASH_END:
            outside.append(e)
        elif s == 0 or e.get("sector") == 0:
            sec0.append(e)
    return [
        ("flash: sector 0 (vendor bootloader) never erased or programmed (HIS 15; A1.8a item 1)", not sec0,
         f"{len(sec0)} operations, first {sec0[0]}" if sec0 else "none"),
        ("flash: no option-byte write (A1.8a item 1)", not option,
         f"{len(option)} writes, first {option[0]}" if option else "none"),
        ("flash: no operation outside the main flash 0x08000000-0x080FFFFF", not outside,
         f"{len(outside)} operations, first {outside[0]}" if outside else "none"),
    ]
