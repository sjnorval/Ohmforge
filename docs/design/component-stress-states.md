# Component stress states: showing when parts approach damage

Status: proposed (design only, no code). Nothing in this document is implemented yet; section 1 lists what exists today. Phase V0 is this document (P2); V1 to V3 are planned in P3 / 0.2, V4 in P4 / 0.3 and V5 in P5 / 0.4 (section 7).
Date: 2026-11-23.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §9 (user interface), §10 (real-world realism, fault injection); [`engine-components.md`](engine-components.md) §3b (`commit` returns `Health`), §3j (telemetry, stress against `[electrical]` ratings); [`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) §6c (the limits table) and §5f (rating checks); [`interconnects.md`](interconnects.md) (conductor ratings); [`schematic-themes.md`](schematic-themes.md) (stress colours per theme); [`wire-routing.md`](wire-routing.md) (schematic look and feel).

Stefan asked for the UI to show, while running or paused, when a change the user makes pushes a component towards a threshold that would damage it. Real parts fail when they exceed their ratings: an LED without enough series resistance, a resistor dissipating more than its rated power, a GPIO pushed above its supply. This plan computes a stress level for every component from the simulated values and the part's limits, shows it on the schematic without changing the schematic's look in the normal case, and offers an optional mode in which parts fail the way real ones do.

## 1. Current state

| Piece | Where it lives today | What it does |
|---|---|---|
| `Health { Ok, Warning(String), Damaged(String) }` returned by `Component::commit` | `of-component-api` | The API for a part to report stress exists; only the bench PSU returns anything but `Ok` (`Warning` when the rail is outside 3.0 to 3.6 V, `of-components::psu`) |
| `FaultKind { Open, Short, StuckAt, … }` | `of-component-api` | Fault kinds a user can inject; no UI or damage path uses them yet |
| Ratings in `part.toml` `[electrical]` | `parts/**` | `if_max` (LEDs, 25 to 30 mA), `vceo` and `ic_max` (NPN), `vdd` min/max (ESP32-C3, BME280), regulator values; read as free-form keys, not checked during simulation |
| LED glow | `of-ui` (`demo.rs` `brightness(ma) = ma / 4 mA` clamped to 0..1; `blink`, `adc_pwm`, `button` schematics draw the LED stroke and a halo with `LED_RED.gamma_multiply(…)`) | Brightness tracks current up to 4 mA and saturates; nothing changes above the LED's rating |
| Supply warning | `of-ui::supply` (`SupplyStatus::outside_rated` when the set point or rail is outside 3.0 to 3.6 V; current-limit droop) | The only rating shown in the UI today |
| Pause and edits | Live playback can pause; the pot, button and bench PSU can be changed while running | No re-solve preview while paused; no event log of rating events |

There is no per-component stress level, no overcurrent or overpower visual, no damage model and no auto-pause.

## 2. Goals and non-goals

Goals:

- A **stress level** per component (and per pin where it matters, such as GPIOs), computed every committed step from simulated values against the part's limits.
- **Four states**: normal, warning (approaching, default above 80 % of a limit), over-limit (above a recommended or absolute maximum), damaged (latched).
- **Visuals that keep the schematic look**: nothing changes in the normal state; warning and worse add an outline or halo, a heat tint, a small badge and, only when damaged, scorch marks; tooltips give the value, the limit and its datasheet reference.
- **Accessible**: each state has a distinct badge shape as well as a colour (colour-blind-safe), and animations respect a reduce-motion setting.
- **Running and paused**: while paused, editing a value re-solves the DC operating point and previews the new stress state before resuming.
- **Optional realistic damage**: off by default; when on, a part that exceeds its absolute maximum fails open or short like real hardware; resettable.
- **One source of truth**: the same stress data goes to the event log, the CLI, sweep checks, the design review and the AI link.

Non-goals:

- Reliability prediction (MTBF, ageing, electromigration over months).
- Exact failure physics; damage modes are plausible, documented simplifications.
- Changing the default schematic look; the normal state draws exactly as today.

## 3. Stress computation

- **Limits** come from, in order: the reviewed limits table ([`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) §6c, D2), today's `[electrical]` ratings (V1 reads these first), conductor ratings from [`interconnects.md`](interconnects.md), and component defaults (for example a resistor's power rating by package when `part.toml` has none, marked as an assumed value).
- **Quantities** per part kind: current (LED forward current, transistor collector current, GPIO source/sink current), voltage (supply pins against abs-max and recommended ranges, reverse voltage, input pins against supply plus margin), power (I²R, V·I), temperature where a thermal model exists, and interface limits (bus clock, bus capacitance).
- **Ratio**: stress = value / limit for each quantity, the worst quantity sets the part's level. Averages follow the limit's definition: a peak limit uses peak values in the step, a continuous limit a windowed average (the E5 measurement service's period-aligned averages), so PWM does not cause false alarms.
- **Derating**: power and current limits are derated with temperature where the datasheet gives a curve (V5, with the P5 thermal model; before that at the user's ambient).
- **States**:

| State | Rule (defaults, configurable) | Latched? |
|---|---|---|
| Normal | below 80 % of every limit | — |
| Warning | 80 % to 100 % of a limit (80 % configurable globally and per quantity), or above a recommended operating limit | No |
| Over-limit | above an absolute maximum, or above a continuous limit for longer than a grace time | No (shows the time over limit) |
| Damaged | over-limit long enough for the part's damage rule (energy or time above abs-max), only counted, never applied, unless realistic damage is on | Yes, until reset |

- **Where it runs**: a stress service in the engine (`of-engine`) evaluates each component's `commit` result plus the generic limit checks, so parts do not each reimplement ratios. Results are readings (`stress(R1)`, `stress(U1.GPIO8)`) like any other.

## 4. Visuals

The normal state is unchanged. Above it, layers are added in the active theme's stress colours ([`schematic-themes.md`](schematic-themes.md)); the default palette is shown here.

| State | Outline / halo | Heat tint | Badge (shape + colour) | Extra |
|---|---|---|---|---|
| Warning | thin amber outline | faint warm tint scaled by stress | amber triangle with “!” | — |
| Over-limit | red outline with soft halo, slow pulse (static if reduce-motion) | stronger tint | red octagon with “×” | tooltip shows the time over limit |
| Damaged | dark outline | charred tint | black-and-red square with a broken-link glyph | scorch marks; optional brief smoke puff (off with reduce-motion) |

- **Tooltip**: quantity, present value, limit, percentage, the limit's source (datasheet revision and page, or “assumed”), and for damaged parts the failure mode.
- **Colour-blind-safe**: badge shapes differ by state; the default stress palette is checked for deuteranopia, protanopia and tritanopia; a high-contrast theme exists.
- **Pins**: a GPIO under stress gets a small badge at the pin, not on the whole chip.
- **Conductors**: wires and traces show the same states on their stroke (outline only; the stroke width stays).

## 5. Running, paused and events

- **Running**: levels update with each UI frame from the latest committed step; the event log records every change of state with its virtual time, part and value; the scope timeline shows markers.
- **Auto-pause** (optional, off by default): pause on the first over-limit or damage event, with the part highlighted.
- **Paused edits with preview**: when the user changes a value while paused (a resistor, the supply voltage, a pot, a wire size), the engine solves the DC operating point of the edited circuit on a copy, computes stress levels and shows them as a preview (“after resume”) with the changed parts outlined; Resume applies the edit at the paused instant; Cancel discards it. Transient effects (inrush, capacitor charge) are noted as not covered by the DC preview.
- **Realistic damage** (V4): when on, a damaged part applies a failure through `FaultKind`: an LED fails open (burns out), a resistor over its power rating drifts high and then fails open, a capacitor over its voltage fails short, a GPIO driven beyond its abs-max becomes stuck or open. Damage is per run, shown with the damaged visuals, and reset from the part's menu or with “Reset all damage”; projects never save damage unless the user asks.

## 6. Outputs beyond the UI

- **CLI**: `ohmforge run` and `ohmforge review` print stress summaries; exit codes for over-limit when asked.
- **Sweeps**: `stress` checks in sweep files ([`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) §5f); per-run peak levels stored.
- **AI link**: `circuit.readings` includes stress levels; the design review cites them.

## 7. Phasing

| Item | Phase | Work | Depends on | Acceptance criteria |
|---|---|---|---|---|
| **V0** Design | P2 (now) | This document; roadmap, README, mdBook and changelog pointers. | — | Reviewed and merged by Stefan. No code change. |
| **V1** Stress service | P3 | Stress ratios from `Health`, `[electrical]` ratings and generic checks; four states; readings; event log; CLI summary. | E2, E5 (#49) averages; E6 (#50) ratings from parts | An LED driven at 35 mA (`if_max` 30 mA) is over-limit, at 26 mA warning, at 10 mA normal; PWM at 50 % duty and 40 mA peak is judged by the right limit; the demo boards stay normal. |
| **V2** Schematic visuals | P3 | Outline, tint, badges, tooltips, pin badges, reduce-motion and colour-blind checks; auto-pause option. | V1; E4 circuit-file schematics; T1 theme model | Normal state renders identically to today (snapshot test); each state has a distinct badge shape; reduce-motion disables pulses and smoke. |
| **V3** Paused edits with preview | P3 | DC re-solve on a copy while paused, preview overlay, resume or cancel. | V1; the editor's parameter edits | Lowering R1 to 22 Ω while paused shows D1 over-limit in the preview without advancing virtual time; Cancel restores the previous state exactly. |
| **V4** Realistic damage | P4 | Damage rules per part kind through `FaultKind`; latched damage; reset; off by default. | V1; fault injection hooks | With damage on, an LED at 3× its abs-max for the rule's time fails open and the circuit current drops accordingly; Reset restores it; with damage off, nothing changes the circuit. |
| **V5** Derating and thermal stress | P5 | Temperature derating curves from the limits table; junction temperature from the thermal model; conductor heating from W2. | V1; D2; P5 thermal model; W2 | A regulator's allowed dissipation falls with ambient temperature as its datasheet curve says; a hot wire reaches over-limit at its insulation rating. |

## 8. Invariants

1. **The normal state looks exactly as today.** Stress visuals are additive.
2. **Damage is opt-in, visible and reversible.**
3. **Every limit shows its source**, or says it is assumed.
4. **Stress never changes the simulation** unless realistic damage is on.
5. **Shape as well as colour** for every state.

## 9. Decided and open questions

Decided (23 November 2026, Stefan):

- **Warning threshold** (former open question 1). 80 % of a limit by default, configurable globally and per quantity.
- **Damage rules** (former open question 2). Simple time-above-abs-max rules.
- **Assumed limits** (former open question 3). Parts with no limit data show stress from generic package ratings, marked “assumed”.
- **Preview depth** (former open question 4). DC preview only.

Open questions: none at present.
