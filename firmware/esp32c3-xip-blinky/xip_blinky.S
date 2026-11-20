/* SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Ohmforge test firmware: ESP32-C3 blinky that runs in place from flash
 * (XIP), booted by the real ROM's direct boot.
 *
 * The image is written to flash offset 0. Its first two words are the
 * direct boot magic (0xaedb041d twice), so the ROM maps the first 8 MiB of
 * flash one to one into the IROM and DROM cache windows and jumps to
 * 0x42000008. From there, without any ROM calls:
 *
 * - zero .bss and set up a stack in DRAM;
 * - switch off the watchdogs the ROM leaves armed for a flash boot (RWDT,
 *   MWDT0 and MWDT1 flash boot protection; super watchdog on auto-feed);
 * - print a banner held in DROM (read through the cache's data bus);
 * - sum a 16-word table in DROM into `drom_sum` (a check that data reads
 *   through the MMU see the right flash bytes);
 * - drive GPIO8 through the IO MUX and GPIO matrix and toggle it every
 *   HALF_PERIOD SYSTIMER ticks (16 MHz): a 500 us period; `toggles`
 *   counts the rising edges. Deadlines are absolute, so the period does not drift.
 */

    .equ UART0,       0x60000000
    .equ GPIO,        0x60004000
    .equ RTC_CNTL,    0x60008000
    .equ IO_MUX,      0x60009000
    .equ TIMG0,       0x6001f000
    .equ TIMG1,       0x60020000
    .equ SYSTIMER,    0x60023000
    .equ WDT_WKEY,    0x50d83aa1
    .equ SWD_WKEY,    0x8f1d312a
    .equ PIN,         8
    .equ HALF_PERIOD, 4000

    .section .text.header, "ax"
    .word 0xaedb041d            /* direct boot magic, checked by the ROM */
    .word 0xaedb041d

    .section .text.entry, "ax"
    .globl _start
_start:
    li   sp, 0x3fcdfff0
    la   t0, __bss_start
    la   t1, __bss_end
1:  bgeu t0, t1, 2f
    sw   zero, 0(t0)
    addi t0, t0, 4
    j    1b
2:
    /* Watchdogs off (as in esp32c3-timer-irq). */
    li   t0, RTC_CNTL
    li   t1, WDT_WKEY
    sw   t1, 0xa8(t0)           /* RTC_CNTL_WDTWPROTECT */
    sw   zero, 0x90(t0)         /* RTC_CNTL_WDTCONFIG0 */
    sw   zero, 0xa8(t0)
    li   t1, SWD_WKEY
    sw   t1, 0xb0(t0)           /* RTC_CNTL_SWD_WPROTECT */
    lw   t2, 0xac(t0)           /* RTC_CNTL_SWD_CONF */
    li   t3, 1 << 31            /* SWD_AUTO_FEED_EN */
    or   t2, t2, t3
    sw   t2, 0xac(t0)
    sw   zero, 0xb0(t0)
    li   t1, WDT_WKEY
    li   t2, 1 << 22            /* WDT_CONF_UPDATE_EN, EN = FLASHBOOT = 0 */
    li   t0, TIMG0
    sw   t1, 0x64(t0)           /* TIMG_WDTWPROTECT */
    sw   t2, 0x48(t0)           /* TIMG_WDTCONFIG0 */
    sw   zero, 0x64(t0)
    li   t0, TIMG1
    sw   t1, 0x64(t0)
    sw   t2, 0x48(t0)
    sw   zero, 0x64(t0)

    /* Banner from DROM. */
    la   a0, banner
    li   a1, UART0
3:  lbu  t0, 0(a0)
    beqz t0, 4f
    sw   t0, 0(a1)
    addi a0, a0, 1
    j    3b
4:
    /* Sum the DROM table. */
    la   a0, table
    li   a2, 16
    li   a3, 0
5:  lw   t0, 0(a0)
    add  a3, a3, t0
    addi a0, a0, 4
    addi a2, a2, -1
    bnez a2, 5b
    la   t0, drom_sum
    sw   a3, 0(t0)

    /* GPIO8: IO MUX function 1 (GPIO matrix), drive 2; OUT_SEL 128. */
    li   t1, IO_MUX + 4 + 4 * PIN
    li   t2, (1 << 12) | (2 << 10)
    sw   t2, 0(t1)
    li   s0, GPIO
    li   t1, 128
    sw   t1, 0x554 + 4 * PIN(s0)
    li   s2, 1 << PIN
    sw   s2, 0x24(s0)           /* GPIO_ENABLE_W1TS */

    call systimer_now
    mv   s1, a0                 /* next deadline */
    li   s3, HALF_PERIOD
    la   s4, toggles

loop:
    sw   s2, 0x08(s0)           /* GPIO_OUT_W1TS */
    lw   t0, 0(s4)
    addi t0, t0, 1
    sw   t0, 0(s4)
    add  s1, s1, s3
    call wait_until
    sw   s2, 0x0c(s0)           /* GPIO_OUT_W1TC */
    add  s1, s1, s3
    call wait_until
    j    loop

/* a0 = SYSTIMER unit 0 value, low 32 bits. Clobbers t5, t6. */
systimer_now:
    li   t5, SYSTIMER
    li   t6, 1 << 30
    sw   t6, 4(t5)              /* UNIT0_OP.UPDATE */
6:  lw   a0, 4(t5)
    slli a0, a0, 2              /* VALUE_VALID (bit 29) -> sign bit */
    bgez a0, 6b
    lw   a0, 0x44(t5)           /* UNIT0_VALUE_LO */
    ret

/* Spin until SYSTIMER reaches s1 (wrap-safe). */
wait_until:
    mv   s5, ra
7:  call systimer_now
    sub  a0, a0, s1
    bltz a0, 7b
    mv   ra, s5
    ret

    .section .rodata
    .globl banner
banner:
    .asciz "ohmforge xip blinky\n"
    .balign 4
    .globl table
table:
    .word 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 0x10000

    .section .bss
    .globl toggles
toggles:
    .word 0
    .globl drom_sum
drom_sum:
    .word 0
