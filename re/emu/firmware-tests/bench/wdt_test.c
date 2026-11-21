/* Watchdog and reset-flag self-test (own test scaffolding, from ST RM0468).
 * First boot (power-on flags): start IWDG1 with a short timeout, refresh it for
 * a while, then stop refreshing so it resets the MCU. Second boot: the reset
 * flags show an IWDG1 reset; clearing them with RMVF works. */
#include "bench.h"

#define RCC_RSR  REG(RCC_BASE + 0xD0)
#define PORRSTF  (1u << 23)
#define PINRSTF  (1u << 22)
#define IWDG1RSTF (1u << 26)
#define RMVF     (1u << 16)

#define IWDG 0x58004800u
#define KR  REG(IWDG + 0x00)
#define PR  REG(IWDG + 0x04)
#define RLR REG(IWDG + 0x08)
#define SR  REG(IWDG + 0x0C)

/* in the NOLOAD D2 SRAM section, not cleared by the startup code */
__attribute__((section(".dmabuf"))) static volatile uint32_t marker;

int main(void)
{
    console_init();
    uint32_t rsr = RCC_RSR;
    puts_("bench wdt_test rsr="); puthex(rsr); puts_("\r\n");
    if(rsr & IWDG1RSTF)
    {
        test_result("wdt_reset_flag", rsr == (IWDG1RSTF | PINRSTF), "rsr", rsr);
        test_result("wdt_ram_kept", marker == 0x5EED1234u, "marker", marker);
        RCC_RSR = RMVF;
        test_result("wdt_flags_cleared", RCC_RSR == 0, "rsr", RCC_RSR);
        return 0;
    }
    test_result("wdt_power_on_flags", (rsr & PORRSTF) && (rsr & PINRSTF) && !(rsr & IWDG1RSTF), "rsr", rsr);
    RCC_RSR = RMVF;                    /* the flags are sticky until cleared */
    test_result("wdt_rmvf", RCC_RSR == 0, "rsr", RCC_RSR);
    marker = 0x5EED1234u;              /* uninitialised RAM: must survive a system reset */
    KR = 0xCCCC;                       /* start */
    KR = 0x5555;                       /* unlock PR/RLR */
    PR = 0;                            /* /4: 8 kHz */
    RLR = 80;                          /* about 10 ms */
    while(SR) {}
    KR = 0xAAAA;
    for(int i = 0; i < 20; i++)        /* keep it alive for a while */
    {
        for(volatile int d = 0; d < 3000; d++) {}
        KR = 0xAAAA;
    }
    puts_("wdt: refreshed, now waiting for the reset\r\n");
    for(;;) {}
}
