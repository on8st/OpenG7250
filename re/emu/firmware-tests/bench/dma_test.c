/* DMA1 + DMAMUX1 self-test (own test scaffolding, from ST RM0468).
 * Requests come from the bench-only request source (BENCHREQ, DMAMUX input
 * BENCHREQ_ID), so the test does not depend on any peripheral model. */
#include "bench.h"

#define DMA1_BASE 0x40020000u
#define LISR   REG(DMA1_BASE + 0x00)
#define LIFCR  REG(DMA1_BASE + 0x08)
#define SCR(s)   REG(DMA1_BASE + 0x10 + 0x18 * (s))
#define SNDTR(s) REG(DMA1_BASE + 0x14 + 0x18 * (s))
#define SPAR(s)  REG(DMA1_BASE + 0x18 + 0x18 * (s))
#define SM0AR(s) REG(DMA1_BASE + 0x1C + 0x18 * (s))
#define SM1AR(s) REG(DMA1_BASE + 0x20 + 0x18 * (s))
#define DMAMUX_CCR(c) REG(0x40020800u + 4 * (c))

#define EN    (1u << 0)
#define TEIE  (1u << 2)
#define HTIE  (1u << 3)
#define TCIE  (1u << 4)
#define P2M   (0u << 6)
#define M2M   (2u << 6)
#define CIRC  (1u << 8)
#define PINC  (1u << 9)
#define MINC  (1u << 10)
#define PWORD (2u << 11)
#define MWORD (2u << 13)
#define DBM   (1u << 18)
#define CT    (1u << 19)
/* LISR flag positions per stream 0..3 */
static const unsigned sh[4] = { 0, 6, 16, 22 };
#define TCIF(s) (1u << (sh[s] + 5))
#define HTIF(s) (1u << (sh[s] + 4))
#define TEIF(s) (1u << (sh[s] + 3))

__attribute__((section(".dmabuf"))) static uint32_t src[16], dst[16], buf0[4], buf1[4];
static volatile uint32_t ht1, tc1, tc2;

void DMA1_Stream1_IRQHandler(void)
{
    uint32_t f = LISR;
    if(f & HTIF(1)) { ht1++; LIFCR = HTIF(1); }
    if(f & TCIF(1)) { tc1++; LIFCR = TCIF(1); }
}

void DMA1_Stream2_IRQHandler(void)
{
    if(LISR & TCIF(2)) { tc2++; LIFCR = TCIF(2); }
}

/* request pulses, then give pending interrupts time to be taken (as on
 * hardware, an interrupt is not taken in the same instruction as the store) */
static void pulses(uint32_t n)
{
    BENCHREQ_PULSE = n;
    __asm volatile ("dsb\n isb" ::: "memory");
    for(volatile int d = 0; d < 200; d++) {}
}

static void test_m2m(void)
{
    for(int i = 0; i < 16; i++) { src[i] = 0xA5000000u + i; dst[i] = 0; }
    LIFCR = 0x3Du << sh[0];
    SPAR(0) = (uint32_t)src; SM0AR(0) = (uint32_t)dst; SNDTR(0) = 16;
    SCR(0) = M2M | PINC | MINC | PWORD | MWORD | EN;
    int ok = 1;
    for(int i = 0; i < 16; i++) if(dst[i] != src[i]) ok = 0;
    test_result("dma_m2m_copy", ok, "dst[15]", dst[15]);
    test_result("dma_m2m_tcif", (LISR & TCIF(0)) && !(SCR(0) & EN) && SNDTR(0) == 0, "lisr", LISR);
}

static void test_circular(void)
{
    for(int i = 0; i < 8; i++) { src[i] = 0x100u + i; dst[i] = 0; }
    LIFCR = 0x3Du << sh[1];
    DMAMUX_CCR(1) = BENCHREQ_ID;
    SPAR(1) = (uint32_t)src; SM0AR(1) = (uint32_t)dst; SNDTR(1) = 8;
    SCR(1) = P2M | CIRC | PINC | MINC | PWORD | MWORD | HTIE | TCIE | EN;
    pulses(3);
    test_result("dma_req_routing", SNDTR(1) == 5 && dst[2] == 0x102u, "ndtr", SNDTR(1));
    SNDTR(1) = 1;   /* protected while enabled: must be ignored */
    test_result("dma_ndtr_protected", SNDTR(1) == 5, "ndtr", SNDTR(1));
    pulses(5);
    test_result("dma_circ_reload", SNDTR(1) == 8 && (SCR(1) & EN) && dst[7] == 0x107u, "ndtr", SNDTR(1));
    test_result("dma_ht_tc_irq", ht1 == 1 && tc1 == 1, "ht<<16|tc", (ht1 << 16) | tc1);
    for(int i = 0; i < 8; i++) src[i] = 0x200u + i;
    pulses(4);
    test_result("dma_circ_wrap", SNDTR(1) == 4 && dst[0] == 0x200u && dst[3] == 0x203u && dst[4] == 0x104u
                && ht1 == 2 && tc1 == 1, "dst0", dst[0]);
    SCR(1) &= ~EN;
    test_result("dma_disable", !(SCR(1) & EN) && SNDTR(1) == 4, "ndtr", SNDTR(1));
}

static void test_double_buffer(void)
{
    for(int i = 0; i < 8; i++) src[i] = 0x300u + i;
    for(int i = 0; i < 4; i++) buf0[i] = buf1[i] = 0;
    LIFCR = 0x3Du << sh[2];
    DMAMUX_CCR(2) = BENCHREQ_ID;
    SPAR(2) = (uint32_t)src; SM0AR(2) = (uint32_t)buf0; SM1AR(2) = (uint32_t)buf1; SNDTR(2) = 4;
    /* stream 1 also listens to the same request: stop it first */
    SCR(1) = 0; DMAMUX_CCR(1) = 0;
    SCR(2) = P2M | DBM | PINC | MINC | PWORD | MWORD | TCIE | EN;
    pulses(4);
    int ct1 = (SCR(2) & CT) != 0;
    pulses(4);
    int ct0 = (SCR(2) & CT) == 0;
    test_result("dma_dbm", ct1 && ct0 && tc2 == 2 && buf0[3] == 0x303u && buf1[0] == 0x300u, "ct1,ct0,tc2", (ct1 << 8) | (ct0 << 4) | tc2);
    SCR(2) = 0; DMAMUX_CCR(2) = 0;
}

static void test_transfer_error(void)
{
    LIFCR = 0x3Du << sh[3];
    SPAR(3) = (uint32_t)src; SM0AR(3) = 0xA0010000u; SNDTR(3) = 4;   /* nothing mapped there */
    SCR(3) = M2M | PINC | MINC | PWORD | MWORD | EN;
    test_result("dma_teif", (LISR & TEIF(3)) && !(SCR(3) & EN), "lisr", LISR);
}

int main(void)
{
    console_init();
    puts_("bench dma_test\r\n");
    RCC_AHB1ENR |= 1u << 0;   /* DMA1EN */
    nvic_enable(12); nvic_enable(13);
    irq_on();
    test_m2m();
    test_circular();
    test_double_buffer();
    test_transfer_error();
    return 0;
}
