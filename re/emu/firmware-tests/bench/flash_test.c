/* Flash erase/program self-test (own test scaffolding, from ST RM0468, flash
 * chapter). Runs from sector 0 and works on sector 7 (0x080E0000). Reports
 * the time of each operation in core cycles (DWT CYCCNT). */
#include "bench.h"

#define FLASH 0x52002000u
#define KEYR1 REG(FLASH + 0x004)
#define CR1   REG(FLASH + 0x00C)
#define SR1   REG(FLASH + 0x010)
#define CCR1  REG(FLASH + 0x014)
#define LOCK  (1u << 0)
#define PG    (1u << 1)
#define SER   (1u << 2)
#define FW    (1u << 6)
#define START (1u << 7)
#define SNB(n) ((uint32_t)(n) << 8)
#define BSY   (1u << 0)
#define WBNE  (1u << 1)
#define QW    (1u << 2)
#define EOP   (1u << 16)
#define PGSERR (1u << 18)
#define INCERR (1u << 21)
#define ERRS  (0x1FEu << 16)

#define S7 0x080E0000u
#define W(a) (*(volatile uint32_t *)(a))

#define DEMCR REG(0xE000EDFCu)
#define DWT_CTRL REG(0xE0001000u)
#define CYC REG(0xE0001004u)

static uint32_t wait_idle(void)
{
    uint32_t t0 = CYC;
    for(uint32_t n = 0; n < 200000000u && (SR1 & (BSY | QW | WBNE)); n++) {}
    return CYC - t0;
}

int main(void)
{
    console_init();
    puts_("bench flash_test\r\n");
    DEMCR |= 1u << 24; CYC = 0; DWT_CTRL |= 1u;

    test_result("flash_locked_at_reset", (CR1 & LOCK) != 0, "cr1", CR1);
    CR1 = PG;
    test_result("flash_cr_locked", (CR1 & PG) == 0, "cr1", CR1);
    KEYR1 = 0x45670123u; KEYR1 = 0xCDEF89ABu;
    test_result("flash_unlock", (CR1 & LOCK) == 0, "cr1", CR1);

    CCR1 = ERRS | EOP;
    CR1 = SER | SNB(7);
    CR1 |= START;
    uint32_t te = wait_idle();
    test_result("flash_erase_eop", (SR1 & EOP) && !(SR1 & ERRS), "sr1", SR1);
    test_result("flash_erased", W(S7) == 0xFFFFFFFFu && W(S7 + 0x1FFFC) == 0xFFFFFFFFu, "w0", W(S7));
    puts_("erase cycles "); putdec(te); puts_("\r\n");
    CR1 = 0;

    CCR1 = ERRS | EOP;
    CR1 = PG;
    for(int i = 0; i < 8; i++) W(S7 + 4 * i) = 0xA5000000u + i;
    uint32_t tp = wait_idle();
    test_result("flash_program_eop", (SR1 & EOP) && !(SR1 & ERRS), "sr1", SR1);
    int ok = 1;
    for(int i = 0; i < 8; i++) if(W(S7 + 4 * i) != 0xA5000000u + i) ok = 0;
    test_result("flash_programmed", ok, "w0", W(S7));
    puts_("program cycles "); putdec(tp); puts_("\r\n");

    CCR1 = ERRS | EOP;
    W(S7 + 0x20) = 0x11111111u; W(S7 + 0x24) = 0x22222222u;
    CR1 |= FW;
    wait_idle();
    test_result("flash_force_write", W(S7 + 0x20) == 0x11111111u && W(S7 + 0x28) == 0xFFFFFFFFu && (SR1 & EOP), "w2", W(S7 + 0x28));
    CR1 = 0;

    CCR1 = ERRS | EOP;
    W(S7 + 0x40) = 0x12345678u;              /* PG clear: must not program */
    wait_idle();
    test_result("flash_write_without_pg", W(S7 + 0x40) == 0xFFFFFFFFu && (SR1 & PGSERR), "sr1", SR1);

    CCR1 = ERRS | EOP;
    CR1 = PG;
    W(S7 + 0x60) = 1; W(S7 + 0x80) = 2;      /* second word before the first is complete */
    CR1 |= FW; wait_idle();
    test_result("flash_incerr", (SR1 & INCERR) != 0, "sr1", SR1);
    CR1 = 0;
    CCR1 = ERRS | EOP;

    CR1 = LOCK;
    test_result("flash_relock", (CR1 & LOCK) != 0, "cr1", CR1);
    return 0;
}
