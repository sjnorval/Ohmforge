# Circuit files

Circuit files (`*.circuit.toml`) describe a circuit for both the simulation and
the schematic: placed parts with typed parameters, firmware by path and
SHA-256, nets in router order, per-instance schematic geometry, keepouts and
probes. The format is [RFC 0003](../rfcs/0003-circuit-file.md) (`format = 1`).

The demo circuits arrive in `demos/` with engine phase E4b
([#48](https://github.com/sjnorval/Ohmforge/issues/48)) and are embedded in the
desktop app.

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
