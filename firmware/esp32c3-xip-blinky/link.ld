/* SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * ESP32-C3 direct boot layout: the flash image starts at flash offset 0,
 * which the ROM maps one to one into both cache windows (IROM 0x42000000
 * and DROM 0x3c000000) before jumping to 0x42000008. Code is linked in
 * IROM; read-only data follows it in flash and is linked at the matching
 * DROM address, so `.rodata`'s load address (in IROM) and run address (in
 * DROM) are the same flash byte. Only .bss is in RAM (zeroed by _start).
 */
ENTRY(_start)
MEMORY
{
    irom (rx) : ORIGIN = 0x42000000, LENGTH = 64K
    dram (rw) : ORIGIN = 0x3fc80000, LENGTH = 320K
}
SECTIONS
{
    .text : {
        KEEP(*(.text.header))
        *(.text.entry)
        *(.text .text.*)
        . = ALIGN(4);
    } > irom
    .rodata (ADDR(.text) + SIZEOF(.text) - 0x42000000 + 0x3c000000) : AT(ADDR(.text) + SIZEOF(.text)) {
        *(.rodata .rodata.*)
        . = ALIGN(4);
    }
    .bss (NOLOAD) : { __bss_start = .; *(.bss .bss.*) *(COMMON) . = ALIGN(4); __bss_end = .; } > dram
    /DISCARD/ : { *(.data .data.*) }
}
