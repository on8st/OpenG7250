/* USART2 through DMA1 self-test (own test scaffolding, from ST RM0468).
 * The bench connects USART2 and USART3 through a UART hub: bytes sent on
 * USART3 arrive on USART2 RX (circular DMA ring), and USART2 TX by DMA
 * arrives on USART3 RX. */
#include "bench.h"

#define DMA1_BASE 0x40020000u
#define LISR   REG(DMA1_BASE + 0x00)
#define LIFCR  REG(DMA1_BASE + 0x08)
#define SCR(s)   REG(DMA1_BASE + 0x10 + 0x18 * (s))
#define SNDTR(s) REG(DMA1_BASE + 0x14 + 0x18 * (s))
#define SPAR(s)  REG(DMA1_BASE + 0x18 + 0x18 * (s))
#define SM0AR(s) REG(DMA1_BASE + 0x1C + 0x18 * (s))
#define DMAMUX_CCR(c) REG(0x40020800u + 4 * (c))
#define REQ_USART2_RX 43u
#define REQ_USART2_TX 44u

#define U2 0x40004400u
#define U3 0x40004800u
#define CR1(b) REG((b) + 0x00)
#define CR3(b) REG((b) + 0x08)
#define BRR(b) REG((b) + 0x0C)
#define ISR(b) REG((b) + 0x1C)
#define ICR(b) REG((b) + 0x20)
#define RDR(b) REG((b) + 0x24)
#define TDR(b) REG((b) + 0x28)
#define UE (1u << 0)
#define RE (1u << 2)
#define TE (1u << 3)
#define IDLE (1u << 4)
#define RXNE (1u << 5)
#define TXE (1u << 7)
#define TEACK (1u << 21)
#define REACK (1u << 22)
#define DMAR (1u << 6)
#define DMAT (1u << 7)

#define EN (1u << 0)
#define HTIE (1u << 3)
#define TCIE (1u << 4)
#define M2P (1u << 6)
#define CIRC (1u << 8)
#define MINC (1u << 10)
static const unsigned sh[4] = { 0, 6, 16, 22 };
#define TCIF(s) (1u << (sh[s] + 5))
#define HTIF(s) (1u << (sh[s] + 4))

__attribute__((section(".dmabuf"))) static uint8_t ring[16], txbuf[5];
static volatile uint32_t ht2, tc2, tc3;

void DMA1_Stream2_IRQHandler(void)
{
    uint32_t f = LISR;
    if(f & HTIF(2)) { ht2++; LIFCR = HTIF(2); }
    if(f & TCIF(2)) { tc2++; LIFCR = TCIF(2); }
}

void DMA1_Stream3_IRQHandler(void)
{
    if(LISR & TCIF(3)) { tc3++; LIFCR = TCIF(3); }
}

static void wait(void) { for(volatile int d = 0; d < 2000; d++) {} }

static void u3_send(uint8_t c)
{
    for(int n = 0; n < 100000 && !(ISR(U3) & TXE); n++) {}
    TDR(U3) = c;
}

int main(void)
{
    console_init();
    puts_("bench usart_test\r\n");
    RCC_AHB1ENR |= 1u << 0;                  /* DMA1EN */
    RCC_APB1LENR |= (1u << 17) | (1u << 18); /* USART2EN, USART3EN */
    nvic_enable(13); nvic_enable(14);
    irq_on();

    BRR(U2) = 0x10F; BRR(U3) = 0x10F;
    CR1(U3) = UE | TE | RE;
    CR1(U2) = UE | TE | RE;
    test_result("usart_ack", (ISR(U2) & (TEACK | REACK)) == (TEACK | REACK), "isr", ISR(U2));

    /* RX: circular ring of 16 bytes on DMA1 stream 2 */
    DMAMUX_CCR(2) = REQ_USART2_RX;
    SPAR(2) = U2 + 0x24; SM0AR(2) = (uint32_t)ring; SNDTR(2) = 16;
    SCR(2) = CIRC | MINC | HTIE | TCIE | EN;   /* P2M, byte sizes */
    CR3(U2) = DMAR;
    for(int i = 0; i < 20; i++) u3_send((uint8_t)(0x40 + i));
    for(int n = 0; n < 200 && SNDTR(2) != 12; n++) wait();
    wait();
    test_result("usart_rx_dma_ndtr", SNDTR(2) == 12, "ndtr", SNDTR(2));
    test_result("usart_rx_dma_ring", ring[0] == 0x50 && ring[3] == 0x53 && ring[4] == 0x44 && ring[15] == 0x4F,
                "ring0..3", ring[0] | (ring[1] << 8) | (ring[2] << 16) | ((uint32_t)ring[3] << 24));
    test_result("usart_rx_dma_irqs", ht2 == 1 && tc2 == 1, "ht<<16|tc", (ht2 << 16) | tc2);
    test_result("usart_idle", (ISR(U2) & IDLE) != 0, "isr", ISR(U2));
    ICR(U2) = IDLE;
    test_result("usart_idle_clear", (ISR(U2) & IDLE) == 0, "isr", ISR(U2));

    /* TX: 5 bytes by DMA1 stream 3, received on USART3 by polling */
    for(int i = 0; i < 5; i++) txbuf[i] = (uint8_t)(0x61 + i);
    while(ISR(U3) & RXNE) (void)RDR(U3);
    DMAMUX_CCR(3) = REQ_USART2_TX;
    SPAR(3) = U2 + 0x28; SM0AR(3) = (uint32_t)txbuf; SNDTR(3) = 5;
    SCR(3) = M2P | MINC | TCIE | EN;
    CR3(U2) = DMAR | DMAT;
    uint8_t got[5] = { 0 }; int k = 0;
    for(int n = 0; n < 200000 && k < 5; n++) if(ISR(U3) & RXNE) got[k++] = (uint8_t)RDR(U3);
    wait();
    test_result("usart_tx_dma", k == 5 && got[0] == 0x61 && got[4] == 0x65 && SNDTR(3) == 0 && tc3 == 1,
                "k<<8|tc3", ((uint32_t)k << 8) | tc3);
    return 0;
}
