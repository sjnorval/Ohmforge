# Ecosystem integrations

Status: proposed (design only, no code). This note records how accepted open tools, libraries and data sources fit Ohmforge, with phases, licence and terms constraints, and a ranked build order.
Date: 2026-11-26.
Related: [`part-creator.md`](part-creator.md) and [`part-creator/sources.md`](part-creator/sources.md); [`parts-availability.md`](parts-availability.md); [`kicad-export.md`](kicad-export.md); [`examples-and-sharing.md`](examples-and-sharing.md); [`model-and-waveform-interop.md`](model-and-waveform-interop.md); [`schematic-layout.md`](schematic-layout.md); [`COMPONENTS.md`](../COMPONENTS.md); [`parts/LICENSE.md`](../../parts/LICENSE.md); [`ROADMAP.md`](../ROADMAP.md).

Ohmforge already has a co-simulation kernel, a growing cited parts library, BOM v1, instruments, and plans for a part creator, KiCad export, availability and shareable packs. The resources below are not a second product. They are how we fill parts faster, hand a design to a board tool, price and order a build, and teach from a running circuit — without taking other people's content into the official library, and without linking copyleft tools into the workspace.

## Contents

1. [Purpose, current state and shared rules](#1-purpose-current-state-and-shared-rules)
2. [The integrations](#2-the-integrations)
3. [Differentiation: the end-to-end loop](#3-differentiation-the-end-to-end-loop)
4. [Ranked sequencing](#4-ranked-sequencing)
5. [Risks and mitigations](#5-risks-and-mitigations)
6. [Open decisions](#6-open-decisions)

Effort in this note: **S** is a few days, **M** is about one to two weeks, **L** is a phase slice (several weeks or a dedicated milestone).

## 1. Purpose, current state and shared rules

### 1.1 Goals and non-goals

Goals:

- Give every accepted resource a home in the existing crates and design notes, with a phase and a definition of done.
- Prefer user-run and out-of-process tools, official APIs, and name references over vendoring third-party libraries into `parts/` or the workspace.
- Use planning aids (popularity, in-stock status at planning time) so the path to about 50 official parts in release 0.2 prefers parts people can actually buy.
- Close an end-to-end loop: a cited part, a simulated circuit with real firmware, checks, a priced and inventory-aware BOM, a shareable rebuildable project, and a KiCad handoff.

Non-goals:

- Shipping symbol, footprint or model content from SnapEDA, Component Search Engine, or any similar service whose terms forbid redistribution, in official packs or starter projects.
- Committing live prices, stock counts, or datasheet PDFs to the repository.
- Linking GPL, LGPL or AGPL tools into Ohmforge crates (the same rule as QEMU, ngspice, OpenVAF and poppler: out of process only).
- Replacing the part creator, `of-kicad`, or `of-bom` with a third-party application.
- Treating any of these tools as a product we are comparing ourselves against in this repository.

### 1.2 What already exists

| Piece | Where it lives | What it actually does today |
|---|---|---|
| Parts schema and official library | `crates/of-parts/src/lib.rs`, `parts/**/part.toml`, `parts/LICENSE.md` | Load and `check`; footprints are name strings; no vendored KiCad libraries; no committed PDFs |
| Analog bridge and SPICE subset | `crates/of-components/src/analog.rs`, `crates/of-netlist/src/spice.rs` | Maps `[analog]` onto the solver; `to_spice`; unencrypted decks only |
| CLI validation | `crates/of-cli/src/main.rs` (`validate_parts`) | Parse, `check()`, require official `license.data` and `artwork` to be `CC-BY-SA-4.0` |
| BOM v1 | `crates/of-bom/src/lib.rs` | First `[[variant]]` only; `price_zar_cents`; `ohmforge bom` CSV and checklist |
| Datasheet tests and ngspice goldens | `crates/of-components/tests/*_datasheet.rs`, `scripts/gen-ngspice-golden.sh`, `crates/of-analog/tests/ngspice_compare.rs` | Cited figure checks; external ngspice; 1 % of span |
| Circuit files and demos | `crates/of-circuit`, `circuits/demos/` | E4 complete; official demos are circuit files |
| Instruments | E5a to E5g on `main` (listed follow-ups remain) | Scope, logic analyser and related captures; CSV and VCD planned or present as in the interop note |
| KiCad export | `of-kicad` is a placeholder; [`kicad-export.md`](kicad-export.md) | Not implemented |
| Availability | [`parts-availability.md`](parts-availability.md) | Design only; LCSC, Mouser, DigiKey in A2; Nexar (Octopart) already listed in A4 |
| Datasheet text (planned) | D3 in the roadmap; part creator C3 | MIT-licensed PDF crates in-process; poppler only as an external tool; proposals, never an automatic write |
| Examples and packs | [`examples-and-sharing.md`](examples-and-sharing.md) | Design only; bundled demos today |
| Schematic auto-layout | [`schematic-layout.md`](schematic-layout.md) | Design only; L1/L2 in P3 |

### 1.3 Shared licence and terms rules

These rules apply to every item in section 2. A resource that cannot meet them is reference-only or is dropped.

- **Code in the workspace stays AGPL-3.0-or-later. Official part data and artwork stay CC-BY-SA-4.0.** Third-party content is used only under a recorded licence that allows the intended use.
- **Copyleft tools (GPL, LGPL, AGPL) stay out of process**, the same way ngspice, QEMU, OpenVAF and poppler already do. We do not link them, and we do not vendor their source into a crate we ship.
- **MIT, BSD, Apache-2.0 and CC0 tools** may be used as libraries or vendored *with attribution* when that is clearly useful; prefer generating our own output (for example our own interactive BOM HTML) when the third-party tool is a KiCad plugin or a web app we do not want as a runtime dependency.
- **Official APIs or written permission only** for supplier and aggregator data, as already decided in [`parts-availability.md`](parts-availability.md) §4. Honour rate limits and `Retry-After`. Users supply their own keys in the OS keychain. The project never puts keys in the tree.
- **No live prices or stock in `parts/` or in git.** Dated offers live in a user-local cache or, later, in the shareable `availability/` dataset only in the form that note already allows (SKU liveness, lifecycle, alternates; never prices or stock in the scheduled job).
- **No redistribution of a third-party symbol, footprint, model or marking database** into official packs unless the licence is recorded, redistribution is clearly allowed, and `parts/LICENSE.md` is updated. Name references and user-local libraries are the default.
- **Datasheet PDFs stay on the user's machine.** Extraction tools run on a path the user chose. We store citations (document id, revision, page or figure) and a URL, not the file.
- **Terms are reviewed when a provider is added or updated**, and the module records the terms version it was written against.

## 2. The integrations

Each subsection uses the same fields: what it is for us, the user-facing feature, how it hooks the existing tree, phase, dependencies, licence and terms, acceptance, and effort.

### 2.1 Official KiCad symbol and footprint libraries

**What it is for us.** The authoritative symbol and footprint names for circuit export and, in C3, for importing a symbol (and later a footprint) into a part pack. This is the library the official KiCad project ships, not a third-party collection.

**User-facing feature.** A circuit exported from Ohmforge opens in a pinned KiCad version with symbols and footprints that resolve against the official libraries the user already has (or that KiCad installs). The part creator can import a symbol from that same pinned library. Missing footprints are listed, not invented.

**Technical integration.** Share mapping tables between [`kicad-export.md`](kicad-export.md) / the future `of-kicad` crate and part-creator C3 ([`part-creator/ratings-symbols-bom.md`](part-creator/ratings-symbols-bom.md), [`part-creator/sources.md`](part-creator/sources.md)). `part.toml` keeps footprint and library *names*, as `parts/LICENSE.md` already requires. Pin the KiCad library and file-format version in the exporter and in the creator (open decision in the part-creator pitfalls note). Do not copy library files into `parts/`.

**Phase / milestone.** P3 with `of-kicad` (name pinning and export). P4 C3 for import of a pinned symbol.

**Dependencies.** E6 for `[symbol]` on the Ohmforge side (a documented subset may ship first). A documented KiCad format pin.

**Licence and terms.** Official KiCad libraries are typically CC-BY-SA with a KiCad exception; the exact SPDX of the pinned revision is recorded at implementation and in `parts/LICENSE.md`. We stay clean by *referencing* libraries the user has, not vendoring them. Generated `.kicad_sch` / netlist files are our output in KiCad's documented formats; we do not link KiCad.

**Acceptance.** An official demo exports and opens in the pinned KiCad version; symbols and named footprints resolve; a part-creator C3 import of one single-unit official symbol round-trips; CI does not contain vendored official library files.

**Effort.** S for pinning, documentation and mapping tables; M once C3 import is included (already on the C3 estimate).

### 2.2 `kitspace/kicad_footprints`

**What it is for us.** An optional lookup when an official KiCad footprint name is missing or a contributor is hunting for a well-known package. It is a secondary source, never the default for official packs.

**User-facing feature.** The part creator can point at a user-local clone and offer a footprint *name* (and, if the licence of that particular footprint allows, a preview). Official packs still store a name, preferably the official KiCad name, not a copied file.

**Technical integration.** A documented search path in [`part-creator/sources.md`](part-creator/sources.md) and C3. No files from this collection are committed under `parts/` unless a specific footprint's licence is recorded and redistribution is allowed — the default is “do not vendor”.

**Phase / milestone.** P4, with C3.

**Dependencies.** C3; official KiCad libraries as the first source.

**Licence and terms.** Mixed provenance. Treat the collection as unredistributable in aggregate. User-local clone; licence recorded per footprint if anything is ever copied. If a licence cannot be confirmed, use only as a name hint.

**Acceptance.** Official validation refuses a pack that embeds an unlicensed third-party footprint file. The creator documents the lookup path and prefers official KiCad names.

**Effort.** S.

### 2.3 Inkscape Electric Symbols (reference only)

**What it is for us.** A CC0 drawing set we may study for conventional pin placement, body proportions and annotation, while we draw our own IEC 60617 and ANSI/IEEE 315 artwork ([`schematic-themes.md`](schematic-themes.md) T3, E6 `[symbol]`, C2).

**User-facing feature.** None directly. Authors follow Ohmforge symbol rules. We do not ship this set as official artwork, even though CC0 would allow reuse, so our symbols stay consistent and cited as our own work.

**Technical integration.** A short “conventions we studied” note in [`part-creator/sources.md`](part-creator/sources.md) and the C2 symbol editor guidance in [`part-creator/ux-and-testing.md`](part-creator/ux-and-testing.md). No files from the set in `parts/`.

**Phase / milestone.** P3 (C2), as a reference while the symbol editor is built.

**Dependencies.** C2.

**Licence and terms.** CC0. Reference-only by this plan: no import into official packs.

**Acceptance.** Official symbol artwork is ours (or another clearly recorded CC-BY-SA-4.0 source). The set is not a runtime dependency.

**Effort.** S (documentation and a checklist of conventions).

### 2.4 Tabula (PDF table extraction)

**What it is for us.** An out-of-process helper that turns datasheet tables (absolute maximum ratings, recommended operating conditions, electrical characteristics) into a CSV the part creator can propose as `[electrical]` fields and citations. It extends D3; it does not replace the planned in-process MIT PDF crates or poppler.

**User-facing feature.** In C3 datasheet-assisted fill, the author can run Tabula (or paste a CSV Tabula wrote) against a local PDF they already have, review every proposed min/typ/max, and accept field by field. Nothing is written without review.

**Technical integration.** Hook D3 ([`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) and the roadmap D3 item) and C3 ([`part-creator/ux-and-testing.md`](part-creator/ux-and-testing.md), [`part-creator/sources.md`](part-creator/sources.md)). A small CLI wrapper or documented command, in the same spirit as `scripts/gen-ngspice-golden.sh`: find the tool, run it on a user path, parse CSV, attach page/table hints to the citation `note`. PDFs stay uncommitted (`parts/LICENSE.md`).

**Phase / milestone.** P4, with D3 and C3. The workflow can be documented for authors in P3.

**Dependencies.** D1 (datasheet URL and cache) is useful but not required for a local PDF the author already has. C3 UI or C1 CLI for applying proposals.

**Licence and terms.** Tabula / `tabula-java` is MIT. The *tool* may be documented or invoked out of process. The *datasheet* remains the vendor's; we extract numbers the author reviews and cite the document. We do not redistribute PDFs or Tabula as a bundled service.

**Acceptance.** From a fixture PDF (a public-domain or self-made table, not a vendor datasheet committed to git), Tabula output proposes the expected ratings; accepting them writes citation notes; rejecting them writes nothing. Official packs still fail validation if required citations are empty.

**Effort.** M (wrapper, CSV mapping, tests with a fixture table), plus documentation in P3 (S).

### 2.5 WebPlotDigitizer (curve digitising)

**What it is for us.** An out-of-process, user-run way to digitise a datasheet curve (typical I–V, drop-out versus current, timing versus capacitance) into points the author can use for a verification fixture or a cited `[electrical]` typ/max. Complements Tabula: tables versus plots.

**User-facing feature.** The C3 fill workflow accepts a CSV or JSON of points the author exported from WebPlotDigitizer, with the figure number in the citation. The creator never silently fits a model to a curve.

**Technical integration.** Same proposal path as Tabula in [`part-creator/sources.md`](part-creator/sources.md) and C3. Optional later: a documented “open this PNG in WebPlotDigitizer” step after a page render (poppler out of process). Do not vendor or link the WebPlotDigitizer application. Do not call Automeris cloud “AI Assist” from Ohmforge.

**Phase / milestone.** P4, with C3 / D3.

**Dependencies.** C3; a local plot image the user supplies.

**Licence and terms.** The WebPlotDigitizer frontend is AGPL-3.0. Automeris cloud features are proprietary. We stay clean by treating it as a **user-run** application: we consume the author's exported points, we do not link or embed the AGPL frontend, and we do not send datasheet images to a third-party cloud from Ohmforge.

**Acceptance.** A fixture plot (ours, in tests) round-trips as reviewed points with a figure citation. No AGPL source appears in the workspace. No network call to Automeris is made by our crates.

**Effort.** S to accept reviewed point files and cite them; M if we add a guided “render this page, here is how to digitise it” assistant (still user-run).

### 2.6 JLCSearch, and prioritising the path to about 50 parts

**What it is for us.** A planning aid so official library growth in [`COMPONENTS.md`](../COMPONENTS.md) prefers parts that are commonly in stock at LCSC / JLCPCB (and that already have, or can have, a clean MPN, LCSC SKU and datasheet). Two official parts already carry LCSC SKUs. This is not a live availability provider; that remains A2 (LCSC API) and A4 (JLCPCB API).

**User-facing feature.** Contributors see a dated backlog: which target parts to add next, with MPN and LCSC number, and a reminder to re-check stock when authoring. Users of the app benefit because official examples are more likely to be orderable. There is no in-app “JLCSearch” panel required in P3.

**Technical integration.** A documented author workflow in [`part-creator/sources.md`](part-creator/sources.md) and a short rule in [`COMPONENTS.md`](../COMPONENTS.md) / [`CONTRIBUTING.md`](../../CONTRIBUTING.md). Optional: an author-local script that queries only what the terms allow and writes a *local* list. If we keep a snapshot in the repo, it lives under docs (not `parts/`), carries a date, and contains identifiers and a popularity or basic/extended class — **not** prices or a live stock count.

**Phase / milestone.** P3 (process and backlog). It should start as soon as we add the next official parts.

**Dependencies.** None for the process. A2 if we later automate through the official LCSC API instead of a web tool.

**Licence and terms.** Confirm JLCSearch's terms before any automated query or scraping. Prefer the official LCSC / JLCPCB APIs already described in [`parts-availability.md`](parts-availability.md) §4a when automation is needed. No redistribution of their search UI or of live stock.

**Acceptance.** `COMPONENTS.md` (or a doc it points at) lists the next official parts with MPN and LCSC number where applicable, a date the popularity was checked, and a note that stock must be re-checked at authoring time. New official parts added after this plan prefer that list unless a maintainer records why not (for example a teaching part that must be a specific manufacturer).

**Effort.** S.

### 2.7 SMD marking database

**What it is for us.** Optional variant lookup: a marking on a package (or a photo the author is inspecting) suggests a candidate MPN or manufacturer, which the author then confirms against the datasheet. It is not an automatic identification of a part.

**User-facing feature.** In C2/C3, “look up marking” proposes candidates. The author still types or confirms the MPN, datasheet and variant.

**Technical integration.** A provider behind the part creator ([`part-creator/sources.md`](part-creator/sources.md)), with the database either user-local or a clearly licensed open dataset. Results attach to a citation or a “proposed, unconfirmed” flag, never as a silent MPN write.

**Phase / milestone.** P4.

**Dependencies.** C2 or C3; a chosen database (open decision in section 6).

**Licence and terms.** Many marking lists have unclear or non-redistributable terms. Do not vendor a database whose licence is unclear. Prefer an open dataset with a recorded SPDX, or a user-supplied file. No scraped commercial codebook in `parts/` or in the app install.

**Acceptance.** A fixture mapping file proposes the expected candidate; the pack is not valid as official until the author confirms the MPN and a datasheet citation. CI does not contain an unlicensed marking dump.

**Effort.** S if a clean dataset is chosen; M if we must support a user-supplied file format and teaching UI only.

### 2.8 Octopart (Nexar) and Findchips

**What it is for us.** Optional dated offer, stock, lifecycle and alternate refresh for a BOM the user already has, in addition to the first-party LCSC, Mouser and DigiKey providers (A2). Nexar (Octopart) is already specified in [`parts-availability.md`](parts-availability.md) §4a and A4. Findchips is the same class of aggregator: useful for a one-off “who stocks this”, not as the default refresh, and only after a terms and API review.

**User-facing feature.** `ohmforge parts refresh` and the in-app “Check availability” already planned (A2, A3, A4): staleness badges, EOL and out-of-stock warnings, never blocking simulation. Snapshot prices in `part.toml` stay lowest-priority, as A1 already says.

**Technical integration.** Host-only `of-suppliers` as designed; user keys in the keychain; `fetched_at` and `Money` in millionths; cache TTLs; no prices in git. Hook A4. Findchips is **not** implemented until access, rate limits and terms are written down in §4a the same way Nexar already is.

**Phase / milestone.** P4 A4 for Nexar (already on the roadmap). Findchips only after that review, same phase or later.

**Dependencies.** A1, A2, A3; user keys; a terms review.

**Licence and terms.** Aggregator terms typically restrict storage, redistribution and display of offers. Follow the availability note: user-local cache, dated facts, no committed prices or stock, evaluation quotas (Nexar is metered per part returned). If Findchips has no official API or forbids this use, it stays a link the user can open, not a provider.

**Acceptance.** Already in the availability note for Nexar: a refresh with the user's key writes dated offers to the cache; `bom check --offline` still works; nothing in git contains prices. Findchips: a terms review recorded, or an explicit “link only” decision.

**Effort.** M for Nexar as part of A4; S for a Findchips review (implementation only if the review says yes).

### 2.9 InvenTree and Part-DB

**What it is for us.** A way to answer “what do I already own?” against a BOM: the user exports (or we read, with permission) their inventory from a local InvenTree or Part-DB instance, and Ohmforge marks BOM lines as in-stock-in-the-lab, short, or untracked.

**User-facing feature.** `ohmforge bom check --inventory <file>` (and later an in-app import): a column or badge per line, plus a shortfall list. Inventory data never leaves the machine unless the user exports it themselves. Simulation does not depend on it.

**Technical integration.** A new import in `of-bom` / `of-suppliers` (file-based first: CSV or the project's documented export). Optional later: a local HTTP API the user enables, with a base URL and token in the keychain, never in the project pack ([`examples-and-sharing.md`](examples-and-sharing.md) already forbids packing secrets). Match on manufacturer + MPN, then SKU.

**Phase / milestone.** P4, after A1 (the offer model and offline check).

**Dependencies.** A1; a documented export schema for each tool.

**Licence and terms.** InvenTree is MIT; we may speak its export format freely. Part-DB is copyleft (confirm the exact SPDX at implementation, expected AGPL). We stay clean by using **file exports and, if needed, HTTP to a user-run server** — we do not link Part-DB, vendor it, or ship it. We do not scrape a hosted instance.

**Acceptance.** A fixture inventory file marks known demo BOM lines as owned or short. A pack export still contains no inventory and no tokens. Missing inventory is a warning, not a failed simulation.

**Effort.** M.

### 2.10 Interactive HTML BOM (InteractiveHtmlBom-style)

**What it is for us.** A self-contained HTML build aid derived from our circuit and BOM, so a user can search lines, see quantities, MPNs, SKUs and ZAR snapshots, and click a line to highlight the corresponding symbol on the schematic (and, later, a footprint if a board export exists). This is our generator, inspired by the interaction model of InteractiveHtmlBom, not a wrap of that KiCad plugin.

**User-facing feature.** `ohmforge bom --html` (and a button in the app) writes a single HTML file the user can open offline, attach to a project pack, or host. Official examples include one for each starter project.

**Technical integration.** `crates/of-bom/src/lib.rs` plus schematic geometry from the E4 circuit file and the existing router (`docs/design/wire-routing.md`). After E6, `[symbol]` art is used; until then, the demos' laid-out schematics are enough to start. Do not depend on Pcbnew or on the InteractiveHtmlBom Python plugin. XLSX and CSV remain as already planned.

**Phase / milestone.** P3 (HTML from circuit + BOM v1 for official demos; arbitrary circuits when E6 lands).

**Dependencies.** BOM v1 (done). E4 (done). E6 for general symbol art; demos can ship earlier.

**Licence and terms.** InteractiveHtmlBom is MIT, but we will **not** vendor it as a runtime dependency (it is a KiCad plugin). If we later reuse a small MIT helper, we keep the notice. Our HTML and script are AGPL with the rest of the code, or a documented more-permissive exception if we want packs to ship a page others can reuse more easily — open decision in section 6.

**Acceptance.** Each official demo produces an HTML BOM that lists every placed part with MPN (where present) and the BOM v1 ZAR snapshot, works offline, and highlights the instance when a row is selected. `cargo test` covers the generator from a fixture circuit. No Python/KiCad dependency in CI.

**Effort.** M.

### 2.11 WaveDrom JSON from captures

**What it is for us.** A portable timing-diagram export from logic-analyser and digital-bus captures, so a cited check or a lesson can show a waveform in documentation and in the mdBook without a screenshot.

**User-facing feature.** From the logic analyser (and later UART or bus decodes), “Export WaveDrom” writes JSON the author can paste into docs or render with WaveDrom. Optional: a preview in the app.

**Technical integration.** Hook [`model-and-waveform-interop.md`](model-and-waveform-interop.md) and the E5 instrument path (alongside CSV and VCD). A small exporter in `of-instruments` or the CLI: selected channels, a time window, a clock if the user set one, and a documented subset of the WaveDrom JSON schema. We emit data; we do not need to bundle the renderer in P3 (MIT, so a later preview is allowed with attribution).

**Phase / milestone.** P3, because E5a–E5g are already on `main`.

**Dependencies.** E5 captures; a chosen window of samples.

**Licence and terms.** WaveDrom is MIT. The JSON schema is used as a format. Bundling the renderer later requires keeping the MIT notice. Export is ours.

**Acceptance.** A fixture capture (for example a demo UART or GPIO toggle) exports JSON that validates against the documented subset and, when rendered, shows the same edges as the VCD or CSV for that window. Tests do not require a browser.

**Effort.** S, or M if an in-app preview is included in the first slice.

### 2.12 KiCanvas (companion viewer)

**What it is for us.** A reference and an optional companion for viewing exported KiCad schematics in a browser, including in the P4 web app, without writing a full KiCad viewer ourselves in the first slice.

**User-facing feature.** After `of-kicad` export, “Open in viewer” can load the `.kicad_sch` in KiCanvas (vendored or clearly attributed) or tell the user how to open it. Ohmforge's own schematic editor remains the place where simulation happens.

**Technical integration.** Post-export hook in [`kicad-export.md`](kicad-export.md) and, in P4, the web app. Optional vendor of the MIT viewer with its licence file. Not a dependency of desktop `ohmforge run`.

**Phase / milestone.** P4, after `of-kicad`.

**Dependencies.** P3 KiCad export; pinned file format.

**Licence and terms.** KiCanvas is MIT. Attribution required if vendored. It reads KiCad files the user already has or that we generated. We do not pass KiCad library sources we are not allowed to redistribute.

**Acceptance.** An official demo export opens in the companion viewer and shows the same symbols and nets as in KiCad for a documented test sheet. The desktop app still runs if the viewer is absent.

**Effort.** S to document and link; M to vendor a preview in the web app.

### 2.13 tscircuit (study only)

**What it is for us.** A study input for circuits-as-code and for schematic auto-layout ideas, feeding [`schematic-layout.md`](schematic-layout.md) L1/L2. Ohmforge already has a text circuit file (`crates/of-circuit`, RFC 0003) and a planned deterministic layout.

**User-facing feature.** None from tscircuit itself. Users keep editing `.circuit.toml` (and later the schematic editor). We may, if useful, document a one-way example of how a simple official demo looks as circuits-as-code — in our format, not theirs.

**Technical integration.** Notes and metrics only in the layout design; no dependency, no vendored runtime, no copy of their library of parts.

**Phase / milestone.** Ongoing study during P3 L1/L2.

**Dependencies.** None.

**Licence and terms.** Study of public ideas and published algorithms is fine. Do not copy source or part data. If a specific technique is adopted, implement it ourselves and record the inspiration in the layout note.

**Acceptance.** No tscircuit code in the tree. Layout work continues against the acceptance already in [`schematic-layout.md`](schematic-layout.md).

**Effort.** S (study and a short addendum to the layout note when a technique is actually adopted).

### 2.14 Rebuild-and-buy starter packs (Kitspace-style)

**What it is for us.** Official examples and user project packs that someone else can open, simulate, and then buy: a Kitspace-style “rebuild this project” path, implemented with our pack format, ZAR BOM, supplier links, and later availability and inventory badges. Optional: a Kitspace-compatible manifest so a user can also publish the same project there, if their licences allow.

**User-facing feature.** Each official example has a pack that contains the circuit, firmware or a rebuild pointer, notes, a BOM (CSV and the HTML from §2.10), and buy links or a “check availability” action. Opening the pack is enough to run the circuit offline. Buying is always a click out to a supplier or a file the user uploads, never a silent order.

**Technical integration.** [`examples-and-sharing.md`](examples-and-sharing.md) pack format (`ohmforge pack` / `unpack`), `crates/of-bom`, availability A1–A3, and the HTML BOM. Starter packs stay CC-BY-SA-4.0 for text and artwork, AGPL for firmware source we own. Secrets and inventory are never packed. A Kitspace manifest, if added, is generated and optional.

**Phase / milestone.** P3 for packs + ZAR BOM + HTML + documented buy links on official examples (using BOM v1 snapshots, clearly dated). Live refresh and inventory land with A3 and §2.9 in P4.

**Dependencies.** Official example gallery item; BOM v1 (done). E6 for arbitrary user circuits; demos can start now. A3 for live badges.

**Licence and terms.** Our packs are our licences. Kitspace itself is a third-party site with its own terms — we do not upload official packs from CI unless a maintainer later decides to and the terms allow it. We do not copy other people's Kitspace projects into `circuits/`. Supplier links must not scrape or deep-link in a way a site forbids; prefer official product URLs or a cart file the API already allows ([`parts-availability.md`](parts-availability.md)).

**Acceptance.** At least five official examples unpack, run, and produce a BOM with MPNs and a dated ZAR total, plus an HTML BOM. A packed project contains no keys. A second machine can open the pack offline and rebuild the firmware from the documented pointer.

**Effort.** M, largely overlapping the already-planned gallery and pack work.

### 2.15 KiBot (KiCad CI round-trips)

**What it is for us.** An out-of-process CI helper that, given an exported KiCad project, produces the artefacts we want to check: schematic PDF or SVG, BOM, ERC if configured, and later Gerber/CPL when we explore fabrication. This is how we regression-test `of-kicad` output without linking KiCad.

**User-facing feature.** Contributors see CI fail if an official demo's export no longer opens or no longer emits the expected BOM fields. Users can optionally run the same config locally after installing KiCad and KiBot.

**Technical integration.** A documented KiBot config and a CI job that starts from `ohmforge` export, then runs KiBot as an external tool (same pattern as `scripts/gen-ngspice-golden.sh`). Not a crate dependency. Hook [`kicad-export.md`](kicad-export.md) and the fabrication “explore” item.

**Phase / milestone.** P3 once `of-kicad` can emit a schematic; broaden in P4.

**Dependencies.** `of-kicad`; a pinned KiCad version in CI; licence-clean official library availability in that image.

**Licence and terms.** KiBot is copyleft (confirm SPDX at implementation). Out of process only; not vendored, not linked, not shipped inside the Ohmforge binary. CI images must follow both KiCad's and KiBot's licences.

**Acceptance.** One official demo export is run through KiBot in CI (or a documented nightly, if the image is too heavy for every PR) and the BOM fields MPN and footprint match a golden. A missing KiBot install skips the job with a clear log, or the job is clearly marked optional, until it is required.

**Effort.** M (export + CI glue), after `of-kicad` itself.

### 2.16 Lessons In Electric Circuits, and an interactive-textbook style

**What it is for us.** Open teaching text we can *link* and, where the licence clearly allows, quote with attribution, plus a style for official lessons that are executable: a short page, a packed circuit, a WaveDrom or scope view, and a cited check. “Ultimate Electronics–style” here means interactive, running examples — we do not copy that work.

**User-facing feature.** In P6 (and as short notes beside P3 official examples), lessons in the mdBook that open a pack and show what to measure. Volume links to Lessons In Electric Circuits where a topic is already explained well.

**Technical integration.** [`examples-and-sharing.md`](examples-and-sharing.md), the mdBook under `docs/src/`, cited checks, WaveDrom exports, and later the part-authoring guide (C5). No wholesale import of a textbook into the repo.

**Phase / milestone.** P3: each official example already has a clear write-up (already planned). P6: a fuller lessons path in “complete documentation”.

**Dependencies.** Official examples; checks; preferably WaveDrom.

**Licence and terms.** Lessons In Electric Circuits is published under a Creative Commons licence (confirm the exact CC version and attribution text before any excerpt). Until that is recorded, **link, do not copy**. Other textbooks named only as a style reference are not sources we excerpt. Our lesson text is CC-BY-SA-4.0.

**Acceptance.** At least one official example page links a relevant open-text section and runs a cited check. Any excerpt, if later approved, carries the required attribution and licence notice. No unlicensed book content in the tree.

**Effort.** S in P3 (links and example write-ups); L for a full interactive lessons set in P6.

## 3. Differentiation: the end-to-end loop

No one tool in this list, and no one Ohmforge crate on its own, is the product. The point is the loop that the pieces close when they are used together.

1. **Datasheet to a verified part.** An author takes a local datasheet, uses Tabula and, when needed, WebPlotDigitizer as user-run helpers, and the part creator to propose ratings, pins, a model and citations. Official KiCad libraries (and only with care, a local footprint lookup) supply names. JLCSearch-style popularity guides *which* part to add so the official set stays buyable. An SMD marking lookup can suggest a variant; the author still confirms it. SPICE import stays licence-recorded and unencrypted. Verification is ngspice out of process and cited datasheet tests (`of-components` tests, `scripts/gen-ngspice-golden.sh`). The official pack that lands in `parts/` is CC-BY-SA-4.0 data we can stand behind.

2. **Simulate with real firmware.** That part is placed in an E4 circuit file and runs on the Ohmforge kernel with unmodified firmware, the same path as the ESP32-C3 demos already on `main`. Instruments (E5) show the analogue and digital behaviour; WaveDrom and standard waveform files make a capture citable in a lesson or a check.

3. **Cited checks.** Compatibility and rating checks ([`compatibility-checks.md`](compatibility-checks.md)) use the citations that came with the part, not unsourced magic numbers. Stress states and signal-flow views can use the same ratings later.

4. **A priced, orderable, inventory-aware BOM.** `of-bom` already emits MPNs and ZAR snapshots. Availability providers (LCSC, Mouser, DigiKey, and later Nexar or another aggregator if terms allow) refresh dated offers into a user-local cache. InvenTree or Part-DB import answers what is already on the shelf. The interactive HTML BOM is the page you take to the bench. Stale prices are labelled; they are never buried in `part.toml` as if they were current.

5. **A shareable, rebuildable project.** `ohmforge pack` carries the circuit, the firmware or a rebuild pointer, the notes, and the BOM. Someone else unpacks it, simulates the same circuit, and can buy the same bill. Official starter packs are the quality bar.

6. **KiCad handoff.** `of-kicad` writes a schematic and netlist against pinned official libraries. KiBot, out of process, is how CI proves the export still works. KiCanvas is an optional web companion for reading that export. Fabrication stays in KiCad, as [`kicad-export.md`](kicad-export.md) already says.

That is the loop we are aiming at: **datasheet → verified part → simulate with real firmware → cited checks → priced, orderable, inventory-aware BOM → shareable rebuildable project → KiCad handoff.** Individual tools already do some of those steps. The work in this note is to make the steps share one part pack, one circuit file, one BOM, and one set of licence rules, so a hobby project can be designed, proven, bought and handed on without starting again in a new tool at each stage.

## 4. Ranked sequencing

Build in this order. Earlier items unblock later ones or are cheap process wins. Items already on the roadmap keep their milestones; this list only ranks the *new or newly specified* work and how it should land relative to those milestones.

| Rank | Item | Phase | Effort | Why this position |
|---|---|---|---|---|
| 1 | JLCSearch-guided official-parts backlog (§2.6) | P3 | S | Release 0.2 is gated on about 50 cited parts. A dated, buyable backlog can start before any new crate exists. |
| 2 | Pinned official KiCad libraries and mapping tables (§2.1) | P3 export; P4 C3 import | S (pin and maps) | Already required by `of-kicad` and C3. Doing the pin and the name maps first keeps export and the creator on the same authority and avoids vendoring. |
| 3 | Interactive HTML BOM and starter-pack rebuild-and-buy (§2.10, §2.14) | P3 | M | This is the first user-visible close of the loop: a demo you can run, pack, and take a bill to the supplier with. BOM v1 and E4 are already on `main`. |
| 4 | WaveDrom export from E5 captures (§2.11) | P3 | S | Instruments are already on `main`. Small, and it feeds lessons and cited timing checks. |
| 5 | Tabula + WebPlotDigitizer in the C3 / D3 fill workflow (§2.4, §2.5) | P4 (workflow text in P3) | M | Raises the quality of the 50 parts; blocked on C3/D3 for the in-app proposals. |
| 6 | Availability A1–A3, then A4 Nexar; Findchips only after review (§2.8) | P3 A1–A3; P4 A4 | already planned (M–L) | Live, dated prices and lifecycle are what make the rebuild-and-buy path honest. |
| 7 | KiBot CI round-trip (§2.15) | P3 once export exists | M | Guards the handoff. |
| 8 | InvenTree / Part-DB inventory check (§2.9) | P4 | M | Natural extension of `bom check` after A1. |
| 9 | KiCanvas companion (§2.12) | P4 | S–M | Useful once there is an export to view. |
| 10 | SMD marking lookup (§2.7) | P4 | S–M | Nice for variant hunting; needs a clean dataset first. |
| 11 | `kitspace/kicad_footprints` as a secondary, user-local lookup (§2.2) | P4 | S | After official KiCad names are the default. |
| 12 | Inkscape Electric Symbols as C2 reference (§2.3) | P3 (docs) | S | Parallel with the symbol editor; no code dependency. |
| 13 | tscircuit study feeding L2 (§2.13) | P3 (study) | S | Already compatible with the layout note; no ship. |
| 14 | Lessons path and open-text links (§2.16) | P3 notes; P6 lessons | S then L | Example write-ups can start once packs exist; a full textbook-style path is a documentation milestone. |

**The first three to build** are therefore: the parts backlog, the official KiCad library pin and maps, and the interactive HTML BOM with rebuild-and-buy packs. WaveDrom is the first follow-up if a fourth small slice is available in P3.

## 5. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Aggregator or supplier **terms of service change**, or a use we relied on is withdrawn | Each provider module records the terms version it was written against; a review is part of adding or updating one (already in the availability note). Features degrade to manual CSV import and a link. No feature may require a single aggregator. |
| **API keys, quotas and rate limits** (Nexar is metered per part; LCSC, Mouser, DigiKey have daily limits) | User-owned keys; budgets; `Retry-After`; Nexar only as a one-off lookup, not the default refresh. The scheduled job, if it ever runs, uses project secrets and never prices in git. Fork PRs do not see those secrets. |
| **PDF table and curve extraction errors** (merged cells, rotated tables, log axes, poor scans, typ/max swapped) | Proposal-only; field-by-field accept; official validation still requires citations; tests use *our* fixture PDFs and plots, never a vendored datasheet; authors are told to check log scales and units (see also part-creator units pitfalls). A bad extract must be easy to discard. |
| **Stale prices and stock** in snapshots or in a docs backlog | Every offer has `fetched_at` (A1). BOM v1 `price_zar_cents` is labelled as a snapshot. JLCSearch backlogs are dated and are not stock. The UI shows staleness badges (A3) and never blocks simulate. |
| **Mixed or unclear licences** in footprint collections, marking databases, or extracted symbols | Default to name references and user-local files. Official `validate-parts` refuses unlicensed embeds. SnapEDA / Component Search Engine content is banned from official packs and starter projects. |
| **Copyleft contamination** (WebPlotDigitizer AGPL, KiBot copyleft, Part-DB expected AGPL) | Out-of-process or user-run only; no vendoring of those trees; CI optional when the tool is absent. |
| **Automeris or other cloud AI** receiving a vendor datasheet we are not allowed to upload | Ohmforge does not send datasheet images or PDFs to a third-party cloud. Digitising stays on the author's machine. |
| **KiCad version drift** so exports or KiBot goldens break | Pin the format and the CI KiCad version; record it next to the mapping tables; one official demo as a canary. |
| **Popularity lists bias the library** toward one assembler or region | The backlog is a guide, not a gate. Maintainers may add a teaching or regional part with a recorded reason. ZAR and South African suppliers remain first-class in the availability plan. |
| **Inventory or API tokens leaking into packs or logs** | Same secret rules as availability: keychain, never project files, never pack, never crash reports. Inventory imports are local. |

## 6. Open decisions

These need a maintainer call before the relevant slice is implemented. They do not block writing the backlog or the design hooks in this pull request.

1. **Confirm the first three slices** in section 4 (backlog, official KiCad pin and maps, interactive HTML BOM + rebuild-and-buy packs), or reorder them.
2. **Repo snapshot of the parts backlog.** May we keep a dated, price-free list of target MPNs and LCSC numbers under `docs/` (not `parts/`), or should that list stay author-local?
3. **Exact KiCad application and official-library revision to pin** (also still open in the part-creator decisions and [`kicad-export.md`](kicad-export.md)).
4. **Which SMD marking dataset**, if any, has a licence we are willing to ship or to document as a user-supplied file only.
5. **Findchips:** terms and API review, or “link only”.
6. **Interactive HTML BOM licence for the generated page:** keep AGPL with the app, or grant a more permissive licence on the generated HTML so a pack is easy to host. Recommendation: AGPL for the generator, and treat the generated HTML as the user's output (they can ship it with their project).
7. **Whether official examples ever ship a Kitspace-compatible manifest**, and whether CI may publish them to any third-party gallery (recommendation: generate an optional manifest; do not publish from CI unless later agreed).
8. **Lessons In Electric Circuits:** record the exact Creative Commons version and required attribution, then decide link-only versus short excerpts.
9. **Generated HTML BOM preview in the desktop app versus a file the user opens in a browser** for the first slice (recommendation: file first, in-app preview later).
10. **KiBot in every PR versus a documented nightly** once `of-kicad` exists (depends on image size and licence of the CI snapshot).

The open part-creator decisions in [`part-creator/pitfalls-and-decisions.md`](part-creator/pitfalls-and-decisions.md) §3 are unchanged.
