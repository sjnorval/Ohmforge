# Circuit files licence

All circuit files (`*.circuit.toml`) in this folder, and the schematic layouts
they store, are licensed under the **Creative Commons Attribution-ShareAlike
4.0 International** licence (CC-BY-SA-4.0):
https://creativecommons.org/licenses/by-sa/4.0/

If you share a modified or derived circuit, you must credit Ohmforge and share
it under the same licence.

The firmware a circuit file names is not covered by this licence: it keeps the
licence of its own source folder (`firmware/`, AGPL-3.0-or-later for Ohmforge's
test firmware). Circuit files reference firmware by path and SHA-256 only.

Do not commit:

- Espressif's ROM or bootloader images (they are fetched by
  `scripts/fetch-esp-rom-elfs.*`, never committed);
- firmware binaries whose licence forbids redistribution;
- KiCad library symbols or footprints; reference them by name only.
