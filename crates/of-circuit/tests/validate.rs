//! Structural validation (no parts library). Every error points at the offending value.

use of_circuit::{diagnostics, parse, parse_unchecked};

/// Join lines with `\n`, so line N of the source is `lines[N - 1]`.
fn src(lines: &[&str]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

const HEAD: [&str; 3] = ["format = 1", "[circuit]", "name = \"x\""];

fn with_head(body: &[&str]) -> String {
    let mut all: Vec<&str> = HEAD.to_vec();
    all.extend_from_slice(body);
    src(&all)
}

fn err(text: &str) -> of_circuit::CircuitError {
    parse(text).expect_err("expected a validation error")
}

#[test]
fn pin_on_two_nets_points_at_the_second_use() {
    let s = "format = 1\n[circuit]\nname = \"x\"\n[[component]]\nref = \"R1\"\npart = \"generic.resistor\"\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [\"R1.1\"]\n[[net]]\nname = \"a\"\npins = [\"R1.1\", \"R1.2\"]\n";
    let e = err(s);
    assert_eq!((e.line, e.column), (13, 9), "{e}");
    assert!(
        e.message.contains("R1.1") && e.message.contains("already on net 'gnd'"),
        "{e}"
    );
}

#[test]
fn duplicate_ref_across_components_and_symbols() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[[symbol]]",
        "ref = \"r1\"", // line 8
        "kind = \"ground\"",
        "at = [0, 0]",
        "body = [0, 0, 1, 1]",
        "pins = []",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (8, 7), "{e}");
    assert!(
        e.message.contains("duplicate ref 'r1'") && e.message.contains("R1"),
        "{e}"
    );
}

#[test]
fn net_pin_must_be_ref_dot_pin_and_name_a_known_ref() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"R1\", \"R9.1\"]", // line 10
    ]);
    let all = diagnostics(&s, &parse_unchecked(&s).unwrap());
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!((all[0].line, all[0].column), (10, 9));
    assert!(all[0].message.contains("expected REF.PIN"), "{}", all[0]);
    assert_eq!((all[1].line, all[1].column), (10, 15));
    assert!(all[1].message.contains("unknown ref 'R9'"), "{}", all[1]);
}

#[test]
fn same_pin_twice_on_one_net() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"R1.1\", \"r1.1\"]",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (10, 17), "{e}");
    assert!(e.message.contains("listed twice"), "{e}");
}

#[test]
fn duplicate_net_names_are_case_insensitive() {
    let s = with_head(&[
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
        "[[net]]",
        "name = \"VDD\"",
        "pins = []",
        "[[net]]",
        "name = \"vdd\"", // line 12
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (12, 8), "{e}");
    assert!(e.message.contains("duplicate net name 'vdd'"), "{e}");
}

#[test]
fn missing_ground_net_points_at_the_ground_setting() {
    let s = src(&[
        "format = 1",
        "[circuit]",
        "name = \"x\"",
        "ground = \"0v\"",
        "[[net]]",
        "name = \"a\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (4, 10), "{e}");
    assert!(
        e.message.contains("ground net '0v'") && e.message.contains("no [[net]]"),
        "{e}"
    );
}

#[test]
fn missing_default_ground_points_at_the_circuit_table() {
    let s = src(&["format = 1", "", "[circuit]", "name = \"x\""]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (3, 1), "{e}");
    assert!(e.message.contains("ground net 'gnd'"), "{e}");
}

#[test]
fn ground_net_needs_ground_kind_and_is_the_only_one() {
    let s = with_head(&["[[net]]", "name = \"gnd\"", "pins = []"]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (5, 8), "{e}");
    assert!(e.message.contains("kind = \"ground\""), "{e}");

    let s = with_head(&[
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
        "[[net]]",
        "name = \"agnd\"", // line 9
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (9, 8), "{e}");
    assert!(e.message.contains("only the ground net 'gnd'"), "{e}");
}

#[test]
fn routed_schematic_pin_must_be_on_a_net() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[component.schematic]",
        "body = [0, 0, 10, 10]",
        "pins = [",
        "  { pin = \"1\", at = [0, 5], dir = \"W\" },",
        "  { pin = \"2\", at = [10, 5], dir = \"E\" },", // line 11
        "  { pin = \"3\", at = [5, 0], dir = \"N\", stub = true },",
        "]",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"R1.1\"]",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (11, 11), "{e}");
    assert!(
        e.message.contains("R1.2") && e.message.contains("not on any net"),
        "{e}"
    );
}

#[test]
fn duplicate_schematic_pin() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[component.schematic]",
        "body = [0, 0, 10, 10]",
        "pins = [",
        "  { pin = \"1\", at = [0, 5], dir = \"W\", stub = true },",
        "  { pin = \"1\", at = [10, 5], dir = \"E\", stub = true },", // line 11
        "]",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (11, 11), "{e}");
    assert!(e.message.contains("pin '1' is drawn twice"), "{e}");
}

#[test]
fn symbol_net_pins_must_be_symbol_pins() {
    let s = with_head(&[
        "[[symbol]]",
        "ref = \"GND\"",
        "kind = \"ground\"",
        "at = [0, 0]",
        "body = [0, 0, 22, 10]",
        "pins = [{ pin = \"1\", at = [11, 0], dir = \"N\" }]",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"GND.1\", \"GND.2\"]", // line 13
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (13, 18), "{e}");
    assert!(e.message.contains("symbol 'GND' has no pin '2'"), "{e}");
}

#[test]
fn unknown_symbol_kind() {
    let s = with_head(&[
        "[[symbol]]",
        "ref = \"P1\"",
        "kind = \"flag\"", // line 6
        "at = [0, 0]",
        "body = [0, 0, 1, 1]",
        "pins = []",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (6, 8), "{e}");
    assert!(e.message.contains("unknown symbol kind 'flag'"), "{e}");
}

fn firmware_src(app: &str, sha: &str) -> String {
    let app_line = format!("app = \"{app}\"");
    let sha_line = format!("app_sha256 = \"{sha}\"");
    with_head(&[
        "[[component]]",
        "ref = \"U1\"",
        "part = \"espressif.esp32-c3\"",
        "[component.firmware]",
        &app_line, // line 8
        &sha_line, // line 9
        "boot = \"direct\"",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"U1.GND\"]",
    ])
}

#[test]
fn bad_sha_string() {
    let e = err(&firmware_src("fw.elf", "ABC"));
    assert_eq!((e.line, e.column), (9, 14), "{e}");
    assert!(e.message.contains("64 lower-case hex"), "{e}");
    let upper = "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
    assert!(
        err(&firmware_src("fw.elf", upper))
            .message
            .contains("64 lower-case hex")
    );
    let ok = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    parse(&firmware_src("fw.elf", ok)).unwrap();
}

#[test]
fn firmware_paths_are_relative_with_forward_slashes() {
    let ok = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    for bad in ["/abs/fw.elf", "C:/fw.elf", "fw\\\\app.elf", ""] {
        let e = err(&firmware_src(bad, ok));
        assert_eq!((e.line, e.column), (8, 7), "{bad}: {e}");
    }
    parse(&firmware_src("../../firmware/esp32c3-blinky/blinky.elf", ok)).unwrap();
}

#[test]
fn partitions_and_their_hash_come_together() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"U1\"",
        "part = \"espressif.esp32-c3\"",
        "[component.firmware]",
        "app = \"app.bin\"",
        "app_sha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"",
        "boot = \"rom\"",
        "partitions = \"pt.bin\"", // line 11
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (11, 14), "{e}");
    assert!(e.message.contains("partitions_sha256"), "{e}");
}

#[test]
fn slide_param_must_be_numeric_when_present() {
    let body = |param: &str| {
        let params = format!("params = {{ position = {param} }}");
        with_head(&[
            "[[component]]",
            "ref = \"RV1\"",
            "part = \"bourns.ptv09a-b103\"",
            &params,
            "[component.schematic]",
            "body = [0, 0, 30, 60]",
            "pins = [",
            "  { pin = \"W\", at = [30, 60], dir = \"E\", stub = true, slide = { to = [30, 0], param = \"position\" } },",
            "]",
            "[[net]]",
            "name = \"gnd\"",
            "kind = \"ground\"",
            "pins = []",
        ])
    };
    parse(&body("0.25")).unwrap();
    let e = err(&body("\"half\""));
    assert_eq!((e.line, e.column), (11, 87), "{e}");
    assert!(e.message.contains("'position' is not a number"), "{e}");
}

#[test]
fn sim_settings_are_checked() {
    let s = with_head(&[
        "[sim]", // line 4
        "max_step = 0",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (4, 1), "{e}");
    assert!(e.message.contains("max_step"), "{e}");
}

#[test]
fn ref_may_not_contain_a_dot() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R.1\"", // line 5
        "part = \"generic.resistor\"",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = []",
    ]);
    let e = err(&s);
    assert_eq!((e.line, e.column), (5, 7), "{e}");
    assert!(e.message.contains("letters, digits"), "{e}");
}

#[test]
fn diagnostics_are_in_file_order() {
    let s = with_head(&[
        "[[component]]",
        "ref = \"R1\"",
        "part = \"generic.resistor\"",
        "[[component]]",
        "ref = \"R1\"", // line 8
        "part = \"generic.resistor\"",
        "[[net]]",
        "name = \"gnd\"",
        "kind = \"ground\"",
        "pins = [\"X.1\"]", // line 13
    ]);
    let all = diagnostics(&s, &parse_unchecked(&s).unwrap());
    let lines: Vec<usize> = all.iter().map(|e| e.line).collect();
    assert_eq!(lines, [8, 13], "{all:?}");
    assert_eq!(parse(&s).unwrap_err(), all[0]);
}
