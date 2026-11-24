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
 *   FQ n      program words n and n + 1 back to back -> SR1, first word of n + 1
 *   FD n      clear the error flags, read the n-th word -> SR1, word
 *   FB n o    clear the error flags, read the byte at offset o of the n-th word -> SR1, byte
 *   FC        erase sector 7 with interrupts masked, polling from RAM: the length by the DWT cycle
 *             counter, TIM5 (32 bits, 1 MHz) and TIM12 (16 bits, 1 MHz, unwrapped in the loop)
 *             -> SR1, cycles, TIM5 us, TIM12 us, worst |cycles / 64 - TIM5| us, polls
 *   FV        the figures of an FC that a reset ended (RAM is kept) -> as FC
 *   FL n      write a log ring entry n as the product's ring lays it out (ring at 0x2401E700) -> 0
 *   FK n      read it back -> its word 0 (low), its first text word (low)
 *   FG n      program word n + 1, then read word n at once, and again some us later
 *             -> SR1 after the first read, SR1 after the second
 *   FS        word 9 and SR1 as the very first code of this start read them -> word, SR1
 *   FH n      TIM2's interrupt (exception 44) pended; its handler spins about n ms -> 0
 *   FU        UsageFault enabled (SHCSR USGFAULTENA) -> SHCSR
 *   FF        the kept record of the last fault (the handler keeps it, then resets by SYSRESETREQ)
 *             -> count, CFSR, HFSR, IPSR at entry, stacked PC, stacked IPSR, EXC_RETURN, the fault's mark
 * Except FC the code runs from flash: an erase stalls it until the erase ends. */
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

#define KEEP(i) W32(0x2401F000u + 4u * (i))
#define T5(o)  REG(0x40000C00u + (o))
#define T12(o) REG(0x40001800u + (o))
#define CYCCNT REG(0xE0001004u)

/* the first code of a start: a read of word 9 of sector 7 (a torn word of a power cut) */
void early_hook(void)
{
    KEEP(8) = W32(S7 + 32u * 9u);
    KEEP(9) = F_SR;
}

static void timers_on(void)
{
    RCC_APB1LENR |= (1u << 3) | (1u << 6);              /* TIM5EN, TIM12EN */
    T5(0x00) = 0; T12(0x00) = 0;
    T5(0x28) = 63; T5(0x2C) = 0xFFFFFFFFu; T5(0x14) = 1; T5(0x10) = 0;
    T12(0x28) = 63; T12(0x2C) = 0xFFFFu; T12(0x14) = 1; T12(0x10) = 0;
    T5(0x00) = 1; T12(0x00) = 1;
    REG(0xE000EDFCu) |= 1u << 24;                       /* DEMCR TRCENA */
    REG(0xE0001FB0u) = 0xC5ACCE55u;                     /* DWT_LAR */
    REG(0xE0001000u) |= 1u;
}

/* runs from RAM (.data): no fetch from the bank during the erase */
__attribute__((section(".data.ramfunc"), noinline, long_call, used))
static void erase_masked(void)
{
    uint32_t c0, t0, l12, u12 = 0, worst = 0, n = 0;
    __asm volatile ("cpsid i");
    F_CR = (1u << 2) | (3u << 4) | (7u << 8);
    c0 = CYCCNT; t0 = T5(0x24); l12 = T12(0x24);
    F_CR |= 1u << 7;
    do
    {
        uint32_t c = CYCCNT - c0, t = T5(0x24) - t0, m = T12(0x24);
        uint32_t us = c / 64u, d = us > t ? us - t : t - us;
        u12 += (m - l12) & 0xFFFFu; l12 = m;
        if(d > worst) worst = d;
        n++;
        KEEP(1) = c; KEEP(2) = t; KEEP(3) = u12; KEEP(4) = worst; KEEP(5) = n;
    } while(F_SR & 7u);
    KEEP(0) = F_SR;
    F_CR = 0;
    __asm volatile ("cpsie i");
}

/* runner self-test T-EMU-29 (cpu.fault): HardFault and UsageFault keep the frame in KEEP(16..23) */
void fault_keep(uint32_t *frame, uint32_t ipsr, uint32_t exc_return)
{
    KEEP(16) = KEEP(16) + 1;
    KEEP(17) = REG(0xE000ED28u); KEEP(18) = REG(0xE000ED2Cu); KEEP(19) = ipsr;
    KEEP(20) = frame[6]; KEEP(21) = frame[7] & 0x1FFu; KEEP(22) = exc_return; KEEP(23) = 0xFA0170FFu;
    puts_("FAULT kept, reset\r\n");
    REG(0xE000ED0Cu) = 0x05FA0004u;       /* SYSRESETREQ */
    for(;;) {}
}
__attribute__((naked)) void hardfault_handler(void)
{
    __asm volatile ("tst lr, #4\n ite eq\n mrseq r0, msp\n mrsne r0, psp\n mrs r1, ipsr\n mov r2, lr\n b fault_keep\n");
}
__attribute__((naked)) void usagefault_handler(void)
{
    __asm volatile ("tst lr, #4\n ite eq\n mrseq r0, msp\n mrsne r0, psp\n mrs r1, ipsr\n mov r2, lr\n b fault_keep\n");
}

static volatile uint32_t spin_ms;
void TIM2_IRQHandler(void)
{
    REG(0xE000E180u) = 1u << 28;          /* ICER0: once */
    for(volatile uint32_t k = 0; k < spin_ms * 6400u; k++) {}
}

static uint16_t command(uint8_t *d, uint16_t len)
{
    uint32_t a = 0, b = 0;
    if(d[1] == 'C' || d[1] == 'V')
    {
        if(d[1] == 'C')
        {
            if(F_CR & 1u) { F_KEYR = 0x45670123u; F_KEYR = 0xCDEF89ABu; }
            F_CCR = F_ERRS | (1u << 16);
            KEEP(0) = 0xFFFFFFFFu;
            timers_on();
            erase_masked();
        }
        for(int i = 0; i < 6; i++) put_be(&d[2 + 4 * i], KEEP(i));
        return 26;
    }
    if(d[1] == 'S') { a = KEEP(8); b = KEEP(9); }
    if((d[1] == 'L' || d[1] == 'K') && len >= 3)
    {
        uint32_t e = 0x2401E700u + 48u + (d[2] & 63u) * 96u;
        if(d[1] == 'L')
        {
            W32(e) = d[2]; W32(e + 4u) = 1234u; W32(e + 8u) = 72u; W32(e + 12u) = 0;
            for(uint32_t i = 0; i < 18; i++) W32(e + 16u + 4u * i) = 0x41424344u + i;
            W32(e + 88u) = 0xC0DEC0DEu; W32(e + 92u) = 0;
        }
        a = W32(e); b = W32(e + 16u);
    }
    if(d[1] == 'G' && len >= 3)
    {
        uint32_t t = S7 + 32u * (d[2] + 1u);
        volatile uint32_t x;
        if(F_CR & 1u) { F_KEYR = 0x45670123u; F_KEYR = 0xCDEF89ABu; }
        F_CCR = F_ERRS | (3u << 25) | (1u << 16);
        F_CR = (1u << 1) | (3u << 4);
        for(uint32_t i = 0; i < 8; i++) W32(t + 4u * i) = 0xA5000000u + ((d[2] + 1u) << 8) + i;
        flash_wait();
        F_CR = 0;
        F_CCR = F_ERRS | (3u << 25) | (1u << 16);
        x = W32(S7 + 32u * d[2]);
        a = F_SR;
        for(volatile uint32_t k = 0; k < 2000; k++) {}
        F_CCR = F_ERRS | (3u << 25);
        x = W32(S7 + 32u * d[2]);
        (void)x;
        b = F_SR;
    }
    if(d[1] == 'D' && len >= 3)
    {
        F_CCR = F_ERRS | (3u << 25);      /* SNECCERR and DBECCERR too */
        b = W32(S7 + 32u * d[2]); a = F_SR;
    }
    if(d[1] == 'B' && len >= 4)
    {
        F_CCR = F_ERRS | (3u << 25);
        b = *(volatile uint8_t *)(S7 + 32u * d[2] + (d[3] & 31u)); a = F_SR;
    }
    if(d[1] == 'Q' && len >= 3)
    {
        if(F_CR & 1u) { F_KEYR = 0x45670123u; F_KEYR = 0xCDEF89ABu; }
        F_CCR = F_ERRS | (1u << 16);
        for(uint32_t k = d[2]; k < d[2] + 2u; k++)
        {
            uint32_t t = S7 + 32u * k;
            F_CR = (1u << 1) | (3u << 4);
            for(uint32_t i = 0; i < 8; i++) W32(t + 4u * i) = 0xA5000000u + (k << 8) + i;
            a = flash_wait(); b = W32(t);
            F_CCR = 1u << 16;
            F_CR = 0;
        }
    }
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
    else if(d[1] == 'H' && len >= 3)
    {
        spin_ms = d[2];
        REG(0xE000E100u) = 1u << 28;      /* ISER0: TIM2 */
        REG(0xE000E200u) = 1u << 28;      /* ISPR0: pended, taken at once */
        __asm volatile ("dsb\n isb\n");
    }
    else if(d[1] == 'U') { REG(0xE000ED24u) |= 1u << 18; a = REG(0xE000ED24u); }
    else if(d[1] == 'T' && len >= 4)
    {
        // PH6 pull changes from the image while the bench drives the pin (T-EMU-32): d[2] pairs of
        // pull-up then no pull, each change followed by d[3] x 1000 spin loops; ends with no pull
        for(uint32_t k = 0; k < d[2]; k++)
            for(uint32_t u = 0; u < 2; u++)
            {
                REG(GPIO(H) + 0x0C) = (REG(GPIO(H) + 0x0C) & ~(3u << 12)) | ((u == 0 ? 1u : 0u) << 12);
                a++;
                for(volatile uint32_t w = 0; w < 1000u * d[3]; w++) {}
            }
    }
    else if(d[1] == 'F')
    {
        for(uint32_t i = 0; i < 8; i++) put_be(&d[2 + 4 * i], KEEP(16 + i));
        return 34;
    }
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
