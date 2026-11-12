# Contributing to Ohmforge

Thank you for helping. This project is pre-alpha, so the best contributions right now are design feedback, part definitions and small, well-tested building blocks.

## Ground rules

- Be kind; see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- Sign off every commit (Developer Certificate of Origin, https://developercertificate.org): `git commit -s`.
- Big changes (component API, file formats, the QEMU sync protocol) start as an RFC in `rfcs/`.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`).

## Licence hygiene (important)

- Code you contribute is licensed MIT OR Apache-2.0; part data and artwork CC-BY-4.0.
- **Never** add a GPL, LGPL or AGPL dependency to the workspace. `cargo deny check` enforces this.
- Do not copy or translate code from GPL projects (Falstad CircuitJS, SimulIDE, QEMU) or proprietary ones (Wokwi). Ideas and published algorithms are fine; code is not.
- QEMU integration lives in a separate repository and process; this workspace only contains the MIT/Apache IPC client.

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check        # if cargo-deny is installed
```

## Adding a part

1. Create `parts/<category>/<manufacturer>-<part>/part.toml`, copying `parts/sensors/bosch-bme280/part.toml`.
2. Fill in pins and electrical parameters with min/typ/max from the datasheet, at least one `[[variant]]` with an MPN, and the `[license]` block.
3. Link the datasheet; do not commit it. Do not commit vendor SPICE models with restrictive terms.
4. If the part needs behaviour beyond the generic models, add a Rust model in `crates/of-components` or a WASM model.
5. Add a small test circuit with expected readings. Bench-measured parts should include a capture.

## Labels

`bug`, `feature`, `docs`, `accuracy`, `performance`, `rfc`; `area:*` (analog, cpu-riscv, cpu-xtensa, peripherals, ui, bom, library, web); `good first issue`, `help wanted`; `part-request`; `phase:P0` to `phase:P6`.
