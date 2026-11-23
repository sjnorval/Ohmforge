# RFC NNNN: Title

- **Status:** Draft | Accepted | Rejected
- **Author:** Name \<email\>
- **Date:** YYYY-MM-DD
- **Tracking issue:** #

Copy this file to `rfcs/NNNN-short-title.md` (next free number) and open a pull request.
RFCs are required for changes to the component API, file formats, and the QEMU sync
protocol (see [`CONTRIBUTING.md`](../CONTRIBUTING.md)). Smaller changes do not need one.

## Summary

One short paragraph: what this proposal changes and why.

## Motivation

Why the current state is not enough. Link related issues, design docs or demos.

## Guide-level explanation

How a contributor or user will experience the change once it lands. Prefer examples
(circuit snippets, API sketches, CLI commands) over abstract claims.

## Reference-level design

Precise behaviour: types, invariants, file layouts, wire protocols, error cases.
Call out determinism requirements (no `HashMap` iteration order, no wall-clock in
simulation results) and any wasm32 impact (forbidden OS/threads dependencies in
simulation crates).

## Determinism and wasm32 impact

- Does this affect replay, snapshots or cross-platform bit-identical runs?
- Do any new crates need to build for `wasm32-unknown-unknown`?

## Licensing impact

Ohmforge code is AGPL-3.0-or-later; parts data, artwork and docs are CC-BY-SA-4.0.
Dependencies must stay on the `deny.toml` allow list (no GPL-2.0-only or LGPL).
Do not copy code from QEMU, Falstad CircuitJS, SimulIDE or Wokwi. QEMU, if used,
stays out of process via `of-cpu-qemu-ipc` and is never linked or `dlopen`ed.

State any new dependencies, licence texts, or content that needs a NOTICE update.

## Alternatives

What else was considered and why it was rejected or deferred.

## Unresolved questions

Open points that reviewers should help settle before acceptance.
