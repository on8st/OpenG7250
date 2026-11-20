# Bench test programs (own test scaffolding, not product code).
# Output -> git-ignored ../../.private/bench/<name>.elf
CROSS   ?= arm-none-eabi-
CC       = $(CROSS)gcc
OUT      = ../../.private/bench
ARCH     = -mcpu=cortex-m7 -mthumb -mfpu=fpv5-d16 -mfloat-abi=hard
CFLAGS   = $(ARCH) -O1 -g3 -ffreestanding -fno-common -Wall -Wextra -Wno-override-init \
           -ffunction-sections -fdata-sections
LDFLAGS  = $(ARCH) -T bench.ld -nostdlib -Wl,--gc-sections
PROGS    = smoke dma_test usart_test

.PHONY: all clean
all: $(PROGS:%=$(OUT)/%.elf)

$(OUT):
	mkdir -p $(OUT)

$(OUT)/%.elf: %.c start.c bench.h bench.ld | $(OUT)
	$(CC) $(CFLAGS) $(LDFLAGS) $< start.c -o $@

clean:
	rm -f $(PROGS:%=$(OUT)/%.elf)
