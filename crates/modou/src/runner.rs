//! The runner — the CI reaction, as a reusable library entry point.
//!
//! [`run`] turns a caller-supplied [`Constitution`] and the process arguments into
//! a process exit code, providing the whole `modou check` contract: flag parsing
//! (`--manifest-path`, `--baseline` / `--write-baseline`, `--format`), the baseline
//! gate and write actions, the human and JSON reports, and the exit-code mapping
//! (`0` clean / warn-only / fully baselined, `1` enforce violation, `2`
//! constitution / scan / usage error). An adopting project declares its own
//! constitution in Rust and gets this contract from one line:
//!
//! ```no_run
//! use modou::prelude::*;
//! fn constitution() -> Constitution { Constitution::new("my-project") }
//! fn main() -> std::process::ExitCode {
//!     modou::run(&constitution(), std::env::args())
//! }
//! ```
//!
//! IO (filesystem, stdout/stderr) is quarantined here; the sibling `engine` module
//! stays the pure functional core (the model plus [`check`](crate::check)), and must
//! not depend on this shell. The numeric work lives in the private [`dispatch`], so
//! the exit code is unit-testable; [`run`] is a thin [`ExitCode`] wrapper.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::engine::{
    Baseline, Constitution, Coverage, Outcome, Report, Severity, ViolationId, apply_baseline,
    check_and_cover, constitution_json, constitution_text, report_json,
};

/// Which runner command was requested. `check` reacts against a workspace; `list`
/// projects the declared constitution and never reacts.
#[derive(PartialEq, Eq)]
enum Command {
    Check,
    List,
}

/// Run the constitution's boundaries against a Cargo workspace and return the
/// process exit code. `args` are the full process arguments (the program name is
/// skipped internally, like a real `main`). Pass `std::env::args()` from a binary.
pub fn run<I, S>(constitution: &Constitution, args: I) -> ExitCode
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    ExitCode::from(dispatch(constitution, args))
}

/// The runner's work, returning the exit code as a number so it is assertable
/// without a subprocess and without inspecting an opaque [`ExitCode`].
fn dispatch<I, S>(constitution: &Constitution, args: I) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut manifest_path: Option<String> = None;
    let mut baseline_path: Option<String> = None;
    let mut write_baseline_path: Option<String> = None;
    let mut format: Option<String> = None;
    let mut warn_uncovered = false;
    let mut args = args.into_iter().map(Into::into).skip(1).peekable();

    // The command is the first positional token; an absent or unrecognized leading
    // token stays `check` (backward compatible). Flags following it never select
    // the command.
    let command = match args.peek().map(String::as_str) {
        Some("list") => {
            args.next();
            Command::List
        }
        Some("check") => {
            args.next();
            Command::Check
        }
        _ => Command::Check,
    };

    // A value-taking flag must be given its value; an absent value is a usage error
    // (exit 2), never a silent downgrade to the default or to a plain check
    // (PROJECT.md: misconfiguration fails loud).
    macro_rules! value {
        ($flag:literal) => {
            match args.next() {
                Some(value) => value,
                None => return usage(concat!($flag, " requires a value")),
            }
        };
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--manifest-path" => manifest_path = Some(value!("--manifest-path")),
            "--baseline" => baseline_path = Some(value!("--baseline")),
            "--write-baseline" => write_baseline_path = Some(value!("--write-baseline")),
            "--format" => format = Some(value!("--format")),
            "--warn-uncovered" => warn_uncovered = true,
            other => {
                if let Some(path) = other.strip_prefix("--manifest-path=") {
                    manifest_path = Some(path.to_string());
                } else if let Some(path) = other.strip_prefix("--baseline=") {
                    baseline_path = Some(path.to_string());
                } else if let Some(path) = other.strip_prefix("--write-baseline=") {
                    write_baseline_path = Some(path.to_string());
                } else if let Some(value) = other.strip_prefix("--format=") {
                    format = Some(value.to_string());
                } else {
                    // An unknown flag, a misspelling, or a stray positional is a
                    // misconfiguration — fail loud (exit 2), never silently ignore
                    // it (PROJECT.md).
                    return usage(&format!("unrecognized argument '{other}'"));
                }
            }
        }
    }

    // `--format` is validated for both commands so the flag contract stays uniform.
    let json = match format.as_deref() {
        None | Some("text") => false,
        Some("json") => true,
        Some(other) => {
            return usage(&format!(
                "unknown --format '{other}' (expected text or json)"
            ));
        }
    };

    // `list` is a projection, not a reaction: it observes nothing (no
    // `--manifest-path`), cannot fail a boundary, and always exits 0. It accepts
    // only `--format`; a check-only flag supplied to `list` is a usage error, not a
    // silent no-op (PROJECT.md: never silently ignore a flag).
    if command == Command::List {
        if manifest_path.is_some()
            || baseline_path.is_some()
            || write_baseline_path.is_some()
            || warn_uncovered
        {
            return usage("list takes only --format; other flags are check-only");
        }
        if json {
            println!("{}", constitution_json(constitution));
        } else {
            println!("{}", constitution_text(constitution));
        }
        return 0;
    }

    // From here on the command is `check`: it requires a workspace to observe.
    // An absent `--manifest-path` defaults to the nearest `Cargo.toml`, cargo-style.
    // Defaulting the target location is not a silent pass: if none is found the run
    // exits 2 (a scan error), never 0.
    let manifest_path = match manifest_path {
        Some(path) => PathBuf::from(path),
        None => match nearest_manifest() {
            Some(path) => path,
            None => {
                let from = std::env::current_dir()
                    .map(|dir| dir.display().to_string())
                    .unwrap_or_else(|_| "the current directory".to_string());
                eprintln!(
                    "Modou: no Cargo.toml found from {from} up to the root; \
                     pass --manifest-path <path>"
                );
                return 2;
            }
        },
    };
    if baseline_path.is_some() && write_baseline_path.is_some() {
        return usage("--baseline and --write-baseline are mutually exclusive");
    }

    // One `cargo metadata` read feeds both the reaction outcome and coverage.
    let (mut outcome, observed_coverage) = check_and_cover(constitution, &manifest_path);

    if let Some(path) = write_baseline_path {
        return write_baseline(&outcome, &path);
    }

    // Coverage is an observation, not a reaction: surfaced only when the constitution
    // was successfully evaluated, omitted on a constitution error (where the error is
    // the story), and never affecting the exit code.
    let coverage = match outcome {
        Outcome::ConstitutionError(_) => None,
        _ => observed_coverage,
    };

    if let Some(path) = baseline_path {
        return gate(&mut outcome, &path, json, coverage.as_ref(), warn_uncovered);
    }

    if json {
        println!("{}", report_json(&outcome, &[], coverage.as_ref()));
    } else {
        report(&outcome);
        if let Some(coverage) = &coverage {
            report_coverage(coverage, warn_uncovered);
        }
    }
    outcome.exit_code()
}

/// Print usage to stderr and return exit 2 — a usage mistake is not architectural
/// drift.
fn usage(message: &str) -> u8 {
    eprintln!(
        "usage:\n  \
         modou check --manifest-path <path/to/Cargo.toml> \
         [--baseline <file> | --write-baseline <file>] [--format text|json] \
         [--warn-uncovered]\n  \
         modou list [--format text|json]"
    );
    eprintln!("error: {message}");
    2
}

/// Walk up from the current directory to the nearest `Cargo.toml`, cargo-style, so
/// `check` can default its target like `cargo` does when `--manifest-path` is omitted.
fn nearest_manifest() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Record the current violations as a baseline. Recording is not judging, so this
/// returns 0; but a constitution that could not be evaluated cannot be pinned.
fn write_baseline(outcome: &Outcome, path: &str) -> u8 {
    if let Outcome::ConstitutionError(message) = outcome {
        eprintln!("Modou constitution error: {message}");
        eprintln!("refusing to write a baseline from a constitution that could not be evaluated");
        return 2;
    }
    let empty = Report {
        violations: Vec::new(),
    };
    let report = match outcome {
        Outcome::Violations(report) => report,
        _ => &empty,
    };
    let baseline = Baseline::of(report);
    match std::fs::write(path, baseline.to_json()) {
        Ok(()) => {
            eprintln!(
                "Modou: wrote {} violation(s) to baseline {path}",
                report.violations.len()
            );
            0
        }
        Err(err) => {
            eprintln!("Modou: cannot write baseline {path}: {err}");
            2
        }
    }
}

/// Gate against a baseline: suppress recorded violations, fail only on new ones,
/// and report stale baseline entries. An unreadable baseline is a scan error.
fn gate(
    outcome: &mut Outcome,
    path: &str,
    json: bool,
    coverage: Option<&Coverage>,
    warn_uncovered: bool,
) -> u8 {
    // A constitution error is the whole story: report it before reading the baseline, so
    // it is never masked by a missing or unreadable baseline file (both exit 2, but the
    // constitution error is the actionable one).
    if let Outcome::ConstitutionError(message) = outcome {
        if json {
            println!("{}", report_json(outcome, &[], None));
        } else {
            eprintln!("Modou constitution error: {message}");
        }
        return 2;
    }

    let baseline = match std::fs::read_to_string(path) {
        Ok(text) => match Baseline::from_json(&text) {
            Ok(baseline) => baseline,
            Err(err) => {
                eprintln!("Modou: invalid baseline {path}: {err}");
                return 2;
            }
        },
        Err(err) => {
            eprintln!("Modou: cannot read baseline {path}: {err}");
            return 2;
        }
    };

    if let Outcome::Violations(report) = outcome {
        apply_baseline(report, &baseline);
    }

    let empty = Report {
        violations: Vec::new(),
    };
    let report = match &*outcome {
        Outcome::Violations(report) => report,
        _ => &empty,
    };
    let stale: Vec<ViolationId> = baseline.stale(report).into_iter().cloned().collect();
    if json {
        println!("{}", report_json(outcome, &stale, coverage));
    } else {
        report_violations(report);
        for entry in &stale {
            eprintln!(
                "Modou: stale baseline entry (no longer violated): {} / {} / {}",
                entry.target, entry.rule, entry.finding
            );
        }
        if let Some(coverage) = coverage {
            report_coverage(coverage, warn_uncovered);
        }
    }
    outcome.exit_code()
}

/// The human-readable `check` report goes to **stderr** as a single stream — clean
/// line, violation/advisory blocks, the baseline summary, coverage, and stale entries
/// alike — so a CI log shows them in a deterministic order rather than interleaving a
/// stderr report with a stdout coverage line. Stdout is reserved for machine output:
/// the `--format json` document and the `list` projection. (This mirrors how `cargo`
/// and `clippy` keep diagnostics on stderr and leave stdout for consumable data.)
fn report(outcome: &Outcome) {
    match outcome {
        Outcome::Clean => eprintln!("Modou: clean — no boundary violated"),
        Outcome::Violations(report) => report_violations(report),
        Outcome::ConstitutionError(message) => {
            eprintln!("Modou constitution error: {message}");
        }
    }
}

/// Print each non-baselined violation as a failure (enforce) or advisory (warn),
/// and summarize how many were suppressed by a baseline.
fn report_violations(report: &Report) {
    if report.violations.is_empty() {
        eprintln!("Modou: clean — no boundary violated");
        return;
    }
    let mut baselined = 0usize;
    for violation in &report.violations {
        if violation.baselined {
            baselined += 1;
            continue;
        }
        let (header, reaction) = match violation.severity {
            Severity::Enforce => ("Modou violation", "CI failed."),
            Severity::Warn => ("Modou advisory", "warning only — CI not failed."),
        };
        eprintln!();
        eprintln!("{header}");
        eprintln!();
        eprintln!("Boundary:\n  {}", violation.target);
        eprintln!("Rule:\n  {}", violation.rule);
        eprintln!("Found:\n  {}", violation.finding);
        eprintln!("Reason:\n  {}", violation.reason);
        eprintln!("Reaction:\n  {reaction}");
    }
    if baselined > 0 {
        eprintln!("Modou: {baselined} pre-existing violation(s) suppressed by baseline");
    }
}

/// Print the workspace coverage summary, and — under `--warn-uncovered` — each
/// uncovered crate as a warn-severity advisory. Coverage is an observation: it is
/// reported but never changes the exit code.
fn report_coverage(coverage: &Coverage, warn_uncovered: bool) {
    let uncovered = coverage.uncovered.len();
    if uncovered == 0 {
        eprintln!(
            "Modou: coverage — all {} workspace crate(s) have a boundary",
            coverage.total
        );
        return;
    }
    eprintln!(
        "Modou: coverage — {uncovered} of {} workspace crate(s) have no boundary",
        coverage.total
    );
    if warn_uncovered {
        for crate_name in &coverage.uncovered {
            eprintln!();
            eprintln!("Modou advisory");
            eprintln!();
            eprintln!("Uncovered crate:\n  {crate_name}");
            eprintln!("Reason:\n  no boundary governs this workspace crate");
            eprintln!("Reaction:\n  warning only — CI not failed.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch;
    use crate::prelude::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> String {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
            .join("Cargo.toml")
            .to_string_lossy()
            .into_owned()
    }

    fn example_constitution() -> Constitution {
        Constitution::new("example").boundary(
            CrateBoundary::crate_("example-core")
                .deny_external_dependencies()
                .because("example-core must stay dependency-light"),
        )
    }

    fn run_args(args: &[&str]) -> u8 {
        dispatch(&example_constitution(), args.iter().map(|s| s.to_string()))
    }

    // The clean→0, violating→1, and json-verdict paths require a real fixture
    // workspace, which the published `.crate` cannot carry (Cargo excludes nested
    // packages). Rather than let those tests silently pass as no-ops from the
    // tarball — the very "silent pass" Modou exists to forbid — they live where they
    // can always run: the CLI integration suite (`tests/cli.rs`, in-repo) drives
    // them end-to-end, and `tests/self_governance.rs` exercises `check` against a
    // real workspace (Modou's own) from the package. The runner unit tests below
    // need no fixture: each asserts an exit code decided during argument parsing,
    // before any workspace is observed.

    // Note: `check` with no --manifest-path now defaults to the nearest Cargo.toml
    // (cargo-style), so its behavior depends on the working directory and is covered
    // end-to-end in tests/cli.rs (with a controlled cwd), not here.

    #[test]
    fn both_baseline_flags_exit_2() {
        assert_eq!(
            run_args(&[
                "modou",
                "check",
                "--manifest-path",
                &fixture("clean"),
                "--baseline",
                "a.json",
                "--write-baseline",
                "b.json",
            ]),
            2
        );
    }

    #[test]
    fn unknown_format_exits_2() {
        assert_eq!(
            run_args(&[
                "modou",
                "check",
                "--manifest-path",
                &fixture("clean"),
                "--format",
                "yaml",
            ]),
            2
        );
    }

    #[test]
    fn flag_missing_its_value_is_a_usage_error() {
        // The foot-gun: a value-taking flag with no following token must fail loud
        // (exit 2), not silently downgrade (--format -> text and exit 0, --baseline
        // / --write-baseline -> a plain check). The trailing flag errors during
        // parsing, before any workspace is observed, so no fixture is needed.
        for flag in [
            "--manifest-path",
            "--baseline",
            "--write-baseline",
            "--format",
        ] {
            assert_eq!(
                run_args(&["modou", "check", "--manifest-path", &fixture("clean"), flag]),
                2,
                "{flag} without a value must exit 2",
            );
        }
    }

    #[test]
    fn list_needs_no_manifest_path_and_exits_0() {
        assert_eq!(run_args(&["modou", "list"]), 0);
    }

    #[test]
    fn list_json_exits_0() {
        assert_eq!(run_args(&["modou", "list", "--format", "json"]), 0);
    }

    #[test]
    fn list_unknown_format_is_a_usage_error() {
        assert_eq!(run_args(&["modou", "list", "--format", "yaml"]), 2);
    }

    #[test]
    fn misspelled_flag_fails_loud_instead_of_being_ignored() {
        // The foot-gun: a typo'd --write-baseline must not silently run a plain
        // check (and write no baseline).
        assert_eq!(
            run_args(&[
                "modou",
                "check",
                "--manifest-path",
                &fixture("violating"),
                "--write-baselin",
                "out.json",
            ]),
            2
        );
    }

    #[test]
    fn unknown_flag_exits_2() {
        assert_eq!(
            run_args(&[
                "modou",
                "check",
                "--manifest-path",
                &fixture("clean"),
                "--frobnicate",
            ]),
            2
        );
    }

    #[test]
    fn stray_positional_exits_2() {
        assert_eq!(
            run_args(&[
                "modou",
                "check",
                "stray",
                "--manifest-path",
                &fixture("clean")
            ]),
            2
        );
    }

    #[test]
    fn list_unknown_flag_exits_2() {
        assert_eq!(run_args(&["modou", "list", "--bogus"]), 2);
    }

    #[test]
    fn list_rejects_check_only_flags() {
        // `list` observes no workspace, so a check-only flag is a usage error (exit 2),
        // never a silent no-op. Each is rejected during parsing/dispatch, no fixture.
        for args in [
            &["modou", "list", "--manifest-path", "Cargo.toml"][..],
            &["modou", "list", "--baseline", "b.json"][..],
            &["modou", "list", "--write-baseline", "b.json"][..],
            &["modou", "list", "--warn-uncovered"][..],
        ] {
            assert_eq!(
                run_args(args),
                2,
                "a check-only flag supplied to list must exit 2: {args:?}",
            );
        }
    }
}
