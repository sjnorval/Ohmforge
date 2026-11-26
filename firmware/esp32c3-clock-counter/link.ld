/* SPDX-License-Identifier: AGPL-3.0-or-later */
ENTRY(_start)
MEMORY
{
    irom (rx) : ORIGIN = 0x42000020, LENGTH = 64K
    drom (r)  : ORIGIN = 0x3c010020, LENGTH = 64K
    dram (rw) : ORIGIN = 0x3fc80000, LENGTH = 320K
}
SECTIONS
{
    .text   : { *(.text.entry) *(.text .text.*) } > irom
    .rodata : { *(.rodata .rodata.*) } > drom
    .data   : { *(.data .data.*) } > dram
    .bss    : { *(.bss .bss.*) *(COMMON) } > dram
}
