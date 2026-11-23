# Schematic wire routing

Status: implemented through phase 4 in `crates/of-ui/src/wiring` (model, orthogonal router, demo ports, hops and rip-up). Later phases (45° mode, waypoints, re-edit) are still open. Under the engine plan ([`engine-components.md`](engine-components.md)) the router's input (components, pins, nets, keepouts) will come from circuit files (phase E4) instead of the hand-written boards in `wiring/*.rs`; the router itself does not change.
Date: 2026-11-21.
Related: decision d1 ([`docs/UX-Research/decisions/schematic-canvas.md`](../UX-Research/decisions/schematic-canvas.md)), canvas interaction spec ([`docs/UX-Research/Screens/schematic-editor-canvas.md`](../UX-Research/Screens/schematic-editor-canvas.md)), roadmap P2 item “egui schematic editor (place, wire, probe)”.

This note specifies how schematic net paths should be generated so they follow ordinary schematic and PCB trace conventions, while keeping the existing Ohmforge schematic look and feel exactly. It is the implementation design under d1 (grid-disciplined, first-class nets, orthogonal snap-to-grid routing). Live voltage colouring and Falstad-style current animation ([`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §9) can ride on the same polylines later; they are not part of the first PRs.

## 1. Current state (before the router, kept for history)

There is no router and no stored net geometry. Each live demo draws its board as hand-authored egui polylines in a local schematic coordinate system (the painter is centred with `left = centre.x − 200`, canvas height 268). A local `line` helper calls `Shape::line`; short stubs use `line_segment`. Paths exist only as literals in source. The only run-time computation is that centre offset, live stroke colour, and the pot wiper position.

| File | Role |
|---|---|
| `crates/of-ui/src/blink/schematic.rs` | LED blink board |
| `crates/of-ui/src/adc_pwm/schematic.rs` | Pot dimmer / LED fade board |
| `crates/of-ui/src/button/schematic.rs` | Push-button board (SW1) |
| `crates/of-ui/src/schematic.rs` | Shared symbols: `draw_psu`, `draw_switch`, hover / hold-to-press targets |
| `crates/of-ui/src/meters_ui.rs` | Probe hit targets and a straight teal meter line between two probes (an overlay, not a net) |
| `crates/of-ui/src/theme.rs` | Stroke colours (`TEAL`, `LED_RED`, muted from the egui style) |

Component symbols are also polylines or shared primitives, often mixed into the same path as the net: the series resistor is a zigzag on the GPIO-to-LED polyline; the LED is a triangle, bar and arrows; GND is three decreasing bars; the pot is a stroked track plus a teal wiper; the switch is `draw_switch`. Stroke language today: 1.7 px muted wires, 2.0 px teal when the GPIO/LED net is on, 2.4 px red LED body when lit, filled 3 px dots at T-junctions (for example the GND symbol tap at `(222, 154)`, and SW1 joining the return).

The demos already follow several schematic conventions by hand: orthogonal VDD and GND rails, pin stubs leaving the chip horizontally (perpendicular to the vertical edge), junction dots only at T-joins, and power or GND labels on short stubs rather than drawing every return. There is no routing grid, no hop/bridge, no autoroute, and no waypoint model.

## 2. Goals and conventions

The router replaces only net polylines. Symbols stay as hand-drawn or shared primitives in the existing stroke language. Visual style (widths, colours, fonts, symbol shapes, probe overlay) does not change unless a later change is explicitly approved.

- Orthogonal (Manhattan) routing on a grid (4–8 px in the current demo coordinate space).
- Pin stubs: the first and last segment of a route leave the pin perpendicular to the symbol edge, with a minimum stub length of one grid step.
- Junction dots only at degree-3 nodes (T-junctions). Never emit a 4-way join; if four segments would meet, jog one net by a grid step and make two T-junctions.
- No route may cross a component body (the body rect plus clearance).
- No foreign-net overlap: two different nets must not share a colinear or coincident segment. Same-net segments may join a trunk.
- Clearance is at least the wire stroke plus a small gap (one grid step is the starting default).
- Crossings: avoid first; if a crossing is unavoidable, draw a hop/bridge (a small semicircle or short arc in the same stroke), not a 4-way dot.
- Route power and ground nets first, then other nets by descending fanout and then by a stable name order.
- Output is deterministic: the same parts, pins, nets and waypoints always produce the same geometry. No RNG.
- Optional 45° PCB-style segments, off by default (schematic stays orthogonal).
- User-editable waypoints come later, with the editor. Until then, routes are fully generated.
- Named power symbols and net labels remain valid ways to connect a net without drawing every return, as the demos do today and as d1 already allows (`VCC` / `GND` labels).

## 3. Data model

A small in-repo model, not a file format yet (the engine plan's circuit file, phase E4, will carry it). Likely a module in `of-ui` at first; a separate `of-schematic` crate only if the breadboard view or KiCad export (P3) needs the same types outside the GUI.

- **Component** — id, reference, body `Rect` (obstacle), and pins.
- **Pin** — id, name, position in schematic units, departure direction (N/E/S/W), and the net it belongs to.
- **Net** — id, name, kind (`Signal`, `Power`, or `Ground`), terminal pin refs, optional waypoints (empty until the editor), and a cached `Route`.
- **Route** — orthogonal (or later 45°) polylines in grid units, a list of degree-3 junction points, and optional hops. The cache is invalidated when a component moves, a pin is reassigned, or a waypoint changes.

Connectivity is the model; geometry is derived (and later may be stored so a user’s waypoints survive a redraw). Probe points can keep their ids and attach to pins or named points on a route. The meter overlay stays a straight line between two probes.

The first PR can declare this model and migrate one demo onto it while still painting the existing hand-authored polylines as fixtures, so there is no visual change.

## 4. Algorithm

A maze router on a modest grid is enough for these boards. Do not take on a PCB autorouter, a C++ engine, or KiCad’s router.

1. Snap the interior of the canvas to the grid. Pins may sit on-grid after a later cleanup; until then, the stub runs from the exact pin to the first grid point in the pin’s departure direction, and routing happens on the grid after that.
2. Mark occupancy: each component body expanded by clearance is blocked. Existing same-net segments are cheap to join (trunk reuse). Foreign-net cells are blocked for overlap and, if a hop is permitted, costly to cross at a single point.
3. For each net, in the fixed order above, connect its terminals (and waypoints, when present). Prefer A\* with costs: unit step, a heavy bend penalty, a higher cost for a hop, and a preference for walking along an existing same-net trunk. Lee’s maze algorithm is an equivalent alternative when a net has many terminals (multi-target expansion from a trunk).
4. If a net cannot be placed, rip up lower-priority conflicting nets and retry in the same deterministic order (rip-up and reroute). Give up only after a fixed number of passes, then fail the test or fall back to an explicit “unrouted” stub for the editor to fix — never silently cross a body or overlap a foreign net.
5. Post-process: split accidental 4-way meetings into two T-junctions; place junction dots only at degree-3 nodes; emit hops only where two foreign nets cross at a point.

The optional 45° mode (later, off by default) adds diagonal steps of equal X/Y, still with a bend cost, still forbidding body crossings and foreign-net overlap.

**Crate:** implement A\* in-tree if it stays small, or use the `pathfinding` crate (MIT OR Apache-2.0, AGPL-compatible; add it to `deny.toml` if used). No GPL/LGPL router, and no code copied from KiCad, CircuitJS, SimulIDE or Wokwi.

## 5. Rendering in egui

The router returns ordinary geometry the existing painters already know how to draw:

- Net polylines → `Shape::line` / `line_segment` with the current strokes (1.7 muted, 2.0 teal when that net is live).
- Junctions → `circle_filled` of radius 3 px in the same colour as the net.
- Hops → a short quadratic Bézier or a small semicircular polyline in the same stroke; do not change `theme.rs`.
- Symbols, labels, hover targets, the LED glow, the pot wiper and the meter overlay stay as they are, fed by the component/pin positions rather than baked into a net path.

Do not introduce a second canvas or a GPU path for wires. Decision d2 (wgpu) is for the oscilloscope, not the schematic.

## 6. Tests

Geometry tests should run without a GUI window (plain Rust unit tests in the crate that owns the router).

- Golden polylines per demo: committed fixtures for blink first, then the pot and button boards. The first PR records today’s hand-authored paths; later PRs update goldens only when a visual change is intended and reviewed.
- Invariants, checked on every golden and on a small set of synthetic pin arrangements:
  - no segment intersects a body rect including clearance;
  - no colinear or coincident overlap between different nets;
  - junctions only at degree-3 nodes (no 4-way dots);
  - stubs leave pins perpendicular and are at least one grid step long;
  - the same input always yields identical routes (byte-for-byte on the serialised geometry).
- Optional later: rendered snapshot tests. They are not required for the first PRs and must not be the only check.

A generated blink route does not have to be pixel-identical to the hand-drawn original if pin positions and symbols stay put and the invariants hold; any visible change still needs a reviewed golden update.

## 7. Phased pull requests

Small, reviewable PRs. None of the first five starts editor chrome (palette, undo, click-to-place). Keep the existing schematic look and feel in every PR.

1. **Data model and blink fixtures.** Declare components, pins and nets for the blink demo; keep painting the current hand-authored routes; add golden geometry tests. No visual change.
2. **Orthogonal grid router.** A\*/Lee with body obstacles, clearance, perpendicular stubs, T-junctions and the cost model; blink switches to generated paths; goldens plus the invariant tests.
3. **Port the other demos.** Pot dimmer / LED fade and the push-button board (pot, switch, power-symbol stubs) use the same model and router.
4. **Crossing policy and rip-up.** Avoid crossings, then hop; rip-up and reroute when a net is blocked; tests for no foreign-net overlap and no body crossings on denser boards.
5. **Optional 45° mode**, off by default, so schematics stay Manhattan and a later PCB-style view can opt in.
6. **Waypoints and click-to-wire**, as a slice of the P2 schematic editor (place, wire, probe). This is where user edits and saved routes appear.
7. **Interactive trace re-edit mode**, in the schematic editor: drag segments, edit waypoints and bends, and re-run autoroute on a net, while keeping the routing invariants.

## 8. Roadmap placement

This work sits under the still-open P2 item “egui schematic editor (place, wire, probe)”, not as a new P3 track. The breadboard view and KiCad schematic/netlist export (`of-kicad`) in P3 should consume the same routes rather than inventing a second path generator.

## 9. Open questions

1. **Grid size** — 4 vs 8 px in today’s demo coordinates, and how that later maps to the canvas spec’s configurable snap (the screen spec mentions a KiCad-like EMU). The first router PR should pick one demo-space pitch and record it.
2. **Pin snapping** — leave pins at their hand-authored coordinates (stubs take up the off-grid remainder) or nudge symbols onto the grid in a dedicated, visually reviewed step.
3. **Power and ground** — keep labelled stubs and implicit name-based connectivity (current demos and d1) as the default, and only draw a full rail when the net has no label or the user asks for one?
4. **Hop style** — decided: a small semicircle in the same stroke, on the later-routed net (power and ground first, then fanout, then name). A vertical hopping wire bulges east; a horizontal hopping wire bulges north. Junction dots are never drawn at a crossing.
5. **Where the code lives** — `of-ui` module vs a later `of-schematic` crate. Start in `of-ui`; extract when P3 breadboard or KiCad export needs the types without pulling in egui.
6. **Persisting routes** — recompute always (simpler, must stay deterministic) vs storing geometry once waypoints exist so a user’s jog does not vanish on the next open. Proposal: recompute until PR 6, then store waypoints (and the cache) with the project.
7. **Live styling** — when §9 voltage colouring or current dots arrive, do they attach to the generated polylines only, leaving symbol strokes as they are? (Recommended: yes.)

None of these should block PR 1.
