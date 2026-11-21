# Bench test programs (own test scaffolding, not product code).
# Output -> git-ignored ../../.private/bench/<name>.elf
CROSS   ?= arm-none-eabi-
CC       = $(CROSS)gcc
OUT      = ../../.private/bench
ARCH     = -mcpu=cortex-m7 -mthumb -mfpu=fpv5-d16 -mfloat-abi=hard
CFLAGS   = $(ARCH) -O1 -g3 -ffreestanding -fno-common -Wall -Wextra -Wno-override-init \
           -ffunction-sections -fdata-sections
LDFLAGS  = $(ARCH) -T bench.ld -nostdlib -Wl,--gc-sections
PROGS    = smoke dma_test usart_test adc_test mdma_test fpga_test net_test wdt_test spi6dma_test dbgmcu_test pwr_test pwr_bad_test clock_test

.PHONY: all clean
all: $(PROGS:%=$(OUT)/%.elf)

$(OUT):
	mkdir -p $(OUT)

$(OUT)/%.elf: %.c start.c bench.h bench.ld | $(OUT)
	$(CC) $(CFLAGS) $(LDFLAGS) $< start.c -o $@

# programs run as product-style images at 0x08020000 (runner self-checks)
$(OUT)/clock_test.elf: clock_test.c start.c bench.h bench_app.ld | $(OUT)
	$(CC) $(CFLAGS) $(ARCH) -T bench_app.ld -nostdlib -Wl,--gc-sections clock_test.c start.c -o $@

$(OUT)/net_test.elf: net_test.c start.c bench.h bench_app.ld | $(OUT)
	$(CC) $(CFLAGS) $(ARCH) -T bench_app.ld -nostdlib -Wl,--gc-sections net_test.c start.c -o $@

clean:
	rm -f $(PROGS:%=$(OUT)/%.elf)
