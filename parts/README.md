# Parts library

One folder per part: `parts/<category>/<manufacturer>-<part>/part.toml`, or
`parts/<category>/<part>/` for generic parts. The schema is defined in
`crates/of-parts`; the `[analog]` section maps a part onto solver devices
(`of_components::analog`, see `AGENTS.md` section 3b). See
[CONTRIBUTING.md](../CONTRIBUTING.md) for how to add a part, and
[LICENSE.md](LICENSE.md) for licensing rules.

Current parts:

| Folder | Part id | Part |
|---|---|---|
| `passives/resistor` | `generic.resistor` | Chip resistor |
| `passives/capacitor-mlcc` | `generic.capacitor-mlcc` | Ceramic chip capacitor (X5R/X7R) |
| `passives/capacitor-tantalum` | `generic.capacitor-tantalum` | Tantalum chip capacitor 22 µF 16 V |
| `leds/red-0603` | `generic.led-red-0603` | LED, red, 0603 |
| `discretes/npn-small-signal` | `generic.npn-small-signal` | Small-signal NPN transistor (MMBT3904 class) |
| `switches/pushbutton-6mm` | `generic.pushbutton` | Tactile push button, 6 mm, SPST-NO (ideal switch: `ron`/`roff`) |
| `regulators/ams1117` | `ams.ams1117-3.3` | AMS1117-3.3 1 A LDO, with our own macromodel (`ams1117.lib`) |
| `sensors/bosch-bme280` | `bosch.bme280` | BME280 temperature, humidity, pressure sensor (supply load model) |
