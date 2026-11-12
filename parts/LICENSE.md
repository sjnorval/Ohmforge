# Parts library licence

All part data (`part.toml`), symbols and breadboard artwork in this folder are
licensed under the **Creative Commons Attribution 4.0 International** licence
(CC-BY-4.0): https://creativecommons.org/licenses/by/4.0/

Rust or WASM behaviour models that live alongside parts are code, and are
licensed MIT OR Apache-2.0 like the rest of the workspace.

Do not commit:

- vendor SPICE models whose terms forbid redistribution (store a URL and a
  checksum in `part.toml` instead);
- KiCad library symbols or footprints (CC-BY-SA 4.0); reference footprints by
  name only;
- datasheets (link to them).

Every `part.toml` must include a `[license]` block; CI rejects parts without one.
