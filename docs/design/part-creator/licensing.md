# Part creator: licensing

Status: proposed (design only). Grounded in `parts/LICENSE.md`, `CONTRIBUTING.md`, `validate_parts` in `crates/of-cli/src/main.rs`, and `DESIGN_AND_PLAN.md` §7 and §13.1.
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

## 1. Three layers

- **Code** (the creator, the validator, Rust models in `of-components`, WASM hosts): AGPL-3.0-or-later. The standalone C5 binary stays AGPL. Packs it *writes* are data, not automatically AGPL.
- **Official part data and artwork** under `parts/`: CC-BY-SA-4.0, as `parts/LICENSE.md` states. `validate-parts` today requires `license.data` and `license.artwork` to equal `CC-BY-SA-4.0` exactly.
- **Bound model files**: their own terms. Original Ohmforge macromodels (`ne555.lib`, `ams1117.lib`) are CC-BY-SA-4.0. Vendor files may not be redistributable.

## 2. Official packs

To merge into `parts/`:

- `[license] data` and `artwork` are `CC-BY-SA-4.0`.
- No vendor SPICE model whose terms forbid redistribution (`parts/LICENSE.md`, `CONTRIBUTING.md`). Store URL + checksum instead (planned `[model_source]`).
- No encrypted file (see [spice-import.md](spice-import.md) §4).
- No vendored third-party KiCad symbol or footprint libraries; footprints are **name references only** (`parts/LICENSE.md` even notes the common KiCad licence exception as a reason not to copy those libraries into the tree). Official KiCad libraries, used at C3 and by `of-kicad`, stay user-local and pinned; see [`../ecosystem-integrations.md`](../ecosystem-integrations.md) §2.1. Official packs must also refuse SnapEDA, Component Search Engine, and similar non-redistributable content.
- No committed datasheet PDFs; link them.
- Attribution for any imported, redistributable text stays in the pack (`NOTICE` or `[model_source]`).
- Share-alike: a derived official part stays CC-BY-SA-4.0 and credits the project, as the parts licence already says.

CI on `parts/` always uses official-mode validation.

## 3. Personal and third-party packs

The user may choose another licence, but it must be declared. Ohmforge will not silently relicense a vendor model as CC-BY-SA-4.0. Upstream contributions to `parts/` still require CC-BY-SA-4.0 and the right to grant it (open question: we do not accept a “personal MIT pack” into the official tree without a relicensing step).

`validate-parts` today would **fail** a personal pack that is lawfully not CC-BY-SA. C1 must add a mode:

- `ohmforge validate-parts parts/` — official (current rules, plus the tighter checks in the data-model note).
- `ohmforge part check <pack> --personal` (name *planned*) — licence must be present and non-empty; encrypted files still refused; redistributable flag respected; CC-BY-SA is not required.

Default: official when the path is inside a checkout’s `parts/` directory, personal otherwise, with the mode printed in the first line of the report so it cannot be missed.

## 4. Tooling and sidecars

ngspice, OpenVAF, and any GPL or LGPL simulator stay **out of process**, like the planned QEMU sidecar. Generated objects from a Verilog-A compile need a licence review before they are committed (see [`../model-and-waveform-interop.md`](../model-and-waveform-interop.md)). The creator must not vendor those compilers.

## 5. Exports

Exports (ngspice deck, later KiCad symbol) we *generate* are our output, but they may contain model text we copied only when `redistributable = true`. If a model is reference-only, the export includes a placeholder and a fetch instruction, not the forbidden body. Attribution from `[model_source]` is copied into the export header.
