/* DBGMCU self-test (own test scaffolding, from ST RM0468). */
#include "bench.h"

#define DBGMCU 0x5C001000u
#define IDCODE  REG(DBGMCU + 0x000)
#define CR      REG(DBGMCU + 0x004)
#define APB4FZ1 REG(DBGMCU + 0x054)

int main(void)
{
    console_init();
    puts_("bench dbgmcu_test\r\n");
    uint32_t id = IDCODE;
    test_result("dbgmcu_dev_id", (id & 0xFFF) == 0x483, "idcode", id);
    IDCODE = 0;
    test_result("dbgmcu_idcode_ro", IDCODE == id, "idcode", IDCODE);
    CR = 0x7u;                        /* DBGSLEEP_D1, DBGSTOP_D1, DBGSTBY_D1 */
    test_result("dbgmcu_cr", (CR & 0x7u) == 0x7u, "cr", CR);
    APB4FZ1 = 1u << 18;               /* freeze IWDG1 in debug */
    test_result("dbgmcu_apb4fz1", APB4FZ1 == (1u << 18), "apb4fz1", APB4FZ1);
    return 0;
}
