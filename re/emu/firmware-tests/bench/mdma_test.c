/* MDMA self-test (own test scaffolding, from ST RM0468), software requests. */
#include "bench.h"

#define MDMA 0x52000000u
#define GISR0   REG(MDMA + 0x00)
#define CISR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x00)
#define CIFCR(c)  REG(MDMA + 0x40 + 0x40 * (c) + 0x04)
#define CCR(c)    REG(MDMA + 0x40 + 0x40 * (c) + 0x0C)
#define CTCR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x10)
#define CBNDTR(c) REG(MDMA + 0x40 + 0x40 * (c) + 0x14)
#define CSAR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x18)
#define CDAR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x1C)
#define CBRUR(c)  REG(MDMA + 0x40 + 0x40 * (c) + 0x20)
#define CLAR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x24)
#define CTBR(c)   REG(MDMA + 0x40 + 0x40 * (c) + 0x28)

#define TEIF  (1u << 0)
#define CTCIF (1u << 1)
#define BRTIF (1u << 2)
#define BTIF  (1u << 3)
#define TCIF  (1u << 4)
#define EN    (1u << 0)
#define CTCIE (1u << 2)
#define SWRQ  (1u << 16)
/* CTCR fields */
#define SINC_INC (2u << 0)
#define DINC_INC (2u << 2)
#define SSIZE(n) ((n) << 4)          /* 0 byte, 1 half, 2 word, 3 double */
#define DSIZE(n) ((n) << 6)
#define SINCOS(n) ((n) << 8)
#define DINCOS(n) ((n) << 10)
#define TLEN(n)  (((n) - 1u) << 18)
#define TRGM(n)  ((n) << 28)
#define SWRM     (1u << 30)
#define WORDS (SSIZE(2) | DSIZE(2) | SINCOS(2) | DINCOS(2))

static uint32_t src[32], dst[64];
static uint16_t hdst[16];
/* linked-list node: CTCR, CBNDTR, CSAR, CDAR, CBRUR, CLAR, CTBR, reserved, CMAR, CMDR */
static uint32_t node[10] __attribute__((aligned(8)));
static volatile uint32_t irqs;

void MDMA_IRQHandler(void)
{
    if(CISR(5) & CTCIF) { irqs++; CIFCR(5) = 0x1F; }
}

static void clear(void) { for(int i = 0; i < 64; i++) dst[i] = 0; }

int main(void)
{
    console_init();
    puts_("bench mdma_test\r\n");
    RCC_AHB3ENR |= 1u << 0;   /* MDMAEN */
    for(int i = 0; i < 32; i++) src[i] = 0xC0DE0000u + i;

    /* buffer mode: 64 bytes in 16-byte buffers, one software request each */
    clear();
    CIFCR(0) = 0x1F;
    CTCR(0) = SWRM | TRGM(0) | TLEN(16) | WORDS | SINC_INC | DINC_INC;
    CBNDTR(0) = 64; CSAR(0) = (uint32_t)src; CDAR(0) = (uint32_t)dst;
    CCR(0) = EN | SWRQ;
    int one = (CISR(0) & TCIF) && !(CISR(0) & BTIF) && dst[3] == src[3] && dst[4] == 0 && (CBNDTR(0) & 0x1FFFF) == 48;
    test_result("mdma_buffer_one", one, "bndt", CBNDTR(0) & 0x1FFFF);
    for(int i = 0; i < 3; i++) { CIFCR(0) = TCIF; CCR(0) = EN | SWRQ; }
    test_result("mdma_buffer_done", (CISR(0) & (BTIF | CTCIF)) == (BTIF | CTCIF) && !(CCR(0) & EN) && dst[15] == src[15],
                "cisr", CISR(0));

    /* block mode with packing: words to half-words, 32 bytes */
    for(int i = 0; i < 16; i++) hdst[i] = 0;
    CIFCR(1) = 0x1F;
    CTCR(1) = SWRM | TRGM(1) | TLEN(32) | SSIZE(2) | DSIZE(1) | SINCOS(2) | DINCOS(1) | SINC_INC | DINC_INC;
    CBNDTR(1) = 32; CSAR(1) = (uint32_t)src; CDAR(1) = (uint32_t)hdst;
    CCR(1) = EN | SWRQ;
    test_result("mdma_packing", hdst[0] == 0x0000 && hdst[1] == 0xC0DE && hdst[2] == 0x0001 && hdst[15] == 0xC0DE
                && (CISR(1) & CTCIF), "h2", hdst[2]);

    /* repeated block: 3 blocks of 8 bytes, destination skips 8 bytes per block */
    clear();
    CIFCR(2) = 0x1F;
    CTCR(2) = SWRM | TRGM(2) | TLEN(8) | WORDS | SINC_INC | DINC_INC;
    CBNDTR(2) = 8 | (2u << 20);
    CBRUR(2) = 8u << 16;            /* DUV = 8, SUV = 0 */
    CSAR(2) = (uint32_t)src; CDAR(2) = (uint32_t)dst;
    CCR(2) = EN | SWRQ;
    test_result("mdma_repeated_block", dst[0] == src[0] && dst[1] == src[1] && dst[2] == 0 && dst[4] == src[2]
                && dst[8] == src[4] && (CISR(2) & (BRTIF | CTCIF)) == (BRTIF | CTCIF), "dst4", dst[4]);

    /* fill: fixed source */
    clear();
    static uint32_t pattern = 0x5A5A5A5Au;
    CIFCR(3) = 0x1F;
    CTCR(3) = SWRM | TRGM(3) | TLEN(16) | WORDS | DINC_INC;   /* SINC fixed */
    CBNDTR(3) = 40; CSAR(3) = (uint32_t)&pattern; CDAR(3) = (uint32_t)dst;
    CCR(3) = EN | SWRQ;
    test_result("mdma_fill", dst[0] == pattern && dst[9] == pattern && dst[10] == 0, "dst10", dst[10]);

    /* linked list: node 1 copies src[0..3] to dst[0..3], node 2 src[4..7] to dst[20..23] */
    clear();
    node[0] = SWRM | TRGM(3) | TLEN(16) | WORDS | SINC_INC | DINC_INC;
    node[1] = 16; node[2] = (uint32_t)&src[4]; node[3] = (uint32_t)&dst[20];
    node[4] = 0; node[5] = 0; node[6] = 0; node[7] = 0; node[8] = 0; node[9] = 0;
    CIFCR(4) = 0x1F;
    CTCR(4) = SWRM | TRGM(3) | TLEN(16) | WORDS | SINC_INC | DINC_INC;
    CBNDTR(4) = 16; CSAR(4) = (uint32_t)src; CDAR(4) = (uint32_t)dst; CLAR(4) = (uint32_t)node;
    CCR(4) = EN | SWRQ;
    test_result("mdma_linked_list", dst[3] == src[3] && dst[20] == src[4] && dst[23] == src[7] && dst[4] == 0
                && (CISR(4) & CTCIF) && !(CCR(4) & EN), "dst20", dst[20]);

    /* interrupt on channel completion (MDMA IRQ 122) */
    nvic_enable(122);
    irq_on();
    CIFCR(5) = 0x1F;
    CTCR(5) = SWRM | TRGM(3) | TLEN(16) | WORDS | SINC_INC | DINC_INC;
    CBNDTR(5) = 16; CSAR(5) = (uint32_t)src; CDAR(5) = (uint32_t)dst;
    CCR(5) = EN | CTCIE | SWRQ;
    for(volatile int d = 0; d < 200; d++) {}
    test_result("mdma_irq", irqs == 1 && (GISR0 & (1u << 5)) == 0, "irqs", irqs);

    /* transfer error: unmapped destination */
    CIFCR(6) = 0x1F;
    CTCR(6) = SWRM | TRGM(3) | TLEN(16) | WORDS | SINC_INC | DINC_INC;
    CBNDTR(6) = 16; CSAR(6) = (uint32_t)src; CDAR(6) = 0xA0010000u;
    CCR(6) = EN | SWRQ;
    test_result("mdma_teif", (CISR(6) & TEIF) && !(CCR(6) & EN), "cisr", CISR(6));
    return 0;
}
