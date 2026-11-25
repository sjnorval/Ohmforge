//! Syntax and schema errors carry 1-based line and column.

use of_circuit::{CircuitError, parse};

#[test]
fn unknown_field_reports_line_and_column() {
    let src = "format = 1\n[circuit]\nname = \"x\"\ncolour = \"red\"\n";
    let e = parse(src).unwrap_err();
    assert_eq!((e.line, e.column), (4, 1), "{e}");
    assert!(e.message.contains("colour"), "{e}");
}

#[test]
fn newer_format_is_refused() {
    let e = parse("format = 2\n[circuit]\nname = \"x\"\n").unwrap_err();
    assert_eq!(e.line, 1);
    assert!(
        e.message.contains("format 2") && e.message.contains("supports 1"),
        "{e}"
    );
}

#[test]
fn format_zero_is_refused() {
    let e = parse("format = 0\n[circuit]\nname = \"x\"\n").unwrap_err();
    assert_eq!((e.line, e.column), (1, 10), "{e}");
}

#[test]
fn toml_syntax_error_has_a_position() {
    let e = parse("format = 1\n[circuit]\nname = \"x\n").unwrap_err();
    assert_eq!(e.line, 3, "{e}");
}

#[test]
fn bad_enum_value_points_at_the_value() {
    let src = "format = 1\n[circuit]\nname = \"x\"\n[[net]]\nname = \"gnd\"\nkind = \"earth\"\npins = []\n";
    let e = parse(src).unwrap_err();
    assert_eq!((e.line, e.column), (6, 8), "{e}");
}

#[test]
fn display_includes_the_path_when_known() {
    let e = CircuitError::at("a\nbc\n", 3..4, "boom");
    assert_eq!((e.line, e.column), (2, 2));
    assert_eq!(e.to_string(), "2:2: boom");
    let e = e.with_path(std::path::Path::new("x.circuit.toml"));
    assert_eq!(e.to_string(), "x.circuit.toml:2:2: boom");
}

#[test]
fn columns_count_characters_not_bytes() {
    let e = CircuitError::at("name = \"\u{2212}\" x", 12..13, "here");
    assert_eq!((e.line, e.column), (1, 11));
}
