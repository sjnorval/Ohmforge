# Compatibility and electrical-rule checks

Status: proposed (design only, no code). This note is the user-facing and `part.toml` side of checks that [`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) already plans as `ohmforge review` (AI4) and the datasheet limits table (D2).
Date: 2026-11-25.
Related: [`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) §6c and §7; [`component-stress-states.md`](component-stress-states.md); [`buses.md`](buses.md) (I2C rise-time checks); [`parts-availability.md`](parts-availability.md); [`COMPONENTS.md`](../COMPONENTS.md).

A simulation that is numerically careful is still easy to wire wrongly: an LED with no series resistor, a 5 V signal into a 3.3 V input, two parts whose logic levels do not meet, a missing pull-up. This note plans two things that should feel like one feature: **cited interface and rating data on the part**, and a **visible check panel** that uses that data, with every number traceable to a datasheet page or a standard.

## 1. Current state

`part.toml` already carries electrical ratings used by the models and by the planned stress service (V1). A limits-table sketch (D2) and a rule-based design review (AI4: missing pull-ups, unterminated CAN, overvoltage on 3.3 V GPIOs, missing decoupling, LEDs without resistors, exceeded ratings) are designed in [`ai-and-scenario-sweeps.md`](ai-and-scenario-sweeps.md) for P4. There is no inspector panel yet, and there is no required citation on every rating.

## 2. Goals and non-goals

Goals:

- Every rating or interface claim that a check uses is stored on the part (or on a cited standard) with a source: document, revision or date, and page or clause. Unsourced numbers are not used as a hard fail.
- A check panel in the app, and the same findings from `ohmforge review`, written in plain language (“this LED on a 3.3 V GPIO has no series resistor”, “this input’s Vih is above that output’s Voh at the stated supply”).
- A first slice in P3, so a user placing parts can see basic wiring mistakes before the full P4 review lands.
- “Will these two pins work together?” for logic levels, supply voltage, and (where the data exists) current, using only cited figures.

Non-goals:

- Inventing a typical voltage when the datasheet was not read. Mark the field pending and skip the check.
- Applying a suggested fix automatically. The user changes the circuit.
- Replacing the P4 review, the D2 limits table, or the stress-state visuals. This note says how those results are shown early, and how `part.toml` carries citations.

## 3. User-facing behaviour

Selecting a part opens an inspector that lists its cited ratings (supply range, logic levels, abs-max, recommended operating conditions) with the source under each figure. Selecting two pins, or running Checks, lists findings with a severity (info, warning, error), the parts involved, and the citation. A finding can be opened to the datasheet page the user already has locally (D1/D2), or to the recorded URL.

`ohmforge review` in P4 remains the batch form of the same rules (text, JSON, SARIF), as already designed. The P3 panel is a subset that does not wait on datasheet extraction (D3) or on MCP.

## 4. Technical approach

- Extend `part.toml` (CC-BY-SA-4.0) with an explicit, cited interface block. A sketch (illustrative, not final):

```toml
# DRAFT: illustrative only.
[interface.gpio]
vih_min = { value = "2.31V", source = { url = "https://example.invalid/datasheet.pdf", page = 17, rev = "3.1", date = "2023-06" } }
vil_max = { value = "0.99V", source = { url = "https://example.invalid/datasheet.pdf", page = 17, rev = "3.1", date = "2023-06" } }
# pending = true  # allowed when a figure is not yet reviewed
```

- Reuse the D2 limits table once it exists; do not keep two unsynchronised rating lists. P3 may start with a small number of hand-cited fields on the official parts, then D2 becomes the single store.
- Rule code lives in a review crate or module (the AI4 plan), with no model bundled and no network call. Each rule declares the fields it needs and is a no-op when they are pending.
- Findings are data (id, nets, parts, severity, message, citations) shared by the panel, the CLI and, later, MCP.

**P3 first-slice rules** (need only E6 and the cited fields actually filled in):

- LED (or other diode indicator) with no series current-limiting element on a GPIO or supply.
- Supply net outside a part’s cited recommended range (and a stronger finding at abs-max).
- Output high or low level that does not meet a connected input’s cited Vih / Vil at that supply, when both sides have citations.
- Missing decoupling on a part that cites a bypass requirement, once the part has that field.

**P3/P4 follow-ons already designed elsewhere:** I2C pull-ups and rise time (B2), CAN termination (B6 / AI4), overvoltage on 3.3 V GPIOs and exceeded ratings (AI4, V1).

## 5. Phasing

- **P3 / 0.2:** `part.toml` citation shape; hand-cited fields on the official example parts; check panel and a CLI that can run the first-slice rules; findings never silently invent a number. Needs E4 and E6.
- **P4 / 0.3:** D2 limits table becomes the common store; AI4 full rule set and Design review panel; datasheet-backed citations as in the existing note.

## 6. Acceptance criteria

- Every first-slice finding on an official example names a citation or is explicitly a topology rule that needs no datasheet (for example “this LED has no series resistor”).
- A part with `pending` ratings does not produce a hard fail from those ratings.
- The blink example with the series resistor is clean; removing the resistor produces one clear LED finding.
- `ohmforge validate-parts` rejects a rating that claims a source but has no page, clause, or equivalent locator, once the schema lands.

## 7. Open questions

- The exact TOML shape, and whether citations live inline as above or in a shared `sources` table referenced by id.
- How much of the first slice can run on today’s demo boards before E4, as tests only.
