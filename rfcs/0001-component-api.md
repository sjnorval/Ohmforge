# RFC 0001: Component API for engine E2

- **Status:** Accepted
- **Author:** Stefan Norval \<sjnorval@gmail.com\>
- **Date:** 2026-11-23
- **Tracking issue:** [#46](https://github.com/sjnorval/Ohmforge/issues/46)

## Summary

Adopt [`docs/design/engine-components.md`](../docs/design/engine-components.md) as the
normative design for phase **E2**: a base engine (`of-engine`) and an extended
`Component` trait so every placed part elaborates into the existing analog solver
through one API. This RFC does not restate that document; it scopes what E2 must
implement, records the invariants that must hold, and lists what is out of scope
until later phases.

## Motivation

Today each desktop demo builds its circuit twice by hand in `of-ui`: an electrical
`Board` and a schematic `Schematic`, with the bench PSU and GPIO pin drivers added
afterwards. That causes hidden return paths, engine glue in a UI crate, and
per-demo meter registration. Issue [#46](https://github.com/sjnorval/Ohmforge/issues/46)
and design E1 ([#43](https://github.com/sjnorval/Ohmforge/pull/43)) plan one
engine API for every real part, including the PSU. `CONTRIBUTING.md` requires an
RFC for component-API changes; the design doc alone is not enough for that gate.

Dependencies already landed: the PSU return path and period-aligned averages
([#45](https://github.com/sjnorval/Ohmforge/issues/45) / [#54](https://github.com/sjnorval/Ohmforge/pull/54))
and the RFC template ([#16](https://github.com/sjnorval/Ohmforge/issues/16) /
[#53](https://github.com/sjnorval/Ohmforge/pull/53)).

## Guide-level explanation

After E2:

- A contributor adds or ports a part by implementing `Component` and registering
  it under the `model` string from `part.toml` (for example `rust:led`).
- Elaboration turns pin-to-net maps into `of-analog` devices by reusing
  `of_components::analog` (resistor, pot legs, LED, switch, and the PSU source
  pair between its own `+` and `−` pins).
- The engine checks invariants before a run starts (one ground, return paths,
  no floating nets) and requires `terminal_currents` so ammeters and KCL checks
  work without per-demo branch lists.
- Active components (a CPU in E3) hand over an `of_core::Engine` via
  `take_engine`; the base engine registers those before the analog solver.

Demos keep working through the existing bridge until later phases move them onto
circuit files (E4). E2 lands the API and five ported models with parity tests;
it does not yet delete the `of-ui` demo assembly code for the ESP32 path.

## Reference-level design

### Normative pointer

For E2, the following sections of
[`docs/design/engine-components.md`](../docs/design/engine-components.md) are
**normative**:

- §3a Engine core (`of-engine` responsibilities)
- §3b Component API (trait sketch and notes)
- §3d Bench PSU as a component
- §3e Passive and active parts (the E2 port list)
- §4 Invariants (especially 1–4 and 6)
- §5 Migration plan, **E2 row only**
- §7 Crate placement (proposal for `of-engine` / `of-component-api`)

Trait method names and exact `ElabCtx` / `EventCtx` signatures in §3b are
**illustrative**. Implementation PRs may adjust them without a new RFC as long as
the invariants and the elaboration-into-`Device` model stay the same. A new RFC
is required if invariants change, or if components stamp a second matrix API
instead of elaborating into `of-analog` devices.

### Draft trait (from design §3b)

```rust
// DRAFT: illustrative only, not a committed API.
pub trait Component: Send {
    fn info(&self) -> &ComponentInfo;
    fn elaborate(&mut self, cx: &mut dyn ElabCtx) -> Result<(), ElabError>;
    fn on_event(&mut self, _ev: &ComponentEvent, _cx: &mut dyn EventCtx) {}
    fn commit(&mut self, _view: &SolutionView, _t: SimTime) -> Health { Health::Ok }
    fn terminal_currents(&self, view: &SolutionView, out: &mut [f64]);
    fn readings(&self, _view: &SolutionView, _out: &mut Vec<(String, f64)>) {}
    fn faults(&self) -> &[FaultKind] { &[] }
    fn take_engine(&mut self) -> Option<Box<dyn Engine>> { None }
}
```

`terminal_currents` is required: currents into each pin, in `info().pins` order,
must sum to zero within solver tolerance.

### Invariants E2 must enforce

1. Exactly one ground net; components never touch solver node 0 except through a
   pin on that net.
2. Every independent source has both terminals on declared pins with a DC path
   between those nets (no implicit ground).
3. Each component's `terminal_currents` sum to zero; in particular PSU `+` and
   `−` balance.
4. No floating nets (every net has a DC path to ground).
5. Active engines are registered before the analog solver.

(Design §4 items 5, 7 and 8 also apply as the migration proceeds; circuit-file
parity is E4.)

### E2 ports

| Part id | Notes |
|---|---|
| `generic.resistor` | Existing bridge |
| `bourns.ptv09a-b103` | Wiper `position` as a host command |
| `generic.led-red-0603` | Existing LED model |
| `generic.pushbutton` | Seeded bounce via `of_components::bounce` |
| `bench.psu` | Moves out of `of-ui` `supply.rs`; explicit `+` / `−`, CV/CC |

### Acceptance (from #46 / design §5 E2)

- Unit tests per component, including terminal-current KCL and PSU `+`/`−` balance.
- An engine-built LED + resistor + PSU circuit matches the existing bridge-built one.
- `of-analog` ngspice suite unchanged; wasm32 check passes.
- No realtime regression on the live demos.

### Non-goals (later RFCs or issues)

- ESP32-C3 as a component (E3, #47)
- Circuit files and demo file-driven UI (E4, #48)
- Instruments / measurement service as components (E5, #49)
- `[symbol]`, `bom = false`, and BOM from components (E6, #50)
- Stable public plugin ABI and WASM plugins (P6)
- Physics layers (design §3j; parked)

## Determinism and wasm32 impact

- Simulation results stay deterministic: `SimTime` only, ordered maps, seeded
  randomness per component when needed. No wall clock in simulation crates.
- New `of-engine` and extended `of-component-api` must build for
  `wasm32-unknown-unknown`. They must not depend on UI or OS crates.

## Licensing impact

No licence change. Code remains AGPL-3.0-or-later; parts data CC-BY-SA-4.0.
No new dependencies in this RFC (docs only). Future `of-engine` work must keep
dependencies on the `deny.toml` allow list and must not link ngspice or QEMU.

## Alternatives

1. **Treat the design doc as the RFC without a file in `rfcs/`.** Rejected:
   `CONTRIBUTING.md` requires an RFC for component-API changes, and #46 calls
   for one (or an explicit agreement that the design doc stands in — this RFC
   *is* that agreement, with a scoped pointer).
2. **Rewrite §3b fully inside this RFC.** Rejected: duplication would drift from
   the living design doc. Normative pointer plus a short trait quote is enough.
3. **Ship `of-engine` in the same PR as the RFC.** Deferred: keep the design gate
   separate so implementation PRs can cite an Accepted RFC.

## Unresolved questions

These may be settled in the first E2 implementation PR without a new RFC unless
they change invariants:

1. Whether elaboration adds devices through `Box<dyn Device>` on `ElabCtx` or
   through helpers that wrap the existing `of_components::analog` bridge.
2. Exact ownership of event codes and watch registration (engine-assigned vs
   component-requested handles).
3. Where `bench.psu`'s `part.toml` lives and whether its `model` string is
   `rust:bench-psu` or another registry key (design §3g examples).
