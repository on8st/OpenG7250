// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Boot stage of the G7250 main firmware (0028 Decision 3).
//!
//! Runs from 0x08020000, where the vendor bootloader starts an application ([HIS] HIS 10).
//! Order: safe state first ([HIS] HIS 85; 0045: within 20 us of the first instruction at
//! 0x08020000, from `__pre_init`), then clocks on HSI, console, watchdog, then the slot choice. It writes only the boot record in the store (sectors 6-7), through the flash guard as
//! `Writer::BootStage`: a trial attempt before a trial start, a rollback (A1.8 U-6;
//! `g7250_update::boot`). Never sector 0, the option bytes, the EEPROM or the Backup SRAM
//! ([HIS] HIS 15). It is never updated over the network (0008 U-7).
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use g7250_board::{FW_BETA, FW_MAJOR, FW_MINOR, FW_BUILD, FW_VERSION, console, early_init, println, reset, safe, wdg};
use g7250_layout::header::{self, Header, Kind};
use g7250_board::flash::RawFlash;
use g7250_layout::select::Choice;
use g7250_layout::{BOOT_STAGE, Range, Slot, Writer};
use g7250_store::Store;

#[unsafe(link_section = ".image_header")]
#[used]
static HEADER: [u8; header::HEADER_LEN] =
    header::to_bytes(&Header::unsealed(Kind::BootStage, BOOT_STAGE.start, FW_VERSION, FW_BETA, env!("G7250_REV")));

/// RAM an application may put its initial stack in (AXI SRAM, see memory-slot-*.x).
const APP_RAM: Range = Range::new(0x2400_0000, 0x2402_0000);

fn slot_name(s: Slot) -> &'static str {
    match s {
        Slot::A => "A",
        Slot::B => "B",
    }
}

/// The first code after the reset handler's stack pointer and vector table set-up, before the
/// RAM initialisation (cortex-m-rt): the RF control pins of HIS 85 at their safe levels within
/// 20 us of the first instruction (A3.6; 0045), before any other initialisation.
/// `board/check-safe-margin.py` checks this path in the built ELF.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __pre_init() {
    safe::apply_at_reset();
}

#[entry]
fn main() -> ! {
    // What the vendor bootloader left, before anything is changed ([HIS] HIS 90, F1-Q3/F1-Q4).
    let start = g7250_board::start_state::capture();
    early_init();
    console::clocks_hsi();
    console::init();
    let cause = reset::read(false);

    println!("");
    println!(
        "OpenG7250 boot {}.{}.{}{} rev {} (G7250-P2, board type 200)",
        FW_MAJOR,
        FW_MINOR,
        FW_BUILD,
        if FW_BETA { "-beta" } else { "" },
        env!("G7250_REV")
    );
    println!("boot: reset cause {}", cause.name());
    start.print();

    if !safe::holds() {
        // The safe state could not be established: class 2 fault, stay here (0007 sub-decision 5).
        println!("boot: FAULT safe state not held; staying in the boot stage");
        hold();
    }
    // Core supply, once and explicitly ([HIS] HIS 10; g7250_safety::supply), before any other
    // peripheral beyond the debug console is brought up (spec A3.5a item 1).
    match g7250_board::supply::configure() {
        g7250_safety::supply::Outcome::Kept => println!("boot: supply PWR_CR3 already SMPS->LDO 2.5 V, not written"),
        g7250_safety::supply::Outcome::Written => println!("boot: supply PWR_CR3 set to SMPS->LDO 2.5 V (was the power-on value)"),
        o => supply_fault(o),
    }

    wdg::start();
    println!("boot: safe state held, watchdog {} ms", wdg::TIMEOUT_MS);

    let valid = |s: Slot| {
        let r = s.range();
        // A word torn by a power loss while the slot was staged reads with an ECC error: the
        // slot is then not bootable, and the scan does not fault (R0).
        let image = match g7250_board::flash::mapped(r) {
            Ok(i) => i,
            Err(at) => {
                println!("boot: slot {} not bootable: unreadable flash word at {:#010x} (ECC)", slot_name(s), at);
                return false;
            }
        };
        match header::validate(image, Kind::Application, r, APP_RAM) {
            Ok(_) => true,
            Err(e) => {
                println!("boot: slot {} not bootable: {:?}", slot_name(s), e);
                false
            }
        }
    };
    let (va, vb) = (valid(Slot::A), valid(Slot::B));
    let valid = |s: Slot| if s == Slot::A { va } else { vb };
    let choice = match Store::open(RawFlash::new(Writer::BootStage), Writer::BootStage) {
        Ok(mut store) => {
            let before = g7250_update::boot::load(&mut store).ok().flatten();
            if let Some(r) = before {
                println!(
                    "boot: record active {} trial {} attempts {} confirmed {} seq {}",
                    slot_name(r.active),
                    r.trial.map(slot_name).unwrap_or("-"),
                    r.attempts_left,
                    r.confirmed,
                    r.seq
                );
            }
            let c = g7250_update::boot::boot_stage(&mut store, valid);
            let after = g7250_update::boot::load(&mut store).ok().flatten();
            if after.is_some_and(|r| r.rolled_back) && before.is_some_and(|r| !r.rolled_back) {
                println!("boot: trial not confirmed: rolled back (U-6)");
            }
            c
        }
        Err(e) => {
            // No record can be read or written: no trial (it could not be counted).
            println!("boot: store not readable ({:?}); no trial boot", e);
            g7250_layout::select::choose(None, valid)
        }
    };
    match choice {
        Choice::Boot(s) | Choice::Trial(s) => {
            let trial = if matches!(choice, Choice::Trial(_)) { " as a trial" } else { "" };
            println!("boot: starting slot {}{} at {:#010x}", slot_name(s), trial, s.range().start);
            console::flush();
            // SAFETY: the slot holds a sealed application linked for this address, checked above.
            unsafe { jump(s.range().start) }
        }
        Choice::NoImage => {
            println!("boot: no bootable image in slot A or B; holding the safe state");
            hold()
        }
    }
}

/// Fault before the network (spec A3.5a; 0040; T-A3-59): safe state held, CH394 RST# held low
/// (it has been since the safe state, and nothing here releases it), no flash write, no other
/// peripheral beyond the debug console, and once per second exactly the line of A3.5a item 2,
/// until power is removed. The IWDG is refreshed in case the option bytes start it from reset
/// ([HIS] HIS 10); the boot stage itself has not started it on this path.
fn supply_fault(o: g7250_safety::supply::Outcome) -> ! {
    let mut line = [0u8; g7250_safety::supply::FAULT_LINE_MAX];
    let n = g7250_safety::supply::fault_line(o, &mut line).unwrap_or(0);
    loop {
        console::write_bytes(&line[..n]);
        for _ in 0..10 {
            console::wait_100ms();
            safe::reassert();
            wdg::refresh();
        }
    }
}

/// Stay in the boot stage: safe state held, watchdog refreshed, a heartbeat on the console.
fn hold() -> ! {
    let mut n: u32 = 0;
    loop {
        for _ in 0..10 {
            console::wait_100ms();
            wdg::refresh();
        }
        n = n.wrapping_add(1);
        println!("boot: idle {}", n);
    }
}

/// Start the application at `base` the way the core starts after reset: vector table there,
/// stack pointer and reset vector from its first two words. Interrupts stay masked; the
/// application unmasks them after its own early init.
unsafe fn jump(base: u32) -> ! {
    unsafe {
        let mut p = cortex_m::Peripherals::steal();
        p.SCB.disable_dcache(&mut p.CPUID);
        p.SCB.vtor.write(base);
        cortex_m::asm::dsb();
        cortex_m::asm::isb();
        cortex_m::asm::bootload(base as *const u32)
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    safe::apply();
    println!("boot: PANIC {}", info);
    // No refresh: the watchdog resets the MCU, and the reset brings the safe state back.
    loop {
        cortex_m::asm::nop();
    }
}
