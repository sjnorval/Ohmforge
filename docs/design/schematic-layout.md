# Schematic auto-layout: fewer crossings, best-practice placement

Status: proposed (design only, no code). Nothing in this document is implemented yet; section 1 lists what exists today. Phase L0 is this document (P2); L1 and L2 are planned in P3 / 0.2, L3 and L4 in P4 / 0.3 (section 7).
Date: 2026-11-23.
Related: [`wire-routing.md`](wire-routing.md) (the orthogonal router, hops, rip-up; its open items: 45° mode, waypoints, re-edit); [`engine-components.md`](engine-components.md) §3g (`[symbol]`, pin kinds) and §3h (circuit files with `at` positions); [`schematic-themes.md`](schematic-themes.md) (net-class colours); [`buses.md`](buses.md) (bus nets); [`component-stress-states.md`](component-stress-states.md); UX decision d1 ([`UX-Research/decisions/schematic-canvas.md`](../UX-Research/decisions/schematic-canvas.md)).

Stefan asked for schematic layouts that reduce wire crossings as much as possible while following layout best practice. The router already draws good wires between fixed parts; what decides how many crossings a schematic has is mostly **where the parts are**. This plan adds automatic placement (whole schematic, a selection, or incremental with pinned parts), measured by clear metrics, feeding the existing router unchanged.

## 1. Current state

- **Positions are hand-placed.** Each demo board's components and pin coordinates are literals in `of-ui/src/wiring/blink.rs`, `dimmer.rs` and `button.rs` (`Schematic` with `Component`, `Pin { pos, dir }`, `Rect` bodies and keepouts in `wiring/model.rs`), in a local coordinate space. Symbols are hand-drawn primitives.
- **Routing is automatic.** The orthogonal grid router (`wiring/router.rs`: A*/Lee with bend cost, pin stubs, T-junctions, body clearance, routing order power → ground → fanout → name) draws every net; crossings are avoided first and otherwise drawn as hops (`wiring/hops.rs`); rip-up and reroute handle blocked nets. Golden files (`wiring/goldens/*.routes.txt`, `*.invariants.txt`) pin the geometry and invariants (no body crossings, no foreign-net overlap, junction dots only at T-joins).
- **Planned**: circuit files carry `at = [x, y]` per component (E4); user waypoints, click-to-wire and a trace re-edit mode are P3 editor items; a 45° mode is optional. There is no placement algorithm and no layout metric beyond the router's invariants.

## 2. Best practices the layout follows

These become placement constraints and lint rules:

1. Signal flow left to right: inputs on the left, outputs on the right.
2. Positive supply towards the top, ground towards the bottom.
3. Power and ground symbols (rail labels, ground symbols) instead of long rails across the sheet.
4. Net labels for long nets and for buses, instead of long wires.
5. Junction dots only at T-junctions; never a four-way junction (split into two T's).
6. Everything on the grid.
7. Decoupling capacitors next to the IC supply pin they serve.
8. Related parts grouped into functional blocks (power input, regulator, MCU, sensors, outputs).
9. Readable text: labels horizontal or rotated to read from the bottom or right, never upside down; no text over wires.
10. No wires over symbols; minimal bends; short wires.

## 3. Approach

**Proposed: a layered (Sugiyama-style) placement with port constraints, then the existing router.**

1. **Group** components into functional blocks: connected-component and net-community clustering, guided by part categories (`power`, `mcu`, `sensor`, `passive` from `part.toml`) and pin kinds from `[symbol]` (E6); decoupling capacitors are attached to their IC pin as satellites rather than placed freely.
2. **Orient** the graph: pin kinds give direction (outputs → inputs; supply pins up, ground down); power and ground nets are removed from the graph and drawn as symbols, which removes most potential crossings at once.
3. **Layer** blocks and parts left to right (longest path, then network-simplex balancing).
4. **Order within layers** to minimise crossings: barycentre / median heuristics with layer sweeps, respecting the fixed order of pins on each symbol side (port constraints), with a deterministic tie-break.
5. **Assign coordinates** on the grid (Brandes–Köpf style balanced placement), keeping user-pinned parts fixed and leaving channels for routing.
6. **Route** with the existing router; long or bus nets get net labels; measure; optionally iterate a few local swaps that reduce the metrics.

**Libraries.** Mature layout engines are not usable as code: ELK (the Eclipse Layout Kernel, whose layered algorithm is the model for steps 3 to 5) is Java under EPL-2.0, and Graphviz's `dot` is EPL too; neither is linked. Their published algorithms (Sugiyama et al. 1981; Gansner et al. 1993; Brandes and Köpf 2001; Schulze, Spönemann and von Hanxleden 2014 on port constraints) are the reference for an own implementation. `layout-rs` (a Rust graph-layout crate) is a candidate to evaluate for the layering steps if its licence passes `cargo-deny`; the schematic-specific parts (port constraints, power symbols, decoupling satellites, grid) are ours either way. The code lives in a pure, wasm-clean module (`of-ui::wiring::layout`, or a new `of-layout` crate if `of-cli` needs it headless).

## 4. Modes

- **Full auto-layout**: place every unpinned part, then route.
- **Tidy selection**: re-place only the selected parts inside the selection's bounding area; everything else stays.
- **Incremental**: when a part is added or a net changes, place the new part with minimal movement of others; pinned parts (a pin icon in the editor, stored as `pinned = true` in the circuit file) never move.
- Every mode shows a **before/after preview** with the metrics side by side, applies as one undoable step, and never changes nets.

## 5. Metrics and lint

| Metric | Meaning |
|---|---|
| Crossings (and hops drawn) | Foreign-net wire crossings |
| Bends | Corners in all routed wires |
| Total wire length | In grid units |
| Rule violations | Four-way junctions, wires over symbols, off-grid points, upside-down text, ground above supply, inputs on the right of their driver |
| Label count | Net labels and power symbols used |

Shown in the editor status bar and the preview, and from the CLI: `ohmforge layout check <circuit>` (metrics and lint, exit code on violations) and `ohmforge layout auto <circuit> [--keep-pinned] [--out file]`.

## 6. Determinism and tests

- Same circuit, same pinned parts and same version give the same positions and routes: fixed iteration orders, ordered maps, seeded tie-breaks.
- Golden tests: auto-layout of the blink, dimmer, fade and button circuits (after E4) and of denser fixtures (an I2C sensor board, a CAN node), with metric thresholds (for example zero crossings on the demos, no rule violations) and geometry goldens like the router's.
- The hand-placed demo layouts stay as they are unless an auto layout is reviewed and accepted to replace one.

## 7. Phasing

| Item | Phase | Work | Depends on | Acceptance criteria |
|---|---|---|---|---|
| **L0** Design | P2 (now) | This document; roadmap, README, mdBook and changelog pointers. | — | Reviewed and merged by Stefan. No code change. |
| **L1** Metrics and lint | P3 | The metrics and rule checks in section 5 on any routed schematic; status bar; `ohmforge layout check`. | Router (done); E4 (#48) circuit files | The four demo boards report their crossings, bends and length, and no rule violations; a fixture with a four-way junction and a wire over a symbol is flagged. |
| **L2** Full auto-layout | P3 | Grouping, orientation, layering, crossing minimisation with port constraints, grid coordinates, power and ground symbols, net labels for long nets, decoupling satellites; then the router; deterministic. | L1; E6 (#50) `[symbol]` and pin kinds | Auto-layout of the demos has no more crossings than the hand layouts and no rule violations; a 20-part fixture's crossings fall at least 50 % against a random placement; two runs give identical goldens. |
| **L3** Tidy selection and incremental layout | P4 | Selection and incremental modes, pinned parts, before/after preview with metrics, one-step undo. | L2; the schematic editor (P3) | Pinned parts never move; undo restores the exact previous geometry; adding a part to a laid-out board moves no more than its neighbours. |
| **L4** Buses and labels | P4 | Bus nets drawn as labelled bus lines with entries (I2C pairs, SPI groups, CANH/CANL), themed by net class; net-label placement rules. | L2; B1 ([`buses.md`](buses.md)); T1 theme net classes | An I2C sensor board lays out with SDA/SCL as a labelled bus and no crossings between bus members. |

## 8. Risks and open questions

| Risk | Impact | Mitigation |
|---|---|---|
| Automatic layouts look “computer-made” | Users prefer their own | Modes are optional; pinned parts; tidy-selection for small fixes; best-practice rules over raw crossing counts |
| Crossing minimisation is NP-hard | Slow on large schematics | Heuristics with an iteration budget; incremental mode for edits |
| Port constraints from symbols vary | Poor orientation for some parts | Pin kinds and sides from `[symbol]`; validator for missing kinds |

Open questions:

1. **Replace the hand-placed demo layouts** with accepted auto layouts once L2 lands, or keep them as hand-made references (proposed)?
2. **Rails or symbols** on small boards: always power and ground symbols, or keep a short drawn VDD rail as today's demos do (proposed: keep rails when the board is small and the rail has no crossings)?
3. **`layout-rs` or own code** for layering, after the licence and quality check?
