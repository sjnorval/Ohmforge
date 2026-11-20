<!--
Thanks for contributing to Ohmforge. Please read CONTRIBUTING.md first.
Contributions are AGPL-3.0-or-later (code) and CC-BY-SA-4.0 (parts data, artwork, docs).
-->

## Summary

<!-- What does this change do, and why? -->

Closes #

## Type of change

- [ ] Bug fix
- [ ] New feature or model
- [ ] New or updated part (`parts/`)
- [ ] Docs only
- [ ] CI, build or tooling

## How was it tested?

<!-- Tests added or changed, circuits compared with ngspice, firmware run, screenshots for UI changes. -->

## Checklist

- [ ] Every commit is signed off (`git commit -s`, Developer Certificate of Origin) and uses Conventional Commits (`feat:`, `fix:`, `docs:` ...)
- [ ] Tests added or updated for the change (new analog features: an analytic test and an ngspice golden circuit within 1 %)
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass
- [ ] UI changes: `cargo clippy -p of-ui -p of-app --features of-ui/gui,of-app/gui --all-targets -- -D warnings` and `cargo test -p of-ui --features gui` pass
- [ ] `cargo deny check` passes (or I couldn't run it locally and CI will)
- [ ] New dependencies, if any, are AGPL-compatible (on the `deny.toml` allow list; no GPL-2.0-only or LGPL) and no code is copied from QEMU, CircuitJS, SimulIDE or Wokwi
- [ ] ngspice is not linked, vendored or shipped (it is only an external tool for golden data)
- [ ] Committed firmware ELFs under `firmware/` are rebuilt with their `build.sh` (zig 0.17.1) if their source changed, and are byte-identical
- [ ] Docs updated: README, `AGENTS.md`, `.grok/rules/ohmforge.md` and `docs/ROADMAP.md` where the change affects them
- [ ] `CHANGELOG.md` has an entry under "Unreleased" for user-visible changes
