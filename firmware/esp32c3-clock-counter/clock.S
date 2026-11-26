/* SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Ohmforge test firmware: ESP32-C3 clock counter, bare metal, no ROM calls.
 *
 * Waits 2 ms for the rail, prints a banner on UART0, switches the watchdogs off (as
 * esp32c3-timer-irq does), then counts rising edges of the clock on
 * CLK_PIN with a GPIO interrupt (IO MUX input enabled, no pulls; GPIO_PINn
 * rising edge -> interrupt matrix source 16 -> CPU interrupt 1). The
 * handler timestamps each edge with SYSTIMER (16 MHz). A gate opens on an
 * edge and closes on the first edge at least GATE ticks later, so it spans
 * whole periods:
 *
 *     period = ticks / periods,  f (centihertz) = 16 MHz * 100 / period
 *
 * The main loop sleeps in `wfi`; after each gate it prints
 * `freq: <Hz>.<cHz> Hz (<n> periods)` on UART0, stores the result in
 * `freq_chz` and counts `gates` (.bss), and toggles the LED on LED_PIN.
 * The next gate opens on the edge that closed this one.
 */

    .equ UART0,      0x60000000
    .equ GPIO,       0x60004000
    .equ RTC_CNTL,   0x60008000
    .equ IO_MUX,     0x60009000
    .equ TIMG0,      0x6001f000
    .equ TIMG1,      0x60020000
    .equ SYSTIMER,   0x60023000
    .equ INTMATRIX,  0x600c2000
    .equ WDT_WKEY,   0x50d83aa1
    .equ SWD_WKEY,   0x8f1d312a
    .equ CLK_PIN,    4
    .equ LED_PIN,    8
    .equ GATE,       1600000        /* 100 ms */
    .equ CHZ_TICKS,  1600000000     /* 16 MHz * 100 */

    .section .text.entry, "ax"
    .globl _start
_start:
    li   sp, 0x3fcdfff0
    la   t0, vectors
    ori  t0, t0, 1              /* vectored mode */
    csrw mtvec, t0
    /* Let the 3.3 V rail and the idle-high TX line settle (the LDO comes up
     * from a cold start) before the banner: 2 ms. */
    call systimer_now
    li   t0, 32000
    add  s1, a0, t0
0:  call systimer_now
    sub  a0, a0, s1
    bltz a0, 0b
    la   a0, banner
    call puts

    /* Watchdogs off. RWDT: unlock, clear EN and FLASHBOOT_MOD_EN, lock. */
    li   t0, RTC_CNTL
    li   t1, WDT_WKEY
    sw   t1, 0xa8(t0)           /* RTC_CNTL_WDTWPROTECT */
    sw   zero, 0x90(t0)         /* RTC_CNTL_WDTCONFIG0 */
    sw   zero, 0xa8(t0)
    /* Super watchdog: unlock, auto-feed, lock. */
    li   t1, SWD_WKEY
    sw   t1, 0xb0(t0)           /* RTC_CNTL_SWD_WPROTECT */
    lw   t2, 0xac(t0)           /* RTC_CNTL_SWD_CONF */
    li   t3, 1 << 31            /* SWD_AUTO_FEED_EN */
    or   t2, t2, t3
    sw   t2, 0xac(t0)
    sw   zero, 0xb0(t0)
    /* MWDT0 and MWDT1 off. */
    li   t1, WDT_WKEY
    li   t2, 1 << 22
    li   t0, TIMG0
    sw   t1, 0x64(t0)           /* TIMG_WDTWPROTECT */
    sw   t2, 0x48(t0)           /* TIMG_WDTCONFIG0 */
    sw   zero, 0x64(t0)
    li   t0, TIMG1
    sw   t1, 0x64(t0)
    sw   t2, 0x48(t0)
    sw   zero, 0x64(t0)

    /* LED_PIN: function 1, drive strength 2; GPIO matrix OUT_SEL 128. */
    li   t1, IO_MUX + 4 + 4 * LED_PIN
    li   t2, (1 << 12) | (2 << 10)
    sw   t2, 0(t1)
    li   s0, GPIO
    li   t1, 128
    sw   t1, 0x554 + 4 * LED_PIN(s0)
    li   s9, 1 << LED_PIN
    sw   s9, 0x24(s0)           /* GPIO_ENABLE_W1TS */

    /* CLK_PIN: function 1, FUN_IE, no pulls; rising-edge interrupt to the CPU. */
    li   t1, IO_MUX + 4 + 4 * CLK_PIN
    li   t2, (1 << 12) | (1 << 9)
    sw   t2, 0(t1)
    li   t1, (1 << 7) | (1 << 13)   /* INT_TYPE = rising, INT_ENA = CPU */
    sw   t1, 0x74 + 4 * CLK_PIN(s0)
    li   t1, 1 << CLK_PIN
    sw   t1, 0x4c(s0)           /* GPIO_STATUS_W1TC */

    /* Interrupt matrix: GPIO_PCPU_INT (source 16) -> CPU interrupt 1. */
    li   t0, INTMATRIX
    li   t1, 1
    sw   t1, 4 * 16(t0)         /* GPIO_INTERRUPT_PRO_MAP */
    sw   t1, 0x114 + 4 * 1(t0)  /* CPU_INT_PRI_1 = 1 */
    sw   t1, 0x194(t0)          /* CPU_INT_THRESH = 1 */
    li   t1, 1 << 1
    sw   t1, 0x104(t0)          /* CPU_INT_ENABLE */
    li   s10, 0                 /* LED state */
    csrsi mstatus, 8            /* MIE */

idle:
    wfi
    la   t0, ready
    lw   t1, 0(t0)
    beqz t1, idle
    csrci mstatus, 8
    sw   zero, 0(t0)
    lw   s4, mb_ticks
    lw   s3, mb_periods
    csrsi mstatus, 8

    divu s5, s4, s3             /* ticks per period */
    li   t0, CHZ_TICKS
    divu s6, t0, s5             /* centihertz */
    la   t0, freq_chz
    sw   s6, 0(t0)
    la   t0, gates
    lw   t1, 0(t0)
    addi t1, t1, 1
    sw   t1, 0(t0)

    la   a0, msg_freq
    call puts
    li   t0, 100
    divu a0, s6, t0
    call putu
    li   a0, '.'
    call putc
    li   t0, 100
    remu t1, s6, t0
    li   t0, 10
    divu a0, t1, t0
    addi a0, a0, '0'
    call putc
    li   t0, 10
    remu a0, t1, t0
    addi a0, a0, '0'
    call putc
    la   a0, msg_hz
    call puts
    mv   a0, s3
    call putu
    la   a0, msg_periods
    call puts

    /* Heartbeat: toggle the LED once per gate. */
    xori s10, s10, 1
    beqz s10, 5f
    sw   s9, 0x08(s0)           /* GPIO_OUT_W1TS */
    j    idle
5:  sw   s9, 0x0c(s0)           /* GPIO_OUT_W1TC */
    j    idle

/* CPU interrupt 1: a rising edge on CLK_PIN. */
gpio_isr:
    addi sp, sp, -32
    sw   ra, 0(sp)
    sw   t0, 4(sp)
    sw   t1, 8(sp)
    sw   t2, 12(sp)
    sw   t5, 16(sp)
    sw   t6, 20(sp)
    sw   a0, 24(sp)
    li   t0, GPIO
    li   t1, 1 << CLK_PIN
    sw   t1, 0x4c(t0)           /* GPIO_STATUS_W1TC */
    call systimer_now
    la   t0, started
    lw   t1, 0(t0)
    bnez t1, 1f
    li   t1, 1
    sw   t1, 0(t0)
    j    2f                     /* the first edge opens the first gate */
1:  la   t0, gate_periods
    lw   t1, 0(t0)
    addi t1, t1, 1
    sw   t1, 0(t0)
    lw   t2, gate_start
    sub  t2, a0, t2             /* ticks since the gate opened */
    li   t0, GATE
    bltu t2, t0, 3f
    la   t0, mb_ticks
    sw   t2, 0(t0)
    la   t0, mb_periods
    sw   t1, 0(t0)
    la   t0, ready
    li   t1, 1
    sw   t1, 0(t0)
2:  la   t0, gate_start
    sw   a0, 0(t0)
    la   t0, gate_periods
    sw   zero, 0(t0)
3:  lw   ra, 0(sp)
    lw   t0, 4(sp)
    lw   t1, 8(sp)
    lw   t2, 12(sp)
    lw   t5, 16(sp)
    lw   t6, 20(sp)
    lw   a0, 24(sp)
    addi sp, sp, 32
    mret

unexpected:
    csrr t0, mcause
    la   t1, fault
    sw   t0, 0(t1)
4:  j    4b

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

/* Write the byte a0 to UART0. Clobbers t6. */
putc:
    li   t6, UART0
    sw   a0, 0(t6)
    ret

/* Write the NUL-terminated string at a0. Clobbers a0, t0, t6. */
puts:
    li   t6, UART0
7:  lbu  t0, 0(a0)
    beqz t0, 8f
    sw   t0, 0(t6)
    addi a0, a0, 1
    j    7b
8:  ret

/* Write a0 in decimal. Clobbers a0, t2, t3, t4, t6. */
putu:
    addi sp, sp, -16
    addi t3, sp, 12
    li   t4, 10
9:  remu t2, a0, t4
    addi t2, t2, '0'
    addi t3, t3, -1
    sb   t2, 0(t3)
    divu a0, a0, t4
    bnez a0, 9b
    li   t6, UART0
    addi t4, sp, 12
10: lbu  t2, 0(t3)
    sw   t2, 0(t6)
    addi t3, t3, 1
    bltu t3, t4, 10b
    addi sp, sp, 16
    ret

/* Vector table: entry n handles mcause n (entry 0 takes exceptions). The
 * C3 needs mtvec 256-byte aligned; entries must be 4-byte jumps. */
    .balign 256
    .option push
    .option norvc
vectors:
    j    unexpected
    j    gpio_isr
    .rept 30
    j    unexpected
    .endr
    .option pop

    .section .rodata
banner:
    .asciz "ohmforge clock counter\n"
msg_freq:
    .asciz "freq: "
msg_hz:
    .asciz " Hz ("
msg_periods:
    .asciz " periods)\n"

    .section .bss
    .globl freq_chz, gates, fault
freq_chz:
    .word 0
gates:
    .word 0
fault:
    .word 0
started:
    .word 0
ready:
    .word 0
gate_start:
    .word 0
gate_periods:
    .word 0
mb_ticks:
    .word 0
mb_periods:
    .word 0
