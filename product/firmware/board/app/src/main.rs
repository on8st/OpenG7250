// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Application of the G7250 main firmware, started by the boot stage from slot A or B.
//!
//! Milestone F1: safe state first, clocks on HSI, console banner, watchdog.
//! Milestone F2: CH394 on SPI6, IP configuration, discovery (`net`).
#![no_std]
#![no_main]

mod net;

use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_time::{Duration, Instant, Ticker, Timer};
use g7250_board::{FW_BETA, FW_BUILD, FW_MAJOR, FW_MINOR, FW_VERSION, console, early_init, println, reset, safe, wdg};
use g7250_layout::header::{self, Header, Kind};
use g7250_layout::{Range, Slot};
use g7250_safety::tx::{MEASURED, RfActuator, TxCore};

/// The radio side of an unkey: back to the safe state ([HIS] HIS 85). This build has no key
/// path at all (no `KeyPermit` can exist, `g7250_safety::tx`), so unkey is only ever the safe
/// state again.
pub struct BoardRf;

impl RfActuator for BoardRf {
    fn unkey(&mut self) {
        safe::reassert();
    }
}

#[cfg(not(feature = "slot-b"))]
const SLOT: Slot = Slot::A;
#[cfg(feature = "slot-b")]
const SLOT: Slot = Slot::B;
const SLOT_RANGE: Range = SLOT.range();

#[unsafe(link_section = ".image_header")]
#[used]
static HEADER: [u8; header::HEADER_LEN] =
    header::to_bytes(&Header::unsealed(Kind::Application, SLOT_RANGE.start, FW_VERSION, FW_BETA, env!("G7250_REV")));

/// The watchdog is refreshed from its own task, so a stalled executor resets the MCU.
const WDG_PET: Duration = Duration::from_millis(250);

#[embassy_executor::task]
async fn watchdog(mut wd: IndependentWatchdog<'static, embassy_stm32::peripherals::IWDG1>) {
    let mut t = Ticker::every(WDG_PET);
    loop {
        // Not transmitting (TX is never armed in this build): the safe state must hold. If it
        // does not, put it back; the fault path comes with the safety core (F3).
        if !safe::holds() {
            safe::reassert();
            println!("app: FAULT safe state found not held; re-applied");
        }
        wd.pet();
        t.next().await;
    }
}

fn clock_config() -> embassy_stm32::Config {
    use embassy_stm32::rcc::*;
    let mut c = embassy_stm32::Config::default();
    // HSI 64 MHz, no HSE, no PLL: the HSE frequency is not in the HIS (question F1-Q2).
    c.rcc.hsi = Some(HSIPrescaler::DIV1);
    c.rcc.csi = false;
    c.rcc.hsi48 = None; // no USB on the Main
    c.rcc.hse = None;
    c.rcc.pll1 = None;
    c.rcc.pll2 = None;
    c.rcc.pll3 = None;
    c.rcc.sys = Sysclk::HSI;
    c.rcc.d1c_pre = AHBPrescaler::DIV1;
    c.rcc.ahb_pre = AHBPrescaler::DIV1;
    c.rcc.apb1_pre = APBPrescaler::DIV1;
    c.rcc.apb2_pre = APBPrescaler::DIV1;
    c.rcc.apb3_pre = APBPrescaler::DIV1;
    c.rcc.apb4_pre = APBPrescaler::DIV1;
    c.rcc.voltage_scale = VoltageScale::Scale1;
    // [HIS] HIS 10 (`d199d33`, MEDIUM): the board is wired for "SMPS supplies the LDO, SMPS output
    // 2.5 V also available to external circuits" (RM0468), and firmware selects this
    // configuration and no other. PWR_CR3 is write-once per power-on: if the vendor bootloader
    // already wrote it, this write has no effect (the boot stage reports the value it found).
    c.rcc.supply_config = SupplyConfig::SMPSExternalLDO(SMPSSupplyVoltage::V2_5);
    c
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    early_init();
    let cause = reset::read(true);

    let p = embassy_stm32::init(clock_config());
    // The HAL init does not touch GPIO levels (feature gpio-init-analog is off); re-assert anyway.
    safe::reassert();
    console::init();
    // SAFETY: early_init cleared every enable and pending bit left by the previous stage.
    unsafe { cortex_m::interrupt::enable() };

    let mut wd = IndependentWatchdog::new(p.IWDG1, wdg::TIMEOUT_MS * 1000);
    wd.unleash();
    wd.pet();

    println!("");
    println!(
        "OpenG7250 main firmware {}.{}.{}{} rev {} slot {} (G7250-P2, board type 200)",
        FW_MAJOR,
        FW_MINOR,
        FW_BUILD,
        if FW_BETA { "-beta" } else { "" },
        env!("G7250_REV"),
        match SLOT {
            Slot::A => "A",
            Slot::B => "B",
        }
    );
    println!("app: reset cause {}", cause.name());
    if cause.is_watchdog() {
        // 0007 sub-decision 5 "Entry": a watchdog reset seen at start-up is a fault.
        println!("app: FAULT watchdog reset seen at start-up");
    }
    // Transmit gate (F3). The boot counter of B10.2 item 5 needs the configuration store, which
    // does not exist yet; with no calibration the core can never arm, so 0 is safe here.
    let tx = TxCore::new(0, &MEASURED);
    println!(
        "app: safe state {}; TX {} (record tx bit 1 = {}, bit 4 = {}); ATU bypass; watchdog {} ms",
        if safe::holds() { "held" } else { "NOT HELD" },
        if tx.calibrated() { "calibrated" } else { "NOT_CALIBRATED: every TX request is rejected" },
        tx.record_tx_bit1() as u8,
        tx.record_tx_bit4() as u8,
        wdg::TIMEOUT_MS
    );
    if tx.calibrated() {
        // Unreachable in this build; a calibrated build must not start without a review.
        panic!("calibrated build without a reviewed key path");
    }

    spawner.spawn(watchdog(wd).unwrap());

    let id = g7250_proto::discovery::Identity { mac: [0; 6], fw_major: FW_MAJOR, fw_minor: FW_MINOR, beta: FW_BETA, num_ddc: 1 };
    let pins = net::NetPins { spi: p.SPI6, sck: p.PG13, mosi: p.PG14, miso: p.PG12, cs: p.PG15, rst: p.PB5 };
    spawner.spawn(net::run(pins, id).unwrap());

    loop {
        Timer::after_secs(10).await;
        println!("app: alive {} s", Instant::now().as_secs());
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    cortex_m::interrupt::disable();
    safe::apply();
    println!("app: PANIC {}", info);
    // No refresh: the watchdog resets the MCU, and the reset brings the safe state back.
    loop {
        cortex_m::asm::nop();
    }
}
