/* PWR self-test, invalid supply configuration (own test scaffolding, RM0468):
 * on a board where the SMPS supplies the LDO, LDO alone does not give a valid
 * core supply: ACTVOSRDY must stay clear. */
#include "bench.h"

#define PWR 0x58024800u
#define CSR1 REG(PWR + 0x04)
#define CR3  REG(PWR + 0x0C)

int main(void)
{
    console_init();
    puts_("bench pwr_bad_test\r\n");
    CR3 = 1u << 1;                         /* LDO only */
    int ready = 0;
    for(int n = 0; n < 100000; n++) if(CSR1 & (1u << 13)) { ready = 1; break; }
    test_result("pwr_invalid_config_no_actvosrdy", !ready, "csr1", CSR1);
    return 0;
}
