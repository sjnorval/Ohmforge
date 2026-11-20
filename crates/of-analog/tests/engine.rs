//! The analog solver under the `of-core` kernel: slices, pin drivers,
//! threshold watches and breakpoints.

use of_analog::devices::{Capacitor, PinDriver, Resistor};
use of_analog::{AnalogEngine, Circuit, Edge, EngineConfig, Options, SetParam, analysis, netlist};
use of_core::{Engine, EngineError, Event, HOST, Kernel, SimTime, Stop, SyncCtx, ValueCode};

fn rc_deck() -> of_analog::netlist::Built {
    let deck = of_netlist::spice::parse_str(
        "rc\nV1 in 0 PULSE(0 5 100u 1u 1u 2m 4m)\nR1 in out 1k\nC1 out 0 1u\n.tran 10u 8m\n.end\n",
    )
    .unwrap();
    netlist::build(&deck).unwrap()
}

#[test]
fn sliced_run_matches_one_transient() {
    let mut b = rc_deck();
    let req = b.tran_request(&b.analyses[0]).unwrap();
    let reference = analysis::transient(&mut b.circuit, &b.options, &req).unwrap();

    let b = rc_deck();
    let cfg = EngineConfig {
        max_step: 10e-6,
        horizon: 8e-3,
        record: true,
        ..EngineConfig::default()
    };
    let eng = AnalogEngine::new("analog", b.circuit, b.options, cfg).unwrap();
    // An odd quantum so slice ends fall between the solver's own steps.
    let mut k = Kernel::new(SimTime::from_ns(7_300));
    let id = k.add_engine(Box::new(eng));
    assert_eq!(
        k.run_until(SimTime::from_ms(8)).unwrap(),
        Stop::Reached(SimTime::from_ms(8))
    );
    let eng = k.engine_as::<AnalogEngine>(id).unwrap();
    let w = eng.waveforms().unwrap();
    let mut worst: f64 = 0.0;
    for i in 0..=800 {
        let t = i as f64 * 10e-6;
        let d = (w.at("v(out)", t).unwrap() - reference.at("v(out)", t).unwrap()).abs();
        worst = worst.max(d);
    }
    // Both runs control the same truncation error; the sliced run only
    // adds extra (shorter) steps at slice ends.
    assert!(worst < 0.005 * 5.0, "worst difference {worst} V");
    assert!(eng.stats().breakpoints >= 4, "source corners still land exactly");
}

/// A stand-in for the CPU: toggles a GPIO by posting events to the analog
/// engine and records the threshold-crossing interrupts it receives.
struct FakeCpu {
    t: SimTime,
    analog: u32,
    toggles: Vec<(SimTime, u32)>,
    irqs: Vec<SimTime>,
}

impl Engine for FakeCpu {
    fn name(&self) -> &str {
        "cpu"
    }
    fn local_time(&self) -> SimTime {
        self.t
    }
    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        // "Firmware": write the GPIO register at the programmed times; each
        // write is a sync point that ends the CPU's quantum.
        if let Some(&(at, code)) = self.toggles.first()
            && at <= limit
        {
            self.toggles.remove(0);
            cx.schedule(at, self.analog, code);
            self.t = at;
            return Ok(at);
        }
        self.t = limit;
        Ok(limit)
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        self.irqs.push(ev.at);
        Ok(())
    }
}

const GPIO_LOW: u32 = 0;
const GPIO_HIGH: u32 = 1;
const IRQ: u32 = 9;

fn gpio_rc() -> Circuit {
    let mut c = Circuit::new();
    let pin = c.node("pin");
    let out = c.node("out");
    c.add(PinDriver::new("gpio", pin, 0, 0.0, 25.0)).unwrap();
    c.add(Resistor::new("r1", pin, out, 975.0)).unwrap();
    c.add(Capacitor::new("c1", out, 0, 1e-6, None)).unwrap();
    c
}

#[test]
fn gpio_drives_rc_and_threshold_interrupt_lands_on_the_crossing() {
    let cfg = EngineConfig {
        max_step: 20e-6,
        resolution: 1e-9,
        record: true,
        ..EngineConfig::default()
    };
    let mut eng = AnalogEngine::new("analog", gpio_rc(), Options::default(), cfg).unwrap();
    let set = |v: f64| {
        vec![SetParam {
            device: "gpio".into(),
            param: "v".into(),
            value: v,
        }]
    };
    eng.on_code(GPIO_HIGH, set(3.3));
    eng.on_code(GPIO_LOW, set(0.0));
    let mut k = Kernel::new(SimTime::from_us(50));
    let cpu = k.add_engine(Box::new(FakeCpu {
        t: SimTime::ZERO,
        analog: 1,
        toggles: vec![(SimTime::from_us(1000), GPIO_HIGH), (SimTime::from_us(6000), GPIO_LOW)],
        irqs: Vec::new(),
    }));
    eng.watch("v(out)", 1.65, Edge::Both, cpu.0, IRQ).unwrap();
    let analog = k.add_engine(Box::new(eng));
    assert_eq!(analog.0, 1);
    k.run_until(SimTime::from_ms(10)).unwrap();

    let tau = 1000.0 * 1e-6;
    let expect = [1e-3 + tau * 2f64.ln()];
    let irqs = k.engine_as::<FakeCpu>(cpu).unwrap().irqs.clone();
    assert_eq!(irqs.len(), 2, "{irqs:?}");
    // Rising: v = 3.3 (1 - exp(-t/tau)) reaches 1.65 V after tau ln 2.
    let rise = irqs[0].as_secs_f64();
    assert!(
        (rise - expect[0]).abs() < 1e-3 * tau,
        "rise at {rise}, expected {}",
        expect[0]
    );
    // Falling from v0 = 3.3 (1 - exp(-5)): reaches 1.65 V after tau ln(2 v0 / 3.3).
    let v0 = 3.3 * (1.0 - (-5.0f64).exp());
    let fall_expect = 6e-3 + tau * (2.0 * v0 / 3.3).ln();
    let fall = irqs[1].as_secs_f64();
    assert!(
        (fall - fall_expect).abs() < 1e-3 * tau,
        "fall at {fall}, expected {fall_expect}"
    );
    let eng = k.engine_as::<AnalogEngine>(analog).unwrap();
    // The recorded point at the crossing sits within the resolution after it.
    let w = eng.waveforms().unwrap();
    let i =
        w.x.iter()
            .position(|&t| (t - rise).abs() < 1e-12)
            .expect("a point at the crossing");
    let v = w.signal("v(out)").unwrap()[i];
    assert!((1.65..1.65 + 1e-4).contains(&v), "v(out) at the crossing = {v}");
    let v_end = eng.value("v(out)").unwrap();
    assert!((v_end - v0 * (-4.0f64).exp()).abs() < 1e-3, "v(out) at 10 ms = {v_end}");
    // The pin sinks the capacitor current: i = (0 - v(out)) / 1000 Ω.
    assert!((eng.value("i(gpio)").unwrap() + v_end / 1000.0).abs() < 1e-7);
}

#[test]
fn breakpoints_and_host_events_pause_the_analog_run() {
    let cfg = EngineConfig::default();
    let mut eng = AnalogEngine::new("analog", gpio_rc(), Options::default(), cfg).unwrap();
    eng.set("gpio", "v", 3.3).unwrap();
    assert!(eng.set("gpio", "nope", 1.0).is_err());
    assert!(eng.set("nobody", "v", 1.0).is_err());
    eng.watch("out", 2.0, Edge::Rising, HOST, 42).unwrap();
    let mut k = Kernel::new(SimTime::from_us(100));
    let a = k.add_engine(Box::new(eng));
    k.add_breakpoint(SimTime::from_us(500));
    assert_eq!(
        k.run_until(SimTime::from_ms(5)).unwrap(),
        Stop::Breakpoint(SimTime::from_us(500))
    );
    let v = k.engine_as::<AnalogEngine>(a).unwrap().value("v(out)").unwrap();
    let exact = 3.3 * (1.0 - (-0.5f64).exp());
    assert!((v - exact).abs() < 2e-3, "v(out) at 500 us = {v}, exact {exact}");
    let Stop::Host(ev) = k.run_until(SimTime::from_ms(5)).unwrap() else {
        panic!("expected the watch event")
    };
    let exact_t = 1e-3 * (3.3f64 / 1.3).ln();
    assert_eq!(ev.code, 42);
    assert!((ev.at.as_secs_f64() - exact_t).abs() < 1e-6, "crossing at {:?}", ev.at);
    assert_eq!(
        k.run_until(SimTime::from_ms(5)).unwrap(),
        Stop::Reached(SimTime::from_ms(5))
    );
}

/// A stand-in for an ADC: asks the analog engine for a voltage at set
/// times and records the replies.
struct FakeAdc {
    t: SimTime,
    analog: u32,
    requests: Vec<SimTime>,
    replies: Vec<(SimTime, u32)>,
}

impl Engine for FakeAdc {
    fn name(&self) -> &str {
        "adc"
    }
    fn local_time(&self) -> SimTime {
        self.t
    }
    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        if let Some(&at) = self.requests.first()
            && at <= limit
        {
            self.requests.remove(0);
            cx.schedule(at, self.analog, SAMPLE);
            self.t = at;
            return Ok(at);
        }
        self.t = limit;
        Ok(limit)
    }
    fn on_event(&mut self, ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        self.replies.push((ev.at, ev.code));
        Ok(())
    }
}

const SAMPLE: u32 = 0x2_0200;
const VOLTS: ValueCode = ValueCode {
    base: 0x8200_0000,
    mask: 0x00ff_ffff,
    lsb: 1e-6,
};

#[test]
fn sample_requests_are_answered_at_once_with_the_node_voltage() {
    let cfg = EngineConfig {
        max_step: 20e-6,
        ..EngineConfig::default()
    };
    let mut eng = AnalogEngine::new("analog", gpio_rc(), Options::default(), cfg).unwrap();
    eng.on_code(
        GPIO_HIGH,
        vec![SetParam {
            device: "gpio".into(),
            param: "v".into(),
            value: 3.3,
        }],
    );
    assert!(eng.sample_on(SAMPLE, "v(nope)", 0, VOLTS).is_err());
    eng.sample_on(SAMPLE, "v(out)", 0, VOLTS).unwrap();
    let mut k = Kernel::new(SimTime::from_us(50));
    let times: Vec<SimTime> = [500, 1500, 2000, 4000].map(SimTime::from_us).to_vec();
    let adc = k.add_engine(Box::new(FakeAdc {
        t: SimTime::ZERO,
        analog: 1,
        requests: times.clone(),
        replies: Vec::new(),
    }));
    k.add_engine(Box::new(eng));
    k.schedule(SimTime::from_us(1000), 1, GPIO_HIGH);
    k.run_until(SimTime::from_ms(5)).unwrap();

    let replies = k.engine_as::<FakeAdc>(adc).unwrap().replies.clone();
    assert_eq!(replies.len(), times.len(), "{replies:?}");
    let tau = 1e-3;
    for (&(at, code), &t) in replies.iter().zip(&times) {
        assert_eq!(at, t, "answered at the request time");
        let v = VOLTS.decode(code).unwrap();
        // v(out) = 3.3 (1 - exp(-(t - 1 ms)/tau)) after the pin goes high.
        let dt = t.as_secs_f64() - 1e-3;
        let expect = if dt <= 0.0 {
            0.0
        } else {
            3.3 * (1.0 - (-dt / tau).exp())
        };
        assert!((v - expect).abs() < 0.01 * 3.3, "at {t:?}: {v} V, expected {expect} V");
    }
}
