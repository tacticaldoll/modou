//! Binary integration tests for the `modou check` runner. These exercise the
//! compiled binary (`CARGO_BIN_EXE_modou`) so the CLI contract — the flag, the
//! usage error, and the process exit code — is governed end to end. No external
//! dependency: the tests drive the binary with `std::process::Command`.

use std::path::PathBuf;
use std::process::Command;

fn modou() -> Command {
    Command::new(env!("CARGO_BIN_EXE_modou"))
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("Cargo.toml")
}

#[test]
fn check_clean_target_exits_0() {
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(0));
}

#[test]
fn check_violating_target_exits_1_with_report() {
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .output()
        .expect("run modou");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("serde") && stderr.contains("violation"),
        "expected a violation report naming serde, got: {stderr}"
    );
}

#[test]
fn missing_manifest_path_is_a_usage_error_exit_2() {
    let output = modou().arg("check").output().expect("run modou");
    assert_eq!(
        output.status.code(),
        Some(2),
        "a missing --manifest-path must exit 2, never 0 or 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("usage") && stderr.contains("--manifest-path"),
        "expected usage guidance on stderr, got: {stderr}"
    );
}

#[test]
fn equals_form_of_manifest_path_is_accepted() {
    let status = modou()
        .arg("check")
        .arg(format!("--manifest-path={}", fixture("clean").display()))
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(0));
}

fn temp_baseline(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("modou-cli-test-{name}.json"))
}

#[test]
fn write_baseline_exits_0_and_creates_a_parseable_file() {
    let out = temp_baseline("write");
    let _ = std::fs::remove_file(&out);
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--write-baseline")
        .arg(&out)
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(0));
    let text = std::fs::read_to_string(&out).expect("baseline file written");
    assert!(
        text.contains("serde"),
        "baseline should record serde, got: {text}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn gate_against_written_baseline_exits_0() {
    let out = temp_baseline("gate-known");
    let _ = std::fs::remove_file(&out);
    modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--write-baseline")
        .arg(&out)
        .status()
        .expect("write baseline");
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--baseline")
        .arg(&out)
        .status()
        .expect("gate");
    assert_eq!(status.code(), Some(0), "all violations are pre-existing");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn gate_fails_on_a_violation_absent_from_the_baseline() {
    let out = temp_baseline("gate-new");
    std::fs::write(&out, r#"{"version":1,"violations":[]}"#).expect("write empty baseline");
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--baseline")
        .arg(&out)
        .status()
        .expect("gate");
    assert_eq!(
        status.code(),
        Some(1),
        "serde is new relative to an empty baseline"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn both_baseline_flags_is_a_usage_error() {
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--baseline")
        .arg(temp_baseline("x"))
        .arg("--write-baseline")
        .arg(temp_baseline("y"))
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(2));
}

#[test]
fn a_malformed_baseline_exits_2() {
    let out = temp_baseline("bad");
    std::fs::write(&out, "not json").expect("write malformed baseline");
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--baseline")
        .arg(&out)
        .status()
        .expect("gate");
    assert_eq!(status.code(), Some(2));
    let _ = std::fs::remove_file(&out);
}

#[test]
fn json_format_emits_a_parseable_violations_document() {
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--format")
        .arg("json")
        .output()
        .expect("run modou");
    assert_eq!(output.status.code(), Some(1));
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    assert_eq!(doc["outcome"], "violations");
    assert_eq!(doc["violations"][0]["kind"], "crate");
    assert_eq!(doc["violations"][0]["finding"], "serde");
}

#[test]
fn equals_form_of_json_format_is_accepted() {
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .arg("--format=json")
        .output()
        .expect("run modou");
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    assert_eq!(doc["outcome"], "clean");
}

#[test]
fn unknown_format_is_a_usage_error() {
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .arg("--format")
        .arg("yaml")
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(2));
}
