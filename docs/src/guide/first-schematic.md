# Your first schematic (preview of the planned workflow)

> **Coming soon. Nothing in steps 1 to 10 works yet.** This page previews how you will create a new schematic from scratch once the planned pieces land: circuit files (engine phase E4) and parts and BOM from components (E6) in [`engine-components.md`](../design/engine-components.md), the schematic editor's place-and-wire work ([`wire-routing.md`](../design/wire-routing.md)), automatic layout ([`schematic-layout.md`](../design/schematic-layout.md)), themes ([`schematic-themes.md`](../design/schematic-themes.md)), stress states ([`component-stress-states.md`](../design/component-stress-states.md)), wires and traces ([`interconnects.md`](../design/interconnects.md)), parts availability ([`parts-availability.md`](../design/parts-availability.md)), firmware upload from your editor ([`editor-integration.md`](../design/editor-integration.md)), and sweeps and design review ([`ai-and-scenario-sweeps.md`](../design/ai-and-scenario-sweeps.md)). The phase each step needs is named in the step; the [roadmap](../roadmap.md) tracks them, and an illustrated version of this guide is a P3 roadmap item. Details marked *planned* are not decided yet. For what you can do **today**, see [What works today](#what-works-today) at the end.

<!-- screenshot: an empty schematic canvas in a new project -->

## 1. Start a new project

*Needs E4 (circuit files).*

Create a new project from the app. A project is a folder holding:

- the **circuit file** (TOML): your components, their positions and parameters, and the nets that connect them; the same file drives both the simulation and the schematic;
- `bom.toml`: your hand-entered BOM prices, suppliers and extra lines (created when you first enter one; A1);
- `sweeps/`: scenario sweep files, when you add some (S1).

The exact menu names and project file layout are *planned*.

## 2. Add the ESP32-C3 and the bench supply

*Needs E3 (done) and E4.*

1. Add the **ESP32-C3** (`espressif.esp32-c3`). Its pins are the chip's own: `3V3`, `GND` and the GPIOs.
2. Add the **bench PSU** and set its voltage (3.3 V for the C3), series resistance and current limit. Connect its `+` to the 3V3 supply net and its `−` to ground; every supply needs both connections.

<!-- screenshot: the ESP32-C3 and the bench PSU placed on the canvas -->

## 3. Place parts from the library

*Needs E6 (parts library and BOM from components) and the schematic editor.*

1. Open the parts library and find a part, for example a resistor, an LED, a potentiometer or a push button. How searching and browsing work is *planned*.
2. Pick the **variant** you will buy (the MPN, package and supplier offers in the part's `part.toml`); the BOM uses it.
3. Click to place the part; it snaps to the grid. Symbols follow your symbol style: IEC 60617 by default (chosen by locale), ANSI/IEEE 315 selectable (T3).

<!-- screenshot: the parts library with a variant selected, and a part being placed -->

## 4. Draw wires

*Needs the schematic editor's click-to-wire and waypoints (P3).*

1. Click a pin, then another pin, to connect them. The router draws an orthogonal path on the grid, with junction dots only at T-junctions and small hops where wires must cross.
2. Use **power and ground symbols** instead of long rails, and **net labels** to connect distant pins by name (L2).
3. Drag waypoints to adjust a route by hand if you want to.

<!-- screenshot: a wire being drawn between two pins, and a ground symbol -->

## 5. Set values and wire sizes

*Needs E4; wire sizes need W1.*

1. Select a part and set its parameters, for example a resistor's resistance or a pot's position. Parameters with a tolerance in `part.toml` show it.
2. Wires are **ideal by default** (no resistance). To include real wiring, set a wire's size in mm² (AWG shown alongside), material and length; its resistance and voltage drop then enter the simulation (W1).

## 6. Tidy the layout

*Needs L1 and L2 (P3); tidy selection needs L3 (P4).*

1. Check the layout metrics: crossings, bends and total wire length, plus best-practice warnings.
2. Run **auto-layout** to place parts with signal flow left to right, supply at the top and ground at the bottom, and as few crossings as possible. Parts you have pinned stay where they are.
3. Or select a few parts and use **tidy selection** (L3). Each layout change shows a before/after preview and can be undone.

<!-- screenshot: before/after preview of an auto-layout with the metrics -->

## 7. Load firmware

*Needs I1 (control API) and I2 (virtual serial port) in P3; the editor plugins I5 and I6 in P4.*

Build your firmware the way you already do, then load it in one of these ways:

- **From a file**: point the ESP32-C3 at your firmware file. How this is chosen in the app is *planned*.
- **From the command line**: `ohmforge flash` (I1), or unmodified `esptool` and `idf.py flash` through the virtual serial port (I2).
- **From your editor**: Upload from VS Code (the Ohmforge extension, I5) or the Arduino IDE 2.x (the Ohmforge board package, I6). With build-folder watching (I4), a rebuild reloads automatically.

## 8. Run, pause and watch

*Needs E5 (instruments), V1 to V3 (stress states), B5 (logic analyser decoders).*

1. **Play**, **Pause** and **Reset** as in today's demos.
2. While **paused**, change a value: Ohmforge solves the new operating point and previews what would happen before you resume (V3).
3. Watch the **stress states**: a part that approaches or exceeds its rating gets an outline, a shape-coded badge and a tooltip with the value, the limit and its source (V2). Auto-pause on over-limit is optional.
4. Add **instruments** to the circuit: voltmeter, ammeter, scope, logger and UART console (E5), and the **logic analyser** with UART, I2C and SPI decoders (B5).

<!-- screenshot: a paused circuit with a stress badge on the LED and the meters open -->

## 9. Save and check the BOM

*Needs E6 and A1; live supplier data needs A2 and A3.*

1. Save the project; the circuit file stores parts, positions, values and wires.
2. Open the **BOM** view, or run `ohmforge bom check <your circuit file>`: every part with its variant, quantity and offers, with the age of each price.
3. Type in prices and suppliers you got yourself; they go in `bom.toml` and no refresh overwrites them.

## 10. Next steps

- **Scenario sweeps** (S1, P3): vary supply, temperature, tolerances and more, and see where the circuit stops working (`ohmforge sweep run`).
- **Design review** (AI4, P4): `ohmforge review` checks for missing pull-ups, overvoltage on GPIOs, LEDs without resistors and exceeded ratings.
- **Themes** (T1, T2, P3): choose another schematic theme; today's colours stay the default.

## What works today

| Works today | Not yet |
|---|---|
| Run the five built-in demos in the desktop app (LED blink, potentiometer dimmer, LED fade, push button poll and irq) and change the bench supply, pot and button | Creating or saving a schematic in the app |
| Simulate your own **analog** circuit as a SPICE netlist with `ohmforge run` | An ESP32-C3 or firmware in a netlist |
| Add a part under `parts/` and check it with `ohmforge validate-parts parts/` | Using a new part in the app |
| Export a demo board's BOM with `ohmforge bom <blink\|dimmer\|fade\|button>` | A BOM for your own circuit |

- **Build and run**: `cargo build --release -p of-app --features gui` and `cargo run --release -p of-app --features gui`; the CLI is `cargo run --release -p of-cli -- <command>`. There are no released binaries yet. All demos except LED blink need `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`) first.
- **SPICE netlists**: example circuits checked against ngspice are in `crates/of-analog/tests/circuits/` (for example `led_dc.cir`, `rc_step.cir`, `mvp_board.cir`). Run one with `cargo run --release -p of-cli -- run <file.cir> --out out.csv --vcd out.vcd`.
- **New boards with firmware** can only be built in Rust today: see `crates/of-soc-esp32c3/tests/` (`blinky_led.rs`, `esp_hal_blinky.rs`), `crates/of-engine/tests/e2_ports.rs`, and the demo boards in `crates/of-ui/src/` with schematic coordinates placed by hand in `crates/of-ui/src/wiring/*.rs`.
