# RFC 0004: Instruments and the measurement service (engine E5)

- **Status:** Draft
- **Author:** Stefan Norval \<sjnorval@gmail.com\>
- **Date:** 2026-11-25
- **Tracking issue:** [#49](https://github.com/sjnorval/Ohmforge/issues/49)

## Summary

Measuring becomes an engine service and instruments become things placed on a
circuit. This RFC fixes three things the later E5 slices build on:

1. the **signal names** the service samples, which are the `[[probe]] signal` strings of RFC 0003;
2. an optional **`[[instrument]]` table** in `*.circuit.toml` (`format = 1`, additive);
3. the **load-element API** a real-world instrument profile uses to load the circuit the way a real meter, probe or burden resistance does.

The UI-free service core (`of_instruments::{Signal, Probeable, MeasurementService}`,
the sample rings and the statistics) lands first in E5a and needs no format or API change.

## Motivation

Until E5 the desktop reads the demos through per-demo glue in `of-ui`:

- the LED current is `(v(gpio8) − v(led_a)) / 330`;
- the meters read a node and branch list filled in by hand for each `DemoKind`;
- the PSU current is averaged in `SessionSim`.

So nothing in a new circuit can be measured without new Rust. The circuit file
already stores `[[probe]]` signals (RFC 0003) but nothing uses them.

Stefan's requirement is that readings "match and resolve as close to real world as
possible". A real meter loads the circuit (10 MΩ input, burden voltage), has finite
resolution and ranges, and averages over its integration time. These effects need
a defined place in the model, with datasheet-sourced numbers only.

See `docs/design/engine-components.md` §3f and the E5 plan.

## Guide-level explanation

A circuit file names what to watch and which instruments to show:

```toml
[[probe]]
name = "LED current"
signal = "i(R1)"

[[instrument]]
ref = "XMM1"
kind = "multimeter"
profile = "ideal"
mode = "dc_a"
probes = ["LED current"]
at = [300, 160]

[[instrument]]
ref = "XSC1"
kind = "scope"
profile = "ideal"
probes = ["LED current", "v(vdd)"]
params = { time_div = "1ms", trigger = "rise", level = 1.0 }
```

- `profile = "ideal"` (the default) reads exactly what the solver has and never changes the circuit, so trace parity is unaffected.
- `profile = "fluke.87v"` picks a real-world profile from `parts/instruments/`. Every figure in it carries its datasheet citation.

`ohmforge run file.circuit.toml --log out.csv` (or `.vcd`) logs the probes and instruments.

In Rust (E5a):

```rust
let mut svc = MeasurementService::new(SamplePlan::EveryAndEvents(50e-6));
let led = svc.watch_default("i(R1)".parse()?);
// after each engine step:
svc.on_step(&mut session, ledc_edge_hit)?;
let mean = stats::average_periods(&svc.channel(led).unwrap().ring.to_vec(), /* … */);
```

## Reference-level design

### Signal names

| Form | `Signal` | Meaning | Source (E5b) |
|---|---|---|---|
| `v(net)` | `NetVolts` | net voltage to ground (V) | analog solution |
| `v(a,b)` | `DiffVolts` | `V(a) − V(b)` (V) | analog solution |
| `i(R1)` | `PinCurrent { pin: None }` | current into pin 1 of a two-pin part (A) | `Component::terminal_currents` |
| `i(U1.3V3)` | `PinCurrent { pin: Some }` | current into the named pin (A) | `terminal_currents`, pin order of `info()` |
| `r(PSU1.volts)` | `Reading` | a named component reading | `Component::readings` |
| `d(U1.GPIO8)` | `Digital` | pin state 0/1 with exact edge times | the component's own engine |

Grammar rules:

- Kind letters are case-insensitive, and spaces around names are ignored.
- Names are non-empty and contain none of `(`, `)`, `,`, `.`.
- The canonical form is lower-case kind with no spaces, for example `v(gpio2,0)`. Parse and print round trip.
- Errors are `Shape`, `Kind` or `Name`.
- `circuit check` (E5e) reports signals that do not resolve, with line and column.

### Measurement service (E5a)

- `Probeable` is what a running circuit offers: `now_secs()` and `sample(&Signal) -> Result<f64, ProbeError>`. `Session` implements it in `of-instruments` (decision D4: `of-instruments` depends on `of-engine`, never the reverse).
- `SamplePlan::Every(step)` or `EveryAndEvents(step)`. The grid is today's `SampleClock`; events are exact instants the run loop reports, LEDC edges today.
- `MeasurementService::watch(signal, window)` returns a `ChannelId`. Watching the same signal twice shares the channel, and ids are never reused.
- `on_step` samples every channel at the same instant, or none (a failed probe records nothing). `record` does the same without consulting the grid, for a run loop that keeps its own clock (the desktop's LEDC-edge loop, E5b), and `set_visible_window` re-sizes every ring and the grid together.
- `Session` as `Probeable` (E5b): `v(net)` reads the solution, with the ground net (`0` or `gnd`) at 0 V when the solver does not name it; `i(R1)` needs a two-pin part; `i(REF.pin)` uses the pin order of `info()`; `r(..)` searches `readings`; `d(..)` is `NotAnalog` (the logic analyser reads edges, E5g). Unknown parts, pins, nets or readings are `Unresolved`.
- Rings are `TraceBuf`: bounded by `MAX_TRACE_SAMPLES`, with retention tied to the window.
- Statistics are pure functions over `(t, value)` slices:
  - `average_window` and `average_periods`, the period-aligned mean from #45;
  - `rising_edges` and `pwm_duty_freq`;
  - `min_max`;
  - `rms`, time-weighted and holding each sample until the next.

### `[[instrument]]` table (E5e)

| Key | Type | Required | Meaning |
|---|---|---|---|
| `ref` | string | yes | unique, `X` prefix by convention; never in the BOM |
| `kind` | `multimeter` \| `scope` \| `logic` \| `logger` \| `uart` | yes | |
| `profile` | string | no (`"ideal"`) | `ideal` or a `parts/instruments/<vendor>.<model>` id |
| `mode` | string | multimeter only | `dc_v`, `dc_a` (later `ac_v`, `ac_a`, `ohms`) |
| `probes` | array of strings | yes | probe names from `[[probe]]`, or inline signals |
| `at` | `[x, y]` | no | schematic placement of the instrument symbol |
| `params` | table | no | kind-specific settings (time/div, trigger, thresholds, file) |

Format and compatibility rules:

- The table is optional and additive, so `format` stays 1 (decision D3). Files without it are unchanged.
- Canonical order puts instruments after probes, one blank line between tables.
- Unknown keys are errors, as elsewhere in RFC 0003.
- Instruments are drawn in the existing schematic style. They are not part of wire routing, so the wiring goldens are unaffected.

### Real-world profiles

Profiles live at `parts/instruments/<vendor>.<model>/instrument.toml` (CC-BY-SA-4.0). Every numeric field has a `source` (document, revision, table or page). The reference profiles (decision D5) are:

- Fluke 87V multimeter;
- Rigol DS1054Z scope;
- Saleae Logic 8 logic analyser;
- Rigol DP832 PSU readback.

The DP832 figures are verified against its datasheet before E5f uses them.

A profile models only deterministic effects:

- loading or burden;
- display resolution, counts, ranges and autorange with `OL`;
- integration and averaging;
- scope vertical quantisation;
- logic thresholds and sample clock.

The accuracy specification is shown as a ± band next to the reading and is never applied as a random error (decision D2).

### Load-element API (E5f)

An instrument with a real profile returns the elements it adds to the circuit:

```rust
pub enum LoadElement {
    /// Shunt from `net` to `reference` (meter or probe input): R in parallel with C.
    Shunt { net: String, reference: String, ohms: f64, farads: f64 },
    /// Series element at a component pin (ammeter burden): the pin is moved
    /// to a new internal net and `ohms` joins it to the original net.
    Series { refdes: String, pin: String, ohms: f64 },
}
```

The engine elaborates these as ordinary analog devices with names under the
instrument's ref, for example `XMM1.rin`, so they show in the solution like any part.
Shunts are added between steps. A series element changes the topology, so it is
applied before the session starts, or through a `Session` re-elaboration that
keeps the time and state. The exact `Session` method is settled in E5f, and it is
the only component-API or engine change in E5. Ideal profiles return no elements.
Trace parity runs only ideal profiles.

## Determinism and wasm32 impact

- The service is deterministic:
  - sampling decisions come only from simulation time;
  - channels are a `Vec` in watch order, with no `HashMap` iteration;
  - no wall clock is used.
- With ideal profiles the recorded samples are the solver's values. Trace parity goldens do not change, and E5a re-blesses with an empty diff.
- `of-instruments` stays free of OS and thread dependencies and builds for `wasm32-unknown-unknown`. `Probeable` lets the web app supply its own source.

## Licensing impact

- There are no new dependencies (`thiserror` and `of-core` are already used).
- Instrument profiles are parts data under CC-BY-SA-4.0. They hold facts taken from vendor datasheets with citations; no vendor text or artwork is copied.

## Alternatives

- **Keep the meters in `of-ui` and add per-demo readouts.** Rejected: nothing would work on a new circuit, and the CLI and web app would disagree with the desktop.
- **Ammeter as a sense resistor always inserted.** Rejected as the default, because it changes every circuit and its goldens. The ideal ammeter reads `terminal_currents` and needs no insertion. A real burden is opt-in.
- **Random error within the accuracy spec.** Rejected: the value would be invented. The band shows the uncertainty honestly.
- **Bumping to `format = 2`.** Not needed for an optional, additive table.

## Unresolved questions

- The exact `Session` API for a series load element at run time (E5f): re-elaboration versus a pre-placed zero-ohm element per ammeter point.
- Whether the scope's 8-bit step uses exactly 8 divisions as full scale on the DS1054Z. It has to be checked against the datasheet before E5f.
- AC and resistance modes for the multimeter are left to a later RFC addendum.
