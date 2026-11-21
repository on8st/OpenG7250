/* Bench test scaffolding (own code, from ST RM0468; not product code).
 * Tests print "TEST <name> PASS" or "TEST <name> FAIL <detail> <value>" on
 * USART1 and finish with "BENCH DONE"; the bench runner reads these lines. */
#ifndef BENCH_H
#define BENCH_H
#include <stdint.h>

#define REG(a) (*(volatile uint32_t *)(a))
#define RCC_BASE      0x58024400u
#define RCC_AHB3ENR   REG(RCC_BASE + 0xD4)
#define RCC_AHB1ENR   REG(RCC_BASE + 0xD8)
#define RCC_AHB4ENR   REG(RCC_BASE + 0xE0)
#define RCC_APB1LENR  REG(RCC_BASE + 0xE8)
#define RCC_APB2ENR   REG(RCC_BASE + 0xF0)
#define RCC_APB4ENR   REG(RCC_BASE + 0xF4)

#define NVIC_ISER(n)  REG(0xE000E100u + 4u * (n))
static inline void nvic_enable(unsigned irq) { NVIC_ISER(irq >> 5) = 1u << (irq & 31); }
static inline void irq_on(void) { __asm volatile ("cpsie i" ::: "memory"); }

/* bench-only request source (platforms/g7250_bench.repl): writing N pulses
 * its output N times into a DMAMUX request input */
#define BENCHREQ_PULSE REG(0xA0001000u)
#define BENCHREQ_ID    113u

void console_init(void);
void putch(char c);
void puts_(const char *s);
void puthex(uint32_t v);
void putdec(uint32_t v);
void test_result(const char *name, int ok, const char *detail, uint32_t value);
void bench_done(void);
#endif
