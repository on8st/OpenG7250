/* PWR self-test (own test scaffolding, from ST RM0468, PWR chapter).
 * Boot 1: CR3 reset value and ACTVOSRDY; write a supply configuration in which
 * the SMPS supplies the LDO; a second write is ignored (write-once); then a
 * watchdog reset. Boot 2: the configuration and its lock survived the system
 * reset. */
#include "bench.h"

#define PWR 0x58024800u
#define CSR1 REG(PWR + 0x04)
#define CR3  REG(PWR + 0x0C)
#define D3CR REG(PWR + 0x18)
#define ACTVOSRDY (1u << 13)
#define RCC_RSR REG(RCC_BASE + 0xD0)
#define IWDG1RSTF (1u << 26)
#define IWDG 0x58004800u

/* SMPSLEVEL 2.5 V (bits 5:4 = 10), SMPSEXTHP, SMPSEN, LDOEN: SMPS supplies the LDO (RM0468 encoding) */
#define SMPS_LDO_25 ((2u << 4) | (1u << 3) | (1u << 2) | (1u << 1))
#define LDO_ONLY    (1u << 1)

int main(void)
{
    console_init();
    uint32_t rsr = RCC_RSR;
    puts_("bench pwr_test\r\n");
    if(rsr & IWDG1RSTF)
    {
        test_result("pwr_lock_survives_reset", (CR3 & 0x3Fu) == SMPS_LDO_25, "cr3", CR3);
        CR3 = LDO_ONLY;
        test_result("pwr_still_locked", (CR3 & 0x3Fu) == SMPS_LDO_25 && (CSR1 & ACTVOSRDY), "cr3", CR3);
        return 0;
    }
    test_result("pwr_cr3_reset", CR3 == 0x00000006u, "cr3", CR3);
    test_result("pwr_actvosrdy_at_reset", (CSR1 & ACTVOSRDY) != 0, "csr1", CSR1);
    CR3 = SMPS_LDO_25;
    test_result("pwr_cr3_written", (CR3 & 0x3Fu) == SMPS_LDO_25 && (CSR1 & ACTVOSRDY), "cr3", CR3);
    test_result("pwr_smpsextrdy", (CR3 & (1u << 16)) != 0, "cr3", CR3);
    CR3 = LDO_ONLY;                        /* write-once: ignored */
    test_result("pwr_write_once", (CR3 & 0x3Fu) == SMPS_LDO_25, "cr3", CR3);
    D3CR = 0u << 14;                       /* VOS scale 0 */
    test_result("pwr_vosrdy", (D3CR & (1u << 13)) != 0, "d3cr", D3CR);
    RCC_RSR = 1u << 16;                    /* clear the reset flags */
    REG(IWDG) = 0xCCCC; REG(IWDG) = 0x5555; REG(IWDG + 4) = 0; REG(IWDG + 8) = 40;
    for(;;) {}                             /* the watchdog resets the MCU */
}
