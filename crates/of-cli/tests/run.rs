//! End-to-end tests of `ohmforge run`.

use std::process::Command;

fn ohmforge() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ohmforge"))
}

#[test]
fn runs_op_and_tran_and_writes_files() {
    let dir = std::env::temp_dir().join(format!("ohmforge-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cir = dir.join("rc.cir");
    std::fs::write(
        &cir,
        "rc\nV1 in 0 PULSE(0 1 0 1n)\nR1 in out 1k\nC1 out 0 1u\n.op\n.tran 10u 1m\n.end\n",
    )
    .unwrap();
    let csv = dir.join("rc.csv");
    let vcd = dir.join("rc.vcd");
    let out = ohmforge()
        .arg("run")
        .arg(&cir)
        .arg("--out")
        .arg(&csv)
        .arg("--vcd")
        .arg(&vcd)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("v(out)"), "{stdout}");
    let data = std::fs::read_to_string(&csv).unwrap();
    assert!(data.starts_with("time,"), "{data}");
    let last: Vec<f64> = data
        .lines()
        .last()
        .unwrap()
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    assert!((last[0] - 1e-3).abs() < 1e-12);
    assert!(std::fs::read_to_string(&vcd).unwrap().contains("$enddefinitions"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn reports_errors_with_a_failure_code() {
    let out = ohmforge().args(["run", "/nonexistent/x.cir"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("error"));
    let out = ohmforge().arg("bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn validate_parts_accepts_the_library() {
    let parts = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
    let out = ohmforge().args(["validate-parts"]).arg(&parts).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("ok  "), "{stdout}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("validated"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
