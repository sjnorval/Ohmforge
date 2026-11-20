//! Interrupt matrix and CPU interrupt controller (TRM chapter 8,
//! "Interrupt Matrix", and section 1.5, "Interrupt Controller").
//!
//! Each of the 62 peripheral interrupt sources is routed by its
//! `*_INTR_MAP` register to one of the CPU interrupts 1 to 31 (0 means not
//! routed). A CPU interrupt's input is the OR of the sources routed to it.
//! The controller then applies, per CPU interrupt:
//!
//! - enable (`CPU_INT_ENABLE`),
//! - type (`CPU_INT_TYPE`: 0 level, 1 edge; edge interrupts latch on the
//!   rising edge of the input and stay pending until `CPU_INT_CLEAR`),
//! - priority 1 to 15 (`CPU_INT_PRI_n`; 0 never fires) against the global
//!   threshold (`CPU_INT_THRESH`): an interrupt is masked when its priority
//!   is below the threshold.
//!
//! Pending, enabled, unmasked interrupts show in `CPU_INT_EIP_STATUS`. The
//! core is shown only the winner (highest priority; on a tie the lowest
//! number), so [`of_cpu_riscv::Hart`]'s own line order never decides. The
//! controller's fixed 4-cycle latency is not modelled: a change is visible
//! from the next instruction.
//!
//! `CPU_INT_CLEAR` acts as a pulse: writing a 1 clears that interrupt's edge
//! latch (it reads back the last value written).

/// Number of peripheral interrupt sources.
pub const SOURCE_COUNT: usize = 62;

/// Peripheral interrupt source numbers (ESP-IDF `ETS_*_SOURCE`).
pub mod source {
    /// GPIO.
    pub const GPIO: u8 = 16;
    /// GPIO non-maskable interrupt.
    pub const GPIO_NMI: u8 = 17;
    /// UART0.
    pub const UART0: u8 = 21;
    /// RTC core (includes the RTC watchdog interrupt).
    pub const RTC_CORE: u8 = 27;
    /// Timer group 0, timer 0.
    pub const TG0_T0: u8 = 32;
    /// Timer group 0, watchdog.
    pub const TG0_WDT: u8 = 33;
    /// Timer group 1, timer 0.
    pub const TG1_T0: u8 = 34;
    /// Timer group 1, watchdog.
    pub const TG1_WDT: u8 = 35;
    /// SYSTIMER comparator 0 (comparators 1 and 2 follow).
    pub const SYSTIMER_TARGET0: u8 = 37;
    /// SYSTIMER comparator 1.
    pub const SYSTIMER_TARGET1: u8 = 38;
    /// SYSTIMER comparator 2.
    pub const SYSTIMER_TARGET2: u8 = 39;
    /// Software interrupt 0 (`FROM_CPU_INTR0`, used by FreeRTOS).
    pub const FROM_CPU_INTR0: u8 = 50;
}

/// Register offsets from [`crate::map::INTERRUPT_BASE`].
pub mod reg {
    /// `*_INTR_MAP` of source `n` is at `MAP + 4 * n` (bits 4:0).
    pub const MAP: u32 = 0x000;
    /// Raw level of sources 0 to 31.
    pub const INTR_STATUS_0: u32 = 0x0f8;
    /// Raw level of sources 32 to 61.
    pub const INTR_STATUS_1: u32 = 0x0fc;
    /// Register clock gate (stored only).
    pub const CLOCK_GATE: u32 = 0x100;
    /// CPU interrupt enable, one bit per interrupt.
    pub const CPU_INT_ENABLE: u32 = 0x104;
    /// CPU interrupt type: 0 level, 1 edge.
    pub const CPU_INT_TYPE: u32 = 0x108;
    /// Write 1 to clear an edge interrupt's latch.
    pub const CPU_INT_CLEAR: u32 = 0x10c;
    /// Pending, enabled and unmasked interrupts.
    pub const CPU_INT_EIP_STATUS: u32 = 0x110;
    /// `CPU_INT_PRI_n` is at `CPU_INT_PRI_0 + 4 * n` (bits 3:0).
    pub const CPU_INT_PRI_0: u32 = 0x114;
    /// Priority threshold (bits 3:0).
    pub const CPU_INT_THRESH: u32 = 0x194;
    /// Version register.
    pub const DATE: u32 = 0x7fc;
}

/// Value of the `INTERRUPT_DATE` register.
pub const DATE: u32 = 0x0200_7210;

/// Interrupt matrix plus CPU interrupt controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntMatrix {
    map: [u8; SOURCE_COUNT],
    sources: u64,
    clock_gate: u32,
    enable: u32,
    edge: u32,
    clear: u32,
    latched: u32,
    inputs: u32,
    pri: [u8; 32],
    thresh: u8,
}

impl Default for IntMatrix {
    fn default() -> Self {
        Self::new()
    }
}

impl IntMatrix {
    /// Reset state: nothing routed, everything disabled.
    pub fn new() -> Self {
        Self {
            map: [0; SOURCE_COUNT],
            sources: 0,
            clock_gate: 1,
            enable: 0,
            edge: 0,
            clear: 0,
            latched: 0,
            inputs: 0,
            pri: [0; 32],
            thresh: 0,
        }
    }

    /// Set the raw levels of all sources (bit `n` = source `n`).
    pub fn set_sources(&mut self, levels: u64) {
        self.sources = levels & ((1 << SOURCE_COUNT) - 1);
        self.update_inputs();
    }

    /// Raw source levels.
    pub fn sources(&self) -> u64 {
        self.sources
    }

    /// CPU interrupt number source `src` is routed to (0 = none).
    pub fn route(&self, src: u8) -> u8 {
        self.map.get(usize::from(src)).copied().unwrap_or(0)
    }

    fn update_inputs(&mut self) {
        let mut inputs = 0u32;
        for (src, &line) in self.map.iter().enumerate() {
            if line != 0 && self.sources & (1 << src) != 0 {
                inputs |= 1 << line;
            }
        }
        // Edge-type interrupts latch on a rising input.
        self.latched |= inputs & !self.inputs & self.edge;
        self.inputs = inputs;
    }

    /// Pending, enabled and unmasked interrupts (`CPU_INT_EIP_STATUS`).
    pub fn eip_status(&self) -> u32 {
        let raw = (self.inputs & !self.edge) | (self.latched & self.edge);
        let mut eip = 0;
        for line in 1..32 {
            let p = self.pri[line];
            if raw & self.enable & (1 << line) != 0 && p != 0 && p >= self.thresh {
                eip |= 1 << line;
            }
        }
        eip
    }

    /// The line to present to the core: the pending interrupt with the
    /// highest priority (lowest number on a tie), as a `mip` bit mask.
    pub fn cpu_lines(&self) -> u32 {
        let eip = self.eip_status();
        let best = (1..32usize)
            .filter(|&l| eip & (1 << l) != 0)
            .max_by_key(|&l| (self.pri[l], std::cmp::Reverse(l)));
        best.map_or(0, |l| 1 << l)
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        Some(match off {
            o if o < reg::INTR_STATUS_0 => u32::from(self.map[(o / 4) as usize]),
            reg::INTR_STATUS_0 => self.sources as u32,
            reg::INTR_STATUS_1 => (self.sources >> 32) as u32,
            reg::CLOCK_GATE => self.clock_gate,
            reg::CPU_INT_ENABLE => self.enable,
            reg::CPU_INT_TYPE => self.edge,
            reg::CPU_INT_CLEAR => self.clear,
            reg::CPU_INT_EIP_STATUS => self.eip_status(),
            o if (reg::CPU_INT_PRI_0..reg::CPU_INT_THRESH).contains(&o) => {
                u32::from(self.pri[((o - reg::CPU_INT_PRI_0) / 4) as usize])
            }
            reg::CPU_INT_THRESH => u32::from(self.thresh),
            reg::DATE => DATE,
            _ => return None,
        })
    }

    /// Write a register. Returns `None` for offsets that do not exist.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            o if o < reg::INTR_STATUS_0 => self.map[(o / 4) as usize] = (v & 0x1f) as u8,
            reg::INTR_STATUS_0 | reg::INTR_STATUS_1 | reg::CPU_INT_EIP_STATUS | reg::DATE => {}
            reg::CLOCK_GATE => self.clock_gate = v & 1,
            reg::CPU_INT_ENABLE => self.enable = v & !1,
            reg::CPU_INT_TYPE => self.edge = v & !1,
            reg::CPU_INT_CLEAR => {
                self.clear = v;
                self.latched &= !v;
            }
            o if (reg::CPU_INT_PRI_0..reg::CPU_INT_THRESH).contains(&o) => {
                self.pri[((o - reg::CPU_INT_PRI_0) / 4) as usize] = (v & 0xf) as u8;
            }
            reg::CPU_INT_THRESH => self.thresh = (v & 0xf) as u8,
            _ => return None,
        }
        self.update_inputs();
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(m: &mut IntMatrix, src: u8, line: u32, pri: u32) {
        m.write(reg::MAP + 4 * u32::from(src), line).unwrap();
        m.write(reg::CPU_INT_PRI_0 + 4 * line, pri).unwrap();
        let en = m.read(reg::CPU_INT_ENABLE).unwrap();
        m.write(reg::CPU_INT_ENABLE, en | (1 << line)).unwrap();
    }

    #[test]
    fn register_layout_matches_the_trm() {
        let mut m = IntMatrix::new();
        // SYSTIMER_TARGET0_INT_MAP is source 37: offset 0x94.
        m.write(0x94, 0xff).unwrap();
        assert_eq!(m.route(source::SYSTIMER_TARGET0), 0x1f, "5-bit field");
        assert_eq!(m.read(0x0f4), Some(0), "last map register (source 61)");
        m.write(0x190, 0xff).unwrap();
        assert_eq!(m.read(0x190), Some(0xf), "CPU_INT_PRI_31 is 4 bits");
        m.write(reg::CPU_INT_THRESH, 0x13).unwrap();
        assert_eq!(m.read(reg::CPU_INT_THRESH), Some(3));
        m.write(reg::CPU_INT_ENABLE, 0xffff_ffff).unwrap();
        assert_eq!(
            m.read(reg::CPU_INT_ENABLE),
            Some(0xffff_fffe),
            "interrupt 0 does not exist"
        );
        assert_eq!(m.read(reg::DATE), Some(DATE));
        assert_eq!(m.read(0x198), None);
        m.set_sources(1 << 40 | 1 << 3);
        assert_eq!(m.read(reg::INTR_STATUS_0), Some(1 << 3));
        assert_eq!(m.read(reg::INTR_STATUS_1), Some(1 << 8));
    }

    #[test]
    fn level_interrupt_follows_its_source() {
        let mut m = IntMatrix::new();
        route(&mut m, source::SYSTIMER_TARGET0, 1, 1);
        assert_eq!(m.cpu_lines(), 0);
        m.set_sources(1 << source::SYSTIMER_TARGET0);
        assert_eq!(m.cpu_lines(), 1 << 1);
        assert_eq!(m.read(reg::CPU_INT_EIP_STATUS), Some(1 << 1));
        m.set_sources(0);
        assert_eq!(m.cpu_lines(), 0, "level: drops with the source");
    }

    #[test]
    fn unrouted_disabled_and_priority_zero_never_fire() {
        let mut m = IntMatrix::new();
        m.set_sources(u64::MAX);
        assert_eq!(m.cpu_lines(), 0, "nothing routed");
        m.write(reg::MAP + 4 * 5, 3).unwrap();
        assert_eq!(m.cpu_lines(), 0, "routed but disabled");
        m.write(reg::CPU_INT_ENABLE, 1 << 3).unwrap();
        assert_eq!(m.cpu_lines(), 0, "priority 0 is never delivered");
        m.write(reg::CPU_INT_PRI_0 + 12, 1).unwrap();
        assert_eq!(m.cpu_lines(), 1 << 3);
    }

    #[test]
    fn threshold_masks_lower_priorities() {
        let mut m = IntMatrix::new();
        route(&mut m, 10, 4, 3);
        m.set_sources(1 << 10);
        m.write(reg::CPU_INT_THRESH, 3).unwrap();
        assert_eq!(m.cpu_lines(), 1 << 4, "priority equal to the threshold passes");
        m.write(reg::CPU_INT_THRESH, 4).unwrap();
        assert_eq!(m.cpu_lines(), 0, "below the threshold: masked");
        assert_eq!(m.read(reg::CPU_INT_EIP_STATUS), Some(0));
    }

    #[test]
    fn highest_priority_wins_then_lowest_number() {
        let mut m = IntMatrix::new();
        route(&mut m, 1, 11, 2);
        route(&mut m, 2, 5, 2);
        route(&mut m, 3, 20, 7);
        m.set_sources(0b1110);
        assert_eq!(m.cpu_lines(), 1 << 20);
        m.set_sources(0b0110);
        assert_eq!(m.cpu_lines(), 1 << 5, "tie: lowest interrupt number");
        assert_eq!(m.read(reg::CPU_INT_EIP_STATUS), Some((1 << 5) | (1 << 11)));
    }

    #[test]
    fn sources_sharing_an_interrupt_are_ored() {
        let mut m = IntMatrix::new();
        route(&mut m, 32, 2, 1);
        route(&mut m, 34, 2, 1);
        m.set_sources(1 << 34);
        assert_eq!(m.cpu_lines(), 1 << 2);
        m.write(reg::MAP + 4 * 34, 0).unwrap();
        assert_eq!(m.cpu_lines(), 0, "unrouting removes the source");
    }

    #[test]
    fn edge_interrupt_latches_until_cleared() {
        let mut m = IntMatrix::new();
        route(&mut m, 7, 9, 1);
        m.write(reg::CPU_INT_TYPE, 1 << 9).unwrap();
        m.set_sources(1 << 7);
        m.set_sources(0);
        assert_eq!(m.cpu_lines(), 1 << 9, "latched after the pulse");
        m.write(reg::CPU_INT_CLEAR, 1 << 9).unwrap();
        assert_eq!(m.cpu_lines(), 0);
        m.write(reg::CPU_INT_CLEAR, 0).unwrap();
        // A level held high does not re-latch without a new rising edge.
        m.set_sources(1 << 7);
        m.write(reg::CPU_INT_CLEAR, 1 << 9).unwrap();
        m.set_sources(1 << 7);
        assert_eq!(m.cpu_lines(), 0);
        m.set_sources(0);
        m.set_sources(1 << 7);
        assert_eq!(m.cpu_lines(), 1 << 9);
        // Disabled interrupts keep their latch and fire once enabled.
        m.write(reg::CPU_INT_ENABLE, 0).unwrap();
        assert_eq!(m.cpu_lines(), 0);
        m.write(reg::CPU_INT_ENABLE, 1 << 9).unwrap();
        assert_eq!(m.cpu_lines(), 1 << 9);
    }
}
