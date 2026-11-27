# Parts library licence

All part data (`part.toml`), symbols and breadboard artwork in this folder are
licensed under the **Creative Commons Attribution-ShareAlike 4.0 International**
licence (CC-BY-SA-4.0): https://creativecommons.org/licenses/by-sa/4.0/

If you share a modified or derived part, you must credit Ohmforge and share it
under the same licence.

Rust or WASM behaviour models that live alongside parts are code, and are
licensed AGPL-3.0-or-later like the rest of the workspace.

Do not commit:

- vendor SPICE models whose terms forbid redistribution (store a URL and a
  checksum in `part.toml` instead);
- KiCad library symbols or footprints (CC-BY-SA 4.0 with KiCad's own exception); reference footprints by
  name only;
- datasheets (link to them);
- symbol, footprint or model content from SnapEDA, Component Search Engine, or any similar service whose terms forbid redistribution.

Every `part.toml` must include a `[license]` block; CI rejects parts without one.
