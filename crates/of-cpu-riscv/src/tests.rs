//! Behavioural tests: one or more per instruction, plus small programs.
#![allow(clippy::unusual_byte_groupings)]

use of_core::SimTime;

use crate::asm::*;
use crate::bus::{AccessKind, Bus, BusError, Ram, Size};
use crate::csr::addr::*;
use crate::hart::*;
use crate::timing::TimingModel;
use crate::trap::{Exception, TrapCause};

const BASE: u32 = 0x8000_0000;
const DATA: u32 = BASE + 0x1000;
const HANDLER: u32 = BASE + 0x800;

fn cfg() -> HartConfig {
    HartConfig {
        ebreak: EbreakAction::Halt,
        ..HartConfig::generic_imac()
    }
}

fn setup(config: HartConfig, prog: &[u32]) -> (Hart, Ram) {
    let mut ram = Ram::new(BASE, 0x10000);
    ram.load_words(BASE, prog).unwrap();
    (Hart::new(config), ram)
}

/// Run a program that ends in `ebreak` (halt) and return the hart.
fn exec_with(config: HartConfig, prog: &[u32]) -> (Hart, Ram) {
    let (mut h, mut ram) = setup(config, prog);
    let exit = h.run(&mut ram, 100_000);
    assert!(
        matches!(exit, RunExit::Halted(HaltReason::Ebreak { .. })),
        "program did not halt: {exit:?} at {:#x}",
        h.pc()
    );
    (h, ram)
}

fn exec(prog: &[u32]) -> Hart {
    exec_with(cfg(), prog).0
}

/// Run `op rd=x3, x1=a, x2=b` and return x3.
fn rr(op: fn(u8, u8, u8) -> u32, a: u32, b: u32) -> u32 {
    let [l1, a1] = li(1, a);
    let [l2, a2] = li(2, b);
    exec(&[l1, a1, l2, a2, op(3, 1, 2), EBREAK]).reg(3)
}

fn ri(op: fn(u8, u8, i32) -> u32, a: u32, imm: i32) -> u32 {
    let [l1, a1] = li(1, a);
    exec(&[l1, a1, op(3, 1, imm), EBREAK]).reg(3)
}

fn shift(op: fn(u8, u8, u32) -> u32, a: u32, s: u32) -> u32 {
    let [l1, a1] = li(1, a);
    exec(&[l1, a1, op(3, 1, s), EBREAK]).reg(3)
}

// ----- RV32I ------------------------------------------------------------

#[test]
fn li_builds_any_constant() {
    for v in [
        0,
        1,
        0x7ff,
        0x800,
        0xfff,
        0x1234_5678,
        0x8000_0000,
        0xffff_f800,
        0xffff_ffff,
        0xdead_beef,
    ] {
        let [a, b] = li(5, v);
        assert_eq!(exec(&[a, b, EBREAK]).reg(5), v, "li {v:#x}");
    }
}

#[test]
fn lui_and_auipc() {
    let h = exec(&[lui(1, 0xabcde000), auipc(2, 0x1000), auipc(3, 0xfffff000), EBREAK]);
    assert_eq!(h.reg(1), 0xabcd_e000);
    assert_eq!(h.reg(2), BASE + 4 + 0x1000);
    assert_eq!(h.reg(3), (BASE + 8).wrapping_sub(0x1000));
}

#[test]
fn add_sub_wrap() {
    assert_eq!(rr(add, 0xffff_ffff, 1), 0);
    assert_eq!(rr(add, 0x7fff_ffff, 1), 0x8000_0000);
    assert_eq!(rr(sub, 0, 1), 0xffff_ffff);
    assert_eq!(rr(sub, 0x8000_0000, 1), 0x7fff_ffff);
    assert_eq!(ri(addi, 5, -6), 0xffff_ffff);
    assert_eq!(ri(addi, 0, 2047), 2047);
    assert_eq!(ri(addi, 0, -2048), (-2048i32) as u32);
}

#[test]
fn comparisons_signed_and_unsigned() {
    assert_eq!(rr(slt, 0xffff_ffff, 0), 1);
    assert_eq!(rr(sltu, 0xffff_ffff, 0), 0);
    assert_eq!(rr(slt, 0, 0), 0);
    assert_eq!(rr(sltu, 0, 1), 1);
    assert_eq!(ri(slti, 0x8000_0000, 0), 1);
    assert_eq!(ri(sltiu, 0, -1), 1); // imm sign-extends to 0xffffffff, compared unsigned
    assert_eq!(ri(sltiu, 1, 1), 0);
}

#[test]
fn logic_ops() {
    assert_eq!(rr(xor, 0xff00_ff00, 0x0ff0_0ff0), 0xf0f0_f0f0);
    assert_eq!(rr(or, 0xff00_0000, 0x0000_00ff), 0xff00_00ff);
    assert_eq!(rr(and, 0xff00_ff00, 0x0ff0_0ff0), 0x0f00_0f00);
    assert_eq!(ri(xori, 0x1234, -1), !0x1234);
    assert_eq!(ri(ori, 0x1000, 0x0ff), 0x10ff);
    assert_eq!(ri(andi, 0xffff_ffff, -16), 0xffff_fff0);
}

#[test]
fn shifts() {
    assert_eq!(shift(slli, 1, 31), 0x8000_0000);
    assert_eq!(shift(srli, 0x8000_0000, 31), 1);
    assert_eq!(shift(srai, 0x8000_0000, 31), 0xffff_ffff);
    assert_eq!(shift(srai, 0x4000_0000, 30), 1);
    // Register shifts use only the low five bits of rs2.
    assert_eq!(rr(sll, 1, 33), 2);
    assert_eq!(rr(srl, 0x8000_0000, 0xffff_ffe1), 0x4000_0000);
    assert_eq!(rr(sra, 0x8000_0000, 4), 0xf800_0000);
}

#[test]
fn x0_is_hardwired() {
    let h = exec(&[addi(0, 0, 5), lui(0, 0x1000), add(1, 0, 0), EBREAK]);
    assert_eq!(h.reg(0), 0);
    assert_eq!(h.reg(1), 0);
}

/// A branch encoder (`rs1, rs2, offset`).
type BranchFn = fn(u8, u8, i32) -> u32;

#[test]
fn branches_taken_and_not() {
    let cases: [(BranchFn, u32, u32, bool); 12] = [
        (beq, 5, 5, true),
        (beq, 5, 6, false),
        (bne, 5, 6, true),
        (bne, 5, 5, false),
        (blt, 0xffff_ffff, 0, true),
        (blt, 0, 0xffff_ffff, false),
        (bge, 0, 0xffff_ffff, true),
        (bge, 0xffff_ffff, 0, false),
        (bltu, 0, 0xffff_ffff, true),
        (bltu, 0xffff_ffff, 0, false),
        (bgeu, 0xffff_ffff, 0, true),
        (bgeu, 3, 3, true),
    ];
    for (op, a, b, taken) in cases {
        let [l1, a1] = li(1, a);
        let [l2, a2] = li(2, b);
        // Taken skips the "x3 = 1" instruction.
        let h = exec(&[l1, a1, l2, a2, op(1, 2, 8), addi(3, 0, 1), EBREAK]);
        assert_eq!(h.reg(3) == 0, taken, "branch {a:#x} vs {b:#x}");
    }
}

#[test]
fn backward_branch_loop() {
    // x1 = 10; do { x2 += x1; x1 -= 1 } while x1 != 0
    let h = exec(&[addi(1, 0, 10), add(2, 2, 1), addi(1, 1, -1), bne(1, 0, -8), EBREAK]);
    assert_eq!(h.reg(2), 55);
}

#[test]
fn jal_and_jalr_link() {
    let h = exec(&[
        jal(1, 12),    // 0: call 12
        addi(5, 0, 7), // 4: after return
        EBREAK,        // 8
        addi(6, 0, 9), // 12: callee
        jalr(0, 1, 0), // 16: ret
    ]);
    assert_eq!(h.reg(1), BASE + 4);
    assert_eq!((h.reg(5), h.reg(6)), (7, 9));
}

#[test]
fn jalr_clears_bit_zero_and_links_before_jump() {
    // rd == rs1: the target uses the old value.
    let [l, a] = li(1, BASE + 13); // odd: bit 0 is cleared, lands on BASE+12
    let h = exec(&[l, a, jalr(1, 1, 0), EBREAK, EBREAK]);
    assert_eq!(h.pc(), BASE + 12);
    assert_eq!(h.reg(1), BASE + 12);
}

#[test]
fn loads_sign_and_zero_extend() {
    let [l, a] = li(1, DATA);
    let prog = [
        l,
        a,
        lb(2, 1, 0),
        lbu(3, 1, 0),
        lh(4, 1, 0),
        lhu(5, 1, 0),
        lw(6, 1, 0),
        lb(7, 1, 3),
        lhu(8, 1, 2),
        EBREAK,
    ];
    let (mut h, mut ram) = setup(cfg(), &prog);
    ram.load_words(DATA, &[0x1234_80f0]).unwrap();
    h.run(&mut ram, 1000);
    assert_eq!(h.reg(2), 0xffff_fff0);
    assert_eq!(h.reg(3), 0xf0);
    assert_eq!(h.reg(4), 0xffff_80f0);
    assert_eq!(h.reg(5), 0x80f0);
    assert_eq!(h.reg(6), 0x1234_80f0);
    assert_eq!(h.reg(7), 0x12);
    assert_eq!(h.reg(8), 0x1234);
}

#[test]
fn stores_write_only_their_width() {
    let [l, a] = li(1, DATA);
    let [l2, a2] = li(2, 0xaabb_ccdd);
    let (_, ram) = exec_with(
        cfg(),
        &[
            l,
            a,
            l2,
            a2,
            sw(0, 1, 0),
            sb(2, 1, 1),
            sh(2, 1, 6),
            sw(2, 1, 8),
            sw(2, 1, -4),
            EBREAK,
        ],
    );
    assert_eq!(ram.peek(DATA, Size::Word), Some(0x0000_dd00));
    assert_eq!(ram.peek(DATA + 4, Size::Word), Some(0xccdd_0000));
    assert_eq!(ram.peek(DATA + 8, Size::Word), Some(0xaabb_ccdd));
    assert_eq!(ram.peek(DATA - 4, Size::Word), Some(0xaabb_ccdd));
}

#[test]
fn fences_are_harmless() {
    let h = exec(&[FENCE, FENCE_I, addi(1, 0, 1), EBREAK]);
    assert_eq!(h.reg(1), 1);
}

// ----- M extension ------------------------------------------------------------

#[test]
fn multiply_family() {
    assert_eq!(rr(mul, 0xffff_ffff, 0xffff_ffff), 1);
    assert_eq!(rr(mul, 0x1234_5678, 0x10), 0x2345_6780);
    assert_eq!(rr(mulh, 0xffff_ffff, 0xffff_ffff), 0); // (-1)(-1) = 1
    assert_eq!(rr(mulh, 0x8000_0000, 0x8000_0000), 0x4000_0000);
    assert_eq!(rr(mulhu, 0xffff_ffff, 0xffff_ffff), 0xffff_fffe);
    assert_eq!(rr(mulhsu, 0xffff_ffff, 0xffff_ffff), 0xffff_ffff); // -1 * (2^32-1)
    assert_eq!(rr(mulhsu, 2, 0x8000_0000), 1);
}

#[test]
fn divide_family_including_edge_cases() {
    assert_eq!(rr(div, (-7i32) as u32, 2), (-3i32) as u32); // rounds toward zero
    assert_eq!(rr(rem, (-7i32) as u32, 2), (-1i32) as u32);
    assert_eq!(rr(divu, 7, 2), 3);
    assert_eq!(rr(remu, 7, 2), 1);
    // Division by zero.
    assert_eq!(rr(div, 5, 0), 0xffff_ffff);
    assert_eq!(rr(divu, 5, 0), 0xffff_ffff);
    assert_eq!(rr(rem, 5, 0), 5);
    assert_eq!(rr(remu, 5, 0), 5);
    // Signed overflow.
    assert_eq!(rr(div, 0x8000_0000, 0xffff_ffff), 0x8000_0000);
    assert_eq!(rr(rem, 0x8000_0000, 0xffff_ffff), 0);
}

#[test]
fn m_disabled_is_illegal() {
    let mut c = cfg();
    c.extensions.m = false;
    let (mut h, mut ram) = setup(c, &[mul(1, 2, 3)]);
    let out = h.step(&mut ram);
    assert!(
        matches!(out, StepOutcome::Trapped { cause: TrapCause::Exception(Exception::IllegalInstruction, raw), .. } if raw == mul(1, 2, 3))
    );
}

// ----- A extension ------------------------------------------------------------

fn amo(op: fn(u8, u8, u8) -> u32, mem: u32, src: u32) -> (u32, u32) {
    let [l, a] = li(1, DATA);
    let [l2, a2] = li(2, src);
    let (mut h, mut ram) = setup(cfg(), &[l, a, l2, a2, op(3, 1, 2), EBREAK]);
    ram.load_words(DATA, &[mem]).unwrap();
    h.run(&mut ram, 1000);
    (h.reg(3), ram.peek(DATA, Size::Word).unwrap())
}

#[test]
fn amo_read_modify_write() {
    assert_eq!(amo(amoswap_w, 1, 2), (1, 2));
    assert_eq!(amo(amoadd_w, 1, 2), (1, 3));
    assert_eq!(amo(amoxor_w, 0b1100, 0b1010), (0b1100, 0b0110));
    assert_eq!(amo(amoand_w, 0b1100, 0b1010), (0b1100, 0b1000));
    assert_eq!(amo(amoor_w, 0b1100, 0b1010), (0b1100, 0b1110));
    assert_eq!(amo(amomin_w, 0xffff_ffff, 1), (0xffff_ffff, 0xffff_ffff));
    assert_eq!(amo(amomax_w, 0xffff_ffff, 1), (0xffff_ffff, 1));
    assert_eq!(amo(amominu_w, 0xffff_ffff, 1), (0xffff_ffff, 1));
    assert_eq!(amo(amomaxu_w, 0xffff_ffff, 1), (0xffff_ffff, 0xffff_ffff));
}

#[test]
fn lr_sc_success_and_failure() {
    let [l, a] = li(1, DATA);
    let prog = [
        l,
        a,
        addi(2, 0, 42),
        lr_w(3, 1),
        sc_w(4, 1, 2), // succeeds: x4 = 0
        sc_w(5, 1, 2), // reservation consumed: x5 = 1
        lr_w(3, 1),
        sw(0, 1, 0), // a store to the reserved word breaks the reservation
        sc_w(6, 1, 2),
        EBREAK,
    ];
    let (h, ram) = exec_with(cfg(), &prog);
    assert_eq!((h.reg(4), h.reg(5), h.reg(6)), (0, 1, 1));
    assert_eq!(ram.peek(DATA, Size::Word), Some(0));
}

#[test]
fn amo_misaligned_and_disabled() {
    let [l, a] = li(1, DATA + 2);
    let (mut h, mut ram) = setup(cfg(), &[l, a, amoadd_w(3, 1, 2)]);
    h.step(&mut ram);
    h.step(&mut ram);
    assert!(matches!(
        h.step(&mut ram),
        StepOutcome::Trapped { cause: TrapCause::Exception(Exception::StoreAddressMisaligned, v), .. } if v == DATA + 2
    ));
    let (mut h, mut ram) = setup(cfg(), &[l, a, lr_w(3, 1)]);
    h.step(&mut ram);
    h.step(&mut ram);
    assert!(matches!(
        h.step(&mut ram),
        StepOutcome::Trapped {
            cause: TrapCause::Exception(Exception::LoadAddressMisaligned, _),
            ..
        }
    ));
    // The ESP32-C3 has no A extension.
    let (mut h, mut ram) = setup(
        HartConfig {
            reset_pc: BASE,
            ..HartConfig::esp32c3()
        },
        &[amoadd_w(3, 1, 2)],
    );
    assert!(matches!(
        h.step(&mut ram),
        StepOutcome::Trapped {
            cause: TrapCause::Exception(Exception::IllegalInstruction, _),
            ..
        }
    ));
}

// ----- Zicsr ------------------------------------------------------------------

#[test]
fn csr_read_write_set_clear() {
    let h = exec(&[
        addi(1, 0, 0x55),
        csrrw(2, MSCRATCH, 1), // x2 = 0, mscratch = 0x55
        addi(1, 0, 0x0f),
        csrrs(3, MSCRATCH, 1), // x3 = 0x55, mscratch = 0x5f
        addi(1, 0, 0x50),
        csrrc(4, MSCRATCH, 1),   // x4 = 0x5f, mscratch = 0x0f
        csrrwi(5, MSCRATCH, 31), // x5 = 0x0f, mscratch = 31
        csrrsi(6, MSCRATCH, 0),  // read only
        csrrci(7, MSCRATCH, 1),  // x7 = 31, mscratch = 30
        csrrs(8, MSCRATCH, 0),
        EBREAK,
    ]);
    assert_eq!(
        (h.reg(2), h.reg(3), h.reg(4), h.reg(5), h.reg(6), h.reg(7), h.reg(8)),
        (0, 0x55, 0x5f, 0x0f, 31, 31, 30)
    );
}

#[test]
fn identification_csrs() {
    let h = exec(&[csrrs(1, MISA, 0), csrrs(2, MHARTID, 0), csrrs(3, MVENDORID, 0), EBREAK]);
    assert_eq!(h.reg(1), 0x4000_1105); // RV32 + A, C, I, M
    assert_eq!((h.reg(2), h.reg(3)), (0, 0));
    let c3 = crate::hart::Extensions {
        m: true,
        a: false,
        c: true,
    }
    .misa();
    assert_eq!(c3, 0x4000_1104);
}

#[test]
fn csr_illegal_accesses_trap() {
    for instr in [
        csrrw(1, MHARTID, 2), // write to read-only
        csrrsi(1, CYCLE, 1),  // write to user read-only shadow
        csrrs(1, 0x7c0, 0),   // unknown CSR with a bus that knows nothing
    ] {
        let (mut h, mut ram) = setup(cfg(), &[instr]);
        assert!(
            matches!(h.step(&mut ram), StepOutcome::Trapped { cause: TrapCause::Exception(Exception::IllegalInstruction, raw), .. } if raw == instr),
            "{instr:#x}"
        );
    }
    // csrrs with rs1 = x0 never writes, so reading a read-only CSR is fine.
    let h = exec(&[csrrs(1, MHARTID, 0), csrrsi(2, CYCLE, 0), EBREAK]);
    assert_eq!(h.reg(1), 0);
}

#[test]
fn counters_count_cycles_and_instructions() {
    let h = exec(&[
        NOP,
        NOP,
        csrrs(1, MINSTRET, 0),
        csrrs(2, MCYCLE, 0),
        csrrs(3, INSTRET, 0),
        EBREAK,
    ]);
    assert_eq!(h.reg(1), 2);
    assert_eq!(h.reg(2), 3); // unit timing
    assert_eq!(h.reg(3), 4);
    assert_eq!(h.elapsed_cycles(), 6);
}

#[test]
fn counter_writes_and_inhibit() {
    let h = exec(&[
        addi(1, 0, 100),
        csrrw(0, MINSTRET, 1), // this instruction does not also increment
        csrrs(2, MINSTRET, 0), // 100
        addi(1, 0, 5),
        csrrw(0, MCOUNTINHIBIT, 1), // inhibit cycle (bit 0) and instret (bit 2)
        NOP,
        NOP,
        csrrs(3, MINSTRET, 0),
        csrrs(4, MCYCLE, 0),
        csrrs(5, MCYCLE, 0),
        EBREAK,
    ]);
    assert_eq!(h.reg(2), 100);
    assert_eq!(h.reg(3), h.csrs().minstret as u32);
    assert_eq!(h.reg(4), h.reg(5));
    assert_eq!(h.elapsed_cycles(), 11); // the monotonic clock never stops
}

struct CustomCsrBus {
    ram: Ram,
    custom: u32,
    cycles: Vec<u64>,
}

impl Bus for CustomCsrBus {
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError> {
        self.ram.read(addr, size, kind)
    }
    fn write(&mut self, addr: u32, size: Size, value: u32, kind: AccessKind) -> Result<(), BusError> {
        self.ram.write(addr, size, value, kind)
    }
    fn csr_cycles(&mut self, cycles: u64) {
        self.cycles.push(cycles);
    }
    fn csr_read(&mut self, csr: u16) -> Option<u32> {
        (csr == 0x7e0).then_some(self.custom)
    }
    fn csr_write(&mut self, csr: u16, value: u32) -> bool {
        if csr == 0x7e0 {
            self.custom = value;
        }
        csr == 0x7e0
    }
}

#[test]
fn unknown_csrs_go_to_the_bus() {
    let mut ram = Ram::new(BASE, 0x1000);
    ram.load_words(BASE, &[addi(1, 0, 3), csrrw(2, 0x7e0, 1), csrrsi(3, 0x7e0, 4), EBREAK])
        .unwrap();
    let mut bus = CustomCsrBus {
        ram,
        custom: 0x10,
        cycles: Vec::new(),
    };
    let mut h = Hart::new(cfg());
    h.run(&mut bus, 100);
    assert_eq!((h.reg(2), h.reg(3), bus.custom), (0x10, 3, 7));
    // The bus sees the cycle count before each custom CSR access.
    assert_eq!(bus.cycles.len(), 2);
    assert!(bus.cycles[0] >= 1 && bus.cycles[1] > bus.cycles[0], "{:?}", bus.cycles);
}

// ----- traps --------------------------------------------------------------------

fn trap_setup(prog: &[u32]) -> (Hart, Ram) {
    let (mut h, mut ram) = setup(cfg(), prog);
    h.csr_poke(MTVEC, HANDLER);
    // Handler: x31 = mcause, x30 = mtval, then halt.
    ram.load_words(HANDLER, &[csrrs(31, MCAUSE, 0), csrrs(30, MTVAL, 0), EBREAK])
        .unwrap();
    (h, ram)
}

fn trap_of(prog: &[u32]) -> (u32, u32, u32) {
    let (mut h, mut ram) = trap_setup(prog);
    h.run(&mut ram, 1000);
    (h.reg(31), h.reg(30), h.csrs().mepc)
}

#[test]
fn ecall_and_ebreak_traps() {
    assert_eq!(trap_of(&[NOP, ECALL]), (11, 0, BASE + 4));
    let mut c = cfg();
    c.ebreak = EbreakAction::Trap;
    let (mut h, mut ram) = setup(c, &[NOP, EBREAK]);
    h.csr_poke(MTVEC, HANDLER);
    ram.load_words(HANDLER, &[csrrs(31, MCAUSE, 0), csrrs(30, MTVAL, 0), jal(0, 0)])
        .unwrap();
    h.run(&mut ram, 20);
    assert_eq!((h.reg(31), h.reg(30), h.csrs().mepc), (3, BASE + 4, BASE + 4));
}

#[test]
fn illegal_instruction_reports_encoding() {
    assert_eq!(trap_of(&[0xffff_ffff]), (2, 0xffff_ffff, BASE));
    assert_eq!(trap_of(&[0]), (2, 0, BASE)); // all-zero parcel: illegal compressed
}

#[test]
fn misaligned_and_faulting_accesses() {
    let [l, a] = li(1, DATA + 1);
    assert_eq!(trap_of(&[l, a, lw(2, 1, 0)]), (4, DATA + 1, BASE + 8));
    assert_eq!(trap_of(&[l, a, sh(2, 1, 0)]), (6, DATA + 1, BASE + 8));
    let [l, a] = li(1, 0x1000);
    assert_eq!(trap_of(&[l, a, lw(2, 1, 0)]), (5, 0x1000, BASE + 8));
    assert_eq!(trap_of(&[l, a, sw(2, 1, 0)]), (7, 0x1000, BASE + 8));
    assert_eq!(trap_of(&[l, a, jalr(0, 1, 0)]), (1, 0x1000, 0x1000));
}

#[test]
fn esp32c3_performs_misaligned_accesses() {
    assert_eq!(HartConfig::esp32c3().misaligned, MisalignedPolicy::Allow);
    assert_eq!(HartConfig::generic_imac().misaligned, MisalignedPolicy::Trap);
}

#[test]
fn misaligned_allowed_policy_splits_accesses() {
    let mut c = cfg();
    c.misaligned = MisalignedPolicy::Allow;
    let [l, a] = li(1, DATA + 1);
    let [l2, a2] = li(2, 0x1122_3344);
    let (h, ram) = exec_with(c, &[l, a, l2, a2, sw(2, 1, 0), lw(3, 1, 0), lhu(4, 1, 1), EBREAK]);
    assert_eq!(h.reg(3), 0x1122_3344);
    assert_eq!(h.reg(4), 0x2233);
    assert_eq!(ram.peek(DATA, Size::Word), Some(0x2233_4400));
}

#[test]
fn misaligned_jump_without_c() {
    let mut c = cfg();
    c.extensions.c = false;
    let (mut h, mut ram) = setup(c, &[jal(0, 6)]);
    h.csr_poke(MTVEC, HANDLER);
    let out = h.step(&mut ram);
    assert!(
        matches!(out, StepOutcome::Trapped { cause: TrapCause::Exception(Exception::InstructionAddressMisaligned, t), .. } if t == BASE + 6)
    );
    assert_eq!(h.csrs().mepc, BASE); // the jump itself faults
    assert_eq!(h.pc(), HANDLER);
}

#[test]
fn trap_entry_and_mret_manage_mstatus() {
    // Enable MIE, ecall; the handler bumps mepc and returns.
    let (mut h, mut ram) = setup(cfg(), &[csrrsi(0, MSTATUS, 8), ECALL, csrrs(5, MSTATUS, 0), EBREAK]);
    h.csr_poke(MTVEC, HANDLER);
    ram.load_words(
        HANDLER,
        &[
            csrrs(6, MSTATUS, 0),
            csrrs(7, MEPC, 0),
            addi(7, 7, 4),
            csrrw(0, MEPC, 7),
            MRET,
        ],
    )
    .unwrap();
    h.run(&mut ram, 1000);
    assert_eq!(h.reg(6) & 0x88, 0x80, "in handler: MIE=0, MPIE=1");
    assert_eq!(h.reg(5) & 0x88, 0x88, "after mret: MIE=1, MPIE=1");
    assert_eq!(h.reg(6) & (3 << 11), 3 << 11, "MPP reads as machine mode");
}

// ----- interrupts -----------------------------------------------------------------

#[test]
fn interrupt_taken_only_when_enabled() {
    let prog = [NOP, NOP, NOP, NOP, EBREAK];
    let (mut h, mut ram) = trap_setup(&prog);
    h.set_interrupt_line(7, true);
    h.step(&mut ram);
    assert_eq!(h.pc(), BASE + 4, "mie = 0: not taken");
    h.csr_poke(MIE, 1 << 7);
    h.step(&mut ram);
    assert_eq!(h.pc(), BASE + 8, "mstatus.MIE = 0: not taken");
    h.csr_poke(MSTATUS, 8);
    let out = h.step(&mut ram);
    assert_eq!(
        out,
        StepOutcome::Trapped {
            pc: BASE + 8,
            cause: TrapCause::Interrupt(7),
            cycles: 1
        }
    );
    assert_eq!(h.pc(), HANDLER);
    assert_eq!(h.csrs().mcause, 0x8000_0007);
    assert_eq!(h.csrs().mepc, BASE + 8, "mepc is the next unexecuted instruction");
    assert_eq!(h.csrs().mstatus & 0x88, 0x80);
}

#[test]
fn vectored_mode_and_priority() {
    let (mut h, mut ram) = setup(cfg(), &[NOP]);
    h.csr_poke(MTVEC, HANDLER | 1);
    h.csr_poke(MIE, 0xffff_ffff);
    h.csr_poke(MSTATUS, 8);
    h.set_interrupt_lines((1 << 7) | (1 << 3) | (1 << 16));
    h.step(&mut ram);
    assert_eq!(h.pc(), HANDLER + 4 * 3, "MSI beats MTI and platform lines");
    assert_eq!(h.csrs().mie, 0xffff_0888, "only implemented lines are writable");
    // Exceptions always go to the base, even in vectored mode.
    let (mut h, mut ram) = setup(cfg(), &[ECALL]);
    h.csr_poke(MTVEC, HANDLER | 1);
    h.step(&mut ram);
    assert_eq!(h.pc(), HANDLER);
}

#[test]
fn esp32c3_platform_lines() {
    let c3 = HartConfig {
        reset_pc: BASE,
        ..HartConfig::esp32c3()
    };
    let (mut h, mut ram) = setup(c3.clone(), &[NOP, NOP]);
    h.csr_poke(MTVEC, HANDLER | 1);
    // No mie on the C3: the interrupt controller gates lines, so only
    // mstatus.MIE matters to the core.
    assert!(!h.csr_poke(MIE, 0xffff_ffff));
    h.set_interrupt_line(20, true);
    h.step(&mut ram);
    assert_eq!(h.pc(), BASE + 4, "mstatus.MIE = 0: not taken");
    h.csr_poke(MSTATUS, 8);
    h.set_interrupt_line(0, true); // line 0 does not exist on the C3
    h.set_interrupt_line(20, false);
    h.step(&mut ram);
    assert_eq!(h.pc(), BASE + 8);
    h.set_interrupt_line(20, true);
    h.step(&mut ram);
    assert_eq!(h.pc(), HANDLER + 80);
    assert_eq!(h.csrs().mcause, 0x8000_0014);
    // csrr a0, mie / mip are illegal instructions on the C3.
    for csr in [MIE, MIP] {
        let (mut h, mut ram) = setup(c3.clone(), &[csrrs(10, csr, 0)]);
        assert!(matches!(
            h.step(&mut ram),
            StepOutcome::Trapped {
                cause: TrapCause::Exception(Exception::IllegalInstruction, _),
                ..
            }
        ));
    }
}

#[test]
fn wfi_sleeps_until_interrupt_pending() {
    let (mut h, mut ram) = trap_setup(&[WFI, addi(1, 0, 1), EBREAK]);
    h.csr_poke(MIE, 1 << 11);
    // Global MIE stays 0: wfi still wakes, execution continues after it.
    assert_eq!(h.run(&mut ram, 100), RunExit::WaitingForInterrupt);
    assert_eq!(h.state(), HartState::WaitingForInterrupt);
    assert_eq!(h.step(&mut ram), StepOutcome::Waiting);
    let before = h.local_time();
    h.idle_until(SimTime(before.0 + 1_000_000));
    assert!(h.local_time() >= SimTime(before.0 + 1_000_000));
    h.set_interrupt_line(11, true);
    assert!(matches!(h.run(&mut ram, 100), RunExit::Halted(_)));
    assert_eq!(h.reg(1), 1);
    assert_eq!(h.reg(31), 0, "no trap taken with MIE = 0");
}

// ----- timing, stepping and debugger API ---------------------------------------------

#[test]
fn esp32c3_clock_and_cycle_costs() {
    let mut c = HartConfig {
        reset_pc: BASE,
        ..HartConfig::esp32c3()
    };
    c.ebreak = EbreakAction::Halt;
    let (mut h, mut ram) = setup(c, &[addi(1, 0, 3), addi(1, 1, -1), bne(1, 0, -4), div(2, 1, 1), EBREAK]);
    assert_eq!(h.cycle_period_ps(), 6250);
    h.run(&mut ram, 1000);
    let t = TimingModel::ESP32C3_ESTIMATE;
    let expected = t.alu + 3 * t.alu + 2 * t.branch_taken + t.branch_not_taken + t.div + t.system;
    assert_eq!(h.elapsed_cycles(), u64::from(expected));
    assert_eq!(h.local_time(), SimTime(u64::from(expected) * 6250));
}

#[test]
fn clock_changes_keep_time_and_scale_later_cycles() {
    let (mut h, mut ram) = setup(cfg(), &[jal(0, 0)]); // spin, 100 MHz
    h.run_until(&mut ram, SimTime::from_us(1));
    assert_eq!(h.elapsed_cycles(), 100);
    h.set_clock_hz(40_000_000);
    assert_eq!(h.cycle_period_ps(), 25_000);
    assert_eq!(h.local_time(), SimTime::from_us(1), "time so far is kept");
    h.run_until(&mut ram, SimTime::from_us(2));
    assert_eq!(h.elapsed_cycles(), 140, "40 cycles in the next microsecond");
    h.reset();
    assert_eq!(
        (h.cycle_period_ps(), h.local_time()),
        (25_000, SimTime::from_us(2)),
        "reset keeps the clock"
    );
}

struct SlowBus(Ram);

impl Bus for SlowBus {
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError> {
        self.0.read(addr, size, kind)
    }
    fn write(&mut self, addr: u32, size: Size, value: u32, kind: AccessKind) -> Result<(), BusError> {
        self.0.write(addr, size, value, kind)
    }
    fn wait_states(&self, addr: u32, kind: AccessKind) -> u32 {
        match kind {
            AccessKind::Fetch => 1,
            _ if addr >= DATA => 10,
            _ => 0,
        }
    }
}

#[test]
fn bus_wait_states_add_cycles() {
    let [l, a] = li(1, DATA);
    let mut ram = Ram::new(BASE, 0x2000);
    ram.load_words(BASE, &[l, a, lw(2, 1, 0)]).unwrap();
    let mut bus = SlowBus(ram);
    let mut h = Hart::new(cfg());
    h.step(&mut bus);
    h.step(&mut bus);
    assert!(matches!(h.step(&mut bus), StepOutcome::Retired { cycles: 12, .. }));
}

#[test]
fn run_until_respects_deadline() {
    let (mut h, mut ram) = setup(cfg(), &[jal(0, 0)]); // spin
    h.set_time_origin(SimTime::from_us(1));
    let deadline = SimTime(SimTime::from_us(1).0 + 1_000_000); // +1 us = 100 cycles at 100 MHz
    assert_eq!(h.run_until(&mut ram, deadline), RunExit::BudgetExhausted);
    assert_eq!(h.elapsed_cycles(), 100);
    assert_eq!(h.local_time(), deadline);
    assert_eq!(h.run_until(&mut ram, deadline), RunExit::BudgetExhausted);
    assert_eq!(h.elapsed_cycles(), 100);
}

#[test]
fn breakpoints_and_single_step() {
    let prog = [addi(1, 0, 1), addi(1, 1, 1), addi(1, 1, 1), addi(1, 1, 1), EBREAK];
    let (mut h, mut ram) = setup(cfg(), &prog);
    assert!(h.add_breakpoint(BASE + 8));
    assert!(!h.add_breakpoint(BASE + 8));
    assert_eq!(h.run(&mut ram, 100), RunExit::Breakpoint(BASE + 8));
    assert_eq!(h.reg(1), 2);
    // Resuming from a breakpoint executes it rather than stopping again.
    let out = h.step(&mut ram);
    assert!(matches!(out, StepOutcome::Retired { pc, raw, .. } if pc == BASE + 8 && raw == prog[2]));
    h.set_pc(BASE + 8);
    h.set_reg(1, 2);
    assert!(matches!(h.run(&mut ram, 100), RunExit::Halted(HaltReason::Ebreak { pc }) if pc == BASE + 16));
    assert_eq!(h.reg(1), 4);
    assert!(h.remove_breakpoint(BASE + 8));
    assert_eq!(h.breakpoints().count(), 0);
    assert_eq!(h.pc(), BASE + 16, "ebreak halt leaves pc on the ebreak");
}

#[test]
fn halt_resume_and_reset() {
    let (mut h, mut ram) = setup(cfg(), &[addi(1, 0, 1), jal(0, 0)]);
    h.halt();
    assert_eq!(h.step(&mut ram), StepOutcome::Halted(HaltReason::Requested));
    assert_eq!(h.run(&mut ram, 10), RunExit::Halted(HaltReason::Requested));
    h.resume();
    h.run(&mut ram, 10);
    assert_eq!(h.reg(1), 1);
    let cycles = h.elapsed_cycles();
    h.reset();
    assert_eq!((h.reg(1), h.pc(), h.csrs().minstret), (0, BASE, 0));
    assert_eq!(h.elapsed_cycles(), cycles, "simulation time is not rewound");
}

struct SyncBus(Ram, bool);

impl Bus for SyncBus {
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError> {
        self.0.read(addr, size, kind)
    }
    fn write(&mut self, addr: u32, size: Size, value: u32, kind: AccessKind) -> Result<(), BusError> {
        if addr == DATA {
            self.1 = true; // pretend this is a peripheral register
        }
        self.0.write(addr, size, value, kind)
    }
    fn take_sync_request(&mut self) -> bool {
        std::mem::take(&mut self.1)
    }
}

#[test]
fn peripheral_access_requests_sync() {
    let [l, a] = li(1, DATA);
    let mut ram = Ram::new(BASE, 0x2000);
    ram.load_words(BASE, &[l, a, NOP, sw(0, 1, 0), NOP, EBREAK]).unwrap();
    let mut bus = SyncBus(ram, false);
    let mut h = Hart::new(cfg());
    assert_eq!(h.run(&mut bus, 100), RunExit::SyncRequested(BASE + 12));
    assert_eq!(h.pc(), BASE + 16);
    assert!(matches!(h.run(&mut bus, 100), RunExit::Halted(_)));
}

#[test]
fn disassemble_at_without_executing() {
    let (h, mut ram) = setup(cfg(), &[addi(10, 0, 42)]);
    let (raw, len, instr) = h.disassemble_at(&mut ram, BASE).unwrap();
    assert_eq!((raw, len), (addi(10, 0, 42), 4));
    assert_eq!(instr.to_string(), "addi a0, zero, 42");
    assert_eq!(h.elapsed_cycles(), 0);
}

// ----- compressed ---------------------------------------------------------------------

fn halfwords(parts: &[u16]) -> Vec<u32> {
    let mut v: Vec<u16> = parts.to_vec();
    if v.len() % 2 == 1 {
        v.push(0x0001); // c.nop padding
    }
    v.chunks(2).map(|c| u32::from(c[0]) | (u32::from(c[1]) << 16)).collect()
}

#[test]
fn compressed_instructions_advance_by_two_and_link_pc_plus_two() {
    // c.li a0,1 ; c.jal +4 ; c.ebreak(skipped) ; c.addi a0,1 (target) ; c.ebreak
    let prog = halfwords(&[0x4505, 0x2011, 0x9002, 0x0505, 0x9002]);
    let mut prog32 = prog.clone();
    prog32.truncate(3);
    let h = exec(&prog32);
    assert_eq!(h.reg(10), 2);
    assert_eq!(h.reg(1), BASE + 4, "c.jal links pc + 2");
    assert_eq!(h.pc(), BASE + 8);
}

#[test]
fn mixed_16_and_32_bit_stream() {
    // c.li a0, 5 ; addi a0, a0, 10 (straddles a word boundary) ; c.ebreak
    let add = addi(10, 10, 10);
    let prog = halfwords(&[0x4515, add as u16, (add >> 16) as u16, 0x9002]);
    let h = exec(&prog);
    assert_eq!(h.reg(10), 15);
}

#[test]
fn compressed_illegal_when_c_disabled_or_reserved() {
    let mut c = cfg();
    c.extensions.c = false;
    let (mut h, mut ram) = setup(c, &halfwords(&[0x4505, 0x0001]));
    assert!(matches!(
        h.step(&mut ram),
        StepOutcome::Trapped {
            cause: TrapCause::Exception(Exception::IllegalInstruction, 0x4505),
            ..
        }
    ));
    // c.lui a0, 0 is reserved; mtval holds just the 16-bit parcel.
    assert_eq!(trap_of(&halfwords(&[0x6501, 0x0001])), (2, 0x6501, BASE));
}

// ----- small programs -------------------------------------------------------------------

#[test]
fn program_fibonacci() {
    // fib(20) iteratively: a0 = n; a1 = 0; a2 = 1
    let h = exec(&[
        addi(10, 0, 20),
        addi(11, 0, 0),
        addi(12, 0, 1),
        beq(10, 0, 24), // loop: exit to the ebreak
        add(13, 11, 12),
        addi(11, 12, 0),
        addi(12, 13, 0),
        addi(10, 10, -1),
        jal(0, -20),
        EBREAK,
    ]);
    assert_eq!(h.reg(11), 6765);
}

#[test]
fn program_memcpy_and_checksum() {
    let src = DATA;
    let dst = DATA + 0x100;
    let [l1, a1] = li(10, dst);
    let [l2, a2] = li(11, src);
    let prog = [
        l1,
        a1,
        l2,
        a2,
        addi(12, 0, 16), // words
        addi(14, 0, 0),  // checksum
        lw(13, 11, 0),   // loop:
        sw(13, 10, 0),
        add(14, 14, 13),
        addi(10, 10, 4),
        addi(11, 11, 4),
        addi(12, 12, -1),
        bne(12, 0, -24),
        EBREAK,
    ];
    let (mut h, mut ram) = setup(cfg(), &prog);
    let words: Vec<u32> = (0..16).map(|i| 0x0101_0101u32.wrapping_mul(i + 1)).collect();
    ram.load_words(src, &words).unwrap();
    assert!(matches!(h.run(&mut ram, 10_000), RunExit::Halted(_)));
    assert_eq!(ram.slice(dst, 64), ram.slice(src, 64));
    assert_eq!(h.reg(14), words.iter().fold(0u32, |a, w| a.wrapping_add(*w)));
}

#[test]
fn program_recursive_factorial_uses_stack() {
    let [ls, as_] = li(2, BASE + 0x8000);
    let prog = [
        ls,
        as_,
        addi(10, 0, 10),
        jal(1, 8), // call fact
        EBREAK,
        // fact(a0): if a0 < 2 return 1
        addi(5, 0, 2),  // 20
        bge(10, 5, 12), // 24 -> 36
        addi(10, 0, 1), // 28
        jalr(0, 1, 0),  // 32
        addi(2, 2, -8), // 36
        sw(1, 2, 4),
        sw(10, 2, 0),
        addi(10, 10, -1),
        jal(1, -32), // 52: recurse to 20
        lw(11, 2, 0),
        lw(1, 2, 4),
        addi(2, 2, 8),
        mul(10, 10, 11),
        jalr(0, 1, 0),
    ];
    let h = exec(&prog);
    assert_eq!(h.reg(10), 3_628_800);
    assert_eq!(h.reg(2), BASE + 0x8000);
}

#[test]
fn program_ecall_service_handler() {
    // A tiny "syscall": a7 = 1 means a0 += a1. The handler skips the ecall.
    let (mut h, mut ram) = setup(
        cfg(),
        &[addi(10, 0, 30), addi(11, 0, 12), addi(17, 0, 1), ECALL, EBREAK],
    );
    h.csr_poke(MTVEC, HANDLER);
    ram.load_words(
        HANDLER,
        &[
            addi(5, 0, 1),
            bne(17, 5, 8),
            add(10, 10, 11),
            csrrs(6, MEPC, 0),
            addi(6, 6, 4),
            csrrw(0, MEPC, 6),
            MRET,
        ],
    )
    .unwrap();
    assert!(matches!(h.run(&mut ram, 1000), RunExit::Halted(_)));
    assert_eq!(h.reg(10), 42);
}

#[test]
fn program_timer_interrupt_counter() {
    // Main: enable MTI, then spin in wfi until the handler has counted 3 ticks.
    let prog = [
        addi(5, 0, 1 << 7),
        csrrs(0, MIE, 5),
        csrrsi(0, MSTATUS, 8),
        addi(6, 0, 3),
        WFI, // 16: loop
        blt(20, 6, -4),
        EBREAK,
    ];
    let (mut h, mut ram) = setup(cfg(), &prog);
    h.csr_poke(MTVEC, HANDLER);
    ram.load_words(HANDLER, &[addi(20, 20, 1), MRET]).unwrap();
    let mut ticks = 0;
    loop {
        match h.run(&mut ram, 1000) {
            RunExit::WaitingForInterrupt => {
                // The "SoC timer" fires; the handler should clear it, but here
                // the test plays the peripheral and drops it after one tick.
                ticks += 1;
                h.set_interrupt_line(7, true);
                let StepOutcome::Trapped { cause, .. } = h.step(&mut ram) else {
                    panic!("no trap")
                };
                assert_eq!(cause, TrapCause::Interrupt(7));
                h.set_interrupt_line(7, false);
            }
            RunExit::Halted(_) => break,
            other => panic!("{other:?}"),
        }
        assert!(ticks < 10);
    }
    assert_eq!((h.reg(20), ticks), (3, 3));
}
