# Schematic themes and symbol styles

Status: proposed (design only, no code). Nothing in this document is implemented yet; section 1 lists what exists today. Phase T0 is this document (P2); T1 to T3 are planned in P3 / 0.2 and T4 in P4 / 0.3 (section 7).
Date: 2026-11-23.
Related: [`wire-routing.md`](wire-routing.md) §2 and §5 (stroke language and rendering in egui); [`component-stress-states.md`](component-stress-states.md) (stress colours and badges); [`schematic-layout.md`](schematic-layout.md) (net classes, buses and labels); [`buses.md`](buses.md) (bus nets); UX decisions d1 (schematic canvas) and the canvas spec in [`UX-Research/`](../UX-Research/); [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §9 and §13.1 (licensing).

Stefan asked to explore more schematic themes, including a cyberpunk / hacker theme for hardware hackers and a theme with the feel of a retro wasteland wrist-terminal. The standing rule is that **the current schematic look stays the default** for colours, strokes and layout: themes are options on top of it, and the default theme renders today's look. Symbol style is the one exception, by Stefan's decision of 23 November 2026 (section 9): symbols default to ISO/IEC conventions (IEC 60617, rectangular resistor) where possible, chosen by locale and configurable. Today's look, including the zigzag resistor, stays unchanged until the T phases land.

## 1. Current state

How `of-ui` defines the schematic look today (23 November 2026):

| What | Where | Value |
|---|---|---|
| Accent colours | `of-ui/src/theme.rs` | `TEAL` (99, 222, 199) for live signals and the VDD rail; `LED_RED` (255, 93, 97) for a lit LED and its halo; `PANEL` (22, 26, 32) and `panel()` (rounded 8 px frame, 14 px margin) |
| Duplicated constants | `of-ui/src/serial.rs` | Its own `PANEL` (same value) and `HIT` (24, 160, 150) for find highlights; `demo.rs` has the Play button text colour (12, 35, 31) inline |
| Base visuals | `of-app/src/main.rs` | egui's dark theme forced (`ThemePreference::Dark`), with `weak_text_color` set to (163, 173, 186) |
| Wire and label colours | `blink`, `adc_pwm`, `button` `schematic.rs` | Wires use `ui.visuals().weak_text_color()` (muted), labels `text_color()`; the scope grid uses the egui `noninteractive.bg_stroke` colour |
| Stroke widths | Same files and `wiring/model.rs` `StrokeStyle { Wire, LiveSignal, Power }` | 1.7 px muted wires, 2.0 px teal live signal, 1.7 px teal power rail, 2.4 px red LED body; filled 3 px junction dots; hop semicircles in the wire's own stroke |
| Symbols | Hand-drawn polylines and `schematic.rs` primitives | Resistor as a zigzag (ANSI/IEEE 315 style, `wiring::r1_zigzag`), LED triangle and bar with arrows, three-bar ground, pot with a teal wiper, `draw_switch`, `draw_psu` |
| Fonts | egui defaults (the `default_fonts` feature) | Proportional and monospace defaults; no custom fonts |

There is no theme struct, no light mode, no theme setting, no symbol-style choice and no contrast check. Colours are scattered constants and egui visuals.

## 2. Goals and non-goals

Goals:

- One **theme data model** that covers everything the schematic and instruments draw, so a theme is a file, not code.
- The **current look as the built-in default theme**, pixel-identical in colours and strokes; symbol shapes follow the symbol-style setting (IEC by default from T3).
- A set of **built-in themes** to explore, including cyberpunk / hacker and wasteland terminal, each passing the contrast checks.
- **Symbol style sets** as a separate switch: IEC 60617 (rectangular resistor, usual in South Africa and Europe; the default) and ANSI/IEEE 315 (zigzag, today's look).
- **User themes** from a folder, with live reload while editing.
- **Theme-aware export**: images and PDFs use a chosen theme (for example paper for printing) independent of the screen theme.
- **Accessibility**: a validator with WCAG contrast ratios, colour-blind-safe palettes, and a reduce-motion setting that every effect respects.

Non-goals:

- Changing the default look or the router's geometry.
- Arbitrary shader plug-ins from theme files; effects are a fixed, reviewed set with parameters.
- Theming outside the schematic and instruments (egui's own widgets follow a light/dark base only).

## 3. Theme data model

Themes are TOML files, licensed CC-BY-SA-4.0 like parts (built-ins in `assets/themes/`, user themes in the platform config folder, `%APPDATA%\Ohmforge\themes\` on Windows).

```toml
# DRAFT: illustrative only.
[theme]
id = "cyberpunk"
name = "Cyberpunk / Hacker"
base = "dark"                 # canvas only: dark | light; the app chrome is not themed
license = "CC-BY-SA-4.0"

[canvas]
background = "#07070d"
grid = { colour = "#1a1a2e", style = "dots", spacing = 8 }

[net]                         # wire colours by net class (wire-routing NetKind plus buses)
power = "#00f0ff"
ground = "#5c6b7a"
signal = "#ff2bd6"
signal_live = "#ff6be3"
bus = "#39ff14"
width = { wire = 1.7, live = 2.0, power = 1.7 }
glow = { live = 0.6, radius = 4 }   # 0 disables

[component]
stroke = "#c8d3e0"
fill = "none"
pin_marker = "#00f0ff"
junction = { colour = "#00f0ff", radius = 3 }
led_on = "#ff3864"

[text]
font = "mono"                 # built-in families: proportional | mono | theme font below
label = "#e0e6f0"
muted = "#8a96a8"
size = 12
font_file = "fonts/ShareTechMono-Regular.ttf"   # optional extra font, OFL only; default is egui's built-in monospace

[interaction]
selection = "#ffe600"
hover = "#7df9ff"

[stress]                      # component-stress-states.md
warning = "#ffb000"
over_limit = "#ff2b2b"
damaged = "#3a0d0d"

[effects]                     # all optional, all off with reduce-motion except static ones
scanlines = { enabled = false, opacity = 0.08 }
crt = { curvature = 0.0, flicker = 0.0, bloom = 0.0 }
```

- Missing keys fall back to the default theme, so small themes stay small.
- Net classes follow the router's `NetKind` (power, ground, signal) plus `bus` from [`buses.md`](buses.md) and [`schematic-layout.md`](schematic-layout.md).
- In code, a `Theme` struct replaces the constants in `theme.rs`, `serial.rs` and the inline colours; schematic drawing reads colours and widths only from it.

## 4. Built-in theme candidates

| Theme | Look | Notes |
|---|---|---|
| **Ohmforge** (default) | Today's look: dark panel, muted wires, teal power and live signals, red LED glow | Colours and strokes pixel-identical to today (snapshot test); symbols follow the symbol-style setting |
| ★ **Cyberpunk / Hacker** (highlighted candidate) | Near-black background, neon cyan power, magenta signals, acid green buses, glow on live nets, monospace labels, optional scanlines | Glow and scanlines respect reduce-motion (scanlines become static or off, no flicker); neon colours chosen to pass the 3:1 graphics and 4.5:1 text contrast checks against the background; a favourite for hardware hackers, so it is the first non-default theme to build |
| ★ **Wasteland Terminal** (highlighted candidate) | Monochrome phosphor green on near-black, with an amber variant; chunky retro monospace labels; soft glow; pixel-style badge icons; optional CRT curvature, scanlines and gentle flicker | Supersedes the separate “retro green CRT” candidate: with effects off it is that theme. Net classes differ by brightness and dash pattern instead of hue, since it is monochrome. All effects off with reduce-motion. See the trademark note below |
| **Blueprint** | White and pale cyan lines on blueprint blue | Classic drawing-office look |
| **Paper / print** | Black on white, no glow, thin grid or none | Default for PDF and print export |
| **Dark high-contrast** | Pure black background, white and saturated accents, thicker strokes | Accessibility |
| **KiCad-like** | Light schematic sheet with the familiar dark-red component and green wire feel | Our own palette in that spirit; no theme files copied from KiCad |
| **Colour-blind-safe** (light and dark) | Okabe–Ito palette for net classes; dash patterns for power and ground | Checked with deuteranopia, protanopia and tritanopia simulation |

**Trademark and asset rules for the Wasteland Terminal theme.** The idea is inspired by the wrist-computer look of a well-known game series; “Pip-Boy”, “Fallout”, “Vault-Tec” and the Vault Boy character are trademarks of Bethesda Softworks / ZeniMax Media. The theme is an original design with a generic name (“Wasteland Terminal”), with no copied artwork, logos, fonts, sounds or character imagery, and none of those names in the theme, its files or its description. Themes use egui's built-in monospace by default; extra fonts are optional and configurable, and must be openly licensed (SIL OFL), for example VT323 or Share Tech Mono, with the licence file shipped next to the font and the licence checked before adding (OFL-1.1 is already on the `deny.toml` allow list for font crates).

## 5. Symbol styles

- A separate setting from the colour theme: **IEC 60617** (rectangular resistor, rectangular fuse, IEC-style inductor; the convention in South Africa through SANS adoption of IEC standards, and in Europe) and **ANSI/IEEE 315** (zigzag resistor, today's look). The default follows ISO/IEC conventions as far as possible: IEC 60617 for South Africa and Europe, with the default chosen by locale and always configurable. Defaults for other locales are still to be researched (section 9). Today's zigzag stays until T3 lands. Each symbol in a part's `[symbol]` (E6) can provide both variants; the generic library parts ship both.
- IEC 60617 is a paid database; symbol shapes are drawn from the common conventions, not copied from it.
- Pin positions are identical in both styles, so the router's goldens do not change between styles.

## 6. Settings, export and validation

- **Theme picker** in Settings: theme, symbol style, reduce-motion, effects on/off, optional extra fonts, and an “export theme” choice. Themes apply to the schematic canvas and instruments only; the app chrome keeps egui's own light or dark visuals. Previews show a small sample schematic.
- **User themes**: any `.toml` in the user themes folder appears in the picker; edits reload live while the app runs; errors show in the picker with the line number and the theme falls back to defaults.
- **Export**: schematic export to SVG, PNG and PDF (the PDF through an MIT or Apache-2.0 crate such as `svg2pdf`) uses the export theme (paper by default).
- **Validator**: `ohmforge themes check <file>` and the picker run the same checks: text against background at least 4.5:1 and wires, symbols and badges at least 3:1 (WCAG 2.1 contrast and non-text contrast), stress colours distinguishable under colour-blindness simulation, every effect disabled by reduce-motion, fonts present and openly licensed.

## 7. Phasing

| Item | Phase | Work | Depends on | Acceptance criteria |
|---|---|---|---|---|
| **T0** Design | P2 (now) | This document; roadmap, README, mdBook and changelog pointers. | — | Reviewed and merged by Stefan. No code change. |
| **T1** Theme model and default theme | P3 | `Theme` struct and TOML loader; move the constants from `theme.rs`, `serial.rs`, `demo.rs` and `of-app` into the default theme; theme picker with Ohmforge and Paper. | — (can land early in P3) | Render snapshot tests of the blink, dimmer, fade and button schematics are pixel-identical before and after; Paper renders black on white. |
| **T2** Built-in themes and validator | P3 | Cyberpunk / Hacker first, then Wasteland Terminal (green and amber), Blueprint, Dark high-contrast, KiCad-like, colour-blind-safe; glow and static scanlines; reduce-motion; `ohmforge themes check`. | T1 | Every built-in theme passes the validator; reduce-motion turns off every animated effect; fonts ship with their OFL licence files. |
| **T3** Symbol styles | P3 | IEC 60617 and ANSI/IEEE 315 variants in `[symbol]` and the generic parts; setting with a locale-based default (IEC for South Africa and Europe). | T1; E6 (#50) `[symbol]` | Switching style changes symbol shapes only; router goldens unchanged. |
| **T4** User themes, effects and export | P4 | User theme folder with live reload; CRT curvature, flicker and bloom as GPU post-effects; SVG/PNG/PDF export with an export theme. | T2; the schematic editor | Editing a user theme updates the open schematic within a second; a PDF export uses Paper while the screen shows Cyberpunk; effects cost under 1 ms per frame on the reference machine. |

## 8. Invariants

1. **Default unchanged**: the Ohmforge theme's colours and strokes are pixel-identical to today; only the symbol style changes, by the setting.
2. **Every theme passes the validator**, including contrast and reduce-motion.
3. **No third-party trademarks or copied assets** in themes; fonts are openly licensed with their licence files.
4. **Themes never change geometry**: routing, layout and symbol pin positions are theme-independent.

## 9. Decided and open questions

Decided (23 November 2026, Stefan):

- **Default symbol style** (former open question 1). ISO/IEC conventions as far as possible: IEC 60617 symbols (rectangular resistor) by default, chosen by locale and always configurable. This replaces today's zigzag default when T3 lands; until then today's look stays unchanged.
- **Fonts** (former open question 2). egui's built-in monospace by default; extra OFL fonts are optional and configurable.
- **Light base** (former open question 3). Themes change the canvas only; the rest of the app keeps egui's own visuals.
- **Community themes** (former open question 4). Theme PRs into `assets/themes/` are accepted under CC-BY-SA-4.0.

Open questions:

1. **Symbol defaults for other locales.** Which symbol style suits locales outside South Africa and Europe (for example North America, where ANSI/IEEE 315 is common, and Asia)? To be researched.
