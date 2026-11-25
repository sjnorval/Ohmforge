# Circuit files

Circuit files (`*.circuit.toml`) describe a circuit for both the simulation and
the schematic: placed parts with typed parameters, firmware by path and
SHA-256, nets in router order, per-instance schematic geometry, keepouts and
probes. The format is [RFC 0003](../rfcs/0003-circuit-file.md) (`format = 1`).

The desktop demos live in `demos/` (engine phase E4b,
[#48](https://github.com/sjnorval/Ohmforge/issues/48)): `blink`, `dimmer`,
`fade`, `button` and `button-irq`. They name the firmware under `firmware/` by
path and SHA-256 and give the ESP32-C3 its datasheet supply current
(`quiescent_a`: 23 mA with the CPU running at 160 MHz, 16 mA for
`button-irq`, which sleeps in `wfi`; datasheet v2.5 Table 5-8). `of-circuit` embeds
them (feature `demos`), and the desktop app draws and runs them with
`--circuit-files` (or `OF_CIRCUIT_FILES=1`; engine phase E4c). The
hand-wired boards stay the default until E4d.

Run one headless (the ROM-booted demos need `scripts/fetch-esp-rom-elfs.sh`):

```powershell
cargo run -p of-cli -- run circuits/demos/blink.circuit.toml --until 3ms --uart
cargo run -p of-cli -- run circuits/demos/dimmer.circuit.toml --until 20ms --out dimmer.csv
```

Check and format files with the CLI:

```powershell
cargo run -p of-cli -- circuit check circuits              # every *.circuit.toml below circuits/
cargo run -p of-cli -- circuit fmt --check circuits        # fails if a file is not in canonical form
cargo run -p of-cli -- circuit fmt circuits                # rewrite in canonical form
cargo run -p of-cli -- circuit fmt --update-hashes FILE    # firmware changed on purpose: record the new SHA-256
```

Errors are printed as `file:line:column: message`. `fmt` does not keep
comments; put anything worth keeping in a `description` or `note` field.

Everything in this folder is CC-BY-SA-4.0 ([LICENSE.md](LICENSE.md)).
