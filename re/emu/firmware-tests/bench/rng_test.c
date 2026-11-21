/* RNG self-test (own test scaffolding, from ST RM0468, RNG chapter). */
#include "bench.h"

#define RNG 0x48021800u
#define R_CR REG(RNG + 0x00)
#define R_SR REG(RNG + 0x04)
#define R_DR REG(RNG + 0x08)
#define RNGEN (1u << 2)
#define DRDY (1u << 0)
#define CECS (1u << 1)
#define SECS (1u << 2)

int main(void)
{
    console_init();
    puts_("bench rng_test\r\n");
    RCC_AHB2ENR |= 1u << 6;                 /* RNGEN */
    R_CR = RNGEN;
    uint32_t v[8]; int got = 0;
    for(int i = 0; i < 8; i++)
    {
        for(int n = 0; n < 100000 && !(R_SR & DRDY); n++) {}
        if(R_SR & DRDY) { v[i] = R_DR; got++; }
    }
    test_result("rng_drdy", got == 8, "got", (uint32_t)got);
    int differ = 0;
    for(int i = 1; i < got; i++) if(v[i] != v[0]) differ = 1;
    test_result("rng_values_vary", differ, "v0", v[0]);
    test_result("rng_no_error_flags", (R_SR & (CECS | SECS)) == 0, "sr", R_SR);
    R_CR = 0;
    return 0;
}
