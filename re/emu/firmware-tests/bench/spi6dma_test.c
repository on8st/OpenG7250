/* SPI6 + DMAMUX2 + BDMA self-test with the CH394 (own test scaffolding, from
 * ST RM0468, his/40 and the W5500 datasheet). Register reads of the Ethernet
 * controller by full-duplex DMA: TX on BDMA channel 1 (request 12), RX on
 * channel 0 (request 11), buffers in SRAM4 (D3 domain). */
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
#define EOT (1u << 3)

#define BDMA 0x58025400u
#define B_ISR  REG(BDMA + 0x00)
#define B_IFCR REG(BDMA + 0x04)
#define CCR(c)   REG(BDMA + 0x08 + 0x14 * (c))
#define CNDTR(c) REG(BDMA + 0x0C + 0x14 * (c))
#define CPAR(c)  REG(BDMA + 0x10 + 0x14 * (c))
#define CM0AR(c) REG(BDMA + 0x14 + 0x14 * (c))
#define DMAMUX2_CCR(c) REG(0x58025800u + 4 * (c))
#define EN (1u << 0)
#define DIR (1u << 4)
#define MINC (1u << 7)
#define TCIF(c) (1u << (4 * (c) + 1))
#define TEIF(c) (1u << (4 * (c) + 3))

__attribute__((section(".d3buf"))) static uint8_t txb[16], rxb[16];
__attribute__((section(".dmabuf"))) static uint8_t d2buf[16];

static void out_pin(int port, int pin, int level)
{
    MODER(port) = (MODER(port) & ~(3u << (2 * pin))) | (1u << (2 * pin));
    BSRR(port) = level ? (1u << pin) : (1u << (pin + 16));
}

/* one W5500 read frame of n data bytes by DMA; returns 1 on EOT */
static int dma_read(uint16_t addr, uint8_t bsb, int n, uint8_t *rxbuf)
{
    int len = 3 + n;
    txb[0] = (uint8_t)(addr >> 8); txb[1] = (uint8_t)addr; txb[2] = (uint8_t)(bsb << 3);
    for(int i = 3; i < len; i++) txb[i] = 0;
    B_IFCR = 0xFFFFFFFFu;
    CCR(0) = 0; CCR(1) = 0;
    DMAMUX2_CCR(0) = 11; DMAMUX2_CCR(1) = 12;
    CPAR(0) = SPI6 + 0x30; CM0AR(0) = (uint32_t)rxbuf; CNDTR(0) = (uint32_t)len;
    CPAR(1) = SPI6 + 0x20; CM0AR(1) = (uint32_t)txb;   CNDTR(1) = (uint32_t)len;
    CCR(0) = MINC | EN;                 /* peripheral to memory, bytes */
    CCR(1) = MINC | DIR | EN;           /* memory to peripheral, bytes */
    S_CR1 = 0;
    S_CR2 = (uint32_t)len;              /* TSIZE */
    S_CFG1 = 7u | (1u << 14) | (1u << 15) | (3u << 28);   /* 8 bit, RXDMAEN, TXDMAEN */
    S_CFG2 = (1u << 22) | (1u << 26);
    BSRR(G) = 1u << 31;                 /* CS# low */
    S_CR1 = (1u << 12) | 1u;
    S_CR1 |= 1u << 9;                   /* CSTART */
    int ok = 0;
    for(int k = 0; k < 100000; k++) if(S_SR & EOT) { ok = 1; break; }
    BSRR(G) = 1u << 15;                 /* CS# high */
    S_IFCR = 0xFF8;
    S_CR1 = 0;
    return ok;
}

int main(void)
{
    console_init();
    puts_("bench spi6dma_test\r\n");
    RCC_AHB4ENR |= 0xFFu | (1u << 21);  /* GPIOA..H, BDMAEN */
    RCC_APB4ENR |= 1u << 5;             /* SPI6EN */
    out_pin(G, 15, 1);
    out_pin(B, 5, 0); out_pin(B, 5, 1);

    int ok = dma_read(0x0039, 0, 1, rxb);
    test_result("spi6dma_version", ok && rxb[3] == 0x04 && (B_ISR & TCIF(0)) && (B_ISR & TCIF(1)), "rx3", rxb[3]);
    ok = dma_read(0x0009, 0, 6, rxb);
    test_result("spi6dma_mac", ok && rxb[3] == 0x02 && rxb[8] == 0x50, "rx8", rxb[8]);
    test_result("spi6dma_ndtr_done", CNDTR(0) == 0 && CNDTR(1) == 0 && !(CCR(0) & EN), "cndtr0", CNDTR(0));
    /* the BDMA reaches the D3 domain only: a D2 SRAM buffer fails */
    dma_read(0x0039, 0, 1, d2buf);
    test_result("spi6dma_d3_only", (B_ISR & TEIF(0)) != 0 && !(CCR(0) & EN), "isr", B_ISR);
    return 0;
}
