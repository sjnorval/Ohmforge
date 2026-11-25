//! `ohmforge circuit check` and `ohmforge circuit fmt` (RFC 0003).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use of_circuit::fs::{FsResolver, find_circuit_files, load_parts, read_text};
use of_circuit::library::{PartsIndex, check_firmware, check_parts, normalize, update_hashes};
use of_circuit::{CircuitError, diagnostics, parse_unchecked, save};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sub {
    Check,
    Fmt,
}

#[derive(Debug)]
struct Args {
    sub: Sub,
    check: bool,
    update_hashes: bool,
    parts: PathBuf,
    paths: Vec<PathBuf>,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let sub = match args.first().map(String::as_str) {
        Some("check") => Sub::Check,
        Some("fmt") => Sub::Fmt,
        Some(other) => return Err(format!("unknown circuit command '{other}' (expected check or fmt)")),
        None => return Err("circuit needs a command (check or fmt)".into()),
    };
    let mut a = Args {
        sub,
        check: false,
        update_hashes: false,
        parts: PathBuf::from("parts"),
        paths: Vec::new(),
    };
    let mut it = args[1..].iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--parts" => a.parts = it.next().ok_or("--parts needs a directory")?.into(),
            "--check" if sub == Sub::Fmt => a.check = true,
            "--update-hashes" if sub == Sub::Fmt => a.update_hashes = true,
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s => a.paths.push(PathBuf::from(s)),
        }
    }
    if a.paths.is_empty() {
        return Err("give at least one circuit file or directory".into());
    }
    Ok(a)
}

/// Entry point for `ohmforge circuit ...`.
pub fn run(args: &[String]) -> ExitCode {
    let a = match parse_args(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("{}", crate::USAGE);
            return ExitCode::from(2);
        }
    };
    let setup = load_parts(&a.parts)
        .map_err(|e| format!("parts library: {e} (use --parts DIR)"))
        .and_then(|lib| find_circuit_files(&a.paths).map(|files| (lib, files)));
    let (lib, files) = match setup {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if files.is_empty() {
        eprintln!("no circuit files found");
        return ExitCode::SUCCESS;
    }
    let mut failed = 0usize;
    for f in &files {
        let ok = match a.sub {
            Sub::Check => check_file(f, &lib),
            Sub::Fmt => fmt_file(f, &lib, &a),
        };
        if !ok {
            failed += 1;
        }
    }
    let verb = if a.sub == Sub::Check { "checked" } else { "formatted" };
    if failed > 0 {
        eprintln!("error: {failed} of {} circuit file(s) failed", files.len());
        ExitCode::FAILURE
    } else {
        eprintln!("{verb} {} circuit file(s)", files.len());
        ExitCode::SUCCESS
    }
}

fn report(path: &Path, errors: Vec<CircuitError>) -> bool {
    let ok = errors.is_empty();
    for e in errors {
        eprintln!("{}", e.with_path(path));
    }
    ok
}

/// Every problem in one file; true when there are none.
fn check_file(path: &Path, lib: &PartsIndex) -> bool {
    let src = match read_text(path) {
        Ok(s) => s,
        Err(e) => return report(path, vec![e]),
    };
    let file = match parse_unchecked(&src) {
        Ok(f) => f,
        Err(e) => return report(path, vec![e]),
    };
    let mut errors = diagnostics(&src, &file);
    errors.extend(check_parts(&src, &file, lib));
    errors.extend(check_firmware(&src, &file, &FsResolver::for_file(path)));
    errors.sort_by_key(|e| (e.line, e.column));
    let ok = report(path, errors);
    if ok {
        println!("ok  {}", path.display());
    }
    ok
}

/// Canonical form for one file; true when it is (or now is) canonical.
fn fmt_file(path: &Path, lib: &PartsIndex, a: &Args) -> bool {
    let src = match read_text(path) {
        Ok(s) => s,
        Err(e) => return report(path, vec![e]),
    };
    let mut file = match parse_unchecked(&src) {
        Ok(f) => f,
        Err(e) => return report(path, vec![e]),
    };
    if a.update_hashes
        && let Err(e) = update_hashes(&src, &mut file, &FsResolver::for_file(path))
    {
        return report(path, vec![e]);
    }
    normalize(&mut file, Some(lib));
    let errors = diagnostics(&src, &file);
    if !errors.is_empty() {
        return report(path, errors);
    }
    let text = save(&file);
    if text == src {
        return true;
    }
    if a.check {
        println!("would reformat {}", path.display());
        return false;
    }
    match std::fs::write(path, text) {
        Ok(()) => {
            println!("formatted {}", path.display());
            true
        }
        Err(e) => report(path, vec![CircuitError::general(e.to_string())]),
    }
}
