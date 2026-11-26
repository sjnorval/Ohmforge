# Official examples and file-based sharing

Status: proposed (design only, no code). Nothing in this document is implemented yet except the bundled desktop demos described in section 1.
Date: 2026-11-25.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §8 (firmware loading), §9 (user interface), future ideas for cloud accounts in [`ROADMAP.md`](../ROADMAP.md); [`engine-components.md`](engine-components.md) (E4 circuit files); [`editor-integration.md`](editor-integration.md) (loading firmware from a file or an editor); [`first-schematic.md`](../src/guide/first-schematic.md); [`ecosystem-integrations.md`](ecosystem-integrations.md) (rebuild-and-buy packs, interactive HTML BOM, lessons).

People should be able to open a finished, well-explained project in one step, and to hand a project to someone else as a file, without creating an account or using a server. This note plans an official example gallery in the app and a shareable project pack that travels as a folder or archive. Cloud accounts and cross-device sync stay a parked future idea; they are not a dependency.

## 1. Current state

On `main` (26 November 2026) the five official demos are circuit files in `circuits/demos/` (E4 complete, #73 and #77 to #80). `ohmforge run <file.circuit.toml>` runs any such file headless. The desktop app still launches those bundled demos from the embedded files rather than a general File → Open of an arbitrary project, and there is no pack or share-link format yet.

## 2. Goals and non-goals

Goals:

- An official gallery of complete, documented examples that open in the app the same way a user project will.
- At least five examples for release 0.2, covering the P2 demo set plus one slightly richer board (for example a sensor on I2C once B2 lands, or a clear “next step” that still runs with the parts available at the time).
- A shareable **project pack**: one folder (or a zip / `.ohmforge` archive of that folder) that contains the circuit, the firmware image or a clear pointer to how to rebuild it, notes, and any extra BOM lines. Opening the pack on another machine, offline, is enough. Official starter packs also include a rebuild-and-buy path: a dated ZAR BOM, an interactive HTML BOM, and supplier links ([`ecosystem-integrations.md`](ecosystem-integrations.md) §2.14). A Kitspace-compatible manifest, if added, is optional and generated; official packs are not published to a third-party gallery from CI unless a maintainer later agrees.
- A stable, documented share-link form that the P4 web app can open (a path, a `file:` URL, or an HTTPS URL of a pack). No login.
- Examples and pack text stay under CC-BY-SA-4.0; any firmware source in the tree stays under AGPL-3.0-or-later unless it is a third-party official image fetched by a pinned script and never committed (the same rule as the ROM and ESP-IDF bootloader).

Non-goals:

- Accounts, a hosted gallery service, or a billing model. Those remain parked under cloud accounts.
- Uploading a pack to Ohmforge infrastructure.
- Bundling other people's closed toolchains or unlicensed firmware.
- Real-time collaboration on a running simulation.

## 3. User-facing behaviour

**Gallery.** File → Examples (name *planned*), and a list on the load screen next to “New project” and “Open”. Each entry shows a title, a short description, the chip, the parts count, and a difficulty mark (beginner or intermediate). Choosing one copies it into a new project folder the user owns, then opens it, so they can edit without changing the original. A short README in the project explains what the circuit does, how to rebuild the firmware, and what to try.

**Packs.** File → Export pack writes a zip or `.ohmforge` archive of the project folder. File → Open pack unpacks into a folder the user chooses and opens it. `ohmforge pack <project>` and `ohmforge unpack <archive> <dir>` do the same on the command line. The pack lists its format version. Secrets never go in a pack: supplier API keys, session tokens and the contents of the OS keychain are excluded, and `bom.toml` only carries prices the user typed themselves.

**Share links.** A pack can be pointed to by a path or an HTTPS URL of the archive (for example a raw file on a git host the user already uses). The desktop app (P3) and the web app (P4) accept that URL, download or open the pack with the user's consent, and then work offline. There is no Ohmforge account in this flow. If the web app is asked to open a pack from the network, it says so and does not send the pack on to any other server.

## 4. Technical approach

- The pack is the E4 project folder with a small `project.toml` manifest (name, format version, chip, firmware path or rebuild recipe, licence notes). Reuse the circuit file as-is; do not invent a second circuit format.
- Official examples live in-tree under `examples/` (path *planned*). CI opens each pack, loads it, and runs a short, deterministic smoke (boot, a named assertion on a pin or a UART line) so examples cannot rot.
- Firmware images in examples follow the existing rule: committed images are built in CI from in-tree source, or fetched by a pinned, checksummed script and never committed when the file cannot be redistributed.
- Share-link parsing is a pure function of the URL or path plus the pack format, so desktop and web stay in agreement. The web implementation comes with `of-web` in P4; P3 documents the form and implements it in the desktop app and the CLI.

## 5. Phasing

- **P3 / 0.2:** gallery in the desktop app; at least five official examples; pack export and import; CLI pack and unpack; documented share-link form; CI smoke for every official example. Needs E4 (and E6 for parts in the examples). An I2C example waits on B2.
- **P4 / 0.3:** the web app opens the same packs and share links, with a network-consent prompt.

## 6. Acceptance criteria

- A new user can open an official example from the gallery, run it, and change a part or a value without editing Rust.
- A pack created on one machine opens on another with no account, including offline, and produces the same smoke result in CI.
- Packs contain no keychain material or API keys.
- The five 0.2 examples each have a short README, a CI smoke, and a BOM with MPNs and a dated ZAR total (CSV and HTML).

## 7. Open questions

- The exact file extension (`.zip` versus `.ohmforge`) and whether a folder-without-archive is a first-class “pack” (likely yes: a pack *is* a folder).
- Whether official examples are copied or opened in place as read-only with an explicit “Make my copy” action. Copying into a user folder is the default above.
