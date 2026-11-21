/*
 * Application variant of bench.ld: the image starts at 0x08020000, where a
 * product image starts on the radio (behind the vendor bootloader, which the
 * emulator replaces by the runner's stand-in, docs/PRODUCT-RUNNER.md).
 * Own test scaffolding, not product code.
 */
MEMORY
{
  FLASH (rx)  : ORIGIN = 0x08020000, LENGTH = 896K
  RAM   (rwx) : ORIGIN = 0x24000000, LENGTH = 320K
  D2RAM (rw)  : ORIGIN = 0x30000000, LENGTH = 32K
  D3RAM (rw)  : ORIGIN = 0x38000000, LENGTH = 16K
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
  .d3buf (NOLOAD) : { . = ALIGN(32); *(.d3buf*) } > D3RAM
}
