/* Network + safe-state scaffold for the product runner (own test scaffolding,
 * not product code). From his/40 (CH394 on SPI6: SCK PG13, MISO PG12, MOSI
 * PG14, CS# PG15, RST# PB5), his/85 (safe-state pins), the W5500 datasheet
 * register map and ST RM0468 (SPI). Sets the RF safe state, brings up the
 * Ethernet controller at 192.168.73.10/24 and echoes every UDP datagram on
 * port 1024 back to its sender. Runs until the emulator stops. */
#include "bench.h"

#define GPIO(p) (0x58020000u + 0x400u * (p))
#define MODER(p) REG(GPIO(p) + 0x00)
#define BSRR(p)  REG(GPIO(p) + 0x18)
enum { A, B, C, D, E, F, G, H };

static void out_pin(int port, int pin, int level)
{
    MODER(port) = (MODER(port) & ~(3u << (2 * pin))) | (1u << (2 * pin));
    BSRR(port) = level ? (1u << pin) : (1u << (pin + 16));
}

#define SPI6 0x58001400u
#define S_CR1  REG(SPI6 + 0x00)
#define S_CFG1 REG(SPI6 + 0x08)
#define S_CFG2 REG(SPI6 + 0x0C)
#define S_SR   REG(SPI6 + 0x14)
#define S_TXDR (*(volatile uint8_t *)(SPI6 + 0x20))
#define S_RXDR (*(volatile uint8_t *)(SPI6 + 0x30))

static uint8_t xfer(uint8_t b)
{
    for(int n = 0; n < 100000 && !(S_SR & (1u << 1)); n++) {}   /* TXP */
    S_TXDR = b;
    for(int n = 0; n < 100000 && !(S_SR & (1u << 0)); n++) {}   /* RXP */
    return S_RXDR;
}

static void cs(int level) { BSRR(G) = level ? (1u << 15) : (1u << 31); }

static void wr(uint16_t addr, uint8_t bsb, const uint8_t *d, int n)
{
    cs(0);
    xfer(addr >> 8); xfer((uint8_t)addr); xfer((uint8_t)((bsb << 3) | 4));
    for(int i = 0; i < n; i++) xfer(d[i]);
    cs(1);
}

static void rd(uint16_t addr, uint8_t bsb, uint8_t *d, int n)
{
    cs(0);
    xfer(addr >> 8); xfer((uint8_t)addr); xfer((uint8_t)(bsb << 3));
    for(int i = 0; i < n; i++) d[i] = xfer(0);
    cs(1);
}

static void wr8(uint16_t a, uint8_t bsb, uint8_t v) { wr(a, bsb, &v, 1); }
static uint8_t rd8(uint16_t a, uint8_t bsb) { uint8_t v; rd(a, bsb, &v, 1); return v; }
static uint16_t rd16(uint16_t a, uint8_t bsb) { uint8_t v[2]; rd(a, bsb, v, 2); return (uint16_t)((v[0] << 8) | v[1]); }
static void wr16(uint16_t a, uint8_t bsb, uint16_t v) { uint8_t b[2] = { (uint8_t)(v >> 8), (uint8_t)v }; wr(a, bsb, b, 2); }

int main(void)
{
    console_init();
    puts_("bench net_test\r\n");
    RCC_AHB4ENR |= 0xFFu;                 /* GPIOA..GPIOH */
    RCC_APB4ENR |= 1u << 5;               /* SPI6EN */

    /* his/85 safe state: T/R at RX levels, both SPI1 chains output-disabled,
     * PA DAC in reset, codec in reset, PD12 low */
    out_pin(F, 4, 1); out_pin(F, 6, 0); out_pin(C, 7, 0); out_pin(H, 11, 0);
    out_pin(A, 10, 1); out_pin(H, 10, 1);
    out_pin(B, 14, 0); out_pin(B, 10, 0); out_pin(D, 12, 0);

    /* CH394: CS# high, release RST# */
    out_pin(G, 15, 1);
    out_pin(B, 5, 0); out_pin(B, 5, 1);
    S_CFG1 = 7u | (3u << 28);             /* 8-bit, clock / 16 */
    S_CFG2 = (1u << 22) | (1u << 26);     /* master, software slave management */
    S_CR1 = (1u << 12) | 1u;              /* SSI, SPE */
    S_CR1 |= 1u << 9;                     /* CSTART */

    static const uint8_t gar[4] = { 192, 168, 73, 1 }, sub[4] = { 255, 255, 255, 0 }, ip[4] = { 192, 168, 73, 10 };
    wr(0x0001, 0, gar, 4);
    wr(0x0005, 0, sub, 4);
    wr(0x000F, 0, ip, 4);
    test_result("net_link", (rd8(0x002E, 0) & 1) != 0, "phycfgr", rd8(0x002E, 0));

    /* socket 0: UDP on port 1024 */
    wr8(0x0000, 1, 0x02);
    wr16(0x0004, 1, 1024);
    wr8(0x0001, 1, 0x01);
    test_result("net_udp_open", rd8(0x0003, 1) == 0x22, "sr", rd8(0x0003, 1));
    puts_("net ready\r\n");

    uint32_t echoes = 0;
    for(;;)
    {
        uint16_t rsr = rd16(0x0026, 1);
        if(rsr < 8) continue;
        uint16_t rxrd = rd16(0x0028, 1);
        uint8_t hdr[8];
        rd(rxrd, 3, hdr, 8);
        uint16_t len = (uint16_t)((hdr[6] << 8) | hdr[7]);
        uint8_t data[512];
        if(len > sizeof data) len = sizeof data;
        rd((uint16_t)(rxrd + 8), 3, data, len);
        wr16(0x0028, 1, (uint16_t)(rxrd + 8 + len));
        wr8(0x0001, 1, 0x40);             /* RECV */
        wr(0x000C, 1, hdr, 4);            /* DIPR = sender */
        wr(0x0010, 1, &hdr[4], 2);        /* DPORT = sender port */
        uint16_t txwr = rd16(0x0024, 1);
        wr(txwr, 2, data, len);
        wr16(0x0024, 1, (uint16_t)(txwr + len));
        wr8(0x0001, 1, 0x20);             /* SEND */
        wr8(0x0002, 1, 0x10);             /* clear SEND_OK */
        echoes++;
        puts_("echo "); putdec(echoes); puts_("\r\n");
    }
}
