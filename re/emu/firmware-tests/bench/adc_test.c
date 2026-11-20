/* ADC1 self-test (own test scaffolding, from ST RM0468). The bench sets the
 * input voltages: INP1 = 0.5 V, INP4 = 1.65 V, INP5 = 3.3 V (full scale). */
#include "bench.h"

#define ADC1 0x40022000u
#define A_ISR   REG(ADC1 + 0x00)
#define A_IER   REG(ADC1 + 0x04)
#define A_CR    REG(ADC1 + 0x08)
#define A_CFGR  REG(ADC1 + 0x0C)
#define A_PCSEL REG(ADC1 + 0x1C)
#define A_SQR1  REG(ADC1 + 0x30)
#define A_DR    REG(ADC1 + 0x40)
#define ADRDY (1u << 0)
#define EOC   (1u << 2)
#define EOS   (1u << 3)
#define OVR   (1u << 4)
#define ADEN    (1u << 0)
#define ADDIS   (1u << 1)
#define ADSTART (1u << 2)
#define ADSTP   (1u << 4)
#define ADVREGEN (1u << 28)
#define DEEPPWD  (1u << 29)
#define ADCAL    (1u << 31)
#define RES_12   (2u << 2)
#define RES_16   (0u << 2)
#define CONT     (1u << 13)
#define DMNGT_CIRC 3u

#define DMA1_BASE 0x40020000u
#define LIFCR  REG(DMA1_BASE + 0x08)
#define SCR(s)   REG(DMA1_BASE + 0x10 + 0x18 * (s))
#define SNDTR(s) REG(DMA1_BASE + 0x14 + 0x18 * (s))
#define SPAR(s)  REG(DMA1_BASE + 0x18 + 0x18 * (s))
#define SM0AR(s) REG(DMA1_BASE + 0x1C + 0x18 * (s))
#define DMAMUX_CCR(c) REG(0x40020800u + 4 * (c))
#define REQ_ADC1 9u

__attribute__((section(".dmabuf"))) static uint16_t samples[8];

static int wait_flag(uint32_t f)
{
    for(int n = 0; n < 2000000; n++) if(A_ISR & f) return 1;
    return 0;
}

static uint32_t near(uint32_t v, uint32_t want) { return v + 2 >= want && v <= want + 2; }

int main(void)
{
    console_init();
    puts_("bench adc_test\r\n");
    RCC_AHB1ENR |= (1u << 5) | (1u << 0);   /* ADC12EN, DMA1EN */

    /* power-up as in RM0468: leave deep power-down, regulator, calibrate, enable */
    A_CR = 0;
    A_CR = ADVREGEN;
    test_result("adc_ldordy", (A_ISR & (1u << 12)) != 0, "isr", A_ISR);
    A_CR = ADVREGEN | ADCAL;
    for(int n = 0; n < 1000 && (A_CR & ADCAL); n++) {}
    test_result("adc_cal_done", (A_CR & ADCAL) == 0, "cr", A_CR);
    A_ISR = ADRDY;
    A_CR = ADVREGEN | ADEN;
    test_result("adc_ready", wait_flag(ADRDY), "isr", A_ISR);

    /* single sequence INP1, INP4 at 12 bit, read by software */
    A_PCSEL = (1u << 1) | (1u << 4) | (1u << 5);
    A_CFGR = RES_12;
    A_SQR1 = 1u | (1u << 6) | (4u << 12);    /* L = 2 conversions: SQ1 = 1, SQ2 = 4 */
    A_CR = ADVREGEN | ADEN | ADSTART;
    int ok1 = wait_flag(EOC); uint32_t d1 = A_DR;
    int ok2 = wait_flag(EOC); uint32_t d2 = A_DR;
    test_result("adc_raw_12bit", ok1 && ok2 && near(d1, 620) && near(d2, 2048), "d1<<16|d2", (d1 << 16) | d2);
    test_result("adc_eos_single", (A_ISR & EOS) && !(A_CR & ADSTART), "isr", A_ISR);

    /* 16 bit, full scale on INP5 */
    A_ISR = 0x7FF;
    A_CFGR = RES_16;
    A_SQR1 = 0u | (5u << 6);
    A_CR = ADVREGEN | ADEN | ADSTART;
    int ok3 = wait_flag(EOC); uint32_t d3 = A_DR;
    test_result("adc_raw_16bit_full", ok3 && d3 == 0xFFFF, "dr", d3);

    /* continuous INP1, INP4 into a circular DMA ring of 8 half-words */
    A_ISR = 0x7FF;
    DMAMUX_CCR(0) = REQ_ADC1;
    LIFCR = 0x3D;
    SPAR(0) = ADC1 + 0x40; SM0AR(0) = (uint32_t)samples; SNDTR(0) = 8;
    SCR(0) = (1u << 8) | (1u << 10) | (1u << 11) | (1u << 13) | 1u;   /* CIRC MINC PSIZE16 MSIZE16 EN */
    A_CFGR = RES_12 | CONT | DMNGT_CIRC;
    A_SQR1 = 1u | (1u << 6) | (4u << 12);
    A_CR = ADVREGEN | ADEN | ADSTART;
    for(volatile int d = 0; d < 400000; d++) {}
    A_CR = ADVREGEN | ADEN | ADSTP;
    int okd = 1;
    for(int i = 0; i < 8; i++) if(!near(samples[i], (i & 1) ? 2048u : 620u)) okd = 0;
    test_result("adc_dma_circular", okd && (SCR(0) & 1u), "s0<<16|s1", ((uint32_t)samples[0] << 16) | samples[1]);
    test_result("adc_stop", !(A_CR & ADSTART), "cr", A_CR);
    A_CR = ADVREGEN | ADEN | ADDIS;
    test_result("adc_disable", !(A_CR & ADEN), "cr", A_CR);
    return 0;
}
