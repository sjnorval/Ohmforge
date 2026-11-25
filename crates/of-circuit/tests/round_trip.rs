//! Canonical writer: byte-identical round trip and normalisation.

use of_circuit::{parse, same_content, save};

const ALL: &str = include_str!("fixtures/all_sections.circuit.toml"); // canonical; every section used

#[test]
fn canonical_text_round_trips_byte_for_byte() {
    assert_eq!(save(&parse(ALL).unwrap()), ALL);
}

#[test]
fn reparse_of_saved_is_the_same_content() {
    let f = parse(ALL).unwrap();
    assert!(same_content(&f, &parse(&save(&f)).unwrap()));
}

#[test]
fn non_canonical_input_normalises() {
    let messy = "format=1\n[circuit]\nname='x'\n[[net]]\nkind='ground'\nname='gnd'\npins=[]\n";
    let canon = save(&parse(messy).unwrap());
    assert!(canon.starts_with("format = 1\n\n[circuit]\nname = \"x\"\n"), "{canon}");
    assert_eq!(save(&parse(&canon).unwrap()), canon);
}

#[test]
fn default_sim_settings_are_left_out() {
    let f = parse("format = 1\n[circuit]\nname = \"x\"\n[sim]\nquantum_us = 50\nseed = 4\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n").unwrap();
    let text = save(&f);
    assert!(text.contains("[sim]\nseed = 4\n\n"), "{text}");
    assert!(!text.contains("quantum_us"), "{text}");
    let f =
        parse("format = 1\n[circuit]\nname = \"x\"\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n").unwrap();
    assert!(!save(&f).contains("[sim]"));
}

#[test]
fn numbers_print_in_their_shortest_round_trip_form() {
    let src = "format = 1\n[circuit]\nname = \"x\"\n[sim]\nmax_step = 0.00002\n\
[[component]]\nref = \"R1\"\npart = \"p\"\nat = [1.0, -2.5]\n\
params = { a = 330.0, b = 3.3, c = 1e-12, d = 2.2e20, e = -0.0005, f = 0.001, g = nan, h = -inf }\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n";
    let text = save(&parse(src).unwrap());
    assert!(text.contains("max_step = 2e-5\n"), "{text}");
    assert!(text.contains("at = [1, -2.5]\n"), "{text}");
    assert!(
        text.contains(
            "params = { a = 330, b = 3.3, c = 1e-12, d = 2.2e20, e = -5e-4, f = 0.001, g = nan, h = -inf }\n"
        ),
        "{text}"
    );
    assert_eq!(save(&parse(&text).unwrap()), text);
}

#[test]
fn strings_are_escaped_and_keep_unicode() {
    let src = "format = 1\n[circuit]\nname = \"say \\\"hi\\\"\\t\\u0001 back\\\\slash \u{2212}\"\n\
[[component]]\nref = \"R1\"\npart = \"p\"\nparams = { \"odd key\" = 1, plain_key-2 = \"\u{3a9}\" }\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n";
    let f = parse(src).unwrap();
    assert_eq!(f.circuit.name, "say \"hi\"\t\u{1} back\\slash \u{2212}");
    let text = save(&f);
    assert!(
        text.contains("name = \"say \\\"hi\\\"\\t\\u0001 back\\\\slash \u{2212}\"\n"),
        "{text}"
    );
    assert!(
        text.contains("params = { \"odd key\" = 1, plain_key-2 = \"\u{3a9}\" }"),
        "{text}"
    );
    let again = parse(&text).unwrap();
    assert!(same_content(&f, &again));
    assert_eq!(save(&again), text);
}

#[test]
fn hand_built_files_round_trip() {
    let cases = [
        "format = 1\n\n[circuit]\nname = \"empty\"\nground = \"gnd\"\n\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n",
        "format = 1\n\n[circuit]\nname = \"zero ground\"\nground = \"0\"\n\n[[net]]\nname = \"0\"\nkind = \"ground\"\npins = []\n",
        "format = 1\n\n[circuit]\nname = \"bare\"\nground = \"gnd\"\n\n[[component]]\nref = \"SW1\"\npart = \"generic.pushbutton\"\n\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [\"SW1.1\", \"SW1.2\"]\n",
    ];
    for c in cases {
        assert_eq!(save(&parse(c).unwrap()), c);
    }
}

#[test]
fn same_content_ignores_spans_and_layout() {
    let a =
        parse("format = 1\n[circuit]\nname = \"x\"\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n").unwrap();
    let b = parse("format=1\n\n\n[circuit]\nname='x'\nground='gnd'\n[[net]]\npins=[]\nname='gnd'\nkind='ground'\n")
        .unwrap();
    assert!(same_content(&a, &b));
    let c =
        parse("format = 1\n[circuit]\nname = \"y\"\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n").unwrap();
    assert!(!same_content(&a, &c));
}
