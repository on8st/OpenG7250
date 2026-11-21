// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Application of the G7250 main firmware, started by the boot stage from slot A or B.
//!
//! Milestone F1: safe state first, clocks on HSI, console banner, watchdog.
//! Milestone F2: CH394 on SPI6, IP configuration, discovery (`net`).
#![no_std]
#![no_main]

mod fault;
mod fpga;
mod net;
mod p2;
mod mgmt_init;
mod nv;
mod update;

use embassy_executor::Spawner;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_time::{Duration, Instant, Ticker, Timer};
use g7250_board::{FW_BETA, FW_BUILD, FW_MAJOR, FW_MINOR, FW_VERSION, console, early_init, println, reset, safe, wdg};
use g7250_layout::header::{self, Header, Kind};
use g7250_layout::{Range, Slot};
use g7250_safety::keypath::Keyer;
use g7250_safety::tx::{KeyPermit, MEASURED, RfActuator, TxCore};

/// The radio side of a key and an unkey. Unkey: back to the safe state ([HIS] HIS 85). Key:
/// the PA, bias and T/R sequence of A3.9a is not written yet, so a permit keys nothing and
/// `TX_STATE` bit 0 stays 0. No `KeyPermit` can exist in this build (`g7250_safety::tx`); the
/// assertion below stops a build with a complete calibration set until the sequence is in.
pub struct BoardRf;

const _: () = assert!(!MEASURED.complete_for_tx(), "calibrated build: write the A3.9a key sequence in BoardRf::key first");

impl RfActuator for BoardRf {
    fn unkey(&mut self) {
        safe::reassert();
    }
}

impl Keyer for BoardRf {
    fn key(&mut self, _permit: &KeyPermit<'_>) -> bool {
        safe::reassert();
        false
    }
    /// The CW waveform goes through the DUC on the FPGA bus, which stays closed (fpga_gate):
    /// nothing to write. Never called, since `key` never reports keyed.
    fn cw_element(&mut self, _on: bool) {}
    /// The ATU latches stay unwritten until P-ATU-1 (HIS 85, HIS-08): a cycle could not tune,
    /// so it fails at once. Never called: no cycle can start in this build.
    fn atu_step(&mut self, _now_ms: u64) -> Option<bool> {
        Some(false)
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
        // The TX-side pins are the TX path check's (tx_path task, A3.6b); the other safety pins
        // here.
        if !safe::holds_except_tx_path() {
            // A3.5: a TX path not blocked (drop event 13), class 2, latched until a reset; the
            // safe state is put back at once (HIS 85).
            safe::reassert();
            fault::raise(g7250_safety::fault::code::TX_PATH_NOT_BLOCKED);
            println!("app: FAULT safe state found not held; re-applied");
        }
        wd.pet();
        update::watchdog_petted(Instant::now().as_millis());
        t.next().await;
    }
}

fn clock_config() -> embassy_stm32::Config {
    use embassy_stm32::rcc::*;
    let mut c = embassy_stm32::Config::default();
    // HSI 64 MHz, no HSE, no PLL: the HSE frequency is not in the HIS (question F1-Q2).
    c.rcc.hsi = Some(HSIPrescaler::DIV1);
    c.rcc.csi = false;
    // HSI48 is the RNG kernel clock (RNGSEL reset value, RM0468); the RNG makes the management
    // challenges (A1.10; [HIS] HIS 10).
    c.rcc.hsi48 = Some(Default::default());
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
    // The instruction cache: code runs from flash with wait states (RM0468). Safe here: the
    // application never writes the flash it runs from. The data cache stays off: flash read back
    // after a program or erase, and the store, would need cache maintenance.
    // SAFETY: the core peripherals are taken once, at the start, for this only.
    unsafe { cortex_m::Peripherals::steal() }.SCB.enable_icache();
    let cause = reset::read(true);
    // TRIP_COUNT over resets without power loss (A3.5; Table B8-2): before anything else reads
    // or writes the persist block.
    let (trips, kept) = g7250_board::persist::start(cause.power_on || cause.brown_out);

    // The boot stage set the core supply once; the HAL init must not write it (vendored patch
    // "supply-once") and runs only if it holds the board's configuration and is ready.
    let (supply_ok, cr3) = g7250_board::supply::ready_for_init();
    if !supply_ok {
        let o = if g7250_safety::supply::is_his10(cr3) {
            g7250_safety::supply::Outcome::NotReady { cr3 }
        } else {
            g7250_safety::supply::Outcome::Mismatch { found: cr3 }
        };
        hold_without_hal(o);
    }
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
    println!("app: TRIP_COUNT {} ({})", trips, if kept { "kept over the reset" } else { "power-on or not found: from 0" });
    if cause.is_watchdog() {
        // 0007 sub-decision 5 "Entry": a watchdog reset seen at start-up is a fault.
        println!("app: FAULT watchdog reset seen at start-up");
        update::set_class2_fault();
    }
    // Non-volatile store in sector 7: boot counter (B10.2 item 5), secret, configuration,
    // calibration set. A store that cannot be opened is reported and the radio runs on defaults.
    let mut store = match g7250_store::Store::open(nv::BoardFlash::new(nv::writer_for(SLOT)), nv::writer_for(SLOT)) {
        Ok(s) => Some(s),
        Err(e) => {
            println!("app: FAULT store in sector 7 cannot be read ({:?}); running on defaults", e);
            None
        }
    };
    let boot_counter = match store.as_mut().map(|s| s.next_boot_counter()) {
        Some(Ok(n)) => n,
        Some(Err(e)) => {
            println!("app: FAULT boot counter not stored ({:?})", e);
            0
        }
        None => 0,
    };
    println!("app: boot counter {}", boot_counter);
    println!(
        "app: FPGA bus {} (bus pins {})",
        if g7250_safety::fpga_gate::permit().is_some() { "parameters cleared" } else { "closed: parameters not cleared (HIS 80); DDC0 not offered" },
        if g7250_safety::fpga_gate::permit().is_some() { "available" } else { "undriven" }
    );
    // Test build only: compact the store at every start, so that the emulator can cut power or
    // stretch the erase at any point of a compaction (docs/emulator-run.md, "Store").
    #[cfg(feature = "test-compact-at-start")]
    if let Some(s) = store.as_mut() {
        println!("app: TEST compaction: live sector {} generation {}", 6 + s.live_sector(), s.generation());
        match s.compact() {
            Ok(()) => println!("app: TEST compaction done: live sector {} generation {}", 6 + s.live_sector(), s.generation()),
            Err(e) => println!("app: TEST compaction FAILED ({:?})", e),
        }
    }
    // Transmit gate (F3). With no calibration the core can never arm.
    let tx = TxCore::new(boot_counter, &MEASURED);
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
    let pins = net::NetPins { spi: p.SPI6, sck: p.PG13, mosi: p.PG14, miso: p.PG12, cs: p.PG15, rst: p.PB5, rx_dma: p.BDMA_CH0, tx_dma: p.BDMA_CH1, int: p.PC13, int_exti: p.EXTI13 };
    let mgmt = mgmt_init::MgmtInit::new(store, tx.arm_state_reg(), SLOT);
    spawner.spawn(net::run(pins, id, mgmt, tx).unwrap());
    // From here the console no longer blocks: lines go into a ring that the net loop and this
    // task pump into the USART FIFO (0054: a blocking line costs about 5 ms).
    console::set_buffered(true);
    spawner.spawn(console_pump().unwrap());
    spawner.spawn(tx_path().unwrap());

    let mut n: u64 = 0;
    loop {
        Timer::after_secs(1).await;
        n += 1;
        // A trial not confirmed 60 s after its start: commanded reset, the boot stage starts the
        // previous image (A1.8 U-6; 0044). Here, not in the network task, so that it also holds
        // when the network never comes up. Nothing is keyed in this build (no RF to wait for).
        if update::trial_pending() && Instant::now().as_millis() >= update::TRIAL_LIMIT_MS {
            println!("app: FAULT trial image not confirmed 60 s after start; restarting into the previous image");
            safe::reassert();
            console::flush();
            cortex_m::peripheral::SCB::sys_reset();
        }
        if n % 10 == 0 {
            println!("app: alive {} s", Instant::now().as_secs());
        }
    }
}

/// Supply not as HIS 10 says (the boot stage should have stopped already): no HAL init, no
/// network, no transmit; the same fault line as the boot stage, once per second (spec A3.5a;
/// 0040). Safe state held, CH394 RST# still low, watchdog refreshed (the boot stage started it).
fn hold_without_hal(o: g7250_safety::supply::Outcome) -> ! {
    console::init();
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

/// Steps 4-5 of a commanded reset (A1.9): relays to the A3.6 state, for a factory reset the
/// Table A1-4 items in the store (A1.6a), then the reset.
pub fn safe_reset(mgmt: &mut mgmt_init::MgmtInit, factory: bool) -> ! {
    safe::reassert();
    if factory {
        if let Err(e) = mgmt_init::factory_reset_store(mgmt) {
            println!("app: FAULT factory reset of the store failed ({})", e);
        }
    }
    println!("app: commanded reset{}", if factory { " (factory reset)" } else { "" });
    console::flush();
    cortex_m::peripheral::SCB::sys_reset()
}

/// Moves buffered console output into the USART FIFO when the net loop does not (idle times).
static CONSOLE_OUT: embassy_sync::signal::Signal<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, ()> = embassy_sync::signal::Signal::new();

/// The TX path check (A3.6b; 0066): every 5 ms while not armed and not keyed, the TX-side pins'
/// configuration and input registers against the safe state; a mismatch over every check for
/// 10 ms raises TX_PATH_NOT_BLOCKED (class 2), sets the safe state again, and stays in the fault.
#[embassy_executor::task]
async fn tx_path() {
    use g7250_safety::txpath::{Check, PERIOD_MS};
    let mut c = Check::new();
    let mut t = Ticker::every(Duration::from_millis(PERIOD_MS));
    loop {
        let open = fault::TX_OPEN.load(core::sync::atomic::Ordering::Relaxed);
        let seen = if open { None } else { safe::tx_path_mismatch() };
        if let Some(m) = c.observe(Instant::now().as_millis(), open, seen) {
            safe::reassert();
            fault::set_detail(m);
            if fault::raise(g7250_safety::fault::code::TX_PATH_NOT_BLOCKED) {
                println!("app: FAULT TX path not blocked: P{:?}{} {:?}", m.port, m.pin, m.what);
            }
        }
        t.next().await;
    }
}

/// Waits for buffered output (the console's hook), then moves it into the USART FIFO; a timer
/// only while bytes wait for FIFO room (16 bytes at 115 200 Bd take about 1.4 ms). No periodic
/// wake while nothing is printed (WFE; emulator time.advance).
#[embassy_executor::task]
async fn console_pump() {
    console::on_output(|| CONSOLE_OUT.signal(()));
    loop {
        if !console::pending() {
            CONSOLE_OUT.wait().await;
        }
        console::pump();
        if console::pending() {
            Timer::after_millis(1).await;
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    cortex_m::interrupt::disable();
    safe::apply();
    console::set_buffered(false); // what is buffered first, then the panic, blocking
    println!("app: PANIC {}", info);
    // No refresh: the watchdog resets the MCU, and the reset brings the safe state back.
    loop {
        cortex_m::asm::nop();
    }
}
