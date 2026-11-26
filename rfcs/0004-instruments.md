# RFC 0004: Instruments and the measurement service (engine E5)

- **Status:** Accepted (2026-11-25)
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
- Diode currents (E5c): every analog diode reports `i(name)`, its anode-to-cathode current at the solution (series-resistance current, or the junction current without `rs`), so `terminal_currents` of LEDs and diodes is real rather than 0. This is an analog-probe addition; the `Component` API (RFC 0001) is unchanged.
- Ideal multimeter (E5c, decided 2026-11-25; `of_instruments::meter`): voltmeter and ammeter readings both average over whole PWM periods of a reference channel (as a DMM's integrating ADC does). `candidates(parts, a, b)` lists every part with a pin on both nets (`0` and `gnd` both name ground). The ammeter reads from `a` toward `b`: into the pin on `a` when that is an end pin of the part (first or last in pin order), else out of the pin on `b`, so a potentiometer's `1`–`W` and `W`–`3` segments read their own current. A pair joined only through middle pins is not offered; when several parts join the two clicked points the user picks one from a chooser; resistance mode waits for E5f.
  Old-versus-new check (E5c, `the_multimeter_matches_the_old_meters_on_all_demos`, to 16 ms, the button closed at 8 ms): every voltmeter (both directions) and every ammeter the pre-E5c meters resolved, the old instantaneous values put through the same whole-period average; the PSU's old value was already averaged. Displayed strings equal and values within max(1 nA, 1 ppm) (1 nV for volts) on all five demos:

  | Demo | Voltmeters | Ammeters | PSU (vdd→0), old = new display | Offered only now |
  |---|---|---|---|---|
  | blink | 12 | 6 | −22.98 mA (bit-identical) | U1 on gpio8/vdd/0 pairs |
  | dimmer | 20 | 10 | −25.88 mA (last digit) | U1 on 6 pairs; RV1 on vdd/0 |
  | fade | 12 | 6 | −23.97 mA (last digit) | U1 on gpio8/vdd/0 pairs |
  | button | 20 | 8 | −27.01 mA (last digits) | U1 on 5 pairs |
  | button-irq | 20 | 8 | −20.02 mA (last digits) | U1 on 5 pairs |

  "Offered only now" pairs were "no single branch" before; Stefan accepted them as a behaviour change (2026-11-25), so the check compares only pairs the old meter resolved. With the old readout deleted, `the_meters_read_the_recorded_values_on_all_demos` pins all 122 recorded readings (displayed text and value, same tolerance).
- PSU face, scope and grid (E5d): `MeasurementService` owns the sample grid (`next_due`, `take_due`; zoom re-steps it in `set_visible_window`), and the desktop's blink, PWM and button run loops step to it, recording through the service (exact LEDC-edge samples on the PWM demos as before). `PsuReadout` (`of_instruments::psu`) watches `r(PSU.regulated_amps)` in a second service the loop records *after* each commit (the component's readings describe the committed step) and builds `of_components::psu::SupplyStatus` from `r(PSU.volts)`, `r(PSU.regulated_amps)` and `r(PSU.mode)`, with the face current averaged over whole PWM periods of the LED current. `Scope` (`of_instruments::scope`) shows one channel raw or as a `ScopeTrace::PeriodAverage` moving average (a scope option rather than a `Reading` variant); time/div, trigger and hold stay in the panel. The desktop's supply settings and status types are `of_components::psu`'s. Parity: on 31ff9f6 vs E5d, the PSU face (volts, amps, averaged amps, mode, rating) and `average_ma` of all five demos at 12 and 16 ms and of blink and dimmer in CC are bit-identical (`the_supply_face_and_led_average_are_unchanged_by_e5d`); the trace goldens (sample times and values), the idle-draw golden and the 122 pinned meter readings are unchanged.
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

#### E5e notes

Recorded when E5e landed (2026-11-26):

- **Signal names are case-sensitive**, as the running `Session` reads them. `circuit check` (`of_circuit::library::check_signals`) reports a name in the wrong case with the declared spelling, for example `v(VDD)` when the net is `vdd`.
- **Strict profiles and params.** Only `profile = "ideal"` resolves until the real-world profiles of E5f; any other id is a `circuit check` error. Only the scope has `params` so far (`time_div` a positive time, `trigger` `rise` or `fall`, `level` a number), and a param on any other kind is an error. A multimeter needs a `mode` and reads exactly one signal.
- **`ohmforge run --log`** writes raw ideal samples on the `--step` grid, read through `Probeable`. It does not apply the multimeter's whole-PWM-period averaging; that stays a display effect of the instrument.
- **Digital `d(..)` signals** are skipped by `--log` with a warning until the logic analyser (E5g), so the VCD holds `real` variables only for now.
- **Parity of the log** is checked against the CLI's own solver column: on blink the `LED current` column equals `i(R1)` from `ohmforge run --out` at the same sample times within max(1 nA, 1 ppm). It is not compared with the desktop's parity-golden grid.
- **Drawing instruments on the desktop schematic is deferred** to a later slice. The demo files carry `XMM1` and `XSC1` without `at`, and the desktop does not read the table yet.

### Real-world profiles

Profiles live at `parts/instruments/<vendor>.<model>/instrument.toml` (CC-BY-SA-4.0). Every numeric field has a `source` (document, revision, table or page). The reference profiles (decision D5) are:

- Fluke 87V multimeter;
- Rigol DS1054Z scope;
- Saleae Logic 8 logic analyser;
- Rigol DP832 PSU readback.

The DS1054Z and DP832 figures below were checked against Rigol's own PDFs on
2026-11-25 (verified); E5f's profiles copy them with these citations. Page numbers are the
printed ones.

Sources:

- [DS]: DS1000Z datasheet DSA20100-2020-09, <https://www.rigol.com/dam/global/downloads/brochures/en/data-sheet/oscilloscopes/DS1000Z_DataSheet_EN.pdf>;
- [PG]: MSO1000Z/DS1000Z Programming Guide PGA19108-1110 (Dec. 2015, software 00.04.03.SP2), <https://www.rigol.com/dam/global/downloads/brochures/en/program-guide/oscilloscopes/DS1000Z_ProgrammingGuide_EN.pdf>;
- [DP]: DP800 datasheet DSH03108-1110-202505 (May 2025), <https://www.rigol.com/dam/global/downloads/brochures/en/data-sheet/dc-powers/DP800_DataSheet_EN.pdf>.

Rigol DS1054Z (verified):

| Figure | Value | Source |
|---|---|---|
| Bandwidth (−3 dB) | DC to 50 MHz (single-shot too); calculated rise time 7 ns | [DS] pp. 6–7 |
| Sample rate | 1 GSa/s single channel, 500 MSa/s dual, 250 MSa/s three or four channels | [DS] p. 6 |
| Memory depth | 24 Mpts single channel, 12 Mpts dual, 6 Mpts three or four channels | [DS] p. 6 |
| Averaging | N = 2, 4, …, 1024 | [DS] p. 6 |
| Input impedance | (1 MΩ ±1 %) ‖ (15 pF ±3 pF) | [DS] p. 6 |
| Vertical resolution | 8 bits (High Resolution mode up to 12 bits) | [DS] p. 6 |
| Vertical scale | 1 mV/div to 10 V/div (1X probe) | [DS] p. 7 |
| Offset range | ±2 V at 1 mV/div to 499 mV/div; ±100 V at 500 mV/div to 10 V/div (1X probe) | [DS] p. 7 |
| DC gain accuracy | ±4 % of full scale below 10 mV/div; ±3 % of full scale at 10 mV/div and above | [DS] p. 7 |
| DC offset accuracy | ±0.1 div ± 2 mV ± 1 % of offset | [DS] p. 7 |
| Vertical code step | VerticalScale / 25 per code, code 127 at the reference (NORMal mode) | [PG] pp. 2-223, 2-224 |

Rigol DP832 PSU readback (verified; the DP832A has the same accuracy, [DP] p. 4):

| Figure | CH1 | CH2 | CH3 | Source |
|---|---|---|---|---|
| Rating | 0–30 V, 0–3 A | 0–30 V, 0–3 A | 0–5 V, 0–3 A; 195 W total | [DP] pp. 1, 7 |
| Programming accuracy, ±(% of output + offset) | V 0.05 % + 20 mV; I 0.2 % + 5 mA | V 0.05 % + 20 mV; I 0.2 % + 5 mA | V 0.1 % + 5 mV; I 0.2 % + 5 mA | [DP] p. 7 |
| Readback accuracy, ±(% of output + offset) | V 0.05 % + 10 mV; I 0.15 % + 5 mA | V 0.05 % + 10 mV; I 0.15 % + 5 mA | V 0.1 % + 5 mV; I 0.15 % + 5 mA | [DP] p. 7 |
| Resolution, standard (programming / readback / display) | V 10 / 10 / 10 mV; I 1 / 1 / 10 mA | same | same | [DP] p. 8 |
| Resolution, high-resolution option | V 1 / 0.1 / 1 mV; I 1 / 0.1 / 1 mA | same | same | [DP] p. 8 |

All DP832 channels: ripple and noise (20 Hz to 20 MHz) < 350 µVrms / 2 mVpp in normal-mode
voltage and < 2 mArms in normal-mode current; load regulation and line regulation ("linear
regulation rate") each < 0.01 % + 2 mV for voltage and < 0.01 % + 250 µA for current
([DP] p. 7). Accuracy is annual at 25 °C ± 5 °C, after calibration at 25 °C and a one-hour
warm-up (note 1). Only readback resolution and accuracy belong to the instrument profile;
regulation and ripple describe the `bench.psu` component, not the readout.

Fluke 87V (verified 2026-11-26 against [FL]: 80 Series V Users Manual, May 2004 Rev. 3,
3/25, <https://media.fluke.com/72b063d9-22bd-44a7-a91b-b2c100f399c5_original%20file.pdf>).
Accuracy is ±([% of reading] + [counts]) at 18 °C to 28 °C, ≤ 90 % RH, for one year after
calibration; the display is 6000 counts (19,999 in 4½-digit mode, where the counts multiply
by 10); Model 87 column. Page numbers are the printed ones.

| Figure | Value | Source |
|---|---|---|
| DC volts | 600.0 mV (0.1 mV): 0.1 % + 1; 6.000 V, 60.00 V, 600.0 V (1 mV, 10 mV, 0.1 V): 0.05 % + 1; 1000 V (1 V): 0.05 % + 1 | [FL] Table 11, p. 42 |
| Volts input impedance | 10 MΩ, < 100 pF (nominal) | [FL] Table 17, p. 47 |
| DC current, µA | 600.0 µA (0.1 µA): 0.2 % + 4; 6000 µA (1 µA): 0.2 % + 2; burden 100 µV/µA (typical) | [FL] Table 13, p. 44 |
| DC current, mA | 60.00 mA (0.01 mA): 0.2 % + 4; 400.0 mA (0.1 mA): 0.2 % + 2; burden 1.8 mV/mA (typical); 400 mA continuous, 600 mA for 18 h at most | [FL] Table 13, p. 44 |
| DC current, A | 6.000 A (1 mA): 0.2 % + 4; 10.00 A (10 mA): 0.2 % + 2; burden 0.03 V/A (typical) | [FL] Table 13, p. 44 |
| Resistance | 600.0 Ω: 0.2 % + 2 (with REL); 6.000 kΩ, 60.00 kΩ: 0.2 % + 1; 600.0 kΩ, 6.000 MΩ: 0.6 % + 1; 50.00 MΩ: 1.0 % + 3 (+0.5 % of reading above 30 MΩ) | [FL] Table 11, p. 42 |
| Ohms test source | open circuit < 7.9 V dc; short-circuit current 1 mA (600 Ω), 100 µA (6 kΩ), 10 µA (60 kΩ), 1 µA (600 kΩ, 6 MΩ), 0.5 µA (50 MΩ), typical | [FL] Table 17, p. 47 |

A profile models only deterministic effects:

- loading or burden;
- display resolution, counts, ranges and autorange with `OL`;
- integration and averaging;
- scope vertical quantisation (V/div ÷ 25 per code on the DS1054Z);
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
keeps the time and state. Decided 2026-11-26 (Stefan): a series ammeter joins the
circuit by re-elaborating it when the ammeter is added, not through a pre-placed
zero-ohm element at every ammeter point. Ideal profiles return no elements.
Trace parity runs only ideal profiles.

#### E5f notes

- **Re-elaboration at build.** `of_circuit::loads` turns each real-world `[[instrument]]` into load elements (a `dc_v` multimeter: its input resistance across the probed nets; a `dc_a` multimeter: its burden in series with the probed pin, on the `params.input` jack `uA`, `mA` (default) or `A`; a scope: its input from each probed net to ground) and `with_loads` adds them to the file as `generic.resistor` and `generic.capacitor-mlcc` placements; `build` applies this before elaborating, so adding a real ammeter rebuilds the session from t = 0. The ideal profile adds nothing, so the demos, goldens and trace parity do not change. Carrying time and state across a run-time re-elaboration (the desktop adding a meter mid-run) is still to do.
- **Names.** Refs cannot contain `.`, so the elements are `XMM1_R1`, `XMM1_C1` and the moved pin's net `XMM1_n1` rather than `XMM1.rin`.
- **Profiles are data.** The figures live in `parts/instruments/<vendor>.<model>/instrument.toml` (`fluke.87v`, `rigol.ds1054z`, `rigol.dp832`; every table has a `source`, and a missing or unknown field is an error); `of_instruments::profile` embeds them with `include_str!` (so wasm32 has them), parses them once (`parse`, `builtins`, `lookup`) and holds only behaviour. `circuit check` accepts `fluke.87v` on a multimeter and `rigol.ds1054z` on a scope; `rigol.dp832` is a readback profile, not an instrument.
- **What a profile computes.** Autorange to the smallest range whose printed full scale holds the reading rounded to its last digit (no hysteresis; the manual gives none), `OL` above the top range (and above 400.0 mA on the mA jack), and the ± band from the table; the DS1054Z quantises at V/div ÷ 25 around code 127, clips to codes 0–255 and bands as gain × 8 div × V/div plus the offset term (taking the 8-division screen as "full scale" is an assumption); the DP832 rounds the readback to 10 mV / 1 mA and bands per channel. The 87V's resistance ranges are in the profile (display and band, including the 0.5 % adder above 30 MΩ), but the meter's test current through an unpowered circuit is not simulated, so resistance mode is not offered yet. `Reading::on(profile, input)` turns an ideal meter reading into what the profile shows, for example `22.98 ± 0.09 mA`.
- **Desktop.** A Profiles row picks Ideal (the default) or a profile for the meters, the LED scope and the PSU readout (`of-ui` `bench::BenchProfiles`): meters show the 87V's value, band and `OL` (mA jack), the scope trace is quantised at its mA/div ÷ 25 with a note of the step and the band without its absolute-volt terms (the current probe's ratio is not modelled), and the PSU face shows the DP832's CH1 readback and bands. It is display only: the desktop does not re-elaborate the circuit with a meter's load mid-run. The 87V's < 100 pF is an upper bound, so the meter adds no capacitance; the scope's 15 pF is nominal (1X probe; probe ratios are not modelled).

## Determinism and wasm32 impact

- The service is deterministic:
  - sampling decisions come only from simulation time;
  - channels are a `Vec` in watch order, with no `HashMap` iteration;
  - no wall clock is used.
- With ideal profiles the recorded samples are the solver's values. Trace parity goldens do not change, and E5a re-blesses with an empty diff.
- `of-instruments` stays free of OS and thread dependencies and builds for `wasm32-unknown-unknown`. `Probeable` lets the web app supply its own source.

## Licensing impact

- There are no new external dependencies. `of-instruments` uses `thiserror` and the internal crates `of-core`, `of-engine` (E5b) and `of-components` (E5d).
- Instrument profiles are parts data under CC-BY-SA-4.0. They hold facts taken from vendor datasheets with citations; no vendor text or artwork is copied.

## Alternatives

- **Keep the meters in `of-ui` and add per-demo readouts.** Rejected: nothing would work on a new circuit, and the CLI and web app would disagree with the desktop.
- **Ammeter as a sense resistor always inserted.** Rejected as the default, because it changes every circuit and its goldens. The ideal ammeter reads `terminal_currents` and needs no insertion. A real burden is opt-in.
- **Random error within the accuracy spec.** Rejected: the value would be invented. The band shows the uncertainty honestly.
- **Bumping to `format = 2`.** Not needed for an optional, additive table.

## Unresolved questions

- **Series load element (resolved 2026-11-26).** Re-elaboration when the ammeter is added, not a pre-placed zero-ohm element (see "Load-element API" and the E5f notes).
- AC and resistance modes for the multimeter are left to a later RFC addendum.

Resolved:

- **DS1054Z full-scale span (resolved 2026-11-25).** The 8-bit step is not 1/256 of 8 divisions. The programming guide gives `YINCrement = VerticalScale/25` in NORMal mode (and in MAX mode while running) and `YREFerence` = 127 ([PG] pp. 2-223, 2-224), so a code is 1/25 of a division and the 256 codes span about 10.24 divisions around the reference. At 1 V/div the step is 40 mV, not 31.25 mV. Caveats: [PG] p. 2-224 also says the screen bottom is code 0 and the top 255, which does not fit 25 codes per division on the 8-division screen; in RAW (memory) mode the guide only says the increment depends on the internal waveform's scale; and the datasheet does not state the ADC span. E5f quantises at V/div ÷ 25 and cites [PG].
