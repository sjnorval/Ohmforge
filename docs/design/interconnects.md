# Interconnects: wire sizes and PCB traces

Status: proposed (design only, no code). Nothing in this document is implemented yet; section 1 lists what exists today. Phase W0 is this document (P2); W1 is planned in P3 / 0.2, W2 and W3 in P4 / 0.3 and W4 in P5 / 0.4 (section 8).
Date: 2026-11-23.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §4 (analog engine), §10 (real-world realism), §11 (bill of materials); [`engine-components.md`](engine-components.md) §3h (circuit files) and §3j (thermal extension point); [`wire-routing.md`](wire-routing.md) (schematic net paths); [`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) (wire size, trace width and length as sweep variables; the AI link's minimum-size suggestion); [`component-stress-states.md`](component-stress-states.md) (conductor overload shown like any other part); [`buses.md`](buses.md) (line capacitance for bus rise times); roadmap items “Breadboard view” and “PCB fabrication services” (P3), “Thermal model” (P5).

Stefan asked for wire gauge and PCB trace width to be editable, with the relevant ISO and IEC standards in mind. A real build has wires and traces with resistance: a thin breadboard jumper on a motor supply drops volts and heats up, and an undersized trace on a board does the same. This plan makes conductors first-class: sizes are set per wire or trace in metric units (mm², the default) with AWG shown as an alternative, their resistance enters the simulation, and warnings appear when a conductor is overloaded or drops too much voltage.

## 1. Current state

What exists on `main` (23 November 2026):

| Piece | Where it lives today | What it does |
|---|---|---|
| Nets in the solver | `of-engine` (`EngineBuilder::net`, `pin_node` looks up the net's node) and `of-analog` (`Circuit::node`) | **Every net is one ideal node.** All pins on a net share one voltage; a wire has zero resistance, zero inductance and zero capacitance, and carries any current |
| Supply resistance | `of-components::psu` (`series_r`, default from `of-ui::supply`) | The bench PSU's internal series resistance is the only resistance between the supply and the board |
| Schematic net geometry | `of-ui::wiring` (`Net { terminals, waypoints, route }`, `Route { polylines, junctions, hops }`) | Drawn paths in schematic units; they carry no physical length, size or material |
| Circuit file plan | [`engine-components.md`](engine-components.md) §3h (`[[net]] name, kind, pins`) | No wire objects or physical attributes planned yet |
| Device models | `of-analog::devices::passive` (resistor with `tc1`/`tc2`), MNA solver | Everything needed to stamp a conductor as a temperature-dependent resistor already exists |
| Breadboard view, PCB export | Roadmap P3 items (not started) | Where physical wires and traces first get a real topology |

There is no gauge, trace width, copper weight, conductor material, ampacity or voltage-drop check anywhere in the code or the parts library.

## 2. Goals and non-goals

Goals:

- **Editable conductor size per wire and per trace**, with project defaults; metric first (mm² for wire, mm and µm for traces), AWG and oz/ft² shown alongside.
- **Standards-aware**: sizes and terms follow IEC 60228 (conductors of insulated cables) and, for automotive-style wire, ISO 6722-1; AWG follows ASTM B258; trace current follows IPC-2152 and the older IPC-2221 formula; clearance follows IPC-2221 and IEC 60664-1.
- **Physics in the simulation**: conductor resistance from resistivity, cross-section, length and temperature; I²R heating; the voltage that actually reaches the load.
- **Warnings people understand**: current above the chosen conductor rating, voltage drop above a threshold, temperature above the insulation rating, shown through the stress states and design review.
- **Off by default for ideal schematics**: existing demos and any circuit without conductor data simulate exactly as today.
- **BOM**: wire by size, colour and length; copper weight as a PCB fabrication parameter.

Non-goals:

- Mains installation design. Fixed wiring rules (IEC 60364-5-52, and SANS 10142-1 in South Africa) are out of scope; Ohmforge is for low-voltage electronics and says so in its warnings.
- Certifying a design. Ampacity figures are guidance from published formulas and our own models, not a compliance statement.
- Field solving, skin and proximity effect, impedance-controlled routing or signal-integrity analysis beyond the lumped values in W4.
- A PCB layout editor. Trace attributes come from the user or from an imported or exported layout.

## 3. Standards and how we use them

Many of these standards are sold, not free. **Tables from paywalled standards are not copied into the repository.** We use published formulas, physical constants and our own derived data, cite the standard by number and clause, and let users enter values from their own copy where a table is the only source.

| Topic | Standard | Free? | How Ohmforge uses it |
|---|---|---|---|
| Metric conductor sizes and classes | IEC 60228 | No | Nominal sizes (0.5, 0.75, 1, 1.5, 2.5, 4, 6, 10 mm² and up) and class names (1 solid, 2 stranded, 5 flexible, 6 very flexible) as labels; its maximum-resistance tables are not copied; resistance is computed from resistivity (section 4) |
| Small metric sizes for vehicles | ISO 6722-1 | No | Sizes below 0.5 mm² used in vehicle and hook-up wire (for example 0.13, 0.22, 0.35 mm²) as selectable labels, to be checked against the standard before release; other hook-up sizes sold by area (0.08, 0.14, 0.25, 0.34 mm²) are free entries |
| AWG | ASTM B258 | No | The AWG diameter formula, which is public: d(n) = 0.127 mm × 92^((36 − n)/39); AWG is shown as the nearest gauge to the chosen mm² and can be typed instead |
| PCB trace current | IPC-2152 (2009) | No | Its findings (board thickness, copper planes and material matter; internal traces are not automatically worse) shape our own thermal model; its charts are not copied |
| PCB trace current, legacy | IPC-2221 | No | The widely published curve fit I = k · ΔT^0.44 · A^0.725 (I in A, ΔT in K, A in mil²; k = 0.048 external, 0.024 internal) as the simple, conservative estimate |
| Clearance and creepage | IPC-2221 (table 6-1), IEC 60664-1 | No | Checks against a minimum the user or the fab profile supplies, with the standard named; voltage, pollution degree and material group are inputs; the tables are not copied |
| Plating in vias | IPC-6012 | No | A user-set barrel plating thickness, defaulting to a fab profile value |

## 4. Physics

### 4a. Resistance

R = ρ(T) · L / A, with ρ(T) = ρ₂₀ · (1 + α · (T − 20 °C)).

| Material | ρ₂₀ (Ω·m) | α (1/K) | Note |
|---|---|---|---|
| Copper | 1.68 × 10⁻⁸ | 0.00393 | Pure copper; annealed copper to the IACS reference is 1.7241 × 10⁻⁸, which a setting can select |
| Tinned copper | copper × 1.02 to 1.05 (estimate) | 0.00393 | Tin coating raises resistance slightly, more for fine strands |
| Copper-clad aluminium (CCA) | about 1.5 to 1.6 × copper (estimate) | about 0.0040 | Cheap jumper and speaker wire; a common cause of surprise voltage drop |
| Aluminium | 2.65 × 10⁻⁸ | about 0.0040 | For completeness |
| PCB copper (electrodeposited) | 1.72 × 10⁻⁸ (default, user-set) | 0.00393 | Plated and foil copper is slightly worse than pure copper |

Values marked estimate are flagged in the material table and can be overridden per project. A stranded conductor uses its nominal cross-section; the class (lay-up) only changes flexibility and the ampacity model's surface area.

Examples at 20 °C: a 20 cm Dupont jumper of 28 AWG (0.081 mm²) is about 41 mΩ, so a 0.5 A load through a supply jumper and its return drops about 41 mV; 1 m of 0.5 mm² copper is about 34 mΩ; a 0.25 mm wide, 35 µm (1 oz) trace is about 2.0 mΩ per mm.

### 4b. Heating and ampacity

- Each conductor dissipates P = I² · R(T). A first-order thermal model per conductor (thermal resistance to ambient per metre and a heat capacity) gives its temperature; the temperature feeds back into R(T). This rides on the thermal extension point in [`engine-components.md`](engine-components.md) §3j and the P5 thermal model; before that, a steady-state estimate is used.
- **Wire in free air**: our own model from convection and radiation off the insulated surface, with the insulation's rated temperature as the limit (user-chosen: PVC 70 °C or 105 °C, silicone 180 °C, PTFE 200 °C). The ampacity shown is the current that reaches that limit at the user's ambient. Bundling and enclosure factors are user-set multipliers.
- **PCB traces**: the IPC-2221 fit as the legacy estimate, and our own conduction model (trace into laminate and nearby planes) informed by the IPC-2152 findings, with board thickness, plane presence and layer as inputs. Where they disagree, both are shown.
- **Vias**: barrel cross-section π · d · t (drill diameter d, plating t), treated as a short conductor with the trace model; the via count per connection is an attribute.

### 4c. Voltage drop

The drop across each conductor, and the total from source to each load pin, are readings. A warning fires when the drop on a supply path exceeds a threshold (default 3 % of the rail, configurable per net) or a load pin falls below its limit from the limits table.

### 4d. Later: inductance and capacitance (W4)

Lumped series inductance and shunt capacitance per length (a straight wire is on the order of 1 nH per mm; a trace's capacitance depends on its height over a plane) for supply ringing, motor switching and bus rise times in [`buses.md`](buses.md).

## 5. Data model

### 5a. Circuit file

```toml
# DRAFT: illustrative only.
[interconnect]
model = "resistive"            # ideal (default) | resistive | thermal
units = "metric"               # metric (default) | awg
ambient_c = 25
drop_warn = 0.03               # fraction of the rail

[interconnect.defaults.wire]
size_mm2 = 0.33                # about 22 AWG, solid breadboard jumper
material = "copper"
class = 1                      # IEC 60228 class: 1 solid, 2 stranded, 5 flexible, 6 very flexible
insulation = { material = "pvc", rated_c = 70, rated_v = 300 }
length_mm = 100

[[wire]]                       # one physical wire between two pins of the same net
net = "vdd"
from = "PSU1.+"
to = "U1.3V3"
size_mm2 = 0.08                # 28 AWG Dupont jumper
material = "cca"
length_mm = 200
colour = "red"

[[trace]]                      # a PCB trace segment
net = "led_a"
from = "R1.2"
to = "D1.A"
width_mm = 0.25
copper_um = 35                 # 1 oz/ft²; 0.5 oz = 17.5 µm, 2 oz = 70 µm
layer = "external"             # external | internal
length_mm = 18
vias = { count = 1, drill_mm = 0.3, plating_um = 20 }
```

- A net with no `[[wire]]` or `[[trace]]` entries stays one ideal node. A net with entries is split into one node per pin, joined by conductor resistors; pins not mentioned join the net's main node through an ideal link, so partial data is valid.
- The breadboard view (P3) writes `[[wire]]` entries for jumpers with defaults per jumper kind: solid 22 AWG (about 0.33 mm²) for cut jumper kits; stranded 26 to 28 AWG (0.13 to 0.08 mm²), CCA assumed until the user says otherwise, for Dupont-style jumpers; and a breadboard contact resistance per contact (an estimate, user-set).
- Units: values are stored in SI-derived metric units; the UI shows mm² with the nearest AWG beside it, or AWG first if the user picks `awg`.

### 5b. Rust sketch

```rust
// DRAFT: illustrative only, not a committed API.
pub struct Conductor {
    pub net: String,
    pub from: PinRef,
    pub to: PinRef,
    pub kind: ConductorKind,       // Wire { size_mm2, class, insulation } | Trace { width_mm, copper_um, layer } | Via { .. }
    pub material: Material,        // rho20, alpha, source (constant or user)
    pub length_mm: f64,
}

pub struct ConductorRating {
    pub ampacity_a: f64,           // at the user's ambient and insulation or board limit
    pub method: RatingMethod,      // FreeAirModel | Ipc2221Fit | BoardConductionModel | UserValue
    pub max_temp_c: f64,
}
```

Conductors are components to the engine (two pins, a temperature-dependent resistance, a `Health` from their rating), so stress states, sweeps and the BOM treat them like any part.

## 6. In the app, BOM and the AI link

- **Editing**: select a wire or trace on the schematic or breadboard view; a property panel shows size (mm² with AWG, or width and copper weight), material, class, insulation, length, and the live current, drop, temperature and rating. Sizes offer the IEC 60228 list and common small sizes; any value can be typed.
- **Visuals**: overloaded conductors use the stress colours and badges from [`component-stress-states.md`](component-stress-states.md); drawn stroke width does not change with size (the schematic look stays), but the tooltip and an optional “show conductor sizes” label do.
- **BOM**: wires roll up into lines per size, material, insulation and colour with total length plus a cut allowance (for example “0.33 mm² solid PVC, red, 1.2 m”); PCB copper weight, finished thickness and minimum trace and space become fabrication parameters on the PCB export.
- **Sweeps**: `wire.<net>.size_mm2`, `length_mm`, `trace.<net>.width_mm` and `copper_um` are sweep variables ([`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) §5b).
- **AI link**: `interconnect.suggest_min_size` returns the smallest listed wire size or trace width that keeps current within the rating and drop within the threshold for the simulated worst case, with the method used.

## 7. Invariants

1. **Ideal by default.** A circuit without conductor data simulates exactly as today (byte-identical readings in the existing golden and parity tests).
2. **Metric first.** Stored values are metric; AWG and oz/ft² are display and entry conveniences.
3. **No copied tables.** No table from a paywalled standard is committed; formulas, constants and our own derived data only, with the standard cited.
4. **Ratings show their method.** Every ampacity or clearance figure states how it was obtained.
5. **Conductors are components.** They use the same component API, `Health`, readings and BOM path as other parts; simulation crates stay wasm-clean.

## 8. Phasing

| Item | Phase | Work | Depends on | Acceptance criteria |
|---|---|---|---|---|
| **W0** Design | P2 (now) | This document; roadmap, README, mdBook and changelog pointers. | — | Reviewed and merged by Stefan. No code change. |
| **W1** Conductor data and resistance | P3 | `[interconnect]`, `[[wire]]`, `[[trace]]` in circuit files; net splitting into conductor resistors with R(T); material table; voltage-drop readings and warnings; property panel; wire lines in the BOM; breadboard jumper defaults. | E4 (#48); E6 (#50) for the BOM; breadboard view (P3) for jumper defaults | A 0.5 A load through 20 cm of 0.08 mm² CCA shows the computed drop within 1 %; existing demos without conductor data give identical readings; mm² ↔ AWG conversion matches the ASTM B258 formula for 10 to 30 AWG; the BOM lists wire by size and length. |
| **W2** Ampacity and self-heating | P4 | Free-air wire thermal model, insulation limits, conductor `Health` into the stress service, ampacity display, `interconnect.suggest_min_size`. | W1; V1 | A current step heats a fixture wire with the expected time constant and resistance rise; exceeding the rating turns the wire to over-limit; the suggested minimum size for a 2 A motor supply passes the drop and temperature checks. |
| **W3** PCB traces, clearance and vias | P4 | Trace width, copper weight, layer and length; IPC-2221 fit and the board conduction model; via current; clearance check against a user or fab minimum by voltage; copper weight in the PCB export. | W1; KiCad export / PCB fabrication item (P3) | IPC-2221 results match the published formula to 0.1 %; a trace narrower than the computed width for its current is flagged; a fab profile's minimum clearance is checked for a 12 V net. |
| **W4** Parasitics and thermal coupling | P5 | Series inductance and shunt capacitance per length; coupling into the P5 thermal model; bus line capacitance for [`buses.md`](buses.md). | W1 to W3; P5 thermal model | Ringing on a long supply lead appears in transient results; I2C rise time grows with wire length as expected for the added capacitance. |

## 9. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Users treat ampacity as certified | Unsafe builds | Clear “guidance, not certification” notes; method shown; mains wiring out of scope |
| Standards' numbers copied by contributors | Copyright problem | Review rule and invariant 3; derive from formulas; users may enter values from their own copies locally |
| Splitting nets enlarges the MNA system | Slower live simulation | Only nets with conductor data split; one resistor per conductor; sparse LU handles the extra nodes |
| Estimates for CCA, tinned copper and contacts are rough | Misleading drops | Marked as estimates; user-settable; bench-measured values can replace them |

## 10. Decided and open questions

Decided (23 November 2026, Stefan):

- Metric mm² is the default unit with AWG alongside; conductors are components; ideal wires remain the default for circuits without conductor data.
- **Default model for new projects** (former open question 1). Ideal wires.
- **Small sizes** (former open question 2). ISO 6722-1 and small hook-up sizes are listed only once each has been checked against the standard.
- **Fab profiles** (former open question 3). Ship profiles for common fabs (minimum trace and space, copper weights, plating) and allow user-defined profiles.
- **IPC-2152 calibration** (former open question 4). Parked. The board conduction model is not calibrated until verified, openly usable data exists; until then it is uncalibrated and says so, and the IPC-2221 fit is shown beside it.

Open questions: none at present.
