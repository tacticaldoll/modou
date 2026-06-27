//! The functional core (engine): the `check` evaluation spine (this file), plus its
//! cohesive child modules — the pure model and DSL (`model`), the report and baseline
//! (`baseline`), the cargo-metadata observation (`cargo_metadata`), the source `use`
//! scanner (`module_scan`, the observation source for module boundaries), and the text
//! and JSON projections (`projection`). The imperative shell — argument parsing,
//! filesystem, stdout/stderr — lives in the sibling `runner` module. This core MUST
//! NOT depend on the shell (functional core, imperative shell), an invariant Modou
//! enforces on itself in `tests/self_governance.rs`.

use std::path::Path;

use serde_json::Value;

mod module_scan;
use module_scan::{
    canonical_module_path, governed_files, imported_module_paths, reachable_modules, rust_files,
};
mod projection;
pub(crate) use projection::{constitution_json, constitution_text, pretty_json, report_json};
mod baseline;
pub use baseline::*;
mod cargo_metadata;
pub(crate) use cargo_metadata::*;
mod model;
pub use model::*;

/// The reaction's outcome. Exit codes separate architectural drift (1) from a
/// misconfiguration (2), so a mistyped crate name is not reported as drift.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// No enforce-severity boundary was violated (exit 0).
    Clean,
    /// One or more boundaries were violated; carries the full report (exit 1 if any
    /// non-baselined enforce violation exists, else exit 0).
    Violations(Report),
    /// The constitution could not be evaluated — a misconfiguration or scan error
    /// (exit 2). Carries a human-readable message.
    ConstitutionError(String),
}

impl Outcome {
    /// `0` clean, warn-only, or fully baselined; `1` when a non-baselined
    /// enforce-severity violation exists; `2` for a constitution/scan error.
    pub fn exit_code(&self) -> u8 {
        match self {
            Outcome::Clean => 0,
            Outcome::Violations(report) => {
                if report.violations.iter().any(|violation| {
                    violation.severity == Severity::Enforce && !violation.baselined
                }) {
                    1
                } else {
                    0
                }
            }
            Outcome::ConstitutionError(_) => 2,
        }
    }
}

// --- Constitution-error messages ---------------------------------------------
//
// Every constitution error carries a self-describing message (PROJECT.md). These
// constructors are the single source for that wording: each kind is built in
// exactly one place, so the text stays consistent across call sites (two of these
// were previously duplicated verbatim) and tests assert against the constructor
// rather than a brittle substring. They add no new error kind and no behavior —
// `Outcome::exit_code` still maps every constitution error to exit 2.

/// The target workspace could not be read — a missing or malformed manifest, or a
/// `cargo metadata` failure. `err` is the underlying cause.
fn unreadable_workspace_error(manifest_path: &Path, err: &str) -> String {
    format!(
        "cannot read target workspace at {}: {err}",
        manifest_path.display()
    )
}

/// A boundary names a crate that is not a member of the target workspace.
fn crate_not_found_error(crate_package: &str) -> String {
    format!("target crate '{crate_package}' not found in the workspace")
}

/// A workspace member's `src` directory could not be located from its manifest.
fn missing_src_error(crate_package: &str) -> String {
    format!("cannot locate src for crate '{crate_package}'")
}

/// A module boundary targets an inline `mod name { … }`, which owns no source file
/// and so cannot be a governed target — distinct from an unknown-module typo.
fn inline_module_target_error(module: &str, crate_package: &str, leaf: &str) -> String {
    format!(
        "module '{module}' in crate '{crate_package}' is declared inline (`mod {leaf} {{ … }}`) and \
         owns no source file; module boundaries govern file-based modules — move it \
         into its own file (e.g. `src/{leaf}.rs`), or target an enclosing file-based \
         module"
    )
}

/// A module boundary targets a path that is not a reachable module of the crate
/// (e.g. a typo), distinct from an inline target.
fn unknown_module_error(module: &str, crate_package: &str) -> String {
    format!(
        "module '{module}' not found among the reachable modules of crate '{crate_package}' \
         (declared via `mod`, file-based)"
    )
}

/// A `restrict_imports_to` boundary targets the crate root `crate`, which has no
/// outward internal edge — every internal import is within its own subtree, so the
/// rule could never react. A silently un-reactive boundary is the false negative the
/// core contract forbids, so this is a misconfiguration, not a passing check.
fn restrict_imports_to_on_crate_error(crate_package: &str) -> String {
    format!(
        "the `restrict_imports_to` rule cannot govern `crate` (the crate root) of crate \
         '{crate_package}': the root has no outward internal edge — every internal import is \
         within its own subtree, so the rule could never react; declare it on a submodule \
         (e.g. `crate::kernel`) instead"
    )
}

/// A `must_not_be_imported_by` boundary protects the crate root `crate`, against which
/// every internal import (`crate::…`) is "the protected module or beneath" — so the rule
/// degenerates into a total internal-import ban, no longer an inbound rule about a
/// specific module. It could never react as an inbound rule, so this is a
/// misconfiguration, not a passing check.
fn must_not_be_imported_by_on_crate_error(crate_package: &str) -> String {
    format!(
        "the `must_not_be_imported_by` rule cannot protect `crate` (the crate root) of crate \
         '{crate_package}': every internal import is within the crate root, so the rule could \
         never react as an inbound rule; declare it on a submodule (e.g. `crate::internal`) \
         instead"
    )
}

/// A governed source file could not be read. Failing loud rather than skipping it,
/// which could hide a real violation.
fn unreadable_governed_file_error(file: &Path, err: &str) -> String {
    format!(
        "cannot read governed source file '{}': {err}",
        file.display()
    )
}

/// Run the constitution's boundaries against the Cargo workspace at `manifest_path`.
///
/// The spine is **resolve -> observe -> compare -> react**: resolve each target to
/// a workspace package, observe (its dependencies, or its source imports), compare
/// against the rule, and return the outcome. An unresolvable target (or an
/// unreadable workspace) is a constitution error, never a silent pass.
pub fn check(constitution: &Constitution, manifest_path: &Path) -> Outcome {
    match cargo_metadata(manifest_path) {
        Ok(metadata) => evaluate(constitution, &metadata),
        Err(err) => Outcome::ConstitutionError(unreadable_workspace_error(manifest_path, &err)),
    }
}

/// Evaluate the constitution's boundaries against already-observed `cargo metadata` —
/// the compare -> react half of the spine, with the metadata read (the one IO step)
/// left to the caller so a single read can feed both evaluation and coverage (see
/// [`check_and_cover`]). An unresolvable target or a scan error is a constitution
/// error, never a silent pass.
fn evaluate(constitution: &Constitution, metadata: &Value) -> Outcome {
    let workspace = workspace_member_names(metadata);
    let mut violations = Vec::new();
    for boundary in constitution.boundaries() {
        match boundary {
            Boundary::Crate(crate_boundary) => {
                if let Err(error) =
                    check_crate_boundary(metadata, &workspace, crate_boundary, &mut violations)
                {
                    return Outcome::ConstitutionError(error);
                }
            }
            Boundary::Module(module_boundary) => {
                if let Err(error) =
                    check_module_boundary(metadata, module_boundary, &mut violations)
                {
                    return Outcome::ConstitutionError(error);
                }
            }
        }
    }

    if violations.is_empty() {
        Outcome::Clean
    } else {
        Outcome::Violations(Report { violations })
    }
}

/// Read the target workspace once and return both the reaction outcome and workspace
/// coverage. Coverage is `Some` whenever the metadata was observed — including when the
/// outcome is a constitution error from a later boundary; the caller decides whether to
/// surface it. It is `None` only when the metadata itself could not be read. One
/// `cargo metadata` spawn feeds both, where `check` plus a separate coverage pass would
/// have spawned twice.
pub(crate) fn check_and_cover(
    constitution: &Constitution,
    manifest_path: &Path,
) -> (Outcome, Option<Coverage>) {
    let metadata = match cargo_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return (
                Outcome::ConstitutionError(unreadable_workspace_error(manifest_path, &err)),
                None,
            );
        }
    };
    let coverage = coverage_from(workspace_member_names(&metadata), constitution);
    (evaluate(constitution, &metadata), Some(coverage))
}

fn check_crate_boundary(
    metadata: &Value,
    workspace: &[String],
    boundary: &CrateBoundary,
    violations: &mut Vec<Violation>,
) -> Result<(), String> {
    let package = find_package(metadata, &boundary.target.package)
        .ok_or_else(|| crate_not_found_error(&boundary.target.package))?;

    for finding in boundary.rule.findings(package, workspace, boundary.kind) {
        violations.push(Violation {
            kind: BoundaryKind::Crate,
            target: boundary.target.package.clone(),
            rule: boundary.rule.label().to_string(),
            finding,
            reason: boundary.reason.clone(),
            severity: boundary.severity,
            baselined: false,
        });
    }
    Ok(())
}

fn check_module_boundary(
    metadata: &Value,
    boundary: &ModuleBoundary,
    violations: &mut Vec<Violation>,
) -> Result<(), String> {
    let package = find_package(metadata, &boundary.crate_package)
        .ok_or_else(|| crate_not_found_error(&boundary.crate_package))?;
    let src_dir = package["manifest_path"]
        .as_str()
        .and_then(|manifest| Path::new(manifest).parent())
        .map(|crate_dir| crate_dir.join("src"))
        .ok_or_else(|| missing_src_error(&boundary.crate_package))?;

    let files = rust_files(&src_dir)?;
    let reachable = reachable_modules(&src_dir, &files)?;
    // The crate-root module names (direct children of `crate`) feed bare-`use` resolution
    // (a root-relative `use foo::…` is the local module only if `foo` is one of them).
    let root_modules: Vec<String> = reachable
        .iter()
        .filter_map(|module| {
            module
                .strip_prefix("crate::")
                .filter(|rest| !rest.contains("::"))
                .map(str::to_string)
        })
        .collect();
    // Canonicalize the declared module and forbidden paths (raw-identifier `r#name` ->
    // `name`) so they compare in the same vocabulary as the observed paths, which are
    // canonicalized at the file, `mod`, and `use` derivations. A boundary may be written
    // with either the raw or plain form and still match.
    let governed_module = canonical_module_path(&boundary.module);
    let governed = governed_files(&src_dir, &files, &governed_module, &reachable);
    if governed.is_empty() {
        // Two distinct misconfigurations, kept apart so the error is self-describing
        // (PROJECT.md): an inline `mod name { … }` is reachable but owns no source file,
        // so it cannot be a governed target — module boundaries govern file-based modules.
        // A path that is not reachable at all is a genuinely unknown module (e.g. a typo).
        // Both exit 2, never a silent pass; only the message differs.
        if reachable.contains(&governed_module) {
            let leaf = governed_module
                .rsplit("::")
                .next()
                .unwrap_or(&governed_module);
            return Err(inline_module_target_error(
                &boundary.module,
                &boundary.crate_package,
                leaf,
            ));
        }
        return Err(unknown_module_error(
            &boundary.module,
            &boundary.crate_package,
        ));
    }

    let rule = boundary.rule.label().to_string();

    // The inbound rule inverts the scope: it scans every reachable file and tests each
    // importing *module* (not an import path) against the forbidden importer, so it has
    // its own evaluation rather than the shared import-path predicate used by the
    // outbound rules below.
    if let ModuleRule::MustNotBeImportedBy { importer } = &boundary.rule {
        // The crate root degenerates: every internal import is "the protected module or
        // beneath", so the rule could never react as an inbound rule. Fail loud (exit 2)
        // rather than silently pass (PROJECT.md).
        if governed_module == "crate" {
            return Err(must_not_be_imported_by_on_crate_error(
                &boundary.crate_package,
            ));
        }
        let forbidden_importer = canonical_module_path(importer);
        let importer_beneath = format!("{forbidden_importer}::");
        let protected_beneath = format!("{governed_module}::");
        // `governed_files(.., "crate", ..)` yields every reachable (file, module) pair —
        // the crate-wide scan, reusing the existing selector with no new scanner.
        let all_files = governed_files(&src_dir, &files, "crate", &reachable);
        let mut offenders: Vec<String> = Vec::new();
        for (file, current_module) in all_files {
            // A file within the protected module's own subtree is never an inbound
            // importer — a module importing itself is not an inbound edge — so it is
            // skipped even when it sits beneath the forbidden importer.
            if current_module == governed_module || current_module.starts_with(&protected_beneath) {
                continue;
            }
            // Only the forbidden importer (or beneath, `::`-delimited) can violate.
            if current_module != forbidden_importer
                && !current_module.starts_with(&importer_beneath)
            {
                continue;
            }
            let text = std::fs::read_to_string(&file)
                .map_err(|err| unreadable_governed_file_error(&file, &err.to_string()))?;
            let imports_protected = imported_module_paths(&text, &current_module, &root_modules)
                .iter()
                .any(|import| import == &governed_module || import.starts_with(&protected_beneath));
            if imports_protected {
                // finding = the importing module path.
                offenders.push(current_module);
            }
        }
        // One violation per offending importer module (the spec's dedup guarantee). A
        // module can be backed by more than one file — a lib+bin package has both
        // `lib.rs` and `main.rs` at module `crate` — so the same importer can be pushed
        // twice; sort then dedup collapses it rather than relying on one-file-per-module.
        offenders.sort();
        offenders.dedup();
        for importer_module in offenders {
            violations.push(Violation {
                kind: BoundaryKind::Module,
                target: governed_module.clone(),
                rule: rule.clone(),
                finding: importer_module,
                reason: boundary.reason.clone(),
                severity: boundary.severity,
                baselined: false,
            });
        }
        return Ok(());
    }

    // Each outbound rule reduces to one predicate over the governed module's observed
    // internal imports — all `crate::…` (the scanner already filters externals). The
    // file/import loop and the Violation it produces are shared; only the predicate (and,
    // for `RestrictImportsTo`, a crate-root pre-check) differ. Containment is
    // `::`-delimited throughout (exact match OR an `x::` prefix), so a sibling like
    // `crate::types_extra` is never mistaken for being beneath `crate::types`.
    let is_violation: Box<dyn Fn(&str) -> bool> = match &boundary.rule {
        ModuleRule::MustNotImport { module } => {
            let forbidden = canonical_module_path(module);
            let beneath = format!("{forbidden}::");
            Box::new(move |import: &str| import == forbidden || import.starts_with(&beneath))
        }
        ModuleRule::RestrictImportsTo { allowed } => {
            // The crate root has no outward internal edge — every import is within its
            // own subtree, so the rule could never react. Fail loud (exit 2) rather than
            // silently pass (PROJECT.md: the one thing the core contract forbids).
            if governed_module == "crate" {
                return Err(restrict_imports_to_on_crate_error(&boundary.crate_package));
            }
            // Canonicalize allowlist entries (raw-id `r#name` -> `name`) like the governed
            // path, so a boundary may be written with either form and still match.
            let allowed: Vec<String> = allowed
                .iter()
                .map(|entry| canonical_module_path(entry))
                .collect();
            let own_beneath = format!("{governed_module}::");
            let governed_self = governed_module.clone();
            Box::new(move |import: &str| {
                let within_own = import == governed_self || import.starts_with(&own_beneath);
                let within_allowed = allowed
                    .iter()
                    .any(|entry| import == entry || import.starts_with(&format!("{entry}::")));
                // A violation is any outward edge: neither within the module's own subtree
                // nor within an allowlist entry.
                !(within_own || within_allowed)
            })
        }
        ModuleRule::MustNotBeImportedBy { .. } => {
            unreachable!("the inbound rule is evaluated above and returns early")
        }
    };
    let mut findings = Vec::new();
    for (file, current_module) in governed {
        // A governed file we cannot read is "cannot judge", not "nothing to judge":
        // silently skipping it could hide a real violation. Fail as a scan error
        // (exit 2), never a silent pass.
        let text = std::fs::read_to_string(&file)
            .map_err(|err| unreadable_governed_file_error(&file, &err.to_string()))?;
        for import in imported_module_paths(&text, &current_module, &root_modules) {
            if is_violation(&import) {
                findings.push(import);
            }
        }
    }
    // One violation per distinct finding. The governed module's subtree can span more
    // than one file (a parent and child file, or `lib.rs` + `main.rs` both at `crate`),
    // so the same forbidden import can be found twice; sort then dedup collapses it —
    // the same identity guarantee the inbound rule makes, now for the outbound rules.
    findings.sort();
    findings.dedup();
    for finding in findings {
        violations.push(Violation {
            kind: BoundaryKind::Module,
            target: governed_module.clone(),
            rule: rule.clone(),
            finding,
            reason: boundary.reason.clone(),
            severity: boundary.severity,
            baselined: false,
        });
    }
    Ok(())
}

/// Workspace coverage: how many workspace members exist and which are governed by no
/// boundary. A projection (an observation), not a reaction — it never changes the exit
/// code. Internal to the runner; not part of the public API.
pub(crate) struct Coverage {
    /// Total number of workspace members.
    pub(crate) total: usize,
    /// Names of workspace members that are the target of no boundary, sorted.
    pub(crate) uncovered: Vec<String>,
}

/// The pure core of coverage: workspace `members` against the crates any boundary
/// targets. A crate counts as covered by a crate boundary on it or a module boundary
/// within it.
fn coverage_from(members: Vec<String>, constitution: &Constitution) -> Coverage {
    let mut targeted: Vec<&str> = Vec::new();
    for boundary in constitution.boundaries() {
        match boundary {
            Boundary::Crate(b) => targeted.push(b.target().package.as_str()),
            Boundary::Module(b) => targeted.push(b.crate_package.as_str()),
        }
    }
    let total = members.len();
    let uncovered = members
        .into_iter()
        .filter(|member| !targeted.contains(&member.as_str()))
        .collect();
    Coverage { total, uncovered }
}

#[cfg(test)]
mod tests {
    //! White-box unit tests for the crate-private machinery — the baseline, the JSON
    //! and text projections, and the source scanner. Black-box behavior (running
    //! `check` against fixture workspaces) lives in `tests/dogfood.rs`.
    use super::*;

    fn one_enforce_violation() -> Report {
        Report {
            violations: vec![Violation {
                kind: BoundaryKind::Crate,
                target: "core".to_string(),
                rule: "deny external dependencies".to_string(),
                finding: "serde".to_string(),
                reason: "core must stay dependency-light".to_string(),
                severity: Severity::Enforce,
                baselined: false,
            }],
        }
    }

    /// An unreadable governed source file must surface as a scan error (exit 2),
    /// not a silent skip that could hide a real module-boundary violation. Unix
    /// only (permission-based) and self-calibrating: it skips under a privileged
    /// user (e.g. root in CI), where mode 0 is still readable, rather than
    /// false-passing.
    #[cfg(unix)]
    #[test]
    fn unreadable_governed_file_is_a_scan_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("modou-unreadable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("create temp src");
        let file = src.join("lib.rs");
        std::fs::write(&file, "use crate::forbidden::Thing;\n").expect("write governed file");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000))
            .expect("drop read permission");

        // Self-calibrating root guard: if mode 0 is still readable, permissions do
        // not bite here, so the premise cannot hold — skip rather than false-pass.
        if std::fs::read_to_string(&file).is_ok() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "x",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });
        let boundary = ModuleBoundary::in_crate("x")
            .module("crate")
            .must_not_import("crate::forbidden")
            .because("the test module must not import the forbidden module");

        let mut violations = Vec::new();
        let result = check_module_boundary(&metadata, &boundary, &mut violations);

        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            result.is_err(),
            "an unreadable governed file must be a scan error, not a silent skip"
        );
    }

    /// An unreadable governed *directory* must surface as a scan error (exit 2), the
    /// same "cannot judge, not nothing to judge" rule as an unreadable file: a skipped
    /// subtree could hide a real module-boundary violation. Unix only and
    /// self-calibrating (skips under a privileged user where mode 0 is still readable).
    #[cfg(unix)]
    #[test]
    fn unreadable_governed_directory_is_a_scan_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("modou-unreadable-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("create temp src");
        std::fs::write(src.join("lib.rs"), "// nothing\n").expect("write lib.rs");
        let sub = src.join("sub");
        std::fs::create_dir_all(&sub).expect("create sub dir");
        std::fs::write(sub.join("inner.rs"), "use crate::forbidden::Thing;\n")
            .expect("write inner");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000))
            .expect("drop dir read/exec permission");

        // Self-calibrating root guard: if the directory is still traversable, the
        // premise cannot hold — skip rather than false-pass.
        if std::fs::read_dir(&sub).is_ok() {
            let _ = std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755));
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }

        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "x",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });
        let boundary = ModuleBoundary::in_crate("x")
            .module("crate")
            .must_not_import("crate::forbidden")
            .because("the test module must not import the forbidden module");

        let mut violations = Vec::new();
        let result = check_module_boundary(&metadata, &boundary, &mut violations);

        let _ = std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755));
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            result.is_err(),
            "an unreadable governed directory must be a scan error, not a silent skip"
        );
    }

    /// A module whose name is a raw identifier (`mod r#type;`, file `type.rs`) must be
    /// governable and its forbidden imports observed — exercising the canonicalization
    /// in `check_module_boundary` end to end. The boundary is declared with the *plain*
    /// form (`crate::type`) and still matches the raw-identifier source.
    #[test]
    fn a_raw_identifier_module_is_governed_and_its_import_observed() {
        let dir = std::env::temp_dir().join(format!("modou-rawid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("create temp src");
        std::fs::write(src.join("lib.rs"), "pub mod r#type;\n").expect("write lib.rs");
        std::fs::write(src.join("type.rs"), "use crate::r#mod::Thing;\n").expect("write type.rs");

        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "x",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });
        let boundary = ModuleBoundary::in_crate("x")
            .module("crate::type")
            .must_not_import("crate::mod")
            .because("a raw-identifier module must be governable");

        let mut violations = Vec::new();
        let result = check_module_boundary(&metadata, &boundary, &mut violations);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            result.is_ok(),
            "a raw-identifier module must be found, not an unknown-module error: {result:?}"
        );
        assert_eq!(
            violations.len(),
            1,
            "the forbidden import from inside the raw-identifier module must be observed: {violations:?}"
        );
        assert_eq!(violations[0].target, "crate::type");
        assert_eq!(violations[0].finding, "crate::mod::Thing");
    }

    /// An inline `mod kernel { … }` is reachable but owns no source file, so it cannot
    /// be a governed target (targets are file-based). The reaction must fail loud (exit 2)
    /// with a *self-describing* error that names the inline cause — not the misleading
    /// "not found among the reachable modules", which would suggest a typo. A genuinely
    /// unknown module still gets the "not found" message.
    #[test]
    fn an_inline_module_target_is_a_self_describing_constitution_error() {
        let dir = std::env::temp_dir().join(format!("modou-inline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("create temp src");
        std::fs::write(
            src.join("lib.rs"),
            "pub mod kernel { use crate::projection::Thing; }\npub mod projection { pub struct Thing; }\n",
        )
        .expect("write lib.rs");

        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "app",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });

        let inline = ModuleBoundary::in_crate("app")
            .module("crate::kernel")
            .must_not_import("crate::projection")
            .because("the kernel must not import a projection");
        let mut violations = Vec::new();
        let inline_err = check_module_boundary(&metadata, &inline, &mut violations)
            .expect_err("an inline target must be a constitution error");
        // Assert against the single-source constructor, not a brittle substring: the
        // inline target reports the inline cause, never the unknown-module message.
        assert_eq!(
            inline_err,
            inline_module_target_error("crate::kernel", "app", "kernel")
        );
        assert_ne!(inline_err, unknown_module_error("crate::kernel", "app"));

        // A genuinely unknown module path still gets the unknown-module message.
        let typo = ModuleBoundary::in_crate("app")
            .module("crate::ghost")
            .must_not_import("crate::projection")
            .because("typo");
        let typo_err = check_module_boundary(&metadata, &typo, &mut violations)
            .expect_err("an unknown module is a constitution error");
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(typo_err, unknown_module_error("crate::ghost", "app"));
    }

    /// Run a module boundary against a synthetic one-package workspace whose `src`
    /// holds `files` (each `(relative path, contents)`), under a unique temp dir keyed
    /// by `name`. Returns the check result and the collected violations.
    fn run_module_check(
        name: &str,
        files: &[(&str, &str)],
        boundary: ModuleBoundary,
    ) -> (Result<(), String>, Vec<Violation>) {
        let dir = std::env::temp_dir().join(format!("modou-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        for (rel, contents) in files {
            let path = src.join(rel);
            std::fs::create_dir_all(path.parent().expect("file has a parent"))
                .expect("create src dirs");
            std::fs::write(&path, contents).expect("write source file");
        }
        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "x",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });
        let mut violations = Vec::new();
        let result = check_module_boundary(&metadata, &boundary, &mut violations);
        let _ = std::fs::remove_dir_all(&dir);
        (result, violations)
    }

    fn restrict_kernel_to_types(governed: &str, allowed: &[&str]) -> ModuleBoundary {
        ModuleBoundary::in_crate("x")
            .module(governed)
            .restrict_imports_to(allowed.to_vec())
            .because("the kernel may import only the allowed modules")
    }

    #[test]
    fn restrict_imports_to_flags_an_import_outside_the_allowlist() {
        let (result, violations) = run_module_check(
            "restrict-outside",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use crate::io::Sink;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].target, "crate::kernel");
        assert_eq!(violations[0].finding, "crate::io::Sink");
    }

    #[test]
    fn restrict_imports_to_is_clean_within_the_allowlist() {
        let (result, violations) = run_module_check(
            "restrict-within",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use crate::types::Id;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(violations.is_empty(), "{violations:?}");
    }

    #[test]
    fn restrict_imports_to_allows_the_governed_modules_own_subtree() {
        // The exact module (`crate::kernel`), a descendant, and a `self::` import all
        // resolve within the governed subtree and are not outward edges — so none need
        // to be listed in the allowlist.
        let (result, violations) = run_module_check(
            "restrict-ownsubtree",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                (
                    "kernel.rs",
                    "use crate::kernel;\nuse crate::kernel::detail::Thing;\nuse self::other::Thing2;\n",
                ),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(violations.is_empty(), "own-subtree imports: {violations:?}");
    }

    #[test]
    fn restrict_imports_to_with_an_empty_allowlist_forbids_outward_imports() {
        let (result, violations) = run_module_check(
            "restrict-empty",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use crate::types::Id;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &[]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].finding, "crate::types::Id");
    }

    #[test]
    fn restrict_imports_to_does_not_treat_a_prefix_colliding_sibling_as_allowed() {
        // The `::`-delimited containment must not let `crate::types_extra` ride in on the
        // `crate::types` allowlist entry — the headline regression guard.
        let (result, violations) = run_module_check(
            "restrict-sibling",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                (
                    "kernel.rs",
                    "use crate::types::Id;\nuse crate::types_extra::Y;\n",
                ),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "only the sibling violates: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate::types_extra::Y");
    }

    #[test]
    fn restrict_imports_to_never_flags_an_external_import() {
        let (result, violations) = run_module_check(
            "restrict-external",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use serde::Deserialize;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &[]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "externals are out of scope: {violations:?}"
        );
    }

    #[test]
    fn restrict_imports_to_governs_a_super_reaching_outward_import() {
        let (result, violations) = run_module_check(
            "restrict-super",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use super::other::Thing;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(
            violations[0].finding, "crate::other::Thing",
            "super:: resolves to an absolute outward path that is governed"
        );
    }

    #[test]
    fn restrict_imports_to_canonicalizes_a_raw_identifier_allowlist_entry() {
        let (result, violations) = run_module_check(
            "restrict-rawid",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use crate::r#type::Thing;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::r#type"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "the raw-id entry canonicalizes to match the import: {violations:?}"
        );
    }

    #[test]
    fn restrict_imports_to_on_the_crate_root_is_a_constitution_error() {
        // The crate root has no outward internal edge, so the rule could never react —
        // fail loud (exit 2), never silently pass.
        let (result, _violations) = run_module_check(
            "restrict-crate",
            &[("lib.rs", "use crate::anything::X;\n")],
            restrict_kernel_to_types("crate", &["crate::types"]),
        );
        let err = result.expect_err("governing `crate` must be a constitution error");
        assert_eq!(err, restrict_imports_to_on_crate_error("x"));
    }

    #[test]
    fn restrict_imports_to_honors_warn_severity_and_its_distinct_label() {
        let (result, violations) = run_module_check(
            "restrict-warn",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "use crate::io::Sink;\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::kernel")
                .restrict_imports_to(["crate::types"])
                .warn()
                .because("the kernel should import only types"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].severity, Severity::Warn);
        // A label distinct from `must not import`, so baseline identity does not collide.
        assert_eq!(violations[0].rule, "restrict imports to");
    }

    fn protect_internal_from(importer: &str) -> ModuleBoundary {
        ModuleBoundary::in_crate("x")
            .module("crate::internal")
            .must_not_be_imported_by(importer)
            .because("internal is private to its layer")
    }

    #[test]
    fn must_not_be_imported_by_flags_the_forbidden_importer_only() {
        let (result, violations) = run_module_check(
            "inbound-basic",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\npub mod api;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "use crate::internal::Secret;\n"),
                ("api.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        // Only crate::http is beneath the forbidden importer; crate::api imports internal
        // too but is outside crate::http, so it is clean.
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].target, "crate::internal");
        assert_eq!(violations[0].finding, "crate::http");
        assert_eq!(violations[0].rule, "module must not be imported by");
    }

    #[test]
    fn must_not_be_imported_by_applies_beneath_the_importer() {
        let (result, violations) = run_module_check(
            "inbound-beneath-importer",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "pub mod v1;\n"),
                ("http/v1.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(
            violations[0].finding, "crate::http::v1",
            "the importer beneath crate::http is named"
        );
    }

    #[test]
    fn must_not_be_imported_by_applies_beneath_the_protected_module() {
        let (result, violations) = run_module_check(
            "inbound-beneath-protected",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "use crate::internal::deep::Thing;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "an import beneath the protected module violates: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate::http");
    }

    #[test]
    fn must_not_be_imported_by_ignores_prefix_colliding_siblings_on_both_sides() {
        let (result, violations) = run_module_check(
            "inbound-collision",
            &[
                (
                    "lib.rs",
                    "pub mod internal;\npub mod http;\npub mod httpx;\n",
                ),
                ("internal.rs", "// protected\n"),
                // forbidden importer is crate::http; crate::http imports a sibling of the
                // protected module (internal_util), which is clean.
                ("http.rs", "use crate::internal_util::X;\n"),
                // crate::httpx is a sibling of the forbidden importer; importing internal
                // is clean.
                ("httpx.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "::-delimited containment must not match siblings on either side: {violations:?}"
        );
    }

    #[test]
    fn must_not_be_imported_by_does_not_flag_the_protected_modules_own_subtree() {
        let (result, violations) = run_module_check(
            "inbound-own-subtree",
            &[
                ("lib.rs", "pub mod a;\n"),
                ("a.rs", "pub mod b;\n"),
                // crate::a::b is the protected module; it imports its own subtree and sits
                // beneath the forbidden importer crate::a — but a module importing itself
                // is not an inbound edge, so it is clean.
                ("a/b.rs", "use crate::a::b::detail::Thing;\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::a::b")
                .must_not_be_imported_by("crate::a")
                .because("a::b is private"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "the protected module's own subtree is not an importer: {violations:?}"
        );
    }

    #[test]
    fn must_not_be_imported_by_ignores_external_imports() {
        let (result, violations) = run_module_check(
            "inbound-external",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "use serde::Deserialize;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "externals are out of scope: {violations:?}"
        );
    }

    #[test]
    fn must_not_be_imported_by_crate_forbids_every_outside_importer() {
        let (result, violations) = run_module_check(
            "inbound-x-crate",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate"),
        );
        assert!(result.is_ok(), "{result:?}");
        // Forbidding importer `crate` means nobody outside internal's own subtree may
        // import it; crate::http violates, internal's own files stay clean.
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].finding, "crate::http");
    }

    #[test]
    fn must_not_be_imported_by_on_the_crate_root_is_a_constitution_error() {
        let (result, _violations) = run_module_check(
            "inbound-m-crate",
            &[("lib.rs", "pub mod http;\n"), ("http.rs", "// nothing\n")],
            ModuleBoundary::in_crate("x")
                .module("crate")
                .must_not_be_imported_by("crate::http")
                .because("the crate root cannot be protected this way"),
        );
        let err = result.expect_err("protecting `crate` must be a constitution error");
        assert_eq!(err, must_not_be_imported_by_on_crate_error("x"));
    }

    #[test]
    fn must_not_be_imported_by_dedups_multiple_imports_from_one_importer() {
        let (result, violations) = run_module_check(
            "inbound-dedup",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                (
                    "http.rs",
                    "use crate::internal::A;\nuse crate::internal::B;\n",
                ),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "one offending importer module yields one violation: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate::http");
    }

    #[test]
    fn must_not_be_imported_by_honors_warn_severity() {
        let (result, violations) = run_module_check(
            "inbound-warn",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http.rs", "use crate::internal::Secret;\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::internal")
                .must_not_be_imported_by("crate::http")
                .warn()
                .because("internal should be private"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].severity, Severity::Warn);
    }

    #[test]
    fn must_not_be_imported_by_projects_its_importer() {
        let constitution = Constitution::new("p").boundary(
            ModuleBoundary::in_crate("app")
                .module("crate::internal")
                .must_not_be_imported_by("crate::http")
                .because("internal is private to its layer"),
        );

        let text = constitution_text(&constitution);
        assert!(
            text.contains("must not be imported by crate::http"),
            "{text}"
        );

        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(
            doc["boundaries"][0]["rule"],
            "module must not be imported by"
        );
        assert_eq!(doc["boundaries"][0]["target"], "crate::internal");
        // The declared forbidden importer projects as `importer`; no `forbidden`/`only`.
        assert_eq!(doc["boundaries"][0]["importer"], "crate::http");
        assert!(doc["boundaries"][0]["forbidden"].is_null());
        assert!(doc["boundaries"][0]["only"].is_null());
    }

    #[test]
    fn must_not_be_imported_by_unknown_protected_module_is_a_constitution_error() {
        // The protected-module validation must fire for the inbound rule too: an unknown
        // `m` is exit 2 before any scan, never a silent clean.
        let (result, _violations) = run_module_check(
            "inbound-unknown-m",
            &[("lib.rs", "pub mod http;\n"), ("http.rs", "// nothing\n")],
            ModuleBoundary::in_crate("x")
                .module("crate::nope")
                .must_not_be_imported_by("crate::http")
                .because("typo target"),
        );
        let err = result.expect_err("an unknown protected module is a constitution error");
        assert_eq!(err, unknown_module_error("crate::nope", "x"));
    }

    #[test]
    fn must_not_be_imported_by_inline_protected_module_is_a_constitution_error() {
        let (result, _violations) = run_module_check(
            "inbound-inline-m",
            &[
                (
                    "lib.rs",
                    "pub mod kernel { pub struct K; }\npub mod http;\n",
                ),
                ("http.rs", "// nothing\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::kernel")
                .must_not_be_imported_by("crate::http")
                .because("inline target"),
        );
        let err = result.expect_err("an inline protected module is a constitution error");
        assert_eq!(
            err,
            inline_module_target_error("crate::kernel", "x", "kernel")
        );
    }

    #[test]
    fn must_not_be_imported_by_matches_a_raw_identifier_importer() {
        // The forbidden importer is declared with a raw identifier; the importing file's
        // module canonicalizes to the same path, so the violation still fires (guards the
        // canonicalization lockstep against a false negative).
        let (result, violations) = run_module_check(
            "inbound-rawid-importer",
            &[
                ("lib.rs", "pub mod internal;\npub mod r#async;\n"),
                ("internal.rs", "// protected\n"),
                ("async.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate::r#async"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].finding, "crate::async");
    }

    #[test]
    fn must_not_be_imported_by_protects_a_raw_identifier_module() {
        let (result, violations) = run_module_check(
            "inbound-rawid-protected",
            &[
                ("lib.rs", "pub mod r#type;\npub mod http;\n"),
                ("type.rs", "// protected\n"),
                ("http.rs", "use crate::r#type::Thing;\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::r#type")
                .must_not_be_imported_by("crate::http")
                .because("type is private"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].target, "crate::type");
        assert_eq!(violations[0].finding, "crate::http");
    }

    #[test]
    fn must_not_be_imported_by_flags_a_mod_rs_backed_importer() {
        let (result, violations) = run_module_check(
            "inbound-modrs",
            &[
                ("lib.rs", "pub mod internal;\npub mod http;\n"),
                ("internal.rs", "// protected\n"),
                ("http/mod.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate::http"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert_eq!(violations[0].finding, "crate::http");
    }

    #[test]
    fn must_not_be_imported_by_orders_multiple_offenders_deterministically() {
        let (result, violations) = run_module_check(
            "inbound-multi",
            &[
                (
                    "lib.rs",
                    "pub mod internal;\npub mod zeta;\npub mod alpha;\n",
                ),
                ("internal.rs", "// protected\n"),
                ("zeta.rs", "use crate::internal::Secret;\n"),
                ("alpha.rs", "use crate::internal::Secret;\n"),
            ],
            protect_internal_from("crate"),
        );
        assert!(result.is_ok(), "{result:?}");
        let findings: Vec<&str> = violations.iter().map(|v| v.finding.as_str()).collect();
        assert_eq!(
            findings,
            ["crate::alpha", "crate::zeta"],
            "multiple offenders are sorted deterministically"
        );
    }

    #[test]
    fn must_not_be_imported_by_dedups_an_importer_backed_by_lib_and_main() {
        // A lib+bin package has both `lib.rs` and `main.rs` at module `crate`. With
        // `must_not_be_imported_by("crate")`, both root files importing the protected
        // module would push `crate` twice — the spec's dedup must collapse it to one.
        let (result, violations) = run_module_check(
            "inbound-lib-and-main",
            &[
                (
                    "lib.rs",
                    "pub mod internal;\nuse crate::internal::Secret;\n",
                ),
                ("main.rs", "use crate::internal::Secret;\n"),
                ("internal.rs", "// protected\n"),
            ],
            protect_internal_from("crate"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "one offending importer module, even when backed by two root files: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate");
    }

    #[test]
    fn must_not_import_dedups_a_finding_across_subtree_files() {
        // crate::kernel spans kernel.rs + kernel/sub.rs; both import the forbidden module.
        // The same finding must be reported once, not once per file.
        let (result, violations) = run_module_check(
            "dedup-mni-subtree",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "pub mod sub;\nuse crate::forbidden::X;\n"),
                ("kernel/sub.rs", "use crate::forbidden::X;\n"),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::kernel")
                .must_not_import("crate::forbidden")
                .because("kernel must not import forbidden"),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "one violation per distinct finding: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate::forbidden::X");
    }

    #[test]
    fn restrict_imports_to_dedups_a_finding_across_subtree_files() {
        let (result, violations) = run_module_check(
            "dedup-rit-subtree",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "pub mod sub;\nuse crate::io::Sink;\n"),
                ("kernel/sub.rs", "use crate::io::Sink;\n"),
            ],
            restrict_kernel_to_types("crate::kernel", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            violations.len(),
            1,
            "one violation per distinct finding: {violations:?}"
        );
        assert_eq!(violations[0].finding, "crate::io::Sink");
    }

    #[test]
    fn outbound_dedup_collapses_identical_findings_but_keeps_distinct_ones() {
        // Two subtree files: one imports X, the other imports X (duplicate) and Y.
        // Result must be {X, Y} — the identical finding collapsed, the distinct one kept.
        let (result, violations) = run_module_check(
            "dedup-distinct",
            &[
                ("lib.rs", "pub mod kernel;\n"),
                ("kernel.rs", "pub mod sub;\nuse crate::forbidden::X;\n"),
                (
                    "kernel/sub.rs",
                    "use crate::forbidden::X;\nuse crate::forbidden::Y;\n",
                ),
            ],
            ModuleBoundary::in_crate("x")
                .module("crate::kernel")
                .must_not_import("crate::forbidden")
                .because("kernel must not import forbidden"),
        );
        assert!(result.is_ok(), "{result:?}");
        let findings: Vec<&str> = violations.iter().map(|v| v.finding.as_str()).collect();
        assert_eq!(
            findings,
            ["crate::forbidden::X", "crate::forbidden::Y"],
            "{violations:?}"
        );
        // And no two violations share an identity (target, rule, finding).
        let mut ids: Vec<_> = violations
            .iter()
            .map(|v| (&v.target, &v.rule, &v.finding))
            .collect();
        let before = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), before, "no duplicate violation identities");
    }

    #[test]
    fn restrict_imports_to_does_not_flag_an_over_popped_super() {
        // `crate::a` over-pops with `super::super`; the path names no internal module, so
        // it must not be observed — and must not be mistaken for an outward edge that the
        // allowlist would flag (the regression this guards).
        let (result, violations) = run_module_check(
            "restrict-super-overflow",
            &[
                ("lib.rs", "pub mod a;\n"),
                ("a.rs", "use super::super::other::X;\n"),
            ],
            restrict_kernel_to_types("crate::a", &["crate::types"]),
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            violations.is_empty(),
            "an over-popped super is not an outward edge: {violations:?}"
        );
    }

    #[test]
    fn baseline_round_trips_through_json() {
        let report = one_enforce_violation();
        let json = Baseline::of(&report).to_json();
        let parsed = Baseline::from_json(&json).expect("a written baseline parses");
        assert!(
            parsed.contains(&report.violations[0]),
            "round-trip must preserve the violation identity"
        );
    }

    #[test]
    fn from_json_rejects_malformed_and_unknown_version() {
        assert!(Baseline::from_json("not json").is_err());
        assert!(Baseline::from_json(r#"{"version":2,"violations":[]}"#).is_err());
        assert!(
            Baseline::from_json(r#"{"violations":[]}"#).is_err(),
            "a missing version must be an error, not a silent empty baseline"
        );
    }

    #[test]
    fn a_baselined_enforce_violation_does_not_fail() {
        let mut report = one_enforce_violation();
        let baseline = Baseline::of(&report);
        apply_baseline(&mut report, &baseline);
        assert!(report.violations[0].baselined);
        assert_eq!(
            Outcome::Violations(report).exit_code(),
            0,
            "a fully baselined run must not fail"
        );
    }

    #[test]
    fn a_new_enforce_violation_fails_against_a_baseline() {
        let baseline = Baseline::from_json(
            r#"{"version":1,"violations":[{"target":"core","rule":"deny external dependencies","finding":"other"}]}"#,
        )
        .unwrap();
        let mut report = one_enforce_violation();
        apply_baseline(&mut report, &baseline);
        assert!(
            !report.violations[0].baselined,
            "serde is not in the baseline"
        );
        assert_eq!(Outcome::Violations(report).exit_code(), 1);
    }

    #[test]
    fn stale_finds_entries_with_no_current_match() {
        let report = one_enforce_violation();
        let baseline = Baseline::from_json(
            r#"{"version":1,"violations":[{"target":"core","rule":"deny external dependencies","finding":"gone"}]}"#,
        )
        .unwrap();
        let stale = baseline.stale(&report);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].finding, "gone");
    }

    #[test]
    fn report_json_projects_a_violation_with_its_kind() {
        let json = report_json(&Outcome::Violations(one_enforce_violation()), &[], None);
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc["outcome"], "violations");
        assert_eq!(doc["exit_code"], 1);
        let violation = &doc["violations"][0];
        assert_eq!(violation["kind"], "crate");
        assert_eq!(violation["finding"], "serde");
        assert_eq!(violation["severity"], "enforce");
        assert_eq!(violation["baselined"], false);
        // `reason` is the repair hint; there is no separate field.
        assert!(violation["reason"].as_str().is_some_and(|r| !r.is_empty()));
        assert!(doc.get("repair_hint").is_none());
    }

    #[test]
    fn report_json_renders_clean_and_constitution_error() {
        let clean: serde_json::Value =
            serde_json::from_str(&report_json(&Outcome::Clean, &[], None)).unwrap();
        assert_eq!(clean["outcome"], "clean");
        assert_eq!(clean["exit_code"], 0);
        assert_eq!(clean["violations"].as_array().unwrap().len(), 0);
        assert!(clean.get("coverage").is_none(), "no coverage when None");

        let error: serde_json::Value = serde_json::from_str(&report_json(
            &Outcome::ConstitutionError("boom".into()),
            &[],
            None,
        ))
        .unwrap();
        assert_eq!(error["outcome"], "constitution_error");
        assert_eq!(error["exit_code"], 2);
        assert_eq!(error["error"], "boom");
    }

    #[test]
    fn report_json_reflects_baseline_and_stale_in_gate() {
        let mut report = one_enforce_violation();
        let baseline = Baseline::of(&report);
        apply_baseline(&mut report, &baseline);
        // A baseline entry that no current violation matches is stale.
        let stale = vec![ViolationId {
            target: "core".to_string(),
            rule: "deny external dependencies".to_string(),
            finding: "gone".to_string(),
        }];
        let doc: serde_json::Value =
            serde_json::from_str(&report_json(&Outcome::Violations(report), &stale, None)).unwrap();
        assert_eq!(doc["exit_code"], 0, "a fully baselined run does not fail");
        assert_eq!(doc["violations"][0]["baselined"], true);
        assert_eq!(doc["stale_baseline"][0]["finding"], "gone");
    }

    #[test]
    fn report_json_includes_coverage_when_present() {
        let coverage = Coverage {
            total: 3,
            uncovered: vec!["memory".to_string()],
        };
        let doc: serde_json::Value =
            serde_json::from_str(&report_json(&Outcome::Clean, &[], Some(&coverage))).unwrap();
        assert_eq!(doc["coverage"]["workspace_crates"], 3);
        assert_eq!(doc["coverage"]["uncovered"][0], "memory");
    }

    #[test]
    fn external_classification_treats_any_non_null_source_as_external() {
        // A path/internal dep has a null `source`; registry, git, and alternative
        // (sparse) registry deps all have a non-null source and must be classified
        // external. The sparse case is the regression guard: a fixed `registry+`/
        // `git+` prefix list would silently pass an alternative `sparse+` registry.
        let package = serde_json::json!({
            "dependencies": [
                { "name": "internal", "source": null, "kind": null },
                {
                    "name": "crates_io",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "kind": null
                },
                { "name": "git_dep", "source": "git+https://example.com/x", "kind": null },
                { "name": "alt_sparse", "source": "sparse+https://my.registry/index/", "kind": null },
                {
                    "name": "a_dev",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "kind": "dev"
                },
            ]
        });
        assert_eq!(
            external_dependencies(&package, DependencyKind::Normal),
            vec![
                "alt_sparse".to_string(),
                "crates_io".to_string(),
                "git_dep".to_string(),
            ],
            "every non-null-source normal dep is external (incl. a sparse alt \
             registry); the null-source internal dep and the dev dep are excluded",
        );
    }

    #[test]
    fn dependency_kind_selects_which_table_is_observed() {
        // `serde` is a normal dep; `proptest` is a dev-dep; `cc` is a build-dep.
        let package = serde_json::json!({
            "dependencies": [
                { "name": "serde", "source": "registry+x", "kind": null },
                { "name": "proptest", "source": "registry+x", "kind": "dev" },
                { "name": "cc", "source": "registry+x", "kind": "build" },
            ]
        });
        let deny = Rule::DenyExternalDependencies { allowed: vec![] };
        // Default (normal) sees only serde; dev sees only proptest; build only cc.
        assert_eq!(
            deny.findings(&package, &[], DependencyKind::Normal),
            vec!["serde".to_string()]
        );
        assert_eq!(
            deny.findings(&package, &[], DependencyKind::Dev),
            vec!["proptest".to_string()]
        );
        assert_eq!(
            deny.findings(&package, &[], DependencyKind::Build),
            vec!["cc".to_string()]
        );
    }

    #[test]
    fn workspace_member_names_are_the_no_deps_packages() {
        // With `--no-deps`, `packages` is exactly the workspace members.
        let metadata = serde_json::json!({
            "packages": [ { "name": "core" }, { "name": "adapters" } ]
        });
        assert_eq!(
            workspace_member_names(&metadata),
            vec!["adapters".to_string(), "core".to_string()],
        );
    }

    #[test]
    fn workspace_rule_flags_only_unlisted_workspace_members() {
        // Deps: two workspace members (core, adapters), one external (serde), and one
        // path dependency that is NOT a workspace member (outside).
        let package = serde_json::json!({
            "dependencies": [
                { "name": "core", "source": null, "kind": null },
                { "name": "adapters", "source": null, "kind": null },
                {
                    "name": "serde",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "kind": null
                },
                { "name": "outside", "source": null, "kind": null },
            ]
        });
        let workspace = vec!["core".to_string(), "adapters".to_string()];

        // Restrict to [core]: adapters is an unlisted workspace member → flagged;
        // serde (external) and outside (path, non-member) are ignored.
        let restrict = Rule::RestrictWorkspaceDependenciesTo {
            allowed: vec!["core".to_string()],
        };
        assert_eq!(
            restrict.findings(&package, &workspace, DependencyKind::Normal),
            vec!["adapters".to_string()],
        );

        // Empty allowlist forbids every workspace member, still ignoring external and
        // the non-member path dependency.
        let forbid_all = Rule::RestrictWorkspaceDependenciesTo { allowed: vec![] };
        assert_eq!(
            forbid_all.findings(&package, &workspace, DependencyKind::Normal),
            vec!["adapters".to_string(), "core".to_string()],
        );
    }

    #[test]
    fn coverage_counts_a_module_only_covered_crate_as_covered() {
        let members = vec!["app".to_string(), "core".to_string(), "memory".to_string()];
        let constitution = Constitution::new("c")
            .boundary(
                CrateBoundary::crate_("core")
                    .forbid_all_workspace_dependencies()
                    .because("core is independent"),
            )
            .boundary(
                ModuleBoundary::in_crate("app")
                    .module("crate::kernel")
                    .must_not_import("crate::projection")
                    .because("layering"),
            );
        let coverage = coverage_from(members, &constitution);
        assert_eq!(coverage.total, 3);
        // `app` is covered by the module boundary, `core` by the crate boundary;
        // only `memory` has no boundary at all.
        assert_eq!(coverage.uncovered, vec!["memory".to_string()]);
    }

    fn mixed_constitution() -> Constitution {
        Constitution::new("my-project")
            .boundary(
                CrateBoundary::crate_("my-core")
                    .deny_external_dependencies()
                    .allow_external(["serde"])
                    .because("my-core must stay dependency-light"),
            )
            .boundary(
                CrateBoundary::crate_("my-core")
                    .forbid_dependency_on(["my-adapters"])
                    .because("the core must not depend on adapters"),
            )
            .boundary(
                ModuleBoundary::in_crate("my-app")
                    .module("crate::domain")
                    .must_not_import("crate::http")
                    .warn()
                    .because("the domain must not import the HTTP layer"),
            )
    }

    #[test]
    fn constitution_text_projects_every_boundary_with_its_parameters() {
        let text = constitution_text(&mixed_constitution());
        assert!(
            text.contains("Constitution: my-project  (3 boundaries)"),
            "{text}"
        );
        assert!(text.contains("crate my-core"), "{text}");
        assert!(
            text.contains("deny external dependencies (allow: serde)"),
            "{text}"
        );
        assert!(text.contains("forbid dependency on: my-adapters"), "{text}");
        assert!(text.contains("module crate::domain in my-app"), "{text}");
        assert!(text.contains("must not import crate::http"), "{text}");
        // Severity and reason both surface.
        assert!(
            text.contains("[warn]") && text.contains("[enforce]"),
            "{text}"
        );
        assert!(
            text.contains("the domain must not import the HTTP layer"),
            "{text}"
        );
    }

    #[test]
    fn constitution_json_projects_boundaries_with_kinds_and_parameters() {
        let json = constitution_json(&mixed_constitution());
        let doc: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(doc["constitution"], "my-project");
        let boundaries = doc["boundaries"].as_array().expect("array");
        assert_eq!(boundaries.len(), 3);

        // Crate boundary with an allowlist.
        assert_eq!(boundaries[0]["kind"], "crate");
        assert_eq!(boundaries[0]["target"], "my-core");
        assert_eq!(boundaries[0]["rule"], "deny external dependencies");
        assert_eq!(boundaries[0]["severity"], "enforce");
        assert_eq!(boundaries[0]["allowed"][0], "serde");

        // Forbid-dependency-on carries its crate list.
        assert_eq!(boundaries[1]["rule"], "forbid dependency on");
        assert_eq!(boundaries[1]["crates"][0], "my-adapters");

        // Module boundary: target is the module path (report convention), plus crate
        // and forbidden import.
        assert_eq!(boundaries[2]["kind"], "module");
        assert_eq!(boundaries[2]["target"], "crate::domain");
        assert_eq!(boundaries[2]["crate"], "my-app");
        assert_eq!(boundaries[2]["forbidden"], "crate::http");
        assert_eq!(boundaries[2]["severity"], "warn");
    }

    #[test]
    fn an_empty_constitution_projects_cleanly() {
        let constitution = Constitution::new("fresh");
        let text = constitution_text(&constitution);
        assert!(
            text.contains("Constitution: fresh  (0 boundaries)"),
            "{text}"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(doc["boundaries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn restrict_to_projects_its_allowlist() {
        let constitution = Constitution::new("p")
            .boundary(
                CrateBoundary::crate_("a")
                    .restrict_dependencies_to(["serde", "types"])
                    .because("a may depend on only serde and types"),
            )
            .boundary(
                CrateBoundary::crate_("b")
                    .restrict_dependencies_to::<[&str; 0], &str>([])
                    .because("b must depend on nothing"),
            );

        let text = constitution_text(&constitution);
        assert!(
            text.contains("restrict dependencies to: serde, types"),
            "{text}"
        );
        assert!(text.contains("restrict dependencies to nothing"), "{text}");

        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(doc["boundaries"][0]["rule"], "restrict dependencies to");
        // A distinct key (`only`, not deny-external's `allowed`) for the closed set.
        assert_eq!(doc["boundaries"][0]["only"][0], "serde");
        assert!(doc["boundaries"][0]["allowed"].is_null());
        // The empty allowlist is still emitted, as `[]`.
        assert_eq!(doc["boundaries"][1]["only"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn module_restrict_imports_to_projects_its_allowlist() {
        let constitution = Constitution::new("p")
            .boundary(
                ModuleBoundary::in_crate("app")
                    .module("crate::kernel")
                    .restrict_imports_to(["crate::types"])
                    .because("the kernel may import only types"),
            )
            .boundary(
                ModuleBoundary::in_crate("app")
                    .module("crate::leaf")
                    .restrict_imports_to::<[&str; 0], &str>([])
                    .because("the leaf may import only its own subtree"),
            );

        let text = constitution_text(&constitution);
        assert!(text.contains("restrict imports to: crate::types"), "{text}");
        assert!(text.contains("restrict imports to nothing"), "{text}");

        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(doc["boundaries"][0]["rule"], "restrict imports to");
        assert_eq!(doc["boundaries"][0]["kind"], "module");
        assert_eq!(doc["boundaries"][0]["target"], "crate::kernel");
        // The closed set uses `only` (the crate-level vocabulary), never `forbidden`.
        assert_eq!(doc["boundaries"][0]["only"][0], "crate::types");
        assert!(doc["boundaries"][0]["forbidden"].is_null());
        // The empty allowlist is still emitted, as `[]`.
        assert_eq!(doc["boundaries"][1]["only"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn an_unreadable_utf8_governed_source_file_is_a_scan_error() {
        // Deterministic on every platform and euid (unlike the permission-based tests,
        // which skip under a privileged user): invalid UTF-8 makes `read_to_string` fail,
        // so a reachable governed-crate source file that cannot be read is a scan error
        // (exit 2), never a silent skip — the core-contract "cannot judge" rule.
        let dir = std::env::temp_dir().join(format!("modou-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("create temp src");
        std::fs::write(src.join("lib.rs"), "pub mod kernel;\n").expect("write lib.rs");
        // 0xFF / 0xFE are not valid UTF-8; read_to_string returns Err on all platforms.
        std::fs::write(src.join("kernel.rs"), [0xFF, 0xFE, 0x00, 0x80]).expect("write kernel.rs");

        let manifest = dir.join("Cargo.toml");
        let metadata = serde_json::json!({
            "packages": [{
                "name": "x",
                "manifest_path": manifest.to_string_lossy().into_owned(),
                "dependencies": [],
            }]
        });
        let boundary = ModuleBoundary::in_crate("x")
            .module("crate::kernel")
            .must_not_import("crate::forbidden")
            .because("kernel must not import forbidden");
        let mut violations = Vec::new();
        let result = check_module_boundary(&metadata, &boundary, &mut violations);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            result.is_err(),
            "an unreadable (invalid-UTF-8) governed source file must be a scan error"
        );
    }

    #[test]
    fn dependency_kind_appears_in_the_projection() {
        let constitution = Constitution::new("p")
            .boundary(
                CrateBoundary::crate_("a")
                    .deny_external_dependencies()
                    .dependency_kind(DependencyKind::Dev)
                    .because("a's dev dependencies stay light"),
            )
            .boundary(
                CrateBoundary::crate_("b")
                    .deny_external_dependencies()
                    .because("b stays light"),
            );
        let text = constitution_text(&constitution);
        assert!(text.contains("(dev dependencies)"), "{text}");
        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(doc["boundaries"][0]["dependency_kind"], "dev");
        // A Normal-kind boundary omits the field entirely (the common projection).
        assert!(doc["boundaries"][1]["dependency_kind"].is_null(), "{doc}");
    }

    #[test]
    fn restrict_workspace_dependencies_to_projects_only_workspace() {
        let constitution = Constitution::new("p")
            .boundary(
                CrateBoundary::crate_("a")
                    .restrict_workspace_dependencies_to(["b"])
                    .because("a may depend on only workspace member b"),
            )
            .boundary(
                CrateBoundary::crate_("c")
                    .forbid_all_workspace_dependencies()
                    .because("c must not depend on any workspace member"),
            );
        let text = constitution_text(&constitution);
        assert!(
            text.contains("restrict workspace dependencies to: b"),
            "{text}"
        );
        assert!(text.contains("forbid all workspace dependencies"), "{text}");
        let doc: serde_json::Value =
            serde_json::from_str(&constitution_json(&constitution)).unwrap();
        assert_eq!(
            doc["boundaries"][0]["rule"],
            "restrict workspace dependencies to"
        );
        // The distinct key (`only_workspace`, not `only`) says which dependency surface
        // is governed — the self-describing distinction with no coverage before now.
        assert_eq!(doc["boundaries"][0]["only_workspace"][0], "b");
        assert!(doc["boundaries"][0]["only"].is_null());
        // The empty allowlist (forbid-all) still emits `only_workspace: []`.
        assert_eq!(
            doc["boundaries"][1]["only_workspace"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
