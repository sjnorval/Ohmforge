//! Host file-system helpers (feature `fs`).
#![cfg(feature = "fs")]

use std::path::{Path, PathBuf};

use of_circuit::fs::{FsResolver, find_circuit_files, load_parts, read_circuit};
use of_circuit::library::Resolver;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("of-circuit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn resolver_reads_relative_to_the_circuit_file() {
    let d = temp_dir("resolve");
    std::fs::create_dir_all(d.join("fw")).unwrap();
    std::fs::write(d.join("fw/app.elf"), b"elf").unwrap();
    let r = FsResolver::for_file(&d.join("demo.circuit.toml"));
    assert_eq!(r.read("fw/app.elf").unwrap(), b"elf");
    assert!(r.read("fw/missing.elf").unwrap_err().contains("missing.elf"));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn errors_carry_the_file_path() {
    let d = temp_dir("path");
    let p = d.join("bad.circuit.toml");
    std::fs::write(&p, "format = 1\n[circuit]\nname = \"x\"\ncolour = 1\n").unwrap();
    let e = read_circuit(&p).unwrap_err();
    assert_eq!(e.path.as_deref(), Some(p.as_path()));
    assert!(e.to_string().starts_with(&format!("{}:4:1: ", p.display())), "{e}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn finds_circuit_files_sorted_in_directories() {
    let d = temp_dir("find");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    for f in [
        "b.circuit.toml",
        "a.circuit.toml",
        "sub/c.circuit.toml",
        "part.toml",
        "notes.md",
    ] {
        std::fs::write(d.join(f), "").unwrap();
    }
    let found = find_circuit_files(std::slice::from_ref(&d)).unwrap();
    let names: Vec<_> = found
        .iter()
        .map(|p| p.strip_prefix(&d).unwrap().to_path_buf())
        .collect();
    assert_eq!(
        names,
        [
            Path::new("a.circuit.toml"),
            Path::new("b.circuit.toml"),
            &Path::new("sub").join("c.circuit.toml")
        ]
    );
    // An explicit file is taken as is, whatever its name.
    assert_eq!(find_circuit_files(&[d.join("notes.md")]).unwrap(), [d.join("notes.md")]);
    assert!(find_circuit_files(&[d.join("nope")]).is_err());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn loads_the_repo_parts_library() {
    let parts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
    let lib = load_parts(&parts).unwrap();
    assert!(lib.get("generic.resistor").is_some());
    assert!(load_parts(&parts.join("does-not-exist")).is_err());
}
