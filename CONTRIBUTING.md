# Contributing to Ohmforge

Thank you for helping. This project is pre-alpha, so the best contributions right now are design feedback, part definitions and small, well-tested building blocks.

## Ground rules

- Be kind; see [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).
- Sign off every commit (Developer Certificate of Origin, https://developercertificate.org): `git commit -s`.
- Big changes (component API, file formats, the QEMU sync protocol) start as an RFC in `rfcs/`.
- Use Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `test:`).
- Questions and open-ended ideas go to [Discussions](https://github.com/sjnorval/Ohmforge/discussions) ([SUPPORT.md](SUPPORT.md)); security problems are reported privately ([SECURITY.md](SECURITY.md)).

## Issues

Open issues with the [forms](https://github.com/sjnorval/Ohmforge/issues/new/choose): **Bug report**, **Feature request** or **New part or model** (MPN, datasheet, model type, suppliers). Blank issues are turned off. Looking for something to work on? Try an issue labelled [`good first issue`](https://github.com/sjnorval/Ohmforge/labels/good%20first%20issue) or [`help wanted`](https://github.com/sjnorval/Ohmforge/labels/help%20wanted), and say in the issue that you are taking it.

## Licence hygiene (important)

- Code you contribute is licensed AGPL-3.0-or-later; part data, artwork and docs CC-BY-SA-4.0.
- Only add dependencies with licences on the `deny.toml` allow list (permissive licences plus AGPL-3.0). **Never** add a GPL-2.0-only or LGPL dependency. `cargo deny check` enforces this.
- Do not copy or translate code from GPL projects (Falstad CircuitJS, SimulIDE, QEMU) or proprietary ones (Wokwi). Ideas and published algorithms are fine; code is not.
- QEMU integration (planned) will live in a separate repository and process; this workspace will only contain the IPC client (`of-cpu-qemu-ipc`).

## Before you open a pull request

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check        # if cargo-deny is installed
```

UI changes also need `cargo clippy -p of-ui -p of-app --features of-ui/gui,of-app/gui --all-targets -- -D warnings` and `cargo test -p of-ui --features gui`. If you change firmware under `firmware/`, rebuild its ELF with its `build.sh` (zig 0.17.1, see [`firmware/README.md`](firmware/README.md)); CI checks the committed ELFs rebuild byte for byte.

The [pull request template](.github/pull_request_template.md) has the full checklist. In short:

- Sign off every commit and add tests for the change.
- Keep the docs current: README, `AGENTS.md`, `.grok/rules/ohmforge.md` and `docs/ROADMAP.md` where your change affects them.
- Add a line under **Unreleased** in [CHANGELOG.md](CHANGELOG.md) for any user-visible change (Keep a Changelog sections: Added, Changed, Deprecated, Removed, Fixed, Security).
- Every pull request needs the maintainer's review before it merges (`.github/CODEOWNERS`). CI on a first-time contributor's pull request starts once the maintainer approves the workflow run.

## Adding a part

1. Create `parts/<category>/<manufacturer>-<part>/part.toml` (or `parts/<category>/<part>/` for generic parts such as `parts/passives/resistor`), copying `parts/sensors/bosch-bme280/part.toml`. Parts that the analog solver should simulate also need an `[analog]` section; see `parts/passives/resistor/part.toml` and `AGENTS.md` section 3b. A planned part creator (CLI first, then a desktop window) will be the guided path; see [`docs/design/part-creator.md`](docs/design/part-creator.md). Until it exists, the hand-written flow below still applies.
2. Fill in pins and electrical parameters with min/typ/max from the datasheet, at least one `[[variant]]` with an MPN, and the `[license]` block.
3. Link the datasheet; do not commit it. Do not commit vendor SPICE models with restrictive terms. Encrypted model files are refused; public unencrypted models need a licence and checksum record ([`docs/design/model-and-waveform-interop.md`](docs/design/model-and-waveform-interop.md)).
4. If the part needs behaviour beyond the generic models, add a Rust model in `crates/of-components` or a WASM model. Under the engine plan ([`docs/design/engine-components.md`](docs/design/engine-components.md), phases E2 to E6, issues #46 to #50), once the engine API exists every part is a component on it: declare all pins, including supply and ground pins (no hidden connections to ground), and do not add per-demo glue in `of-ui`.
5. Add a small test circuit with expected readings (see `crates/of-components/tests/`). Bench-measured parts should include a capture.
6. Run `cargo run -p of-cli -- validate-parts parts/` and confirm your part prints `ok`.

## Labels

Labels come in four groups; the maintainer sets them during triage.

- **Type:** `bug`, `enhancement`, `documentation`, `question`, `part-request`, `accuracy` (differs from ngspice, a datasheet or hardware), `performance`, `rfc`, `dependencies`.
- **Status:** `good first issue`, `help wanted`, `needs design`, `blocked`.
- **Phase:** `phase:P0` to `phase:P6`, matching [`docs/ROADMAP.md`](docs/ROADMAP.md), plus `future idea` for parked ideas. Each unfinished phase also has a [milestone](https://github.com/sjnorval/Ohmforge/milestones).
- **Area:** `area:analog`, `area:cpu`, `area:soc`, `area:firmware`, `area:ui`, `area:parts`, `area:bom`, `area:ci`, `area:docs`.

Pull request labels also sort GitHub's generated release notes (`.github/release.yml`).
