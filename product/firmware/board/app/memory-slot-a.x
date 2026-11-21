/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Application, slot A: sectors 2-3 (g7250-layout SLOT_A). */
MEMORY
{
  @FLASH@
  /* AXI SRAM; 128 KiB is present whatever the TCM/AXI sharing option ([HIS] HIS 15). */
  RAM   : ORIGIN = 0x24000000, LENGTH = 128K - 128
  /* The top 128 bytes of that AXI SRAM: values kept over a reset without power loss
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
  .pad_text : { BYTE(0xFF); FILL(0xFF); . = ALIGN(16); } > FLASH
} INSERT AFTER .text;
SECTIONS
{
  .pad_rodata : { BYTE(0xFF); FILL(0xFF); . = ALIGN(16); } > FLASH
} INSERT AFTER .rodata;
