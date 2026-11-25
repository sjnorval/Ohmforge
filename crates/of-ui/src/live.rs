//! Simulation worker thread: the kernel never ticks on the UI thread.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use of_components::BounceSettings;
use of_soc_esp32c3::UartByte;

use crate::kind::DemoKind;
use of_instruments::{Multimeter, PartPins, Reading};

use crate::session_sim::{SessionSim, StartOptions};
use crate::supply::{SupplySettings, SupplyStatus};
use crate::trace::{self, DEFAULT_VISIBLE_WINDOW};

/// Slowest playback speed (simulation-seconds per wall-second).
pub(crate) const MIN_SPEED: f64 = 1e-6;
/// Fastest playback speed.
pub(crate) const MAX_SPEED: f64 = 10.0;

pub(crate) fn clamp_speed(s: f64) -> f64 {
    s.clamp(MIN_SPEED, MAX_SPEED)
}

/// Book `sim_adv / speed` wall time against `anchor` and return a park of at
/// most 16 ms, so very low speeds do not busy-spin and the worker can still
/// take commands.
fn park_after_slice(sim_adv: f64, speed: f64, now: Instant, anchor: &mut Instant) -> Option<Duration> {
    let speed = speed.max(MIN_SPEED);
    let secs = sim_adv / speed;
    if secs.is_finite() && secs > 0.0 {
        *anchor = anchor.checked_add(Duration::from_secs_f64(secs)).unwrap_or(*anchor);
    }
    anchor
        .checked_duration_since(now)
        .map(|d| d.min(Duration::from_millis(16)))
}

/// Commands the UI sends to the worker.
pub(crate) enum Command {
    SetKind(DemoKind),
    SetSpeed(f64),
    SetPlaying(bool),
    Reset,
    SetPot(f64),
    SetButton(bool),
    SetBounce(BounceSettings),
    SetSupply(SupplySettings),
    PushUart(Vec<u8>),
    /// Visible scope span in seconds (time/div x divisions).
    SetScopeWindow(f64),
    /// The placed meters (id, meter); replaces the previous set (E5c).
    SetMeters(Vec<(u64, Multimeter)>),
    Shutdown,
}

/// Latest values the UI can paint.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub kind: DemoKind,
    pub booting: bool,
    pub error: Option<String>,
    pub time: f64,
    pub playing: bool,
    pub speed: f64,
    /// Achieved simulation-seconds per wall-second (1.0 = realtime).
    pub rt_factor: f64,
    pub lagging: bool,
    pub pot: f64,
    pub button: bool,
    pub bounce: BounceSettings,
    pub led_ma: f64,
    pub led_instant: f64,
    pub wiper_v: Option<f64>,
    pub button_v: Option<f64>,
    pub pwm_duty: Option<f64>,
    pub pwm_freq: Option<f64>,
    pub adc: Option<(u32, u32)>,
    pub supply: SupplySettings,
    pub supply_status: SupplyStatus,
    pub led_trace: Vec<(f64, f64)>,
    pub uart: Vec<UartByte>,
    pub uart_baud: Option<f64>,
    /// Every part's pins and nets, for placing meters (E5c).
    pub layout: Vec<PartPins>,
    /// Each placed meter's reading, by meter id.
    pub meter_readings: Vec<(u64, Reading)>,
    /// Bumped on every boot/reset so the UI can drop stale UART.
    pub generation: u64,
}

impl Snapshot {
    fn empty(kind: DemoKind, supply: SupplySettings) -> Self {
        Self {
            kind,
            booting: false,
            error: None,
            time: 0.0,
            playing: false,
            speed: 1.0,
            rt_factor: 0.0,
            lagging: false,
            pot: 0.5,
            button: false,
            bounce: BounceSettings::default(),
            led_ma: 0.0,
            led_instant: 0.0,
            wiper_v: None,
            button_v: None,
            pwm_duty: None,
            pwm_freq: None,
            adc: None,
            supply,
            supply_status: SupplyStatus::default(),
            led_trace: Vec::new(),
            uart: Vec::new(),
            uart_baud: None,
            layout: Vec::new(),
            meter_readings: Vec::new(),
            generation: 0,
        }
    }
}

/// UI-side handle to the live worker.
pub(crate) struct LiveSession {
    tx: Sender<Command>,
    shared: Arc<Mutex<Snapshot>>,
    thread: Option<JoinHandle<()>>,
}

impl LiveSession {
    /// Start `kind`, built from its embedded circuit file ([`SessionSim`]).
    pub(crate) fn start(kind: DemoKind) -> Self {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Snapshot::empty(kind, SupplySettings::default())));
        let shared_w = Arc::clone(&shared);
        let thread = thread::spawn(move || worker(rx, shared_w, kind));
        Self {
            tx,
            shared,
            thread: Some(thread),
        }
    }

    pub(crate) fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Worker {
    kind: DemoKind,
    sim: Result<SessionSim, String>,
    playing: bool,
    speed: f64,
    pot: f64,
    button: bool,
    bounce: BounceSettings,
    supply: SupplySettings,
    rt_factor: f64,
    lagging: bool,
    booting: bool,
    generation: u64,
    scope_visible: f64,
    meters: Vec<(u64, Multimeter)>,
}

impl Worker {
    fn boot(&mut self, kind: DemoKind) {
        self.kind = kind;
        self.booting = kind.needs_rom();
        self.playing = false;
        self.button = false;
        if !kind.has_pot() {
            self.pot = 0.5;
        }
        self.sim = Err("booting".into());
    }

    /// Build `kind` from its embedded circuit file (E4d: the only path).
    fn finish_boot(&mut self) {
        let opts = StartOptions {
            supply: self.supply,
            position: self.pot,
            ..StartOptions::default()
        };
        self.sim = SessionSim::start(self.kind, opts);
        if self.kind.has_button()
            && let Ok(s) = &mut self.sim
            && let Err(e) = s.set_bounce(self.bounce)
        {
            self.sim = Err(e);
        }
        self.booting = false;
        self.playing = self.sim.is_ok();
        self.apply_scope_window();
        self.apply_meters();
        self.generation = self.generation.saturating_add(1);
        self.rt_factor = 0.0;
        self.lagging = false;
    }

    fn time(&self) -> f64 {
        self.sim.as_ref().map_or(0.0, SessionSim::time)
    }

    fn advance(&mut self, t: f64) -> Result<(), String> {
        match &mut self.sim {
            Ok(s) => s.advance(t),
            Err(e) => Err(e.clone()),
        }
    }

    fn apply_pot(&mut self) -> Result<(), String> {
        match &mut self.sim {
            Ok(s) if s.kind() == DemoKind::Dimmer => s.set_position(self.pot),
            _ => Ok(()),
        }
    }

    fn apply_button(&mut self) -> Result<(), String> {
        match &mut self.sim {
            Ok(s) if s.kind().has_button() => s.set_closed(self.button),
            _ => Ok(()),
        }
    }

    fn apply_bounce(&mut self) -> Result<(), String> {
        match &mut self.sim {
            Ok(s) if s.kind().has_button() => s.set_bounce(self.bounce),
            _ => Ok(()),
        }
    }

    fn apply_supply(&mut self) -> Result<(), String> {
        match &mut self.sim {
            Ok(s) => s.set_supply(self.supply),
            Err(_) => Ok(()),
        }
    }

    fn push_uart(&mut self, bytes: &[u8]) {
        if let Ok(s) = &mut self.sim {
            s.push_uart_rx(bytes);
        }
    }

    /// Sample the placed meters' signals (and only those).
    fn apply_meters(&mut self) {
        if let Ok(s) = &mut self.sim {
            let keep: Vec<Multimeter> = self.meters.iter().map(|(_, m)| m.clone()).collect();
            s.retain_meters(&keep);
            for m in &keep {
                s.watch_meter(m);
            }
        }
    }

    fn apply_scope_window(&mut self) {
        let visible = self.scope_visible.max(trace::visible_window(trace::TIME_DIVS[0]));
        if let Ok(s) = &mut self.sim {
            s.set_scope_window(visible);
        }
    }

    fn snapshot(&mut self) -> Snapshot {
        let mut snap = Snapshot::empty(self.kind, self.supply);
        snap.booting = self.booting;
        snap.playing = self.playing;
        snap.speed = self.speed;
        snap.rt_factor = self.rt_factor;
        snap.lagging = self.lagging;
        snap.pot = self.pot;
        snap.button = self.button;
        snap.bounce = self.bounce;
        snap.supply = self.supply;
        snap.generation = self.generation;
        match &mut self.sim {
            Err(_) if self.booting => {}
            Err(e) => snap.error = Some(e.clone()),
            Ok(s) => {
                file_snapshot(s, &mut snap);
                snap.meter_readings = self.meters.iter().map(|(id, m)| (*id, s.read_meter(m))).collect();
            }
        }
        snap
    }
}

/// A [`Snapshot`] of a running demo: the fields each kind fills.
fn file_snapshot(s: &mut SessionSim, snap: &mut Snapshot) {
    let kind = s.kind();
    snap.time = s.time();
    snap.supply_status = s.supply_status();
    snap.led_trace = s.trace();
    match kind {
        DemoKind::Dimmer | DemoKind::Fade => {
            snap.led_ma = s.average_ma();
            snap.led_instant = snap.led_ma;
            let (d, f) = s.pwm_duty_freq();
            snap.pwm_duty = d;
            snap.pwm_freq = f;
            if kind == DemoKind::Dimmer {
                snap.pot = s.position();
                snap.wiper_v = s.wiper_volts().ok();
                snap.adc = s.last_adc_uart();
            }
        }
        DemoKind::Blink => {
            snap.led_instant = s.current_ma().unwrap_or(0.0);
            snap.led_ma = s.average_ma();
            let (d, f) = s.pwm_duty_freq();
            snap.pwm_duty = d;
            snap.pwm_freq = f;
        }
        DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
            snap.led_instant = s.current_ma().unwrap_or(0.0);
            snap.led_ma = s.average_ma();
            snap.button = s.closed();
            snap.bounce = s.bounce();
            snap.button_v = s.button_volts().ok();
        }
    }
    snap.uart = s.uart_bytes().to_vec();
    snap.uart_baud = s.uart_baud();
    snap.layout = s.layout();
}

fn worker(rx: Receiver<Command>, shared: Arc<Mutex<Snapshot>>, start: DemoKind) {
    let mut w = Worker {
        kind: start,
        sim: Err("starting".into()),
        playing: false,
        speed: 1.0,
        pot: 0.5,
        button: false,
        bounce: BounceSettings::default(),
        supply: SupplySettings::default(),
        rt_factor: 0.0,
        lagging: false,
        booting: true,
        generation: 0,
        scope_visible: DEFAULT_VISIBLE_WINDOW,
        meters: Vec::new(),
    };
    // Publish a booting/starting snapshot, then build the first sim.
    publish(&shared, w.snapshot());
    w.boot(start);
    publish(&shared, w.snapshot());
    w.finish_boot();
    publish(&shared, w.snapshot());

    let mut anchor = Instant::now();
    loop {
        let timeout = if w.playing && w.sim.is_ok() {
            Duration::from_millis(0)
        } else {
            Duration::from_millis(16)
        };
        match rx.recv_timeout(timeout) {
            Ok(Command::Shutdown) => break,
            Ok(cmd) => {
                if !handle(&mut w, cmd, &shared) {
                    break;
                }
                if w.playing {
                    anchor = Instant::now();
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if w.playing && w.sim.is_ok() {
            let now = Instant::now();
            let wall = now.saturating_duration_since(anchor).as_secs_f64().min(0.05);
            let speed = w.speed.max(1e-6);
            let want = w.time() + wall * speed;
            let t0 = Instant::now();
            let sim0 = w.time();
            let deadline = t0 + Duration::from_millis(8);
            let mut err = None;
            while w.time() < want && Instant::now() < deadline {
                let step = (want - w.time()).min(200e-6);
                if let Err(e) = w.advance(w.time() + step) {
                    err = Some(e);
                    break;
                }
            }
            let wall_work = t0.elapsed().as_secs_f64().max(1e-9);
            let sim_adv = (w.time() - sim0).max(0.0);
            let capacity = sim_adv / wall_work;
            const ALPHA: f64 = 0.2;
            w.rt_factor = if w.rt_factor == 0.0 {
                capacity
            } else {
                ALPHA * capacity + (1.0 - ALPHA) * w.rt_factor
            };
            if let Some(e) = err {
                w.sim = Err(e);
                w.playing = false;
                publish(&shared, w.snapshot());
                continue;
            }
            if w.time() >= want - 1e-9 {
                w.lagging = false;
                if let Some(sleep) = park_after_slice(sim_adv, speed, Instant::now(), &mut anchor) {
                    thread::sleep(sleep);
                }
            } else {
                // Could not catch up in this slice: drop leftover debt so we
                // run as fast as we can without skipping simulated time.
                w.lagging = true;
                anchor = Instant::now();
            }
            publish(&shared, w.snapshot());
        }
    }
}

fn handle(w: &mut Worker, cmd: Command, shared: &Arc<Mutex<Snapshot>>) -> bool {
    match cmd {
        Command::Shutdown => return false,
        Command::SetKind(kind) => {
            if kind != w.kind {
                // Meters belong to one circuit.
                w.meters.clear();
                w.boot(kind);
                publish(shared, w.snapshot());
                w.finish_boot();
                publish(shared, w.snapshot());
            }
        }
        Command::Reset => {
            w.boot(w.kind);
            publish(shared, w.snapshot());
            w.finish_boot();
            publish(shared, w.snapshot());
        }
        Command::SetSpeed(s) => w.speed = clamp_speed(s),
        Command::SetScopeWindow(visible) => {
            w.scope_visible = visible.max(trace::visible_window(trace::TIME_DIVS[0]));
            w.apply_scope_window();
        }
        Command::SetPlaying(p) => w.playing = p && w.sim.is_ok(),
        Command::SetPot(p) => {
            w.pot = p.clamp(0.0, 1.0);
            if let Err(e) = w.apply_pot() {
                w.sim = Err(e);
                w.playing = false;
            }
            publish(shared, w.snapshot());
        }
        Command::SetButton(c) => {
            w.button = c;
            if let Err(e) = w.apply_button() {
                w.sim = Err(e);
                w.playing = false;
            }
            publish(shared, w.snapshot());
        }
        Command::SetBounce(b) => {
            w.bounce = b;
            if let Err(e) = w.apply_bounce() {
                w.sim = Err(e);
                w.playing = false;
            }
            publish(shared, w.snapshot());
        }
        Command::SetSupply(s) => {
            w.supply = s.sanitized();
            if let Err(e) = w.apply_supply() {
                w.sim = Err(e);
                w.playing = false;
            }
            publish(shared, w.snapshot());
        }
        Command::PushUart(bytes) => w.push_uart(&bytes),
        Command::SetMeters(meters) => {
            w.meters = meters;
            w.apply_meters();
            publish(shared, w.snapshot());
        }
    }
    true
}

fn publish(shared: &Arc<Mutex<Snapshot>>, snap: Snapshot) {
    if let Ok(mut g) = shared.lock() {
        *g = snap;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{DemoKind, MAX_SPEED, MIN_SPEED, SessionSim, StartOptions, clamp_speed, park_after_slice};

    fn report(name: &str, sim_s: f64, run: impl FnOnce()) {
        let t0 = Instant::now();
        run();
        let wall = t0.elapsed().as_secs_f64().max(1e-9);
        eprintln!(
            "RT capacity {name}: {sim_s:.3} s sim in {wall:.3} s wall ({:.1}× realtime)",
            sim_s / wall
        );
    }

    fn skip_without_rom(e: &impl std::fmt::Display) -> bool {
        let msg = e.to_string();
        if msg.contains("fetch-esp-rom-elfs") || msg.contains("not bundled") {
            assert!(
                std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
                "OF_REQUIRE_ESP_ROM is set but the ROM was not found: {msg}"
            );
            true
        } else {
            false
        }
    }

    /// Start `kind` from its circuit file, or `None` without the fetched ROM.
    fn file_sim(kind: DemoKind) -> Option<SessionSim> {
        match SessionSim::start(kind, StartOptions::default()) {
            Ok(sim) => Some(sim),
            Err(e) if skip_without_rom(&e) => None,
            Err(e) => panic!("{kind:?}: {e}"),
        }
    }

    fn report_counters(name: &str, before: crate::stats::PerfSnapshot, after: crate::stats::PerfSnapshot, wall: f64) {
        let d = before.delta(after);
        let rt = d.time / wall.max(1e-9);
        eprintln!(
            "RT {name}: {:.3} s sim in {wall:.3} s wall ({rt:.3}x realtime); analog accepted={} newton={} fact={} lte_rej={} bp={}; hart minstret={} mcycle={}; samples={}",
            d.time,
            d.accepted,
            d.newton_iterations,
            d.factorisations,
            d.rejected_lte,
            d.breakpoints,
            d.minstret,
            d.mcycle,
            d.samples
        );
    }

    #[test]
    fn realtime_capacity_report() {
        report("blink 20 ms", 0.020, || {
            file_sim(DemoKind::Blink).expect("blink").advance(0.020).unwrap();
        });
        const SPAN: f64 = 0.100;
        for (label, kind) in [
            ("blink 100 ms", DemoKind::Blink),
            ("dimmer 100 ms", DemoKind::Dimmer),
            ("fade 100 ms", DemoKind::Fade),
            ("button-poll 100 ms", DemoKind::ButtonPoll),
            ("button-irq 100 ms", DemoKind::ButtonIrq),
        ] {
            let Some(mut sim) = file_sim(kind) else { continue };
            let s0 = sim.perf_snapshot();
            let t0 = Instant::now();
            sim.advance(sim.time() + SPAN).unwrap();
            report_counters(label, s0, sim.perf_snapshot(), t0.elapsed().as_secs_f64());
        }
    }

    /// Dedicated dimmer realtime-factor report (release builds are the
    /// meaningful number; debug is accepted so CI still exercises the path).
    #[test]
    fn dimmer_realtime_factor_report() {
        const SPAN: f64 = 0.200;
        let Some(mut sim) = file_sim(DemoKind::Dimmer) else {
            return;
        };
        // Warm the ROM/boot path and the decode cache before timing.
        sim.advance(sim.time() + 0.020).unwrap();
        let s0 = sim.perf_snapshot();
        let t0 = Instant::now();
        sim.advance(sim.time() + SPAN).unwrap();
        let wall = t0.elapsed().as_secs_f64().max(1e-9);
        let d = s0.delta(sim.perf_snapshot());
        let rt = d.time / wall;
        eprintln!(
            "dimmer realtime factor: {rt:.3}× ({:.3} s sim in {wall:.3} s wall; minstret={} analog_steps={})",
            d.time, d.minstret, d.accepted
        );
        assert!(
            d.time > SPAN * 0.95,
            "expected about {SPAN} s of sim progress, got {}",
            d.time
        );
        assert!(rt > 0.0, "realtime factor must be positive");
        // Release floor from #44 / #61 (interpreter-bound; ~0.22× typical).
        #[cfg(not(debug_assertions))]
        assert!(
            rt > 0.15,
            "dimmer release realtime regressed below the #44 floor: {rt:.3}×"
        );
    }

    #[test]
    fn long_run_keeps_the_scope_bounded() {
        // button-irq sleeps in wfi, so several simulated minutes are cheap
        // after the analog hold optimisation.
        let Some(mut sim) = file_sim(DemoKind::ButtonIrq) else {
            return;
        };
        let cap = sim.trace_capacity();
        sim.advance(sim.time() + 180.0).unwrap();
        let n = sim.trace().len();
        assert!(n <= cap, "trace grew past the ring capacity: {n} > {cap}");
        assert!(n > 8, "trace should still hold a window of samples, got {n}");
        assert!(cap <= crate::trace::MAX_TRACE_SAMPLES);
    }

    /// Wait (up to 60 s wall) for `done`, returning the last snapshot.
    fn wait_for(live: &super::LiveSession, done: impl Fn(&super::Snapshot) -> bool) -> super::Snapshot {
        let t0 = Instant::now();
        loop {
            let snap = live.snapshot();
            // `Some("booting")` is the worker's placeholder until the boot ends.
            let failed = snap.error.as_deref().is_some_and(|e| e != "booting");
            if done(&snap) || failed || t0.elapsed() > Duration::from_secs(60) {
                return snap;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The live worker boots each demo from its circuit file, runs it, and
    /// switches demos.
    #[test]
    fn the_worker_runs_the_demos_from_their_circuit_files() {
        use super::Command;
        use crate::kind::DemoKind;

        let live = super::LiveSession::start(DemoKind::Blink);
        live.send(Command::SetSpeed(0.1));
        live.send(Command::SetPlaying(true));
        let snap = wait_for(&live, |s| s.time > 2e-3);
        assert_eq!(snap.error, None);
        assert!(snap.time > 2e-3, "blink stalled at {} s", snap.time);
        assert!(!snap.led_trace.is_empty());
        assert!(
            (3.2..3.31).contains(&snap.supply_status.volts),
            "{:?}",
            snap.supply_status
        );
        let mut parts: Vec<_> = snap.layout.iter().map(|p| p.refdes.as_str()).collect();
        parts.sort_unstable();
        assert_eq!(parts, ["D1", "PSU", "R1", "U1"], "{parts:?}");

        // A placed voltmeter reads the rail; an ammeter through PSU its current.
        let psu = of_instruments::candidates(&snap.layout, "vdd", "0")
            .unwrap()
            .into_iter()
            .find(|c| c.refdes == "PSU")
            .expect("PSU joins vdd and ground");
        live.send(Command::SetMeters(vec![
            (1, of_instruments::Multimeter::voltmeter("vdd", "0")),
            (2, of_instruments::Multimeter::ammeter(&psu)),
        ]));
        let t = snap.time;
        let snap = wait_for(&live, |s| s.time > t + 1e-3 && s.meter_readings.len() == 2);
        match &snap.meter_readings[..] {
            [
                (1, of_instruments::Reading::Volts(v)),
                (2, of_instruments::Reading::Amps { amps, part }),
            ] => {
                assert!((3.2..3.31).contains(v), "{v}");
                assert!((-30e-3..-15e-3).contains(amps), "{amps}");
                assert_eq!(part, "PSU");
            }
            other => panic!("{other:?}"),
        }

        live.send(Command::SetKind(DemoKind::ButtonIrq));
        let snap = wait_for(&live, |s| s.kind == DemoKind::ButtonIrq && !s.booting && s.time > 1e-3);
        if let Some(e) = &snap.error {
            assert!(skip_without_rom(e), "{e}");
            return;
        }
        assert!(snap.time > 1e-3, "button-irq stalled at {} s", snap.time);
        assert!(snap.button_v.is_some());
        assert!(snap.layout.iter().any(|p| p.refdes == "SW1"));
        assert!(snap.meter_readings.is_empty(), "meters are cleared with the circuit");
    }

    #[test]
    fn clamp_speed_includes_micro_realtime() {
        assert_eq!(clamp_speed(1e-9), MIN_SPEED);
        assert_eq!(clamp_speed(MIN_SPEED), MIN_SPEED);
        assert_eq!(clamp_speed(1.0), 1.0);
        assert_eq!(clamp_speed(100.0), MAX_SPEED);
    }

    #[test]
    fn slow_pacing_parks_instead_of_spinning() {
        let mut anchor = Instant::now();
        let now = anchor;
        // 1 us of sim at 1e-6x is 1 s of wall; a single park is capped at 16 ms.
        let sleep = park_after_slice(1e-6, 1e-6, now, &mut anchor);
        assert_eq!(sleep, Some(Duration::from_millis(16)));
        assert!(anchor.duration_since(now) >= Duration::from_millis(990));

        let now2 = Instant::now();
        let sleep2 = park_after_slice(0.0, 1e-6, now2, &mut anchor);
        assert!(sleep2.is_some(), "should park while still ahead of the clock");
        let d = sleep2.unwrap();
        assert!(d > Duration::ZERO);
        assert!(d <= Duration::from_millis(16));
    }
}
