/* SPI6 timing self-test (own test scaffolding, ST RM0468, his/40, W5500-style
 * frames to the CH394). SPI6 kernel clock = PCLK4 = 64 MHz at reset, MBR = 3
 * (/16): f_SCK = 4 MHz, 2 us per 8-bit frame. A 1003-frame read (3-byte header
 * plus 1000 data bytes) of the CH394 is timed with TIM2 at 1 MHz:
 *   - by CPU polling (TXP/RXP): about 2006 us plus loop overhead;
 *   - by BDMA (RX channel 0, TX channel 1): about 2006 us.
 * The runner's model is timed only for SPI6 in product runs (Timed). */
#include "bench.h"

#define GPIO(p) (0x58020000u + 0x400u * (p))
#define MODER(p) REG(GPIO(p) + 0x00)
#define BSRR(p)  REG(GPIO(p) + 0x18)
enum { A, B, C, D, E, F, G, H };

#define SPI6 0x58001400u
#define S_CR1  REG(SPI6 + 0x00)
#define S_CR2  REG(SPI6 + 0x04)
#define S_CFG1 REG(SPI6 + 0x08)
#define S_CFG2 REG(SPI6 + 0x0C)
#define S_SR   REG(SPI6 + 0x14)
#define S_IFCR REG(SPI6 + 0x18)
#define S_TXDR8 (*(volatile uint8_t *)(SPI6 + 0x20))
#define S_RXDR8 (*(volatile uint8_t *)(SPI6 + 0x30))
#define RXP (1u << 0)
#define TXP (1u << 1)
#define EOT (1u << 3)

#define BDMA 0x58025400u
#define B_IFCR REG(BDMA + 0x04)
#define CCR(c)   REG(BDMA + 0x08 + 0x14 * (c))
#define CNDTR(c) REG(BDMA + 0x0C + 0x14 * (c))
#define CPAR(c)  REG(BDMA + 0x10 + 0x14 * (c))
#define CM0AR(c) REG(BDMA + 0x14 + 0x14 * (c))
#define DMAMUX2_CCR(c) REG(0x58025800u + 4 * (c))
#define EN (1u << 0)
#define DIR (1u << 4)
#define MINC (1u << 7)

#define TIM2 0x40000000u
#define T_CR1 REG(TIM2 + 0x00)
#define T_EGR REG(TIM2 + 0x14)
#define T_CNT REG(TIM2 + 0x24)
#define T_PSC REG(TIM2 + 0x28)
#define T_ARR REG(TIM2 + 0x2C)

#define N 1003
__attribute__((section(".d3buf"))) static uint8_t txb[N], rxb[N];

static void out_pin(int port, int pin, int level)
{
    MODER(port) = (MODER(port) & ~(3u << (2 * pin))) | (1u << (2 * pin));
    BSRR(port) = level ? (1u << pin) : (1u << (pin + 16));
}

static void setup(int dma)
{
    S_CR1 = 0;
    S_CR2 = N;                                           /* TSIZE */
    S_CFG1 = 7u | (dma ? (1u << 14) | (1u << 15) : 0) | (3u << 28);   /* 8 bit, MBR /16 */
    S_CFG2 = (1u << 22) | (1u << 26);                    /* master, SSM */
    txb[0] = 0x00; txb[1] = 0x00; txb[2] = (uint8_t)(3u << 3);   /* socket 0 RX buffer, read */
    for(int i = 3; i < N; i++) txb[i] = 0;
}

static uint32_t polled(void)
{
    setup(0);
    BSRR(G) = 1u << 31;
    uint32_t t0 = T_CNT;
    S_CR1 = (1u << 12) | 1u;
    S_CR1 |= 1u << 9;                                    /* CSTART */
    int sent = 0, got = 0;
    while(got < N)
    {
        if(sent < N && (S_SR & TXP)) S_TXDR8 = txb[sent++];
        if(S_SR & RXP) rxb[got++] = S_RXDR8;
    }
    while(!(S_SR & EOT)) { }
    uint32_t t1 = T_CNT;
    BSRR(G) = 1u << 15;
    S_IFCR = 0xFF8; S_CR1 = 0;
    return t1 - t0;
}

static uint32_t by_dma(void)
{
    setup(1);
    B_IFCR = 0xFFFFFFFFu;
    CCR(0) = 0; CCR(1) = 0;
    DMAMUX2_CCR(0) = 11; DMAMUX2_CCR(1) = 12;
    CPAR(0) = SPI6 + 0x30; CM0AR(0) = (uint32_t)rxb; CNDTR(0) = N;
    CPAR(1) = SPI6 + 0x20; CM0AR(1) = (uint32_t)txb; CNDTR(1) = N;
    CCR(0) = MINC | EN;
    CCR(1) = MINC | DIR | EN;
    BSRR(G) = 1u << 31;
    uint32_t t0 = T_CNT;
    S_CR1 = (1u << 12) | 1u;
    S_CR1 |= 1u << 9;
    while(!(S_SR & EOT)) { }
    uint32_t t1 = T_CNT;
    uint32_t left = CNDTR(0);
    BSRR(G) = 1u << 15;
    S_IFCR = 0xFF8; S_CR1 = 0;
    return left ? 0xFFFFFFFFu : t1 - t0;
}

int main(void)
{
    console_init();
    puts_("bench spi6time_test\r\n");
    RCC_AHB4ENR |= 0xFFu | (1u << 21);                   /* GPIOA..H, BDMAEN */
    RCC_APB4ENR |= 1u << 5;                              /* SPI6EN */
    RCC_APB1LENR |= 1u << 0;                             /* TIM2EN */
    T_PSC = 63; T_ARR = 0xFFFFFFFFu; T_EGR = 1; T_CR1 = 1;   /* 1 MHz */
    out_pin(G, 15, 1);
    out_pin(B, 5, 0); out_pin(B, 5, 1);

    uint32_t tp = polled();
    puts_("spi6 polled 1003 frames: "); putdec(tp); puts_(" us\r\n");
    test_result("spi6time_polled_us", tp >= 1990 && tp <= 2600, "us", tp);
    uint32_t td = by_dma();
    puts_("spi6 dma 1003 frames: "); putdec(td); puts_(" us\r\n");
    test_result("spi6time_dma_us", td >= 1990 && td <= 2100, "us", td);
    return 0;
}
