/* RF FPGA bus self-test (own test scaffolding), written from his/80 only:
 * D0-D7 = PE0-PE7, CMD = PE8, STB = PE9, DRDY = PE10 (EXTI10). A rising STB
 * with CMD high latches a command; with CMD low it moves one data byte. */
#include "bench.h"

#define GPIOE 0x58021000u
#define E_MODER REG(GPIOE + 0x00)
#define E_IDR   REG(GPIOE + 0x10)
#define E_BSRR  REG(GPIOE + 0x18)
#define CMD (1u << 8)
#define STB (1u << 9)
#define SYSCFG_EXTICR3 REG(0x58000400u + 0x10)
#define EXTI_RTSR1  REG(0x58000000u + 0x00)
#define EXTI_IMR1   REG(0x58000000u + 0x80)
#define EXTI_PR1    REG(0x58000000u + 0x88)

static void data_out(void) { E_MODER = (E_MODER & ~0xFFFFu) | 0x5555u; }
static void data_in(void)  { E_MODER &= ~0xFFFFu; }

static void command(uint8_t c)
{
    data_out();
    E_BSRR = (0xFFu << 16) | c | CMD;
    E_BSRR = STB;
    E_BSRR = (STB | CMD) << 16;
}

static void write_byte(uint8_t d)
{
    data_out();
    E_BSRR = (0xFFu << 16) | d;
    E_BSRR = STB;
    E_BSRR = STB << 16;
}

static uint8_t read_byte(void)
{
    data_in();
    E_BSRR = STB;
    uint8_t d = (uint8_t)(E_IDR & 0xFF);
    E_BSRR = STB << 16;
    return d;
}

static int32_t s24(const uint8_t *b) { int32_t v = (b[0] << 16) | (b[1] << 8) | b[2]; return (v << 8) >> 8; }

static volatile uint32_t drdy;
static volatile int32_t iq[4][2];

void EXTI15_10_IRQHandler(void)
{
    if(EXTI_PR1 & (1u << 10))
    {
        EXTI_PR1 = 1u << 10;
        if(drdy < 4)
        {
            uint8_t b[6];
            command(0x04);
            for(int i = 0; i < 6; i++) b[i] = read_byte();
            iq[drdy][0] = s24(&b[0]); iq[drdy][1] = s24(&b[3]);
        }
        drdy++;
    }
}

int main(void)
{
    console_init();
    puts_("bench fpga_test\r\n");
    RCC_AHB4ENR |= 1u << 4;   /* GPIOEEN */
    RCC_APB4ENR |= 1u << 1;   /* SYSCFGEN */
    E_MODER = (E_MODER & ~(0xFu << 16)) | (0x5u << 16);   /* PE8, PE9 outputs; PE10 input */
    E_BSRR = (CMD | STB) << 16;

    /* 0x00 loopback */
    command(0x00); write_byte(0xA5); uint8_t a = read_byte();
    command(0x00); write_byte(0x5A); uint8_t b = read_byte();
    test_result("fpga_loopback", a == 0xA5 && b == 0x5A, "a<<8|b", ((uint32_t)a << 8) | b);

    /* 0x08 version: three bytes */
    command(0x08);
    uint8_t v0 = read_byte(), v1 = read_byte(), v2 = read_byte();
    test_result("fpga_version", v0 == 1 && v1 == 0 && v2 == 0, "ver", ((uint32_t)v0 << 16) | (v1 << 8) | v2);

    /* 0x02 status: seven bytes, default all zero */
    command(0x02);
    uint32_t st = 0;
    for(int i = 0; i < 7; i++) st |= read_byte();
    test_result("fpga_status", st == 0, "or", st);

    /* 0x01 frame: 24 bytes, rate index 3 (one sample per DRDY) */
    command(0x01);
    for(int i = 0; i < 24; i++) write_byte(i == 23 ? 3 : (uint8_t)i);
    command(0x00);   /* the next command ends the frame */
    test_result("fpga_frame_sent", 1, 0, 0);

    /* DRDY on EXTI10 -> RX I/Q read of one sample per DRDY */
    SYSCFG_EXTICR3 = (SYSCFG_EXTICR3 & ~(0xFu << 8)) | (4u << 8);   /* line 10 = port E */
    EXTI_RTSR1 |= 1u << 10;
    EXTI_IMR1 |= 1u << 10;
    nvic_enable(40);
    irq_on();
    for(int n = 0; n < 4000000 && drdy < 4; n++) {}
    test_result("fpga_drdy", drdy >= 4, "drdy", drdy);
    int changing = iq[0][0] != iq[1][0] || iq[0][1] != iq[1][1];
    int nonzero = iq[0][0] != 0 || iq[0][1] != 0;
    int bounded = 1;
    for(int k = 0; k < 4; k++)
        if(iq[k][0] > 0x7FFFFF || iq[k][0] < -0x800000 || iq[k][1] > 0x7FFFFF || iq[k][1] < -0x800000) bounded = 0;
    test_result("fpga_rx_iq", changing && nonzero && bounded, "i0", (uint32_t)iq[0][0]);
    return 0;
}
