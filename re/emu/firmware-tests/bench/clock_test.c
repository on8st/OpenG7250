/* Clock self-test (own test scaffolding, ST RM0468 / Arm v7-M). At the reset
 * clock (HSI 64 MHz, all prescalers /1): TIM2 counts at 1 MHz (PSC 63) and its
 * update every 50 ms toggles PB0; SysTick at 1 ms (core clock) toggles PB1
 * every 50 ticks. The main loop sleeps in WFI. The runner trace measures both
 * periods in emulated time (runner self-check T-EMU-15). */
#include "bench.h"

#define GPIOB 0x58020400u
#define B_MODER REG(GPIOB + 0x00)
#define B_ODR   REG(GPIOB + 0x14)
#define TIM2 0x40000000u
#define T_CR1  REG(TIM2 + 0x00)
#define T_DIER REG(TIM2 + 0x0C)
#define T_SR   REG(TIM2 + 0x10)
#define T_EGR  REG(TIM2 + 0x14)
#define T_PSC  REG(TIM2 + 0x28)
#define T_ARR  REG(TIM2 + 0x2C)
#define SYST_CSR REG(0xE000E010u)
#define SYST_RVR REG(0xE000E014u)
#define SYST_CVR REG(0xE000E018u)

static volatile uint32_t ticks;

void TIM2_IRQHandler(void)
{
    if(T_SR & 1u) { T_SR = ~1u; B_ODR ^= 1u << 0; }
}

void SysTick_Handler(void)
{
    if(++ticks % 50 == 0) B_ODR ^= 1u << 1;
}

int main(void)
{
    console_init();
    puts_("bench clock_test\r\n");
    RCC_AHB4ENR |= 1u << 1;                       /* GPIOBEN */
    RCC_APB1LENR |= 1u << 0;                      /* TIM2EN */
    B_MODER = (B_MODER & ~0xFu) | 0x5u;           /* PB0, PB1 outputs */
    T_PSC = 63; T_ARR = 49999; T_EGR = 1; T_SR = 0;
    T_DIER = 1; nvic_enable(28);
    T_CR1 = 1;
    SYST_RVR = 64000 - 1; SYST_CVR = 0; SYST_CSR = 7;   /* processor clock, interrupt, enable */
    irq_on();
    puts_("clock_test running\r\n");
    for(;;) __asm volatile ("wfi");
}
