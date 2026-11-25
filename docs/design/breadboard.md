# Breadboard view

Status: proposed (design only, no code). The schematic router in [`wire-routing.md`](wire-routing.md) is planned to feed this view; there is no breadboard UI yet.
Date: 2026-11-25.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §9; [`wire-routing.md`](wire-routing.md); [`interconnects.md`](interconnects.md) (W1 jumper defaults); [`engine-components.md`](engine-components.md) (E4 circuit files, E6 parts); [`compatibility-checks.md`](compatibility-checks.md).

The breadboard view is a second picture of the same circuit: a standard 2.54 mm solderless breadboard, with the same nets, the same simulation and the same BOM as the schematic. It exists to catch mistakes that a symbol view hides — a part that does not fit, two pins in the same tied row that should not be, a jumper that is longer than it looks.

## 1. Current state

The desktop demos draw a schematic only. Breadboard geometry is not implemented. Interconnects W1 already plans breadboard jumper defaults once circuit files exist.

## 2. Goals and non-goals

Goals:

- A 2D breadboard, drawn at true 2.54 mm pitch, that the user can place modules and jumper wires on.
- One circuit file, two views. Switching from schematic to breadboard (and back) does not change the netlist or the simulation.
- Physical checks: shorted rows, parts whose body or pin span does not fit, jumpers that land in the wrong rail, power rails left unconnected.
- Jumpers appear as wires in the BOM with a default length and colour (W1).

Non-goals:

- A 3D bench, enclosure CAD, or a physics playground. Those are parked future ideas, not part of P2 to P6.
- A second, independent circuit. The breadboard is not a different design.
- Autorouting that pretends a breadboard can always match an arbitrary schematic without help. If a placement cannot be honoured, the view says so and the user moves parts.

## 3. User-facing behaviour

A tab or split (name *planned*) switches between Schematic and Breadboard. The board is a common 400- or 830-point layout: two sides of 5-hole rows, a gutter, and power rails along the long edges. The ESP32-C3 development module, passives, buttons and sensors sit at their real pin spacing, taken from `[breadboard]` (or equivalent) data in `part.toml` plus the module outline.

The user places a part so its pins drop into holes, then places jumpers hole to hole. Tied holes of a row are one net, the same as on a real board. A rail short, a pin in a row that is already used by another net, or a part that overhangs the board is a visible error, in the same family of checks as [`compatibility-checks.md`](compatibility-checks.md). Running the simulation from this view uses the same engine path as the schematic: the LED still lights, the meters still probe nets.

## 4. Technical approach

- Reuse the E4 circuit file. Breadboard placement is extra data on components and on jumper wires (hole coordinates, board id), not a second netlist.
- Hole connectivity is a static map of a named breadboard type (`generic.breadboard-830`, and so on) implemented as an ordinary component with many internally tied pins.
- The schematic router is not forced to draw jumpers. Jumpers are explicit hole-to-hole segments; the shared path data from [`wire-routing.md`](wire-routing.md) is for the schematic view and for KiCad export.
- Body and pin-span tests use millimetre dimensions in `part.toml` (CC-BY-SA-4.0). If a part has no breadboard data yet, it can still appear on the schematic; the breadboard view marks it as unplaced rather than guessing.

## 5. Phasing

- **P3 / 0.2:** one standard board type, placement, jumpers, rail and row checks, view switching, jumper lines in the BOM. Needs E4, E6, and W1 for jumper defaults. The official examples should include at least one project that looks right on the breadboard as well as on the schematic.

## 6. Acceptance criteria

- The blink (or dimmer) example can be placed on the 830-point board with the C3 module, a resistor, an LED and the bench supply, and the simulation result matches the schematic view.
- Shorting two rails with a jumper is reported as an error before or as the run starts, and is visible on the board.
- A part without breadboard dimensions does not silently pick holes; it stays unplaced with a clear mark.
- Switching views does not rewrite nets or invent connections.

## 7. Open questions

- Whether the first release includes a 400-point board as well as 830, and whether a “half+” or mini board is worth a second type.
- How development modules that do not fit a single row (wide ESP32-C3 boards) sit: spanning the gutter is the usual real-world choice and should be the default.
