/* Startup and console for the bench test programs (own code, RM0468). */
#include "bench.h"

extern uint32_t _estack, _sidata, _sdata, _edata, _sbss, _ebss;
int main(void);

void default_handler(void) { puts_("FAULT unexpected exception\r\n"); bench_done(); for(;;) {} }
__attribute__((weak)) void hardfault_handler(void) { puts_("FAULT HardFault\r\n"); bench_done(); for(;;) {} }
__attribute__((weak)) void usagefault_handler(void) { default_handler(); }

#define WEAK __attribute__((weak, alias("default_handler")))
void DMA1_Stream0_IRQHandler(void) WEAK;
void DMA1_Stream1_IRQHandler(void) WEAK;
void DMA1_Stream2_IRQHandler(void) WEAK;
void DMA1_Stream3_IRQHandler(void) WEAK;
void ADC_IRQHandler(void) WEAK;
void USART2_IRQHandler(void) WEAK;
void MDMA_IRQHandler(void) WEAK;
void EXTI15_10_IRQHandler(void) WEAK;
void TIM2_IRQHandler(void) WEAK;
void TIM5_IRQHandler(void) WEAK;
void TIM12_IRQHandler(void) WEAK;
void SysTick_Handler(void) WEAK;

/* runs as the first code of the image, before .data and .bss are set (registers and fixed RAM only) */
__attribute__((weak)) void early_hook(void) {}

void reset_handler(void)
{
    early_hook();
    uint32_t *s = &_sidata, *d = &_sdata;
    while(d < &_edata) *d++ = *s++;
    for(d = &_sbss; d < &_ebss; ) *d++ = 0;
    REG(0xE000ED88u) |= 0xFu << 20;   /* CPACR: FPU on */
    main();
    bench_done();
    for(;;) { __asm volatile ("wfi"); }
}

/* 16 system vectors + 163 interrupts (RM0468 vector table) */
__attribute__((section(".isr_vector"), used))
void (*const vectors[16 + 163])(void) = {
    [0] = (void (*)(void))&_estack,
    [1] = reset_handler,
    [2 ... 15] = default_handler,
    [3] = hardfault_handler,
    [6] = usagefault_handler,
    [16 ... 16 + 162] = default_handler,
    [16 + 11] = DMA1_Stream0_IRQHandler,
    [16 + 12] = DMA1_Stream1_IRQHandler,
    [16 + 13] = DMA1_Stream2_IRQHandler,
    [16 + 14] = DMA1_Stream3_IRQHandler,
    [16 + 18] = ADC_IRQHandler,
    [16 + 38] = USART2_IRQHandler,
    [15] = SysTick_Handler,
    [16 + 28] = TIM2_IRQHandler,
    [16 + 50] = TIM5_IRQHandler,
    [16 + 43] = TIM12_IRQHandler,
    [16 + 40] = EXTI15_10_IRQHandler,
    [16 + 122] = MDMA_IRQHandler,
};

/* USART1 console. The pins are not set to their alternate function: the
 * emulator does not need it to transmit (a real board would). */
#define USART1_BASE 0x40011000u
#define U1_CR1 REG(USART1_BASE + 0x00)
#define U1_BRR REG(USART1_BASE + 0x0C)
#define U1_ISR REG(USART1_BASE + 0x1C)
#define U1_TDR REG(USART1_BASE + 0x28)

void console_init(void)
{
    RCC_APB2ENR |= 1u << 4;           /* USART1EN */
    U1_BRR = 0x364;
    U1_CR1 = (1u << 3) | (1u << 0);   /* TE, UE */
}

void putch(char c)
{
    uint32_t n = 0;
    while(!(U1_ISR & (1u << 7)) && ++n < 100000) {}   /* TXE */
    U1_TDR = (uint8_t)c;
}

void puts_(const char *s) { while(*s) putch(*s++); }

void puthex(uint32_t v)
{
    puts_("0x");
    for(int i = 28; i >= 0; i -= 4) putch("0123456789ABCDEF"[(v >> i) & 0xF]);
}

void putdec(uint32_t v)
{
    char b[11]; int i = 10; b[i] = 0;
    do { b[--i] = (char)('0' + v % 10); v /= 10; } while(v && i);
    puts_(&b[i]);
}

void test_result(const char *name, int ok, const char *detail, uint32_t value)
{
    puts_("TEST "); puts_(name); puts_(ok ? " PASS" : " FAIL");
    if(!ok && detail) { putch(' '); puts_(detail); putch(' '); puthex(value); }
    puts_("\r\n");
}

void bench_done(void) { puts_("BENCH DONE\r\n"); }
