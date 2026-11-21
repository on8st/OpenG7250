// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Boot stage of the G7250 main firmware (0028 Decision 3).
//!
//! Runs from 0x08020000, where the vendor bootloader starts an application ([HIS] HIS 10).
//! Order: safe state first ([HIS] HIS 85), then clocks on HSI, console, watchdog, then the slot
//! choice. It never writes flash in milestone F1; it is never updated over the network (0008 U-7).
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use g7250_board::{FW_BETA, FW_MAJOR, FW_MINOR, FW_BUILD, FW_VERSION, console, early_init, println, reset, safe, wdg};
use g7250_layout::header::{self, Header, Kind};
use g7250_layout::select::{self, Choice};
use g7250_layout::{BOOT_STAGE, CONFIG, Range, Slot};

#[unsafe(link_section = ".image_header")]
#[used]
static HEADER: [u8; header::HEADER_LEN] =
    header::to_bytes(&Header::unsealed(Kind::BootStage, BOOT_STAGE.start, FW_VERSION, FW_BETA, env!("G7250_REV")));

/// RAM an application may put its initial stack in (AXI SRAM, see memory-slot-*.x).
const APP_RAM: Range = Range::new(0x2400_0000, 0x2402_0000);

fn flash(r: Range) -> &'static [u8] {
    // SAFETY: the range is inside the on-chip flash, which is always mapped and readable.
    unsafe { core::slice::from_raw_parts(r.start as *const u8, r.len() as usize) }
}

fn slot_name(s: Slot) -> &'static str {
    match s {
        Slot::A => "A",
        Slot::B => "B",
    }
}

#[entry]
fn main() -> ! {
    early_init();
    console::clocks_hsi();
    console::init();
    wdg::start();
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

    if !safe::holds() {
        // The safe state could not be established: class 2 fault, stay here (0007 sub-decision 5).
        println!("boot: FAULT safe state not held; staying in the boot stage");
        hold();
    }
    println!("boot: safe state held, watchdog {} ms", wdg::TIMEOUT_MS);

    let record = select::last_record(flash(CONFIG));
    let valid = |s: Slot| {
        let r = s.range();
        match header::validate(flash(r), Kind::Application, r, APP_RAM) {
            Ok(_) => true,
            Err(e) => {
                println!("boot: slot {} not bootable: {:?}", slot_name(s), e);
                false
            }
        }
    };
    let mut choice = select::choose(record, &valid);
    if let Choice::Trial(_) = choice {
        // F1 does not write the boot record, so it cannot count a trial attempt. Without that
        // count a broken trial image would be restarted for ever; start the active slot instead.
        println!("boot: trial boot not supported in this boot stage; using the active slot");
        let r = record.map(|r| select::BootRecord { trial: None, ..r });
        choice = select::choose(r, &valid);
    }
    match choice {
        Choice::Boot(s) | Choice::Trial(s) => {
            println!("boot: starting slot {} at {:#010x}", slot_name(s), s.range().start);
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

/// Stay in the boot stage: safe state held, watchdog refreshed, a heartbeat on the console.
fn hold() -> ! {
    let mut n: u32 = 0;
    loop {
        for _ in 0..10 {
            cortex_m::asm::delay(console::HSI_HZ / 10);
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
