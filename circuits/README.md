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
them (feature `demos`), and the desktop app draws and runs them (its only
path since engine phase E4d).

`ne555-astable` (LED flasher), `ne555-monostable` (button one-shot) and
`ne555-pwm-dimmer` (two-diode duty control on a pot) are 555
circuits with no microcontroller. The astable starts cold
(`[sim] uic = true`: every net at 0 V and the supply applied at t = 0), since
an astable has no DC operating point; `[sim] ic = { net = volts }` sets
chosen nets at t = 0. The desktop picker runs all three: a 555 painter draws
the file's own layout (route goldens in
`crates/of-ui/src/wiring/goldens/ne555-*.routes.txt`), every routed pin is a
meter probe point, and the scope and multimeter come from the file's
`[[instrument]]` tables.

`esp32c3-555-clock` feeds a 555 astable (about 97 Hz) to an ESP32-C3 that
counts its edges (`firmware/esp32c3-clock-counter`, our own assembly, direct
boot with no ROM) and prints the frequency on UART0. An NE555 needs at least
4.5 V, so the circuit is wired like a 5 V C3 dev board: the 555 runs on the
5 V input, an AMS1117-3.3 LDO (22 µF tantalum on its output) makes the C3's
3.3 V, and a 10k/33k divider brings the 555 output down to about 3.1 V at
GPIO4 (above V_IH, below VDD + 0.3 V; R3 also limits clamp current). A CMOS
TLC555 on the 3.3 V rail would avoid the divider, but there is no part model
for it yet. The file has a scope on the clock and the timing capacitor, a
logic analyser on GPIO4 and TX (GPIO21) and a UART console on TX.

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
