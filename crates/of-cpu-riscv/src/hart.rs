//! The hart: architectural state and the fetch-decode-execute state machine.

use std::collections::BTreeSet;

use of_core::SimTime;

use crate::bus::{AccessKind, Bus, BusError, Size};
use crate::compressed;
use crate::csr::{CsrAccess, CsrFile, MSTATUS_MIE, MSTATUS_MPIE, addr};
use crate::decode::{AluOp, AmoOp, BranchOp, CsrOp, CsrSrc, Instr, LoadOp, MulOp, StoreOp, decode};
use crate::timing::TimingModel;
use crate::trap::{Exception, TrapCause, select_interrupt};

/// Optional ISA extensions. RV32I, Zicsr and Zifencei are always present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extensions {
    /// Integer multiply/divide.
    pub m: bool,
    /// Atomics (ESP32-C6 has them; the ESP32-C3 does not).
    pub a: bool,
    /// Compressed instructions.
    pub c: bool,
}

impl Extensions {
    /// The `misa` value for these extensions (MXL = 32-bit).
    pub fn misa(&self) -> u32 {
        let mut v = (1 << 30) | (1 << 8); // MXL=1, I
        if self.a {
            v |= 1;
        }
        if self.c {
            v |= 1 << 2;
        }
        if self.m {
            v |= 1 << 12;
        }
        v
    }
}

/// How misaligned loads and stores behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MisalignedPolicy {
    /// Raise an address-misaligned exception (default).
    Trap,
    /// Perform the access byte by byte.
    Allow,
}

/// What `ebreak` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EbreakAction {
    /// Raise a breakpoint exception (normal firmware behaviour).
    Trap,
    /// Halt the hart and report to the debugger (GDB software breakpoints).
    Halt,
}

/// Static configuration of a hart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HartConfig {
    /// Value of `mhartid`.
    pub hart_id: u32,
    /// PC after reset. The SoC crate sets this to the ROM reset vector.
    pub reset_pc: u32,
    /// Enabled extensions.
    pub extensions: Extensions,
    /// Core clock in hertz (converts cycles to simulation time).
    pub clock_hz: u64,
    /// Which `mie`/`mip` bits (interrupt lines) exist.
    pub irq_mask: u32,
    /// Whether the standard `mie` and `mip` CSRs exist. The ESP32-C3 has
    /// neither (its interrupt controller enables and reports the lines;
    /// touching `mie`/`mip` is an illegal instruction), so every line in
    /// `irq_mask` is then enabled as far as the core is concerned.
    pub mie_mip: bool,
    /// Misaligned data access behaviour.
    pub misaligned: MisalignedPolicy,
    /// `ebreak` behaviour.
    pub ebreak: EbreakAction,
    /// Cycle cost model.
    pub timing: TimingModel,
}

impl HartConfig {
    /// ESP32-C3: RV32IMC + Zicsr + Zifencei at 160 MHz, 31 interrupt lines
    /// (bits 1 to 31) gated by the SoC's interrupt controller rather than
    /// `mie` (no `mie`/`mip` CSRs), estimated timings. Misaligned loads
    /// and stores are performed, not trapped: the C3 core splits them in
    /// hardware (ESP32-C3 errata CPU-863 describes that split), and
    /// ESP-IDF code relies on it (the second-stage bootloader reads packed
    /// image-header fields with `lhu` at odd addresses).
    pub fn esp32c3() -> Self {
        Self {
            hart_id: 0,
            reset_pc: 0x4000_0000,
            extensions: Extensions {
                m: true,
                a: false,
                c: true,
            },
            clock_hz: 160_000_000,
            irq_mask: 0xffff_fffe,
            mie_mip: false,
            misaligned: MisalignedPolicy::Allow,
            ebreak: EbreakAction::Trap,
            timing: TimingModel::ESP32C3_ESTIMATE,
        }
    }

    /// ESP32-C6 high-performance core: like the C3 plus the A extension.
    pub fn esp32c6() -> Self {
        Self {
            extensions: Extensions {
                m: true,
                a: true,
                c: true,
            },
            ..Self::esp32c3()
        }
    }

    /// Generic RV32IMAC with standard interrupt bits and unit timing.
    pub fn generic_imac() -> Self {
        Self {
            hart_id: 0,
            reset_pc: 0x8000_0000,
            extensions: Extensions {
                m: true,
                a: true,
                c: true,
            },
            clock_hz: 100_000_000,
            irq_mask: 0xffff_0888,
            mie_mip: true,
            misaligned: MisalignedPolicy::Trap,
            ebreak: EbreakAction::Trap,
            timing: TimingModel::UNIT,
        }
    }
}

/// Why a hart stopped executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltReason {
    /// The debugger asked it to stop.
    Requested,
    /// `ebreak` executed with [`EbreakAction::Halt`].
    Ebreak {
        /// Address of the `ebreak`.
        pc: u32,
    },
}

/// Run state of the hart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HartState {
    /// Executing instructions.
    Running,
    /// Stalled in `wfi` until an enabled interrupt is pending.
    WaitingForInterrupt,
    /// Stopped for the debugger.
    Halted(HaltReason),
}

/// The result of one [`Hart::step`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// An instruction retired.
    Retired {
        /// Its address.
        pc: u32,
        /// Raw encoding (16-bit parcels are zero-extended).
        raw: u32,
        /// Decoded form (compressed instructions show their expansion).
        instr: Instr,
        /// Cycles consumed, including wait states.
        cycles: u32,
    },
    /// A trap was taken (exception or interrupt).
    Trapped {
        /// PC at which the trap was taken (`mepc`).
        pc: u32,
        /// Cause.
        cause: TrapCause,
        /// Cycles consumed.
        cycles: u32,
    },
    /// The hart is waiting for an interrupt; no time passed.
    Waiting,
    /// The hart is halted; nothing happened.
    Halted(HaltReason),
    /// The bus deferred a load ([`Bus::defer_load`]): the instruction at
    /// `pc` did not execute and no time passed.
    Deferred {
        /// Its address.
        pc: u32,
    },
}

/// Why [`Hart::run`] or [`Hart::run_until`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunExit {
    /// The cycle budget or time deadline was reached.
    BudgetExhausted,
    /// The PC hit a debugger breakpoint (not yet executed).
    Breakpoint(u32),
    /// The hart halted.
    Halted(HaltReason),
    /// The hart executed `wfi` and no interrupt is pending. Call
    /// [`Hart::idle_until`] once the scheduler knows the next event time.
    WaitingForInterrupt,
    /// The bus requested a sync point (peripheral access) after the
    /// instruction at this PC.
    SyncRequested(u32),
    /// The bus deferred a load ([`Bus::defer_load`]); the instruction at
    /// this PC has not executed. Run again once the bus is ready.
    Deferred(u32),
}

struct Flow {
    next_pc: u32,
    taken: bool,
    wait: u32,
}

type ExecResult = Result<Flow, (Exception, u32)>;

/// An RV32 hart (hardware thread) in machine mode.
#[derive(Debug, Clone)]
pub struct Hart {
    config: HartConfig,
    regs: [u32; 32],
    pc: u32,
    csr: CsrFile,
    state: HartState,
    reservation: Option<u32>,
    elapsed_cycles: u64,
    /// Local time at `origin_cycles` elapsed cycles.
    time_origin: SimTime,
    /// Elapsed cycles at the last clock change (0 if none).
    origin_cycles: u64,
    period_ps: u64,
    breakpoints: BTreeSet<u32>,
    /// Set by a load the bus deferred, so `step` does not trap.
    deferred: bool,
    dcache: DecodeCache,
}

impl Hart {
    /// A hart in its reset state.
    pub fn new(config: HartConfig) -> Self {
        let mut csr = CsrFile::new(config.extensions.misa(), config.hart_id, config.irq_mask);
        csr.mie_mip = config.mie_mip;
        let period_ps = (1_000_000_000_000 / config.clock_hz.max(1)).max(1);
        Self {
            pc: config.reset_pc,
            config,
            regs: [0; 32],
            csr,
            state: HartState::Running,
            reservation: None,
            elapsed_cycles: 0,
            time_origin: SimTime::ZERO,
            origin_cycles: 0,
            period_ps,
            breakpoints: BTreeSet::new(),
            deferred: false,
            dcache: DecodeCache::new(),
        }
    }

    /// Reset architectural state (keeps breakpoints, elapsed time and the
    /// current clock: the SoC decides the clock after a reset).
    pub fn reset(&mut self) {
        let keep_bp = std::mem::take(&mut self.breakpoints);
        let (elapsed, origin, origin_cycles, period) = (
            self.elapsed_cycles,
            self.time_origin,
            self.origin_cycles,
            self.period_ps,
        );
        *self = Self::new(self.config.clone());
        self.breakpoints = keep_bp;
        self.elapsed_cycles = elapsed;
        self.time_origin = origin;
        self.origin_cycles = origin_cycles;
        self.period_ps = period;
    }

    /// Change the core clock from now on (the SoC's clock tree switched
    /// source or divider). Time so far is kept; later cycles last
    /// `1 / hz`. Cycle counters are unaffected.
    pub fn set_clock_hz(&mut self, hz: u64) {
        let now = self.local_time();
        self.time_origin = now;
        self.origin_cycles = self.elapsed_cycles;
        self.period_ps = (1_000_000_000_000 / hz.max(1)).max(1);
    }

    // ----- inspection and debugger API ---------------------------------

    /// Configuration.
    pub fn config(&self) -> &HartConfig {
        &self.config
    }
    /// Program counter.
    pub fn pc(&self) -> u32 {
        self.pc
    }
    /// Set the program counter.
    pub fn set_pc(&mut self, pc: u32) {
        self.pc = pc;
    }
    /// Register `x[i]` (x0 is always 0).
    pub fn reg(&self, i: u8) -> u32 {
        self.regs[usize::from(i & 31)]
    }
    /// Write register `x[i]` (writes to x0 are ignored).
    pub fn set_reg(&mut self, i: u8, value: u32) {
        if i & 31 != 0 {
            self.regs[usize::from(i & 31)] = value;
        }
    }
    /// All 32 registers.
    pub fn regs(&self) -> &[u32; 32] {
        &self.regs
    }
    /// The CSR file.
    pub fn csrs(&self) -> &CsrFile {
        &self.csr
    }
    /// Read a core CSR without side effects.
    pub fn csr_peek(&self, csr: u16) -> Option<u32> {
        self.csr.peek(csr)
    }
    /// Write a core CSR as the debugger (no permission checks).
    pub fn csr_poke(&mut self, csr: u16, value: u32) -> bool {
        let ok = self.csr.poke(csr, value);
        self.csr.counter_written = 0;
        ok
    }
    /// Run state.
    pub fn state(&self) -> HartState {
        self.state
    }
    /// Stop executing (debugger "interrupt").
    pub fn halt(&mut self) {
        self.state = HartState::Halted(HaltReason::Requested);
    }
    /// Leave the halted state.
    pub fn resume(&mut self) {
        if matches!(self.state, HartState::Halted(_)) {
            self.state = HartState::Running;
        }
    }
    /// Add a breakpoint at `pc`. Returns `false` if one was already there.
    pub fn add_breakpoint(&mut self, pc: u32) -> bool {
        self.breakpoints.insert(pc)
    }
    /// Remove a breakpoint. Returns `false` if there was none.
    pub fn remove_breakpoint(&mut self, pc: u32) -> bool {
        self.breakpoints.remove(&pc)
    }
    /// Breakpoints in address order.
    pub fn breakpoints(&self) -> impl Iterator<Item = u32> + '_ {
        self.breakpoints.iter().copied()
    }

    // ----- time ---------------------------------------------------------

    /// Cycles elapsed since construction (monotonic; unlike `mcycle`,
    /// software cannot change it).
    pub fn elapsed_cycles(&self) -> u64 {
        self.elapsed_cycles
    }
    /// Length of one clock cycle in picoseconds (6250 at 160 MHz).
    pub fn cycle_period_ps(&self) -> u64 {
        self.period_ps
    }
    /// Set the simulation time that corresponds to zero elapsed cycles
    /// (at the current clock; forgets earlier clock changes).
    pub fn set_time_origin(&mut self, t: SimTime) {
        self.time_origin = t;
        self.origin_cycles = 0;
    }
    /// The hart's local simulation time.
    pub fn local_time(&self) -> SimTime {
        SimTime(self.time_origin.0 + (self.elapsed_cycles - self.origin_cycles) * self.period_ps)
    }

    fn account(&mut self, cycles: u32, retired: bool) {
        self.elapsed_cycles += u64::from(cycles);
        let written = std::mem::take(&mut self.csr.counter_written);
        if self.csr.mcountinhibit & 1 == 0 && written & 1 == 0 {
            self.csr.mcycle = self.csr.mcycle.wrapping_add(u64::from(cycles));
        }
        if retired && self.csr.mcountinhibit & 4 == 0 && written & 4 == 0 {
            self.csr.minstret = self.csr.minstret.wrapping_add(1);
        }
    }

    /// While waiting for an interrupt, let time pass up to `deadline`
    /// (the counters keep running, as the core clock is not gated).
    pub fn idle_until(&mut self, deadline: SimTime) {
        if self.state != HartState::WaitingForInterrupt {
            return;
        }
        let now = self.local_time();
        if deadline > now {
            let cycles = (deadline.0 - now.0).div_ceil(self.period_ps);
            self.elapsed_cycles += cycles;
            if self.csr.mcountinhibit & 1 == 0 {
                self.csr.mcycle = self.csr.mcycle.wrapping_add(cycles);
            }
        }
    }

    // ----- interrupts -----------------------------------------------------

    /// Drive interrupt line `line` (a `mip` bit) high or low.
    pub fn set_interrupt_line(&mut self, line: u8, level: bool) {
        let bit = (1u32 << (line & 31)) & self.config.irq_mask;
        if level {
            self.csr.mip |= bit;
        } else {
            self.csr.mip &= !bit;
        }
    }
    /// Replace all interrupt line levels at once.
    pub fn set_interrupt_lines(&mut self, mip: u32) {
        self.csr.mip = mip & self.config.irq_mask;
    }

    fn pending_enabled(&self) -> u32 {
        if self.config.mie_mip {
            self.csr.mip & self.csr.mie
        } else {
            self.csr.mip
        }
    }

    // ----- execution --------------------------------------------------------

    fn trap(&mut self, cause: TrapCause, epc: u32) {
        self.csr.mepc = epc;
        self.csr.mcause = cause.mcause();
        self.csr.mtval = match cause {
            TrapCause::Exception(_, tval) => tval,
            TrapCause::Interrupt(_) => 0,
        };
        let mie = self.csr.mstatus & MSTATUS_MIE != 0;
        self.csr.mstatus &= !(MSTATUS_MIE | MSTATUS_MPIE);
        if mie {
            self.csr.mstatus |= MSTATUS_MPIE;
        }
        self.reservation = None;
        let base = self.csr.mtvec & !3;
        self.pc = match cause {
            TrapCause::Interrupt(line) if self.csr.mtvec & 1 == 1 => base.wrapping_add(4 * u32::from(line)),
            _ => base,
        };
    }

    /// Execute one instruction (or take one interrupt).
    pub fn step(&mut self, bus: &mut dyn Bus) -> StepOutcome {
        match self.state {
            HartState::Halted(r) => return StepOutcome::Halted(r),
            HartState::WaitingForInterrupt => {
                if self.pending_enabled() == 0 {
                    return StepOutcome::Waiting;
                }
                self.state = HartState::Running;
            }
            HartState::Running => {}
        }
        if self.csr.mstatus & MSTATUS_MIE != 0
            && let Some(line) = select_interrupt(self.pending_enabled())
        {
            let pc = self.pc;
            let cause = TrapCause::Interrupt(line);
            self.trap(cause, pc);
            let cycles = self.config.timing.trap_entry;
            self.account(cycles, false);
            return StepOutcome::Trapped { pc, cause, cycles };
        }

        let pc = self.pc;
        match self.fetch_execute(bus, pc) {
            Ok((raw, instr, flow, cycles)) => {
                self.pc = flow.next_pc;
                self.account(cycles, true);
                if let (Instr::Ebreak, EbreakAction::Halt) = (instr, self.config.ebreak) {
                    self.state = HartState::Halted(HaltReason::Ebreak { pc });
                }
                StepOutcome::Retired { pc, raw, instr, cycles }
            }
            Err(_) if std::mem::take(&mut self.deferred) => StepOutcome::Deferred { pc },
            Err((exc, tval, cycles)) => {
                let cause = TrapCause::Exception(exc, tval);
                self.trap(cause, pc);
                let cycles = cycles + self.config.timing.trap_entry;
                self.account(cycles, false);
                StepOutcome::Trapped { pc, cause, cycles }
            }
        }
    }

    /// Fetch the instruction at `pc`: returns the raw parcel(s) and length.
    pub fn fetch(&self, bus: &mut dyn Bus, pc: u32) -> Result<(u32, u32), (Exception, u32)> {
        let c = self.config.extensions.c;
        if pc & 1 != 0 || (!c && pc & 3 != 0) {
            return Err((Exception::InstructionAddressMisaligned, pc));
        }
        let fault = |e: BusError| (Exception::InstructionAccessFault, e.addr);
        let lo = bus.read(pc, Size::Half, AccessKind::Fetch).map_err(fault)?;
        if compressed::is_compressed(lo as u16) {
            return Ok((lo, 2));
        }
        let hi = bus
            .read(pc.wrapping_add(2), Size::Half, AccessKind::Fetch)
            .map_err(fault)?;
        Ok(((hi << 16) | lo, 4))
    }

    /// Fetch and decode the instruction at `pc` without executing it
    /// (debugger disassembly view). Compressed instructions are expanded.
    pub fn disassemble_at(&self, bus: &mut dyn Bus, pc: u32) -> Option<(u32, u32, Instr)> {
        let (raw, len) = self.fetch(bus, pc).ok()?;
        let word = if len == 2 { compressed::expand(raw as u16)? } else { raw };
        Some((raw, len, decode(word)?))
    }

    #[allow(clippy::type_complexity)]
    fn fetch_execute(&mut self, bus: &mut dyn Bus, pc: u32) -> Result<(u32, Instr, Flow, u32), (Exception, u32, u32)> {
        let no_cycles = |(e, v): (Exception, u32)| (e, v, 0);
        let (raw, len) = self.fetch(bus, pc).map_err(no_cycles)?;
        let mut wait = bus.wait_states(pc, AccessKind::Fetch);
        let illegal = (Exception::IllegalInstruction, raw, 0);
        let (_word, instr) = if let Some((craw, clen, word, instr)) = self.dcache.lookup(pc)
            && craw == raw
            && clen == len
        {
            (word, instr)
        } else {
            let word = if len == 2 {
                if !self.config.extensions.c {
                    return Err(illegal);
                }
                compressed::expand(raw as u16).ok_or(illegal)?
            } else {
                raw
            };
            let instr = decode(word).ok_or(illegal)?;
            self.dcache.insert(pc, raw, len, word, instr);
            (word, instr)
        };
        let ext = self.config.extensions;
        match instr {
            Instr::MulDiv { .. } if !ext.m => return Err(illegal),
            Instr::Amo { .. } if !ext.a => return Err(illegal),
            _ => {}
        }
        let flow = self.execute(bus, instr, pc, len, raw).map_err(|(e, v)| {
            let c = self.config.timing.cycles(instr.class(), false);
            (e, v, c)
        })?;
        wait += flow.wait;
        let cycles = self.config.timing.cycles(instr.class(), flow.taken) + wait;
        Ok((raw, instr, flow, cycles))
    }

    fn x(&self, r: u8) -> u32 {
        self.regs[usize::from(r)]
    }
    fn wr(&mut self, rd: u8, v: u32) {
        if rd != 0 {
            self.regs[usize::from(rd)] = v;
        }
    }

    fn jump_target(&self, target: u32) -> Result<u32, (Exception, u32)> {
        let align = if self.config.extensions.c { 1 } else { 3 };
        if target & align != 0 {
            return Err((Exception::InstructionAddressMisaligned, target));
        }
        Ok(target)
    }

    fn load(
        &mut self,
        bus: &mut dyn Bus,
        addr: u32,
        size: Size,
        kind: AccessKind,
    ) -> Result<(u32, u32), (Exception, u32)> {
        let n = size.bytes();
        let (mis, fault) = match kind {
            AccessKind::Load => (Exception::LoadAddressMisaligned, Exception::LoadAccessFault),
            _ => (Exception::StoreAddressMisaligned, Exception::StoreAccessFault),
        };
        let misaligned = !addr.is_multiple_of(n);
        if misaligned && (kind != AccessKind::Load || self.config.misaligned == MisalignedPolicy::Trap) {
            return Err((mis, addr));
        }
        if bus.defer_load(addr, self.local_time()) {
            self.deferred = true;
            return Err((fault, addr));
        }
        if misaligned {
            let mut v = 0u32;
            let mut wait = 0;
            for k in 0..n {
                let a = addr.wrapping_add(k);
                let b = bus.read(a, Size::Byte, kind).map_err(|e| (fault, e.addr))?;
                wait += bus.wait_states(a, kind);
                v |= b << (8 * k);
            }
            return Ok((v, wait));
        }
        let v = bus.read(addr, size, kind).map_err(|e| (fault, e.addr))?;
        Ok((v, bus.wait_states(addr, kind)))
    }

    fn store(
        &mut self,
        bus: &mut dyn Bus,
        addr: u32,
        size: Size,
        value: u32,
        kind: AccessKind,
    ) -> Result<u32, (Exception, u32)> {
        let n = size.bytes();
        if !addr.is_multiple_of(n) {
            if kind != AccessKind::Store || self.config.misaligned == MisalignedPolicy::Trap {
                return Err((Exception::StoreAddressMisaligned, addr));
            }
            let mut wait = 0;
            for k in 0..n {
                let a = addr.wrapping_add(k);
                bus.write(a, Size::Byte, (value >> (8 * k)) & 0xff, kind)
                    .map_err(|e| (Exception::StoreAccessFault, e.addr))?;
                wait += bus.wait_states(a, kind);
            }
            self.invalidate_reservation(addr, n);
            return Ok(wait);
        }
        bus.write(addr, size, value, kind)
            .map_err(|e| (Exception::StoreAccessFault, e.addr))?;
        self.invalidate_reservation(addr, n);
        Ok(bus.wait_states(addr, kind))
    }

    fn invalidate_reservation(&mut self, addr: u32, n: u32) {
        if let Some(r) = self.reservation
            && addr < r.wrapping_add(4)
            && r < addr.wrapping_add(n)
        {
            self.reservation = None;
        }
    }

    fn csr_op(
        &mut self,
        bus: &mut dyn Bus,
        op: CsrOp,
        rd: u8,
        csr: u16,
        src: CsrSrc,
        raw: u32,
    ) -> Result<(), (Exception, u32)> {
        let illegal = (Exception::IllegalInstruction, raw);
        let (operand, src_is_zero) = match src {
            CsrSrc::Reg(r) => (self.x(r), r == 0),
            CsrSrc::Imm(v) => (u32::from(v), v == 0),
        };
        let writes = op == CsrOp::Rw || !src_is_zero;
        let reads = op != CsrOp::Rw || rd != 0;
        let old = match self.csr.access(csr, writes) {
            CsrAccess::Ok(v) => v,
            CsrAccess::Illegal => return Err(illegal),
            CsrAccess::Unknown => {
                if (csr >> 10) == 3 && writes {
                    return Err(illegal);
                }
                bus.csr_cycles(self.elapsed_cycles);
                if reads || op != CsrOp::Rw {
                    let v = bus.csr_read(csr).ok_or(illegal)?;
                    if writes {
                        let new = combine(op, v, operand);
                        if !bus.csr_write(csr, new) {
                            return Err(illegal);
                        }
                    }
                    self.wr(rd, v);
                } else if !bus.csr_write(csr, operand) {
                    return Err(illegal);
                }
                return Ok(());
            }
        };
        if writes {
            self.csr.poke(csr, combine(op, old, operand));
        }
        self.wr(rd, old);
        Ok(())
    }

    fn execute(&mut self, bus: &mut dyn Bus, instr: Instr, pc: u32, len: u32, raw: u32) -> ExecResult {
        let seq = pc.wrapping_add(len);
        let mut flow = Flow {
            next_pc: seq,
            taken: false,
            wait: 0,
        };
        match instr {
            Instr::Lui { rd, imm } => self.wr(rd, imm),
            Instr::Auipc { rd, imm } => self.wr(rd, pc.wrapping_add(imm)),
            Instr::Jal { rd, offset } => {
                flow.next_pc = self.jump_target(pc.wrapping_add(offset as u32))?;
                self.wr(rd, seq);
                flow.taken = true;
            }
            Instr::Jalr { rd, rs1, offset } => {
                let target = self.x(rs1).wrapping_add(offset as u32) & !1;
                flow.next_pc = self.jump_target(target)?;
                self.wr(rd, seq);
                flow.taken = true;
            }
            Instr::Branch { op, rs1, rs2, offset } => {
                let (a, b) = (self.x(rs1), self.x(rs2));
                let take = match op {
                    BranchOp::Eq => a == b,
                    BranchOp::Ne => a != b,
                    BranchOp::Lt => (a as i32) < (b as i32),
                    BranchOp::Ge => (a as i32) >= (b as i32),
                    BranchOp::Ltu => a < b,
                    BranchOp::Geu => a >= b,
                };
                if take {
                    flow.next_pc = self.jump_target(pc.wrapping_add(offset as u32))?;
                    flow.taken = true;
                }
            }
            Instr::Load { op, rd, rs1, offset } => {
                let addr = self.x(rs1).wrapping_add(offset as u32);
                let size = match op {
                    LoadOp::Lb | LoadOp::Lbu => Size::Byte,
                    LoadOp::Lh | LoadOp::Lhu => Size::Half,
                    LoadOp::Lw => Size::Word,
                };
                let (v, wait) = self.load(bus, addr, size, AccessKind::Load)?;
                let v = match op {
                    LoadOp::Lb => v as u8 as i8 as i32 as u32,
                    LoadOp::Lh => v as u16 as i16 as i32 as u32,
                    _ => v,
                };
                self.wr(rd, v);
                flow.wait = wait;
            }
            Instr::Store { op, rs1, rs2, offset } => {
                let addr = self.x(rs1).wrapping_add(offset as u32);
                let size = match op {
                    StoreOp::Sb => Size::Byte,
                    StoreOp::Sh => Size::Half,
                    StoreOp::Sw => Size::Word,
                };
                flow.wait = self.store(bus, addr, size, self.x(rs2), AccessKind::Store)?;
                self.dcache.invalidate();
            }
            Instr::OpImm { op, rd, rs1, imm } => {
                let v = alu(op, self.x(rs1), imm as u32);
                self.wr(rd, v);
            }
            Instr::Op { op, rd, rs1, rs2 } => {
                let v = alu(op, self.x(rs1), self.x(rs2));
                self.wr(rd, v);
            }
            Instr::MulDiv { op, rd, rs1, rs2 } => {
                let v = muldiv(op, self.x(rs1), self.x(rs2));
                self.wr(rd, v);
            }
            Instr::Amo { op, rd, rs1, rs2, .. } => {
                let addr = self.x(rs1);
                if addr & 3 != 0 {
                    let e = if op == AmoOp::Lr {
                        Exception::LoadAddressMisaligned
                    } else {
                        Exception::StoreAddressMisaligned
                    };
                    return Err((e, addr));
                }
                match op {
                    AmoOp::Lr => {
                        let (v, wait) = self.load(bus, addr, Size::Word, AccessKind::Load)?;
                        self.reservation = Some(addr);
                        self.wr(rd, v);
                        flow.wait = wait;
                    }
                    AmoOp::Sc => {
                        if self.reservation == Some(addr) {
                            flow.wait = self.store(bus, addr, Size::Word, self.x(rs2), AccessKind::Atomic)?;
                            self.wr(rd, 0);
                            self.dcache.invalidate();
                        } else {
                            self.wr(rd, 1);
                        }
                        self.reservation = None;
                    }
                    _ => {
                        let (old, w1) = self.load(bus, addr, Size::Word, AccessKind::Atomic)?;
                        let src = self.x(rs2);
                        let new = match op {
                            AmoOp::Swap => src,
                            AmoOp::Add => old.wrapping_add(src),
                            AmoOp::Xor => old ^ src,
                            AmoOp::And => old & src,
                            AmoOp::Or => old | src,
                            AmoOp::Min => (old as i32).min(src as i32) as u32,
                            AmoOp::Max => (old as i32).max(src as i32) as u32,
                            AmoOp::Minu => old.min(src),
                            AmoOp::Maxu => old.max(src),
                            AmoOp::Lr | AmoOp::Sc => unreachable!(),
                        };
                        let w2 = self.store(bus, addr, Size::Word, new, AccessKind::Atomic)?;
                        self.wr(rd, old);
                        flow.wait = w1 + w2;
                        self.dcache.invalidate();
                    }
                }
            }
            Instr::Csr { op, rd, csr, src } => self.csr_op(bus, op, rd, csr, src, raw)?,
            Instr::Fence { .. } => {}
            Instr::FenceI => self.dcache.invalidate(),
            Instr::Ecall => return Err((Exception::EcallFromM, 0)),
            Instr::Ebreak => {
                if self.config.ebreak == EbreakAction::Trap {
                    return Err((Exception::Breakpoint, pc));
                }
                flow.next_pc = pc; // halt on the ebreak; debugger decides
            }
            Instr::Mret => {
                let mpie = self.csr.mstatus & MSTATUS_MPIE != 0;
                self.csr.mstatus &= !MSTATUS_MIE;
                if mpie {
                    self.csr.mstatus |= MSTATUS_MIE;
                }
                self.csr.mstatus |= MSTATUS_MPIE;
                self.reservation = None;
                flow.next_pc = self.csr.peek(addr::MEPC).unwrap_or(0);
                flow.taken = true;
            }
            Instr::Wfi => {
                if self.pending_enabled() == 0 {
                    self.state = HartState::WaitingForInterrupt;
                }
            }
        }
        Ok(flow)
    }

    /// Run until `max_cycles` have elapsed or something stops the hart.
    /// A breakpoint at the starting PC is ignored so that resuming works.
    pub fn run(&mut self, bus: &mut dyn Bus, max_cycles: u64) -> RunExit {
        let limit = self.elapsed_cycles.saturating_add(max_cycles);
        let mut first = true;
        while self.elapsed_cycles < limit {
            if !first && self.state == HartState::Running && self.breakpoints.contains(&self.pc) {
                return RunExit::Breakpoint(self.pc);
            }
            first = false;
            let pc = self.pc;
            match self.step(bus) {
                StepOutcome::Halted(r) => return RunExit::Halted(r),
                StepOutcome::Waiting => return RunExit::WaitingForInterrupt,
                StepOutcome::Deferred { pc } => return RunExit::Deferred(pc),
                StepOutcome::Retired { .. } | StepOutcome::Trapped { .. } => {}
            }
            if let HartState::Halted(r) = self.state {
                return RunExit::Halted(r);
            }
            if bus.take_sync_request() {
                return RunExit::SyncRequested(pc);
            }
            if self.state == HartState::WaitingForInterrupt {
                return RunExit::WaitingForInterrupt;
            }
        }
        RunExit::BudgetExhausted
    }

    /// Run until the hart's local time reaches `deadline` (it may overshoot
    /// by at most one instruction) or something stops it.
    pub fn run_until(&mut self, bus: &mut dyn Bus, deadline: SimTime) -> RunExit {
        let now = self.local_time();
        if deadline <= now {
            return RunExit::BudgetExhausted;
        }
        let cycles = (deadline.0 - now.0).div_ceil(self.period_ps);
        self.run(bus, cycles)
    }
}

fn combine(op: CsrOp, old: u32, operand: u32) -> u32 {
    match op {
        CsrOp::Rw => operand,
        CsrOp::Rs => old | operand,
        CsrOp::Rc => old & !operand,
    }
}

/// Integer ALU operation.
pub fn alu(op: AluOp, a: u32, b: u32) -> u32 {
    match op {
        AluOp::Add => a.wrapping_add(b),
        AluOp::Sub => a.wrapping_sub(b),
        AluOp::Sll => a << (b & 31),
        AluOp::Slt => u32::from((a as i32) < (b as i32)),
        AluOp::Sltu => u32::from(a < b),
        AluOp::Xor => a ^ b,
        AluOp::Srl => a >> (b & 31),
        AluOp::Sra => ((a as i32) >> (b & 31)) as u32,
        AluOp::Or => a | b,
        AluOp::And => a & b,
    }
}

/// M-extension operation, including the spec's division-by-zero and
/// overflow results.
pub fn muldiv(op: MulOp, a: u32, b: u32) -> u32 {
    let (sa, sb) = (a as i32, b as i32);
    match op {
        MulOp::Mul => a.wrapping_mul(b),
        MulOp::Mulh => ((i64::from(sa) * i64::from(sb)) >> 32) as u32,
        MulOp::Mulhsu => ((i64::from(sa) * i64::from(b)) >> 32) as u32,
        MulOp::Mulhu => ((u64::from(a) * u64::from(b)) >> 32) as u32,
        MulOp::Div => {
            if b == 0 {
                u32::MAX
            } else {
                sa.wrapping_div(sb) as u32
            }
        }
        MulOp::Divu => a.checked_div(b).unwrap_or(u32::MAX),
        MulOp::Rem => {
            if b == 0 {
                a
            } else {
                sa.wrapping_rem(sb) as u32
            }
        }
        MulOp::Remu => a.checked_rem(b).unwrap_or(a),
    }
}

/// Direct-mapped decode cache, invalidated by `fence.i` and stores (including
/// stores to instruction memory). The fetch itself still goes to the bus so
/// wait-states and access faults stay exact.
const DECODE_CACHE_LINES: usize = 1024;

#[derive(Clone, Copy, Debug)]
struct DecodeSlot {
    epoch: u32,
    pc: u32,
    raw: u32,
    len: u32,
    word: u32,
    instr: Instr,
}

#[derive(Clone, Debug)]
struct DecodeCache {
    epoch: u32,
    slots: Box<[DecodeSlot]>,
}

impl DecodeCache {
    fn new() -> Self {
        let dummy = DecodeSlot {
            epoch: 0,
            pc: 0,
            raw: 0,
            len: 0,
            word: 0,
            instr: Instr::Lui { rd: 0, imm: 0 },
        };
        Self {
            epoch: 1,
            slots: vec![dummy; DECODE_CACHE_LINES].into_boxed_slice(),
        }
    }

    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            for s in self.slots.iter_mut() {
                s.epoch = 0;
            }
            self.epoch = 1;
        }
    }

    fn lookup(&self, pc: u32) -> Option<(u32, u32, u32, Instr)> {
        let s = &self.slots[((pc >> 1) as usize) & (DECODE_CACHE_LINES - 1)];
        if s.epoch == self.epoch && s.pc == pc {
            Some((s.raw, s.len, s.word, s.instr))
        } else {
            None
        }
    }

    fn insert(&mut self, pc: u32, raw: u32, len: u32, word: u32, instr: Instr) {
        let i = ((pc >> 1) as usize) & (DECODE_CACHE_LINES - 1);
        self.slots[i] = DecodeSlot {
            epoch: self.epoch,
            pc,
            raw,
            len,
            word,
            instr,
        };
    }
}
