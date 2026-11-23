/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Application, slot B: sectors 4-5 (g7250-layout SLOT_B). */
MEMORY
{
  @FLASH@
  /* AXI SRAM; 128 KiB is present whatever the TCM/AXI sharing option ([HIS] HIS 15). */
  RAM   : ORIGIN = 0x24000000, LENGTH = 128K - 6400
  LOGRING : ORIGIN = 0x2401E700, LENGTH = 6272
  /* Below and above: values kept over a reset without power loss: LOGRING, the log ring
     (A1.7, board/common logmem.rs), and the top 128 bytes, the counters and the handover
     (TRIP_COUNT, board/common persist.rs); outside RAM above, so no data, bss or stack. */
  PERSIST : ORIGIN = 0x2401FF80, LENGTH = 128
  /* SRAM4 (D3 domain, 16 KiB, RM0468): the only RAM the BDMA reaches, so the only place for
     SPI6 DMA buffers. The Backup SRAM at 0x38800000 is never used ([HIS] HIS 15 rule 4). */
  SRAM4 : ORIGIN = 0x38000000, LENGTH = 16K
}

/* Buffers for BDMA (SPI6): `#[unsafe(link_section = ".sram4")]`; not initialised at start. */
SECTIONS
{
  .sram4 (NOLOAD) : ALIGN(32)
  {
    *(.sram4 .sram4.*);
    . = ALIGN(32);
  } > SRAM4
} INSERT AFTER .uninit;
/* Kept over a reset, never initialised by the start code (persist.rs checks it). */
SECTIONS
{
  .persist (NOLOAD) : ALIGN(8)
  {
    KEEP(*(.persist .persist.*));
  } > PERSIST
} INSERT AFTER .sram4;
SECTIONS
{
  .logring (NOLOAD) : ALIGN(8)
  {
    KEEP(*(.logring .logring.*));
  } > LOGRING
} INSERT AFTER .persist;
/* Keep the stack limit and heap start in RAM, not after .sram4 (cortex-m-rt takes `.`). */
_stack_end = ALIGN(__euninit, 4);
__sheap = _stack_end;

/* Image header at offset 0x400, after the vector table (g7250-layout HEADER_OFFSET). */
_stext = ORIGIN(FLASH) + 0x440;
SECTIONS
{
  .image_header ORIGIN(FLASH) + 0x400 :
  {
    KEEP(*(.image_header));
  } > FLASH
} INSERT AFTER .vector_table;

/* Fill the alignment gaps after .text and .rodata with 0xFF, so that the loadable image is one
   gap-free run of bytes from its start (g7250-imgtool refuses an image with a gap). */
SECTIONS
{
  .pad_text : { BYTE(0xFF); FILL(0xFF); . = ALIGN(32); } > FLASH
} INSERT AFTER .text;
/* The fault path of A3.6 (board/common stop.rs) on whole flash words: the pin code from a word
   start, each fault vector's entry and its literal on one word of its own, so that the bound of
   check-safe-margin.py does not move with the code around it. Gaps filled with 0xFF. */
SECTIONS
{
  .fault_text : ALIGN(32)
  {
    FILL(0xFF);
    KEEP(*(.text.g7250_fault_pins));
    . = ALIGN(32);
    KEEP(*(.text.g7250_fault_vector_3));
    . = ALIGN(32);
    KEEP(*(.text.g7250_fault_vector_4));
    . = ALIGN(32);
    KEEP(*(.text.g7250_fault_vector_5));
    . = ALIGN(32);
    KEEP(*(.text.g7250_fault_vector_6));
    . = ALIGN(16);
  } > FLASH
} INSERT AFTER .pad_text;
SECTIONS
{
  .pad_rodata : { BYTE(0xFF); FILL(0xFF); . = ALIGN(16); } > FLASH
} INSERT AFTER .rodata;
