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
fn absent_manifest_path_defaults_to_the_nearest_workspace() {
    // Run from inside the clean fixture workspace with no --manifest-path: the runner
    // walks up to its Cargo.toml (cargo-style) and evaluates it — clean, so exit 0.
    let workspace_dir = fixture("clean")
        .parent()
        .expect("fixture has a parent dir")
        .to_path_buf();
    let status = modou()
        .current_dir(&workspace_dir)
        .arg("check")
        .status()
        .expect("run modou");
    assert_eq!(
        status.code(),
        Some(0),
        "an absent --manifest-path must default to the nearest Cargo.toml"
    );
}

#[test]
fn absent_manifest_path_with_no_cargo_toml_exits_2() {
    // From a directory with no Cargo.toml up to the root, check cannot find a
    // workspace: it exits 2 (a scan error), never a silent 0.
    let empty = std::env::temp_dir().join("modou-cli-no-manifest");
    std::fs::create_dir_all(&empty).expect("create empty dir");
    let status = modou()
        .current_dir(&empty)
        .arg("check")
        .status()
        .expect("run modou");
    assert_eq!(
        status.code(),
        Some(2),
        "no Cargo.toml found must exit 2, never 0"
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

#[test]
fn coverage_is_reported_in_text_output() {
    // The text report (coverage included) is on stderr as a single stream; stdout is
    // reserved for machine output (`--format json`) and the `list` projection.
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .output()
        .expect("run modou");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("coverage"),
        "expected a coverage line on stderr, got: {stderr}"
    );
}

#[test]
fn coverage_object_is_in_the_json_report() {
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .arg("--format")
        .arg("json")
        .output()
        .expect("run modou");
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    assert!(
        doc["coverage"]["workspace_crates"].is_number(),
        "expected a coverage object, got: {doc}"
    );
    // The `uncovered` array is the load-bearing half of the coverage object — a
    // regression emitting the count but dropping the list would otherwise pass.
    assert!(
        doc["coverage"]["uncovered"].is_array(),
        "coverage must carry an `uncovered` array, got: {doc}"
    );
}

#[test]
fn warn_uncovered_does_not_change_a_clean_exit() {
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .arg("--warn-uncovered")
        .status()
        .expect("run modou");
    assert_eq!(
        status.code(),
        Some(0),
        "an advisory flag must not fail a clean run"
    );
}

#[test]
fn warn_uncovered_does_not_mask_an_enforced_violation() {
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--warn-uncovered")
        .status()
        .expect("run modou");
    assert_eq!(
        status.code(),
        Some(1),
        "an enforced violation must still fail under --warn-uncovered"
    );
}

#[test]
fn constitution_error_omits_coverage_text() {
    // The sample constitution targets `example-core`, absent from the `layered`
    // workspace, so the run is a constitution error (exit 2). Coverage is omitted: the
    // error is the whole story, not a coverage report over an un-evaluated constitution.
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("layered"))
        .output()
        .expect("run modou");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("coverage"),
        "a constitution error must omit coverage, got stdout: {stdout}"
    );
}

#[test]
fn constitution_error_omits_coverage_json() {
    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("layered"))
        .arg("--format")
        .arg("json")
        .output()
        .expect("run modou");
    assert_eq!(output.status.code(), Some(2));
    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    assert_eq!(doc["outcome"], "constitution_error");
    assert!(
        doc.get("coverage").is_none(),
        "a constitution error must omit the coverage object, got: {doc}"
    );
}

#[test]
fn gate_mode_json_reflects_baseline_and_stale_entries() {
    // Write the real baseline for the violating fixture, then append a no-longer-violated
    // entry. Gating under --format json must show the real violation suppressed
    // (baselined: true, exit 0) and the extra entry reported as stale.
    let out = temp_baseline("gate-json");
    let written = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--write-baseline")
        .arg(&out)
        .status()
        .expect("write baseline");
    assert_eq!(written.code(), Some(0));

    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read baseline"))
            .expect("baseline is JSON");
    // Append a stale entry: same target/rule as the real violation, a finding that is not
    // currently violated. Derived from the real entry so no identity is hard-coded.
    let mut ghost = doc["violations"][0].clone();
    ghost["finding"] = serde_json::json!("ghost-not-violated");
    doc["violations"]
        .as_array_mut()
        .expect("violations array")
        .push(ghost);
    std::fs::write(&out, serde_json::to_string(&doc).expect("serialize"))
        .expect("rewrite baseline");

    let output = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("violating"))
        .arg("--baseline")
        .arg(&out)
        .arg("--format")
        .arg("json")
        .output()
        .expect("gate json");
    let _ = std::fs::remove_file(&out);

    assert_eq!(
        output.status.code(),
        Some(0),
        "all real violations baselined -> exit 0"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is valid JSON");
    let suppressed = report["violations"]
        .as_array()
        .expect("violations array")
        .iter()
        .find(|v| v["finding"] == "serde")
        .expect("the real violation is present");
    assert_eq!(suppressed["baselined"], true, "got: {report}");
    let stale = report["stale_baseline"].as_array().expect("stale array");
    assert!(
        stale.iter().any(|e| e["finding"] == "ghost-not-violated"),
        "the no-longer-violated baseline entry must be reported stale, got: {report}"
    );
}

#[test]
fn empty_equals_form_value_is_a_usage_error() {
    // `--format=` (equals form, empty value) takes the strip_prefix path and must fail
    // loud, not silently fall back to a default.
    let status = modou()
        .arg("check")
        .arg("--manifest-path")
        .arg(fixture("clean"))
        .arg("--format=")
        .status()
        .expect("run modou");
    assert_eq!(status.code(), Some(2));
}
