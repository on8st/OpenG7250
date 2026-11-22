/* Timer monotonicity self-test (own test scaffolding, ST RM0468). TIM12 at 1 MHz, free running
 * over 16 bits, with a compare interrupt at the half period (CC1), the update interrupt and an
 * alarm compare (CC2) moved on at each alarm: the way a tick-less time driver uses a 16-bit
 * timer. The main loop reads the counter and the 32-bit time built from it without a pause and
 * counts every step backwards; the interrupt counts every entry with the counter before its event. */
#include "bench.h"

#define TIM12 0x40001800u
#define T(o) REG(TIM12 + (o))

static volatile uint32_t period;
static volatile uint32_t alarms, half, upd;
static volatile uint32_t early, latest;     /* entries with CNT before the event; worst CNT past it */

void TIM12_IRQHandler(void)
{
    uint32_t sr = T(0x10);
    uint32_t c = T(0x24) & 0xFFFFu;
    if((sr & 3u) == 1u && c >= 0x8000u) early++;
    if((sr & 3u) == 2u && c < 0x8000u) early++;
    if((sr & 3u) == 1u && c < 0x8000u && c > latest) latest = c;
    if((sr & 3u) == 2u && c >= 0x8000u && c - 0x8000u > latest) latest = c - 0x8000u;
    T(0x10) = ~sr;
    if(sr & 1u) { period++; upd++; }
    if(sr & 2u) { period++; half++; }
    if(sr & 4u) { alarms++; T(0x38) = (T(0x24) + 137u + (alarms & 63u)) & 0xFFFFu; }
}

static uint32_t now(void)
{
    uint32_t p = period;
    __asm volatile ("dmb");
    uint32_t c = T(0x24) & 0xFFFFu;
    return (p << 15) + (c ^ ((p & 1u) << 15));
}

int main(void)
{
    uint32_t n = 0, backt = 0, backc = 0, worst = 0, pt = 0, pc = 0, next = 2000000u;
    console_init();
    puts_("bench timmono_test\r\n");
    RCC_APB1LENR |= 1u << 6;                      /* TIM12EN */
    T(0x28) = 63; T(0x2C) = 0xFFFFu; T(0x14) = 1; T(0x10) = 0;
    T(0x34) = 0x8000u; T(0x38) = 500u;
    T(0x0C) = 1u | 2u | 4u;
    nvic_enable(43);
    T(0x00) = 1;
    irq_on();
    pt = now(); pc = T(0x24) & 0xFFFFu;
    for(;;)
    {
        uint32_t t = now();
        uint32_t c = T(0x24) & 0xFFFFu;
        if((n & 15u) == 0) T(0x40) = n & 0xFFFFu;                  /* a register write at any moment (CC4, not enabled) */
        uint32_t d = (c - pc) & 0xFFFFu;
        if((int32_t)(t - pt) < 0) { backt++; if(pt - t > worst) worst = pt - t; }
        if(d > 0x8000u) backc++;
        pt = t; pc = c; n++;
        if((int32_t)(t - next) > 0)
        {
            next += 2000000u;
            puts_("timmono t "); puthex(t); puts_(" n "); puthex(n); puts_(" time_back "); puthex(backt);
            puts_(" worst_us "); puthex(worst); puts_(" cnt_back "); puthex(backc);
            puts_(" early "); puthex(early); puts_(" latest_us "); puthex(latest);
            puts_(" alarms "); puthex(alarms); puts_(" upd "); puthex(upd); puts_(" half "); puthex(half); puts_("\r\n");
        }
    }
}
