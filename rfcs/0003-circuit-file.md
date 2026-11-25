# RFC 0003: Circuit file format (engine E4)

- **Status:** Draft
- **Author:** Stefan Norval \<sjnorval@gmail.com\>
- **Date:** 2026-11-25
- **Tracking issue:** [#48](https://github.com/sjnorval/Ohmforge/issues/48)

## Summary

Define `*.circuit.toml` (`format = 1`): one versioned TOML file that describes a
circuit for **both** the simulation and the schematic. It holds the placed parts
with typed parameters, firmware by path and SHA-256, nets in router order,
per-instance schematic geometry, schematic-only symbols, keepouts and optional
probes. A new `of-circuit` crate parses it with line and column errors,
validates it, and writes a canonical form that round-trips byte for byte.
Engine phase E4 then builds the blink, dimmer, fade and button demos from these
files instead of hand-written Rust.

RFC 0002 is reserved for bus ports ([`buses.md`](../docs/design/buses.md)), so
this RFC is 0003.

## Motivation

Today each desktop demo is assembled twice by hand in `of-ui`: an electrical
`Board` plus `supply::add_supply` and `pins::add_pin_drivers` for the
simulation, and a `wiring::Schematic` with absolute coordinates for the router.
The two can drift, every new demo needs Rust code, and nothing can be saved or
loaded. Engine E2 and E3 ([#46](https://github.com/sjnorval/Ohmforge/issues/46),
[#47](https://github.com/sjnorval/Ohmforge/issues/47)) put every part, the bench
PSU and the ESP32-C3 behind one component API; E4
([#48](https://github.com/sjnorval/Ohmforge/issues/48)) needs a file format to
drive that API. `CONTRIBUTING.md` requires an RFC for file formats. The roadmap
P0 item "Project file format v0" is also waiting for the circuit half.

## Guide-level explanation

A circuit file lives anywhere; the demos live in `circuits/demos/` (E4b) and are
embedded in the desktop app. Paths inside a file are relative to the file.

```toml
format = 1

[circuit]
name = "LED blink"
ground = "gnd"
description = "Bare-metal blinky on GPIO8 through R1 into a red LED."

[[component]]
ref = "U1"
part = "espressif.esp32-c3"
at = [18, 49]

[component.firmware]
app = "../../firmware/esp32c3-blinky/blinky.elf"
app_sha256 = "<64 lower-case hex>"
boot = "direct"

[component.schematic]
body = [0, 0, 102, 100]
pins = [
  { pin = "3V3", id = "vdd", name = "VDD", at = [0, 15], dir = "W" },
  { pin = "GPIO8", id = "gpio8", name = "GPIO8", at = [102, 33], dir = "E" },
  { pin = "GND", id = "gnd", name = "GND", at = [102, 84], dir = "E" },
]

[[component]]
ref = "PSU"
part = "bench.psu"
at = [18, 210]
params = { current_limit = 0.5, on = true, series_r = 0.1, volts = 3.3 }

[component.schematic]
body = [0, 0, 78, 52]
pins = [
  { pin = "+", id = "plus", name = "+", at = [78, 14], dir = "E" },
  { pin = "-", id = "minus", name = "−", at = [78, 42], dir = "E" },
]

[[symbol]]
ref = "GND"
kind = "ground"
id = "gnd_sym"
at = [211, 174]
body = [0, 0, 22, 10]
pins = [
  { pin = "1", id = "pin", at = [11, 0], dir = "N" },
]

[[net]]
name = "vdd"
label = "VDD"
kind = "power"
pins = ["U1.3V3", "PSU.+"]

[[net]]
name = "gnd"
label = "GND"
kind = "ground"
pins = ["U1.GND", "D1.K", "PSU.-", "GND.1"]

[schematic]
keepouts = [
  { rect = [18, 28, 112, 40], note = "U1 / ESP32-C3 label" },
]

[[probe]]
name = "LED current"
signal = "i(R1)"
```

(Abridged: R1, D1 and the LED nets are left out.)

The CLI checks and formats files:

```text
$ ohmforge circuit check circuits
circuits/demos/blink.circuit.toml:31:19: 'D1.X': part 'generic.led-red-0603' has no pin 'X' (pins: A, K)
error: 1 of 5 circuit file(s) failed

$ ohmforge circuit fmt --check circuits      # CI: exit 1 and "would reformat FILE" if not canonical
$ ohmforge circuit fmt --update-hashes FILE  # the firmware changed on purpose
```

Both take `--parts DIR` (default `parts`). A directory argument means every
`*.circuit.toml` below it.

## Reference-level design

### Sections

| Section | Contents |
|---|---|
| `format` | Integer schema version. The loader refuses `0` and anything newer than it knows, with "format N is not supported (this Ohmforge supports 1)". |
| `[circuit]` | `name`; `ground` (the one reference net, default `gnd`); optional `description`. |
| `[sim]` | `quantum_us` (default 50), `max_step` in seconds (default 5e-6), `corner` (`typ`, `min` or `max`; default `typ`), optional `temperature_c`, `seed` (default 0). The defaults reproduce today's blink demo. |
| `[[component]]` | `ref`, library `part` id, optional schematic origin `at = [x, y]`, `params` (numbers, booleans, text or lists of names; TOML integers read as numbers), optional `seed` (defaults to `[sim] seed`), optional `[component.firmware]` and `[component.schematic]`. |
| `[component.firmware]` | `app` (ELF for `boot = "direct"`, flash image for `boot = "rom"`) and `app_sha256`; optional `partitions` with `partitions_sha256`; optional `ready_text` (boot is complete once UART0 printed it; also the pre-boot cache key). Never firmware bytes, never the ROM. |
| `[component.schematic]` | Per-instance geometry until E6 adds `[symbol]` to `part.toml`: optional `id` (default: lower-case ref) and `label` (default: the ref), `body = [x0, y0, x1, y1]` relative to `at`, and `pins`. |
| schematic pin | `pin` (electrical pin name), optional `id` and `name` (default: `pin`), `at` relative to the origin, `dir` (`N`, `E`, `S`, `W`), `stub = true` for a stub-only pin that connects by name (`Pin { routed: false }`), optional `slide = { to = [x, y], param = "position" }` (position `at + (to - at) * param`; the pot wiper). |
| `[[symbol]]` | Schematic-only symbols with no electrical model (`kind = "ground"` in format 1): `ref`, `kind`, optional `id`, `at`, `body`, `pins`. |
| `[[net]]` | `name` (solver node), optional `label`, `kind` (`signal`, `power` or `ground`; default `signal`), `pins` as `REF.PIN` in router order, optional `waypoints` (empty until the editor stores them). |
| `[schematic]` | `keepouts = [{ rect = [x0, y0, x1, y1], note = "..." }]` in absolute coordinates. |
| `[[probe]]` | `name` and `signal` (for example `i(R1)` or `v(vdd)`); stored and round-tripped, used by E5. |

**Not stored:** routes (the router recomputes them), firmware bytes and the ROM,
prices and hand BOM entries (a `bom.toml` beside the circuit file, keyed by
ref, [parts-availability.md](../docs/design/parts-availability.md) §6), and
symbol artwork (E6).

### Validation

Every error carries a 1-based line and column of the offending value and is
printed as `file:line:column: message`. Unknown keys are errors
(`deny_unknown_fields` everywhere), so typos fail with a position. Structural
checks (no parts library needed):

1. `ref` is unique across components and symbols (case-insensitive) and uses
   letters, digits, `_` and `-` only.
2. A net pin is `REF.PIN` with a known ref; a symbol's net pins name one of its
   own schematic pins.
3. A `REF.PIN` is on at most one net, and only once.
4. Net names are unique (case-insensitive).
5. Exactly one net is named `[circuit] ground` and it has `kind = "ground"`; no
   other net has that kind.
6. Every schematic pin that is not a `stub` is on a net; a pin is drawn once.
7. Firmware paths are relative to the file with `/` separators; `app_sha256`
   and `partitions_sha256` are 64 lower-case hex characters; `partitions` and
   its hash come together.
8. `slide.param` names a numeric parameter of the component, or one it does not
   set (then 0.5 at build time).
9. Known symbol kinds; positive `[sim] quantum_us` and `max_step`.

Checks against the parts library (`of-circuit` feature `engine`):

- every `part` exists;
- every `REF.PIN` and schematic `pin` names a pin of the part, by its **name** in
  `part.toml` (case-insensitive; pin numbers are not accepted);
- the SHA-256 of each firmware file matches the recorded hash. A mismatch is a
  hard error that shows the new hash; `ohmforge circuit fmt --update-hashes`
  rewrites it. There is no silent acceptance, because sweeps and replays rely on
  the hash to know the firmware changed.

### Canonical form

`save` is a hand-written writer, so the layout is stable and readable, and the
committed files are stored in canonical form (CI runs
`ohmforge circuit fmt --check circuits` and `ohmforge circuit check circuits`):

1. `format`, then `[circuit]` (`ground` is always written).
2. `[sim]` with only the keys that differ from the defaults; left out when none do.
3. Components: `ref`, `part`, `at`, `params` (one inline table, keys sorted),
   `seed`, then `[component.firmware]` and `[component.schematic]` with one inline
   pin table per line.
4. Symbols, then nets (`name`, `label`, `kind`, `pins`, `waypoints`), then
   `[schematic] keepouts` one per line, then probes. One blank line between tables.

Numbers: integral values print without `.0` (coordinates like `fmt_coord` in the
schematic goldens); physical values below `1e-3` or from `1e15` print as `{:e}`
(`5e-6`); others print in the shortest form that reads back to the same value.
Strings use TOML basic-string escapes; non-ASCII stays UTF-8 (`−`). `fmt` also
writes refs in net pins as declared and pin names as `part.toml` spells them.

`save` does not keep comments. Anything worth keeping goes in `description` or
a keepout `note`. Comment preservation (for example with `toml_edit`, MIT/Apache)
is left for the editor.

### Building (E4b-2)

`of_circuit::build::build(src, file, parts, registry, resolver)` turns a parsed
file into a running `of_engine::Session`; `build_engine` stops after
elaboration. In order:

1. Parts and pins are checked against the library, and every firmware file is
   read and its SHA-256 compared. The first problem is returned with the line
   and column of the part, pin or hash (a wrong hash shows the actual one).
2. The ground net must be `gnd` (or `0`) for now; another name is an error at
   `[circuit] ground`.
3. Each component's electrical pins are taken from the nets in file order,
   spelled as in `part.toml`. Its model (`part.toml` `model`) must be in the
   registry. Params pass through typed (`ParamValue`); numbers and flags also
   reach `ElabCtx::param`. The seed is the component's, or `[sim] seed`.
4. Firmware: `direct` passes the ELF; `rom` first asks the resolver for a
   pre-booted chip (`Resolver::booted(part, app_sha256, ready_text)`, the
   desktop's pre-boot cache), then reads the app and partition table and asks
   `Resolver::rom(part)` for the fetched mask ROM and bootloader.
5. Components are added in file order (symbols are skipped), the circuit is
   elaborated, and the session starts with `[sim] quantum_us` and `max_step`.

The registry decides the corner and temperature of library parts
(`build::corner` maps `[sim] corner`); `ohmforge run` builds it from `[sim]`.

### Versioning

Within format 1, new **optional** sections and keys may be added without a
bump: `[interconnect]`, `[[wire]]` and `[[trace]]` (W1), bus `level` (B1),
`pinned` (L2), and `[symbol]` defaults from E6 making `[component.schematic]` an
override. A breaking change bumps `format`, and the loader refuses formats newer
than it knows.

### Crate placement

| Piece | Where |
|---|---|
| Format types, parse with positions, structural validation, canonical `save` | `of-circuit`, default features (serde and toml only) |
| Parts-library and firmware-hash checks through a `Resolver` trait | `of-circuit` feature `engine` (adds `of-parts`, `of-firmware`) |
| Circuit file → `of_engine::Session` builder (`build::build`, `build::build_engine`), `EmbeddedResolver`; model `Registry` | `of-circuit` feature `engine` (adds `of-engine`, `of-component-api`, `of-core`); `Registry` and `Session` in `of-engine` |
| `FsResolver` (paths relative to the file, `with_rom` for the fetched mask ROM), file and parts-directory loading, `fetch_pinned` | `of-circuit` feature `fs` |
| The shipped `circuits/demos` files and the firmware they name, embedded | `of-circuit` feature `demos` |
| Circuit file → `wiring::Schematic` | E4c: `of-ui/src/wiring/from_circuit.rs` with no default features |

Loading inside `of-engine` was rejected: the engine would depend on the parts
library and the file format, against design §3 ("it knows nothing about
specific parts"). `project.toml` (P0) can live in `of-circuit` later.

## Determinism and wasm32 impact

- Parameters are a `BTreeMap`, and validation uses `BTreeMap`/`BTreeSet` only, so
  errors and saved text are in a fixed order. No wall clock, no randomness.
- `of-circuit` with default features or `engine` builds for
  `wasm32-unknown-unknown`; host I/O is only behind `fs`.
- Parity with today's hand-wired demos (E4b): exact on GPIO edges, UART text,
  ADC codes and LEDC duty, within 1 ns on edge times, and within
  `max(1 nA, 1 ppm)` on analog samples. Parity runs set the ESP32-C3's supply
  current (`quiescent_a`) to 0 on both paths; the shipped demo files and the
  hand-wired desktop boards both keep it, and the blink file as shipped is a
  separate, reviewed golden (`blink.idle-draw.trace.txt`: identical edges and
  UART; PSU current higher by just under 23 mA, the LED a few µA lower from the
  2.3 mV rail drop).
- Physical values (E4c; Stefan: "the readings should match and resolve as close
  to real world as possible"), the same on both paths:
  - **ESP32-C3 supply current** (`quiescent_a`, a resistor from `3V3` to `GND`
    that draws it at 3.3 V): ESP32-C3 Series Datasheet v2.5, Table 5-8
    (Modem-sleep, Wi-Fi clock gated, 3.3 V, 25 °C, typical), CPU at 160 MHz,
    as every demo runs (`CpuClock::max()` in the esp-hal apps; the bare-metal
    blink starts in the 160 MHz state). **23 mA** with the CPU running
    (blink, dimmer, fade, button), **16 mA** with the CPU idle in `wfi`
    (button-irq), both the "all peripheral clocks disabled" column; with every
    peripheral clocked the table gives 28 and 21 mA. The firmware clocks only
    the peripherals it uses, so the lower column is the nearer one. The current
    is constant: it does not follow the CPU between running and `wfi`, or the
    clock during boot (activity-dependent current is the P4 power profiler).
    E4b used 5 mA, which is below every Modem-sleep figure in the table.
  - **Idle GPIO pad, including the dimmer's ADC pin GPIO2:** 1 GΩ to ground
    (3.3 nA at 3.3 V), inside the datasheet's input-current limit
    (I_IH / I_IL at most 50 nA, Table 5-4, which gives no typical value). The
    hand-wired dimmer now has this pad too, so the E4b-1 exception (dimmer PSU
    3.5 nA) is gone: all three paths match at `max(1 nA, 1 ppm)`. The dimmer
    golden was re-recorded for it (PSU 0.13 to 2.67 nA higher; LED, edges and
    UART unchanged).
- Accepted exception (Stefan, 25 November 2026, E4b-1), everything else keeps
  the tolerances above:
  - **Against the committed goldens: 1 % relative on samples off the 50 µs
    scope grid** (the samples taken on LEDC edges, so far only in the fade
    demo). The goldens are recorded on Linux x86-64 (CI); Windows MSVC's libm
    moves those edge samples by up to 0.35 %. Every run is also compared with
    the hand-wired demo on the same machine at the strict tolerances, which
    holds on Linux and Windows.

## Licensing impact

Circuit files are data: CC-BY-SA-4.0, through `circuits/LICENSE.md` (the same
wording as `parts/LICENSE.md`). Firmware keeps its own licence and is referenced
by path and hash only. ROM and bootloader images are never committed. No new
dependencies: `serde`, `toml` and the internal `of-parts` and `of-firmware`
(its own SHA-256) are already in the workspace.

## Alternatives

1. **RON.** Rust-shaped and supports comments, but adds a dependency and is less
   familiar to hand-edit than TOML; `part.toml` is TOML already.
2. **JSON.** No comments, noisy to hand-edit, and no gain over TOML's spans.
3. **KiCad netlist or `.kicad_sch`.** Carries no simulation settings, firmware
   or Ohmforge parameters; it stays an export (`of-kicad`), not the source.
4. **Comment-preserving writer (`toml_edit`) now.** Deferred to the editor:
   a canonical writer is simpler, and `note`/`description` fields cover the demos.

## Unresolved questions

Settled on 25 November 2026 (E4 plan, all recommendations agreed): ground
defaults to `gnd`; quiescent load off in parity tests and on in shipped demos;
the parity tolerances above; hash mismatch is a hard error with
`--update-hashes`; no comment preservation; five demo files including
`button-irq`; four PRs (E4a format, E4b session and demos, E4c schematic,
E4d switch-over, which drops the `--circuit-files` flag); files in
`circuits/demos/`, embedded in the app; per-instance geometry stays in the file
until E6.

Settled in E4b-2: the ROM-booted demos use the app's banner line as
`ready_text` (for example `"ohmforge esp-hal pot dimmer\n"`), and the pre-boot
cache is asked by part, app SHA-256 and ready text (`Resolver::booted`).

Settled in E4c: the schematic is drawn from each instance's `[schematic]`
geometry (`wiring::from_circuit`), and equals the hand-written board, so the
routes and the wiring goldens stay byte for byte; the pot wiper is drawn at the
live `position`. The desktop builds the files with the parts embedded by
`of_circuit::demos::parts`. The flag is `--circuit-files` (also the
`OF_CIRCUIT_FILES` environment variable and the `circuit_files` preference;
the window title stays "Ohmforge"). The hand-wired boards draw the same
ESP32-C3 supply current and have the same GPIO2 pad (see Determinism), so both
paths show the same readings.

Still open: whether `bench.psu` gets a `bom = false` marker before E6.
