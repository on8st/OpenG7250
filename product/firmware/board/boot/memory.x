/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Boot stage: sector 1, where the vendor bootloader starts an application ([HIS] HIS 10). */
MEMORY
{
  @FLASH@
  /* AXI SRAM; 128 KiB is present whatever the TCM/AXI sharing option ([HIS] HIS 15). The top
     128 bytes (0x2401FF80) are the application's persist block (board/common persist.rs),
     kept over a reset: the boot stage's data, bss and stack stay below it. */
  RAM   : ORIGIN = 0x24000000, LENGTH = 128K - 128
  /* The persist block: the boot stage writes only its TX path check handover there
     (persist.rs write_handover, A3.6b), just before the jump. */
  PERSIST : ORIGIN = 0x2401FF80, LENGTH = 128
}

/* Not initialised by the start code: the application reads it (persist.rs). */
SECTIONS
{
  .persist (NOLOAD) : ALIGN(8)
  {
    KEEP(*(.persist .persist.*));
  } > PERSIST
} INSERT AFTER .uninit;
/* Keep the stack limit and heap start in RAM, not after .persist (cortex-m-rt takes `.`). */
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
