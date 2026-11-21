// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 ON8ST
//! Blocking debug console on USART1 TX (PB6, AF7; [HIS] HIS 90 step 4: "Main debug console
//! (USART1 TX on PB6, RX on PB7, AF7; VERIFIED)"). 115200 8N1 is this firmware's choice.
//! Used by the boot stage and by panic output; the application has its own async console on the
//! same pins after `embassy_stm32::init`.
//!
//! Only PB6 is configured; PB7 (RX) is left untouched until a console input exists.

use crate::pac;
use crate::pac::gpio::vals::{Moder, Ospeedr, Ot, Pupdr};
use crate::pac::rcc::vals::{Hpre, Hsidiv, Ppre, Sw, Usart16910sel};

pub const BAUD: u32 = 115_200;
pub const HSI_HZ: u32 = 64_000_000;

/// Put the system clock on HSI 64 MHz with all bus prescalers at /1, whatever the previous
/// stage left. No external clock is assumed: the HSE frequency of the Main is not in the HIS
/// (HIS question F1-Q2).
pub fn clocks_hsi() {
    // Enough flash wait states for 64 MHz in any voltage scale before the switch
    // (RM0468, table "FLASH recommended number of wait states"); never lowered here.
    let acr = pac::FLASH.acr().read();
    if acr.latency() < 3 {
        pac::FLASH.acr().modify(|w| {
            w.set_latency(3);
            w.set_wrhighfreq(2);
        });
        while pac::FLASH.acr().read().latency() < 3 {}
    }
    pac::RCC.cr().modify(|w| {
        w.set_hsion(true);
        w.set_hsidiv(Hsidiv::DIV1);
    });
    while !pac::RCC.cr().read().hsirdy() {}
    pac::RCC.cfgr().modify(|w| w.set_sw(Sw::HSI));
    while pac::RCC.cfgr().read().sws() != Sw::HSI {}
    pac::RCC.d1cfgr().modify(|w| {
        w.set_d1cpre(Hpre::DIV1);
        w.set_hpre(Hpre::DIV1);
        w.set_d1ppre(Ppre::DIV1);
    });
    pac::RCC.d2cfgr().modify(|w| {
        w.set_d2ppre1(Ppre::DIV1);
        w.set_d2ppre2(Ppre::DIV1);
    });
    pac::RCC.d3cfgr().modify(|w| w.set_d3ppre(Ppre::DIV1));
}

/// Configure USART1 TX on PB6. Call after [`clocks_hsi`].
pub fn init() {
    pac::RCC.d2ccip2r().modify(|w| w.set_usart16910sel(Usart16910sel::PCLK2));
    pac::RCC.apb2enr().modify(|w| w.set_usart1en(true));
    let _ = pac::RCC.apb2enr().read();
    let g = pac::GPIOB;
    // Idle level first: AF mode with the USART still disabled could show a low on TX; set the
    // pin high as an output, then hand it to the USART.
    g.bsrr().write(|w| w.set_bs(6, true));
    g.otyper().modify(|w| w.set_ot(6, Ot::PUSH_PULL));
    g.pupdr().modify(|w| w.set_pupdr(6, Pupdr::PULL_UP));
    g.ospeedr().modify(|w| w.set_ospeedr(6, Ospeedr::LOW_SPEED));
    g.afr(0).modify(|w| w.set_afr(6, 7));
    let u = pac::USART1;
    u.cr1().write(|w| w.set_ue(false));
    u.brr().write_value(pac::usart::regs::Brr((HSI_HZ + BAUD / 2) / BAUD));
    u.cr1().write(|w| {
        w.set_te(true);
        w.set_ue(true);
    });
    g.moder().modify(|w| w.set_moder(6, Moder::ALTERNATE));
}

/// Wait 100 ms on the core SysTick timer (polled, no interrupt), with the CPU on HSI 64 MHz
/// ([`clocks_hsi`]). Used where the HAL timer is not running (boot stage, fault holds).
pub fn wait_100ms() {
    // SAFETY: SysTick is a core timer; early_init stopped it and nothing else uses it on these
    // paths.
    let mut syst = unsafe { cortex_m::Peripherals::steal() }.SYST;
    syst.set_clock_source(cortex_m::peripheral::syst::SystClkSource::Core);
    syst.set_reload(HSI_HZ / 10 - 1); // 6 399 999, fits the 24-bit reload
    syst.clear_current();
    syst.disable_interrupt();
    syst.enable_counter();
    while !syst.has_wrapped() {}
    syst.disable_counter();
}

pub fn write_bytes(b: &[u8]) {
    let u = pac::USART1;
    for &c in b {
        while !u.isr().read().txe() {
            crate::wdg::refresh();
        }
        u.tdr().write_value(pac::usart::regs::Dr(c as u32));
    }
}

/// Wait until the last byte has left the shift register.
pub fn flush() {
    while !pac::USART1.isr().read().tc() {}
}

pub struct Console;

impl core::fmt::Write for Console {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for part in s.split_inclusive('\n') {
            if let Some(line) = part.strip_suffix('\n') {
                write_bytes(line.as_bytes());
                write_bytes(b"\r\n");
            } else {
                write_bytes(part.as_bytes());
            }
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let _ = writeln!($crate::console::Console, $($arg)*);
    }};
}
