/*
 * Bench test programs (own test scaffolding, not product code). Generic
 * STM32H72x map as the emulator's Main platform models it: flash bank 1
 * 1 MiB @ 0x08000000, AXI SRAM 320 KiB @ 0x24000000 (the stack at its top).
 * A D2 SRAM window @ 0x30000000 (32 KiB) holds DMA buffers (.dmabuf).
 */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08000000, LENGTH = 1024K
  RAM   (rwx) : ORIGIN = 0x24000000, LENGTH = 320K
  D2RAM (rw)  : ORIGIN = 0x30000000, LENGTH = 32K
}
_estack = ORIGIN(RAM) + LENGTH(RAM);
ENTRY(reset_handler)
SECTIONS
{
  .isr_vector : { KEEP(*(.isr_vector)) } > FLASH
  .text : { *(.text*) *(.rodata*) . = ALIGN(4); } > FLASH
  _sidata = LOADADDR(.data);
  .data : { . = ALIGN(4); _sdata = .; *(.data*) . = ALIGN(4); _edata = .; } > RAM AT > FLASH
  .bss : { . = ALIGN(4); _sbss = .; *(.bss*) *(COMMON) . = ALIGN(4); _ebss = .; } > RAM
  .dmabuf (NOLOAD) : { . = ALIGN(32); *(.dmabuf*) } > D2RAM
}
