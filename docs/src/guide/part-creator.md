# Authoring a part (preview)

This page previews the planned part creator: a tool that writes a complete
part pack (`part.toml`, pins, symbol, model binding, cited ratings, MPNs and
licence records). It does not work yet. The design is in
[`docs/design/part-creator.md`](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/part-creator.md).

For what you can do **today**, add a folder under `parts/` by hand and check it
with `ohmforge validate-parts parts/`. See [Contributing](../contributing.md)
and [`CONTRIBUTING.md`](https://github.com/sjnorval/Ohmforge/blob/main/CONTRIBUTING.md#adding-a-part).

## Create a part pack

*Needs the P3 creator, first the CLI (C1) and then the desktop window (C2).*

`ohmforge part new` (name *planned*) starts a pack. In the desktop app, New
part opens the same wizard. You fill in the identity, the pin table, cited
ratings, at least one orderable variant, and you bind either a built-in analog
kind or an unencrypted SPICE subcircuit. “Check model” runs the validator and,
when ngspice is installed, an out-of-process comparison on a small testbench.

Encrypted model files are refused. Models whose terms forbid redistribution
are referenced (URL and checksum) and fetched onto your machine, never copied
into the official tree.

## Import and export

*SPICE import and an ngspice-runnable deck are P3; KiCad symbol import and
export, and datasheet-assisted fill, are P4.*

You can start from a public SPICE library, and later from a KiCad symbol or a
datasheet. You can export a part pack as a folder, a KiCad symbol, and a
testbench to run in your own ngspice. Exporting a whole *circuit* to KiCad
stays on the [KiCad export](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/kicad-export.md)
path; this tool is for one part.

## What this is not

The part creator is not the circuit schematic editor, and it is not a place to
write firmware. Firmware stays in your editor ([Editor
integration](https://github.com/sjnorval/Ohmforge/blob/main/docs/design/editor-integration.md)).
A later standalone build (P6) is meant to be usable even if you never run an
ESP32 simulation.
