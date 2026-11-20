/* Bench smoke target: console banner and a GPIO blink (own test scaffolding). */
#include "bench.h"

#define GPIOB_BASE  0x58020400u
#define GPIOB_MODER REG(GPIOB_BASE + 0x00)
#define GPIOB_ODR   REG(GPIOB_BASE + 0x14)

int main(void)
{
    console_init();
    puts_("bench smoke: hello\r\n");
    RCC_AHB4ENR |= 1u << 1;                         /* GPIOBEN */
    GPIOB_MODER = (GPIOB_MODER & ~3u) | 1u;         /* PB0 output */
    int toggles = 0;
    for(int i = 0; i < 6; i++)
    {
        GPIOB_ODR ^= 1u;
        for(volatile int d = 0; d < 20000; d++) {}
        toggles++;
    }
    test_result("console", 1, 0, 0);
    test_result("blink", toggles == 6 && (GPIOB_ODR & 1u) == 0, "odr", GPIOB_ODR);
    return 0;
}
