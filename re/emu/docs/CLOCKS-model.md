# Clock model (Main)

`models/csharp/G7250Clocks.cs` (`sysbus.clocks`), loaded by `product_run.py`
and `m1_run.py`. Source: ST RM0468 (public), RCC chapter.

## Why

Renode 1.17's STM32 timers count from a fixed input frequency set in its
platform description (250 MHz) and do not follow the RCC; SysTick counted from
a fixed NVIC frequency the runner set (275 MHz, the stock image's PLL clock).
A product image at the reset clock (HSI 64 MHz, all prescalers /1) therefore
saw its timer clock 3.9 times and SysTick 4.3 times too fast. Found with the
first product images (2026-11-21); every timing-dependent product result
before the fix is invalid.

## What it models

Follows every RCC write (through the RCC hook of the watchdog model):
- sys_ck from CFGR.SW: HSI (64 MHz / HSIDIV), CSI (4 MHz), HSE (`hseHz`,
  default 25 MHz: a model parameter for the stock regression only; the HSE of
  the board is not recorded, his/10), PLL1_P (source from PLLCKSELR, DIVM1,
  DIVN1, DIVP1, FRACN1 when PLL1FRACEN);
- the core clock (D1CPRE): SysTick input (NVIC frequency) and, in product
  runs, the CPU's instruction rate (PerformanceInMips = core MHz). Stock runs
  keep Renode's default rate (`setCpuRate: false`): the timers do not depend on
  it, and the fully logged stock boot would otherwise take far longer;
- hclk (HPRE), the APB clocks (D2PPRE1, D2PPRE2) and the timer kernel clocks
  by the TIMPRE rule (TIMPRE = 0: the APB clock if its prescaler is 1, else
  twice it; TIMPRE = 1: hclk if the prescaler is 1, 2 or 4, else four times the
  APB clock) for TIM2–7, 12–14 (APB1) and TIM1, 8, 15–17 (APB2).

## Own general-purpose timers (TIM2–5, TIM12–14)

`models/csharp/G7250Tim.cs` with `platforms/g7250_main_tim.repl` replaces
Renode's `STM32_Timer` for these instances in product and stock runs. Renode's
timer keeps internal counters per compare channel created with the platform
frequency, and a peripheral reset through the RCC restores that frequency; the
first product images used TIM12's compare channel and ran 3.9 times fast even
with the clock model. The own model per RM0468: CR1 (CEN, UDIS, URS, OPM, ARPE),
DIER, SR, EGR (UG, CCxG), CNT, PSC (preloaded, active from the next update
event), ARR (preloaded with ARPE), CCR1–4 with compare interrupts, update
interrupt, one-pulse mode, 16 or 32 bit; counting in emulated time at the
kernel clock the clock model gives (`SetKernelClock`) / (PSC + 1). Not
modelled: down and center-aligned counting, slave and trigger modes, input
capture, PWM outputs on pins, DMA requests. Other timers (TIM1, 6–8, 15–17)
stay Renode's, with their frequency set by the clock model.

## Not modelled

Clock switching delays, HSI/CSI calibration, PLL2/PLL3, kernel clock
selections other than the timers', LSE/LSI accuracy.

## Self-test

`firmware-tests/bench/clock_test.c` (product-style at 0x08020000, main loop in
WFI): TIM2 at a 1 MHz count toggles PB0 every 50 ms, SysTick at 1 ms toggles
PB1 every 50 ticks, TIM5's compare channel 1 toggles PB2 every 50 ms; the
runner trace measures all three: 50.0 ms each in emulated time (without the
model: TIM2 12.8 ms). With a product image (fw-dffed29) the image's own
"alive" lines come at 10.19 s for 10 s (boot about 0.2 s). `m1_smoke.sh` [8].
