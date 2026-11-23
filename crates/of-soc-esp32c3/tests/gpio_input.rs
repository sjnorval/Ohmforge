//! Input timing in the kernel: GPIO input events that another engine
//! produces mid-slice (as the analog solver does at a threshold crossing)
//! reach a polling hart before it reads `GPIO_IN`, not a slice later.

use of_core::{Engine, EngineError, Kernel, SimTime, Stop, SyncCtx};
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, input_code};

const CODE: u32 = 0x4038_0000;

fn i_type(imm: i32, rs1: u32, f3: u32, rd: u32, op: u32) -> u32 {
    ((imm as u32 & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op
}
fn lui(rd: u32, imm20: u32) -> u32 {
    (imm20 << 12) | (rd << 7) | 0x37
}
fn addi(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(imm, rs1, 0, rd, 0x13)
}
fn srli(rd: u32, rs1: u32, sh: i32) -> u32 {
    i_type(sh, rs1, 5, rd, 0x13)
}
fn lw(rd: u32, rs1: u32, imm: i32) -> u32 {
    i_type(imm, rs1, 2, rd, 0x03)
}
fn sw(rs2: u32, rs1: u32, imm: i32) -> u32 {
    let imm = imm as u32;
    ((imm >> 5) << 25) | (rs2 << 20) | (rs1 << 15) | (2 << 12) | ((imm & 31) << 7) | 0x23
}
fn jal0(off: i32) -> u32 {
    let o = off as u32;
    (((o >> 20) & 1) << 31) | (((o >> 1) & 0x3ff) << 21) | (((o >> 11) & 1) << 20) | (((o >> 12) & 0xff) << 12) | 0x6f
}

/// A chip running `prog` from IRAM.
fn chip(prog: &[u32]) -> Esp32c3 {
    let mut soc = Esp32c3::new();
    let bytes: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();
    soc.bus.load(CODE, &bytes).unwrap();
    soc.hart.set_pc(CODE);
    soc
}

/// GPIO8 = GPIO9, as fast as possible.
fn mirror() -> Esp32c3 {
    chip(&[
        lui(5, 0x60004),   // t0 = GPIO
        addi(6, 0, 0x100), // GPIO8
        sw(6, 5, 0x24),    // ENABLE_W1TS
        lw(7, 5, 0x3c),    // loop: t2 = GPIO_IN
        srli(7, 7, 1),     // bit 9 -> bit 8
        sw(7, 5, 0x04),    // GPIO_OUT
        jal0(-12),
    ])
}

/// Sends input events to engine 0 at fixed times, discovering each one only
/// when it advances past it (like an analog threshold crossing): it stops
/// there, so the slice ends at the event.
struct Stimulus {
    t: SimTime,
    events: Vec<(SimTime, u32)>,
}

impl Engine for Stimulus {
    fn name(&self) -> &str {
        "stimulus"
    }
    fn local_time(&self) -> SimTime {
        self.t
    }
    fn is_quiescent(&self) -> bool {
        // Pending mid-slice discoveries act like analog threshold crossings.
        self.events.is_empty()
    }
    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        if let Some(&(at, code)) = self.events.first().filter(|e| e.0 <= limit) {
            self.events.remove(0);
            cx.schedule(at, 0, code);
            self.t = at.max(SimTime(cx.now().0 + 1));
        } else {
            self.t = limit;
        }
        Ok(self.t)
    }
}

fn kernel(cpu: Esp32c3Engine, events: Vec<(SimTime, u32)>) -> Kernel {
    let mut k = Kernel::new(SimTime::from_us(50));
    k.add_engine(Box::new(cpu));
    k.add_engine(Box::new(Stimulus {
        t: SimTime::ZERO,
        events,
    }));
    k
}

fn edges_of(k: &mut Kernel, pin: u8) -> Vec<(SimTime, Option<bool>)> {
    let soc = &k.engine_as::<Esp32c3Engine>(of_core::EngineId(0)).unwrap().soc;
    soc.pin_edges()
        .iter()
        .filter(|e| e.pin == pin)
        .map(|e| (e.at, e.out.level))
        .collect()
}

#[test]
fn polling_reads_inputs_at_the_instant_of_the_load() {
    let mut cpu = Esp32c3Engine::new("esp32c3", mirror());
    cpu.link_input(9, false);
    let press = [SimTime(10_000_123), SimTime(23_456_789), SimTime(23_500_000)];
    let events = press
        .iter()
        .zip([true, false, true])
        .map(|(&t, l)| (t, input_code(9, l)))
        .collect();
    let mut k = kernel(cpu, events);
    let end = SimTime::from_us(60);
    assert_eq!(k.run_until(end).unwrap(), Stop::Reached(end));
    let edges = edges_of(&mut k, 8);
    // Enable drives low first; then one edge per input change.
    assert_eq!(edges[0].1, Some(false));
    let rest = &edges[1..];
    assert_eq!(
        rest.iter().map(|e| e.1).collect::<Vec<_>>(),
        [Some(true), Some(false), Some(true)]
    );
    for (&(t, _), &at) in rest.iter().zip(&press) {
        // Within one pass of the loop (four instructions, two of them
        // peripheral accesses) at 160 MHz.
        let lag = t.0.checked_sub(at.0).expect("output after the input");
        assert!(lag <= 100_000, "lag {lag} ps");
    }
}

#[test]
fn events_for_unlinked_pins_are_ignored() {
    let cpu = Esp32c3Engine::new("esp32c3", mirror());
    let mut k = kernel(cpu, vec![(SimTime::from_us(5), input_code(9, true))]);
    k.run_until(SimTime::from_us(20)).unwrap();
    // GPIO9 has its reset pull-up and nothing outside: GPIO8 follows it
    // high once and stays there.
    let edges = edges_of(&mut k, 8);
    assert_eq!(edges.iter().map(|e| e.1).collect::<Vec<_>>(), [Some(false), Some(true)]);
    assert!(edges[1].0 < SimTime::from_us(1));
}

#[test]
fn max_lead_bounds_how_far_a_busy_core_runs_ahead() {
    let busy = || chip(&[jal0(0)]);
    let mut skews = Vec::new();
    for (lead, bound) in [
        (None, SimTime::from_us(50)),
        (Some(SimTime::from_us(2)), SimTime::from_us(2)),
    ] {
        let mut cpu = Esp32c3Engine::new("esp32c3", busy());
        cpu.link_input(9, false);
        cpu.set_max_lead(lead);
        let mut k = kernel(cpu, vec![(SimTime(7_000_001), input_code(9, true))]);
        k.run_until(SimTime::from_us(100)).unwrap();
        let skew = k.stats().max_skew;
        assert!(skew <= bound, "{lead:?}: skew {skew:?}");
        skews.push(skew);
    }
    // Unbounded, the core is well ahead when the event arrives.
    assert!(skews[0] > SimTime::from_us(2), "{skews:?}");
}
