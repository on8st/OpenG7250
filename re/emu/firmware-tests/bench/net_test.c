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

/* Commands in a datagram that starts with 'F' (runner self-test T-EMU-23); the reply is the
 * same first two bytes and then big-endian words:
 *   FE        erase sector 7                        -> SR1, word at 0x080E0000
 *   FP n      program the n-th 32-byte word of it   -> SR1, first word read back
 *   FW        start the watchdog (0.5 s, not fed)   -> 0
 *   FR        RCC_RSR, then RMVF                    -> RSR
 *   FM        0x2401E700 and 0x2401FFFC, then a marker is written to both -> the two old words
 *   FX        software reset (no reply)
 * The code runs from flash: an erase stalls it until the erase ends. */
#define FLASHC 0x52002000u
#define F_KEYR REG(FLASHC + 0x004)
#define F_CR   REG(FLASHC + 0x00C)
#define F_SR   REG(FLASHC + 0x010)
#define F_CCR  REG(FLASHC + 0x014)
#define F_ERRS (0x1FEu << 16)
#define S7     0x080E0000u
#define W32(a) (*(volatile uint32_t *)(a))

static void put_be(uint8_t *d, uint32_t v) { d[0] = (uint8_t)(v >> 24); d[1] = (uint8_t)(v >> 16); d[2] = (uint8_t)(v >> 8); d[3] = (uint8_t)v; }

static uint32_t flash_wait(void)
{
    while(F_SR & 7u) {}                   /* BSY, WBNE, QW */
    return F_SR;
}

static uint16_t command(uint8_t *d, uint16_t len)
{
    uint32_t a = 0, b = 0;
    if(d[1] == 'E' || d[1] == 'P')
    {
        if(F_CR & 1u) { F_KEYR = 0x45670123u; F_KEYR = 0xCDEF89ABu; }
        F_CCR = F_ERRS | (1u << 16);
    }
    if(d[1] == 'E')
    {
        F_CR = (1u << 2) | (3u << 4) | (7u << 8);
        F_CR |= 1u << 7;
        a = flash_wait(); b = W32(S7);
        F_CR = 0;
    }
    else if(d[1] == 'P' && len >= 3)
    {
        uint32_t t = S7 + 32u * d[2];
        F_CR = (1u << 1) | (3u << 4);
        for(int i = 0; i < 8; i++) W32(t + 4u * i) = 0xA5000000u + ((uint32_t)d[2] << 8) + i;
        a = flash_wait(); b = W32(t);
        F_CR = 0;
    }
    else if(d[1] == 'W')
    {
        REG(0x58004800u) = 0xCCCCu; REG(0x58004800u) = 0x5555u;
        REG(0x58004804u) = 0; REG(0x58004808u) = 4000u; REG(0x58004800u) = 0xAAAAu;
    }
    else if(d[1] == 'R') { a = REG(RCC_BASE + 0xD0); REG(RCC_BASE + 0xD0) = 1u << 16; b = REG(RCC_BASE + 0xD0); }
    else if(d[1] == 'M')
    {
        a = W32(0x2401E700u); b = W32(0x2401FFFCu);
        W32(0x2401E700u) = 0x4B455054u; W32(0x2401FFFCu) = 0x4B455055u;
    }
    else if(d[1] == 'X') { REG(0xE000ED0Cu) = 0x05FA0004u; for(;;) {} }
    put_be(&d[2], a); put_be(&d[6], b);
    return 10;
}

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
        if(len >= 2 && data[0] == 'F' && data[1] >= 'A' && data[1] <= 'Z') len = command(data, len);
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
