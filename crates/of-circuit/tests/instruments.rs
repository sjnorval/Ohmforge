//! `[[instrument]]` tables and probe signals (RFC 0004, engine E5e): parse,
//! canonical save, structural checks and signal resolution, each error at the
//! offending value.

use of_circuit::{CircuitError, diagnostics, parse, parse_unchecked, save};

/// Join lines with `\n`, so line N of the source is `lines[N - 1]`.
fn src(lines: &[&str]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// A two-part circuit; line 21 is the first line after it.
const BASE: [&str; 20] = [
    "format = 1",
    "",
    "[circuit]",
    "name = \"x\"",
    "ground = \"gnd\"",
    "",
    "[[component]]",
    "ref = \"R1\"",
    "part = \"generic.resistor\"",
    "",
    "[[component]]",
    "ref = \"U1\"",
    "part = \"espressif.esp32-c3\"",
    "",
    "[[net]]",
    "name = \"vdd\"",
    "kind = \"power\"",
    "pins = [\"R1.1\", \"U1.3V3\"]",
    "",
    "[[net]]",
];

fn with_base(body: &[&str]) -> String {
    let mut all: Vec<&str> = BASE.to_vec();
    all.extend_from_slice(&["name = \"gnd\"", "kind = \"ground\"", "pins = [\"R1.2\", \"U1.GND\"]"]);
    all.extend_from_slice(body);
    src(&all)
}

/// 1-based line and column of the first `needle` in `text`.
fn at(text: &str, needle: &str) -> (usize, usize) {
    let i = text.find(needle).unwrap_or_else(|| panic!("no {needle:?}"));
    let before = &text[..i];
    (
        before.matches('\n').count() + 1,
        before.rsplit('\n').next().unwrap().chars().count() + 1,
    )
}

fn only(errors: Vec<CircuitError>) -> CircuitError {
    assert_eq!(errors.len(), 1, "{errors:?}");
    errors.into_iter().next().unwrap()
}

fn structural(text: &str) -> CircuitError {
    let f = parse_unchecked(text).unwrap_or_else(|e| panic!("{e}"));
    only(diagnostics(text, &f))
}

const PROBES: [&str; 7] = [
    "",
    "[[probe]]",
    "name = \"LED current\"",
    "signal = \"i(R1)\"",
    "",
    "[[probe]]",
    "name = \"Rail\"",
];

fn probes_and(body: &[&str]) -> String {
    let mut all: Vec<&str> = PROBES.to_vec();
    all.push("signal = \"v(vdd)\"");
    all.extend_from_slice(body);
    with_base(&all)
}

#[test]
fn instruments_round_trip_in_canonical_form() {
    let text = probes_and(&[
        "",
        "[[instrument]]",
        "ref = \"XMM1\"",
        "kind = \"multimeter\"",
        "profile = \"ideal\"",
        "mode = \"dc_a\"",
        "probes = [\"LED current\"]",
        "at = [300, 160]",
        "",
        "[[instrument]]",
        "ref = \"XSC1\"",
        "kind = \"scope\"",
        "probes = [\"LED current\", \"v(vdd)\"]",
        "params = { level = 1, time_div = \"1ms\", trigger = \"rise\" }",
    ]);
    let f = parse(&text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(f.format.get_ref(), &1, "the table is additive: format stays 1");
    assert_eq!(save(&f), text);
    let scope = &f.instruments[1];
    assert_eq!(
        (scope.reference(), scope.kind(), scope.profile(), scope.mode()),
        ("XSC1", "scope", "ideal", None)
    );
    assert_eq!(f.instruments[0].at, Some([300.0, 160.0]));
    assert_eq!(f.probe("Rail").map(|p| p.signal()), Some("v(vdd)"));
}

#[test]
fn files_without_instruments_are_unchanged() {
    let text = probes_and(&[]);
    let f = parse(&text).unwrap();
    assert!(f.instruments.is_empty());
    assert_eq!(save(&f), text);
}

#[test]
fn an_unknown_instrument_key_is_an_error_with_its_line() {
    let text = probes_and(&[
        "",
        "[[instrument]]",
        "ref = \"XSC1\"",
        "kind = \"scope\"",
        "probes = [\"Rail\"]",
        "colour = \"red\"",
    ]);
    let e = parse(&text).expect_err("unknown key");
    assert!(e.message.contains("colour"), "{e}");
    assert_eq!(e.line, at(&text, "colour").0, "{e}");
}

#[test]
fn structural_instrument_errors_point_at_the_value() {
    let cases: [(&[&str], &str, &str); 9] = [
        (
            &["kind = \"meter\"", "probes = [\"Rail\"]"],
            "\"meter\"",
            "unknown instrument kind 'meter'",
        ),
        (
            &["kind = \"multimeter\"", "probes = [\"Rail\"]"],
            "\"multimeter\"",
            "needs a mode",
        ),
        (
            &["kind = \"multimeter\"", "mode = \"ohms\"", "probes = [\"Rail\"]"],
            "\"ohms\"",
            "unknown multimeter mode 'ohms'",
        ),
        (
            &["kind = \"scope\"", "mode = \"dc_v\"", "probes = [\"Rail\"]"],
            "\"dc_v\"",
            "only a multimeter has a mode",
        ),
        (
            &[
                "kind = \"multimeter\"",
                "mode = \"dc_v\"",
                "probes = [\"Rail\", \"v(vdd)\"]",
            ],
            "\"v(vdd)\"",
            "a multimeter reads one signal",
        ),
        (
            &["kind = \"scope\"", "probes = [\"Rail\", \"Nope\"]"],
            "\"Nope\"",
            "no [[probe]] named 'Nope'",
        ),
        (
            &["kind = \"scope\"", "profile = \"fluke\"", "probes = [\"Rail\"]"],
            "\"fluke\"",
            "<vendor>.<model>",
        ),
        (
            &[
                "kind = \"scope\"",
                "probes = [\"Rail\"]",
                "params = { time_div = \"fast\" }",
            ],
            "\"XSC1\"",
            "time_div must be a time",
        ),
        (
            &["kind = \"scope\"", "probes = [\"Rail\"]", "params = { zoom = 2 }"],
            "\"XSC1\"",
            "unknown scope param 'zoom'",
        ),
    ];
    for (lines, needle, want) in cases {
        let mut body = vec!["", "[[instrument]]", "ref = \"XSC1\""];
        body.extend_from_slice(lines);
        let text = probes_and(&body);
        let e = structural(&text);
        assert!(e.message.contains(want), "{want}: {e}");
        let probe_tables = text.find("[[instrument]]").unwrap();
        let (line, column) = at(&text[probe_tables..], needle);
        let offset = text[..probe_tables].matches('\n').count();
        assert_eq!((e.line, e.column), (line + offset, column), "{want}: {e}");
    }
}

#[test]
fn instrument_refs_share_the_ref_namespace_and_probe_names_are_unique() {
    let text = probes_and(&[
        "",
        "[[instrument]]",
        "ref = \"r1\"",
        "kind = \"scope\"",
        "probes = [\"Rail\"]",
    ]);
    let e = structural(&text);
    assert!(e.message.contains("duplicate ref 'r1'"), "{e}");
    assert_eq!((e.line, e.column), at(&text, "\"r1\""));

    let text = with_base(&[
        "",
        "[[probe]]",
        "name = \"A\"",
        "signal = \"v(vdd)\"",
        "",
        "[[probe]]",
        "name = \"A\"",
        "signal = \"i(R1)\"",
    ]);
    let e = structural(&text);
    assert!(e.message.contains("duplicate probe name 'A'"), "{e}");
    assert_eq!(e.line, text.lines().count() - 1, "{e}");
}

#[cfg(feature = "engine")]
mod signals {
    use of_circuit::library::{PartsIndex, check_signals};
    use of_circuit::parse;

    use super::*;

    fn library() -> PartsIndex {
        let parts = [
            include_str!("../../../parts/passives/resistor/part.toml"),
            include_str!("../../../parts/mcu/esp32-c3/part.toml"),
        ];
        PartsIndex::new(parts.iter().map(|s| of_parts::parse_part(s).unwrap()))
    }

    fn problems(text: &str) -> Vec<CircuitError> {
        let f = parse(text).unwrap_or_else(|e| panic!("{e}"));
        check_signals(text, &f, &library())
    }

    #[test]
    fn good_signals_resolve() {
        let text = probes_and(&[
            "",
            "[[probe]]",
            "name = \"Pin\"",
            "signal = \"i(U1.3V3)\"",
            "",
            "[[probe]]",
            "name = \"Rail to ground\"",
            "signal = \"v(vdd,0)\"",
            "",
            "[[probe]]",
            "name = \"Supply\"",
            "signal = \"r(U1.volts)\"",
            "",
            "[[instrument]]",
            "ref = \"XLA1\"",
            "kind = \"logic\"",
            "probes = [\"d(U1.GPIO8)\"]",
            "",
            "[[instrument]]",
            "ref = \"XMM1\"",
            "kind = \"multimeter\"",
            "mode = \"dc_v\"",
            "probes = [\"Rail\"]",
        ]);
        assert!(problems(&text).is_empty(), "{:?}", problems(&text));
    }

    #[test]
    fn bad_signals_are_reported_at_the_signal() {
        let cases = [
            ("x(R1)", "unknown signal kind"),
            ("v(led)", "no net 'led'"),
            ("v(VDD)", "is written 'vdd'"),
            ("i(U1)", "needs a two-pin part"),
            ("i(U1.gpio8)", "is written 'GPIO8'"),
            ("i(U1.NOPE)", "has no pin 'NOPE'"),
            ("i(R9)", "no component 'R9'"),
            ("r(r1.volts)", "is written 'R1'"),
        ];
        for (signal, want) in cases {
            let line = format!("signal = \"{signal}\"");
            let text = with_base(&["", "[[probe]]", "name = \"P\"", &line]);
            let e = only(problems(&text));
            assert!(
                e.message.contains(want) && e.message.contains("probe 'P'"),
                "{signal}: {e}"
            );
            assert_eq!((e.line, e.column), at(&text, &format!("\"{signal}\"")), "{signal}: {e}");
        }
    }

    #[test]
    fn instruments_need_signals_of_their_kind_and_an_ideal_profile() {
        let cases: [(&[&str], &str, &str); 5] = [
            (
                &["kind = \"multimeter\"", "mode = \"dc_v\"", "probes = [\"LED current\"]"],
                "\"LED current\"]",
                "a dc_v multimeter reads v(..)",
            ),
            (
                &["kind = \"multimeter\"", "mode = \"dc_a\"", "probes = [\"v(vdd)\"]"],
                "\"v(vdd)\"",
                "a dc_a multimeter reads i(..)",
            ),
            (
                &["kind = \"scope\"", "probes = [\"d(U1.GPIO8)\"]"],
                "\"d(U1.GPIO8)\"",
                "belongs on a logic analyser",
            ),
            (
                &["kind = \"scope\"", "probes = [\"v(nope)\"]"],
                "\"v(nope)\"",
                "no net 'nope'",
            ),
            (
                &["kind = \"scope\"", "profile = \"rigol.ds1054z\"", "probes = [\"Rail\"]"],
                "\"rigol.ds1054z\"",
                "unknown instrument profile",
            ),
        ];
        for (lines, needle, want) in cases {
            let mut body = vec!["", "[[instrument]]", "ref = \"X1\""];
            body.extend_from_slice(lines);
            let text = probes_and(&body);
            let e = only(problems(&text));
            assert!(e.message.contains(want) && e.message.starts_with("X1: "), "{want}: {e}");
            let start = text.find("[[instrument]]").unwrap();
            let (l, c) = at(&text[start..], needle);
            assert_eq!(
                (e.line, e.column),
                (l + text[..start].matches('\n').count(), c),
                "{want}: {e}"
            );
        }
    }
}
