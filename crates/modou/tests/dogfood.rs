//! Black-box integration tests: Modou checks in-repo fixture workspaces through the
//! public API (`check` / the constitution builders). White-box unit tests for the
//! crate-private machinery (baseline, projections, scanner) live in `src/engine.rs`.
//! Self-contained: no external directory is referenced.

use std::path::PathBuf;

use modou::prelude::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .join("Cargo.toml")
}

fn example_constitution() -> Constitution {
    Constitution::new("example").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .because("example-core must stay dependency-light"),
    )
}

#[test]
fn boundary_is_declared_in_rust() {
    let constitution = example_constitution();
    assert_eq!(constitution.boundaries().len(), 1);
    match &constitution.boundaries()[0] {
        Boundary::Crate(boundary) => {
            assert_eq!(boundary.target().package, "example-core");
            assert!(!boundary.reason().is_empty());
        }
        other => panic!("expected a crate boundary, got {other:?}"),
    }
}

#[test]
fn clean_fixture_passes() {
    let outcome = check(&example_constitution(), &fixture("clean"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "expected clean, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn violating_fixture_fails_and_names_the_dependency() {
    let outcome = check(&example_constitution(), &fixture("violating"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "serde"),
            "expected serde to be named, got {report:?}"
        ),
        other => panic!("expected violations, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn unresolvable_target_is_a_constitution_error() {
    let constitution = Constitution::new("example").boundary(
        CrateBoundary::crate_("does-not-exist")
            .deny_external_dependencies()
            .because("absent on purpose"),
    );
    let outcome = check(&constitution, &fixture("clean"));
    assert!(
        matches!(outcome, Outcome::ConstitutionError(_)),
        "expected constitution error, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 2);
}

#[test]
fn multiple_boundaries_aggregate_their_violations() {
    let constitution = Constitution::new("multi")
        .boundary(
            CrateBoundary::crate_("core-a")
                .deny_external_dependencies()
                .because("core-a must stay dependency-light"),
        )
        .boundary(
            CrateBoundary::crate_("core-b")
                .deny_external_dependencies()
                .because("core-b must stay dependency-light"),
        );
    let outcome = check(&constitution, &fixture("violating-multi"));
    match &outcome {
        Outcome::Violations(report) => {
            let targets: Vec<&str> = report
                .violations
                .iter()
                .map(|v| v.target.as_str())
                .collect();
            assert!(
                targets.contains(&"core-a") && targets.contains(&"core-b"),
                "expected a violation for each crate, got {report:?}"
            );
        }
        other => panic!("expected aggregated violations, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn constitution_error_supersedes_a_violation() {
    // One boundary is violated (example-core declares serde); another targets a
    // crate that does not exist. The unresolvable target supersedes: exit 2, not 1.
    let constitution = Constitution::new("supersede")
        .boundary(
            CrateBoundary::crate_("example-core")
                .deny_external_dependencies()
                .because("example-core must stay dependency-light"),
        )
        .boundary(
            CrateBoundary::crate_("does-not-exist")
                .deny_external_dependencies()
                .because("absent on purpose"),
        );
    let outcome = check(&constitution, &fixture("violating"));
    assert!(
        matches!(outcome, Outcome::ConstitutionError(_)),
        "a misconfiguration must supersede a violation, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 2);
}

#[test]
fn supersede_holds_regardless_of_boundary_order() {
    // Same two boundaries as above, declared unresolvable-first: still exit 2.
    let constitution = Constitution::new("supersede-reordered")
        .boundary(
            CrateBoundary::crate_("does-not-exist")
                .deny_external_dependencies()
                .because("absent on purpose"),
        )
        .boundary(
            CrateBoundary::crate_("example-core")
                .deny_external_dependencies()
                .because("example-core must stay dependency-light"),
        );
    let outcome = check(&constitution, &fixture("violating"));
    assert_eq!(
        outcome.exit_code(),
        2,
        "order must not change the outcome class"
    );
}

#[test]
fn allow_external_permits_the_listed_dependency() {
    let constitution = Constitution::new("allow").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .allow_external(["serde"])
            .because("example-core may use serde, nothing else external"),
    );
    let outcome = check(&constitution, &fixture("violating"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "an allowlisted external dep must not violate, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn allow_external_does_not_permit_others() {
    let constitution = Constitution::new("allow-other").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .allow_external(["thiserror"])
            .because("only thiserror is allowed"),
    );
    let outcome = check(&constitution, &fixture("violating"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "serde"),
            "serde is not allowlisted and must still violate, got {report:?}"
        ),
        other => panic!("expected a violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn forbid_dependency_on_external_crate_violates() {
    let constitution = Constitution::new("forbid-external").boundary(
        CrateBoundary::crate_("example-core")
            .forbid_dependency_on(["serde"])
            .because("example-core must not depend on serde"),
    );
    let outcome = check(&constitution, &fixture("violating"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "serde"),
            "expected serde to be named, got {report:?}"
        ),
        other => panic!("expected a violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn forbid_dependency_on_internal_crate_catches_layering() {
    // `core` path-depends on `adapters`. The forbid rule catches this internal
    // layering violation; the external rule (below) does not even see it.
    let forbids = Constitution::new("layering").boundary(
        CrateBoundary::crate_("core")
            .forbid_dependency_on(["adapters"])
            .because("core must not depend on adapters"),
    );
    let outcome = check(&forbids, &fixture("layered"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "adapters"),
            "expected adapters to be named, got {report:?}"
        ),
        other => panic!("expected a layering violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);

    // Contrast: the external rule ignores the internal path dependency.
    let external = Constitution::new("external-only").boundary(
        CrateBoundary::crate_("core")
            .deny_external_dependencies()
            .because("core must stay free of external deps"),
    );
    assert!(
        matches!(check(&external, &fixture("layered")), Outcome::Clean),
        "the external rule must not flag an internal path dependency"
    );
}

#[test]
fn forbid_dependency_on_unused_crate_is_clean() {
    let constitution = Constitution::new("forbid-absent").boundary(
        CrateBoundary::crate_("core")
            .forbid_dependency_on(["tokio"])
            .because("core must not depend on tokio"),
    );
    let outcome = check(&constitution, &fixture("layered"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "forbidding an undepended crate must be clean, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn warn_violation_is_reported_but_does_not_fail() {
    let constitution = Constitution::new("advisory").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .warn()
            .because("observe example-core before enforcing"),
    );
    let outcome = check(&constitution, &fixture("violating"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "serde"),
            "a warn violation must still be reported, got {report:?}"
        ),
        other => panic!("expected the violation to be reported, got {other:?}"),
    }
    assert_eq!(
        outcome.exit_code(),
        0,
        "a warn-only run must not fail the reaction"
    );
}

#[test]
fn an_enforce_violation_fails_even_alongside_a_warn() {
    let constitution = Constitution::new("mixed")
        .boundary(
            CrateBoundary::crate_("core-a")
                .deny_external_dependencies()
                .because("core-a is enforced"),
        )
        .boundary(
            CrateBoundary::crate_("core-b")
                .deny_external_dependencies()
                .warn()
                .because("core-b is only observed"),
        );
    let outcome = check(&constitution, &fixture("violating-multi"));
    match &outcome {
        Outcome::Violations(report) => assert_eq!(
            report.violations.len(),
            2,
            "both boundaries are reported, got {report:?}"
        ),
        other => panic!("expected violations, got {other:?}"),
    }
    assert_eq!(
        outcome.exit_code(),
        1,
        "an enforce violation must fail regardless of warn boundaries"
    );
}

#[test]
fn boundary_defaults_to_enforce() {
    let constitution = example_constitution();
    match &constitution.boundaries()[0] {
        Boundary::Crate(boundary) => assert_eq!(
            boundary.severity(),
            Severity::Enforce,
            "a boundary with no explicit severity must default to enforce"
        ),
        other => panic!("expected a crate boundary, got {other:?}"),
    }
    assert_eq!(check(&constitution, &fixture("violating")).exit_code(), 1);
}

#[test]
fn module_import_violation_is_detected() {
    let constitution = Constitution::new("layering").boundary(
        ModuleBoundary::in_crate("app")
            .module("crate::kernel")
            .must_not_import("crate::projection")
            .because("the kernel must not depend on a projection"),
    );
    let outcome = check(&constitution, &fixture("module-layered"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report
                .violations
                .iter()
                .any(|v| v.kind == BoundaryKind::Module
                    && v.target == "crate::kernel"
                    && v.finding.starts_with("crate::projection")),
            "expected a kernel->projection module violation, got {report:?}"
        ),
        other => panic!("expected a violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn module_allowed_direction_is_clean() {
    let constitution = Constitution::new("layering").boundary(
        ModuleBoundary::in_crate("app")
            .module("crate::projection")
            .must_not_import("crate::kernel")
            .because("a projection may not import the kernel"),
    );
    let outcome = check(&constitution, &fixture("module-layered"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "projection does not import kernel, got {outcome:?}"
    );
}

#[test]
fn module_boundary_supports_warn() {
    let constitution = Constitution::new("advisory").boundary(
        ModuleBoundary::in_crate("app")
            .module("crate::kernel")
            .must_not_import("crate::projection")
            .warn()
            .because("observe the kernel layering before enforcing"),
    );
    let outcome = check(&constitution, &fixture("module-layered"));
    assert!(
        matches!(outcome, Outcome::Violations(_)),
        "a warn violation is still reported, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0, "warn must not fail the reaction");
}

#[test]
fn unknown_module_is_a_constitution_error() {
    let constitution = Constitution::new("typo").boundary(
        ModuleBoundary::in_crate("app")
            .module("crate::ghost")
            .must_not_import("crate::projection")
            .because("module does not exist"),
    );
    let outcome = check(&constitution, &fixture("module-layered"));
    assert!(
        matches!(outcome, Outcome::ConstitutionError(_)),
        "a module matching no source file must be a constitution error, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 2);
}

#[test]
fn an_undeclared_orphan_root_file_is_not_a_false_module_violation() {
    // Regression (root-module detection): the crate root does a bare external
    // `use serde::…`, and an orphan `src/serde.rs` exists that no `mod serde;` declares.
    // The bare use is external, so a `must_not_import("crate::serde")` boundary must stay
    // clean — the scanner must not mis-resolve the import to the internal `crate::serde::…`
    // just because a like-named file exists. Crate-root modules come from `mod`
    // declarations, not filenames.
    let constitution = Constitution::new("orphan-guard").boundary(
        ModuleBoundary::in_crate("orphan")
            .module("crate")
            .must_not_import("crate::serde")
            .because("nothing in orphan may import a crate::serde module"),
    );
    let outcome = check(&constitution, &fixture("module-orphan"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "an undeclared orphan file must not create a module-boundary false positive, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn a_use_inside_a_macro_rules_body_is_not_a_false_module_violation() {
    // Regression (macro-generated imports out of scope): the fixture's only
    // `use crate::ghost::…` lives inside an uninvoked `macro_rules!` body. The scanner
    // must not observe it, so a `must_not_import("crate::ghost")` boundary stays clean.
    let constitution = Constitution::new("macro-guard").boundary(
        ModuleBoundary::in_crate("macrodemo")
            .module("crate")
            .must_not_import("crate::ghost")
            .because("a use inside a macro_rules body is macro-generated and out of scope"),
    );
    let outcome = check(&constitution, &fixture("module-macro"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "a use inside a macro_rules body must not create a module-boundary false positive, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn a_use_inside_a_macro_invocation_body_is_not_a_false_module_violation() {
    // Regression: the fixture's only `use crate::ghost::…` is inside a macro INVOCATION
    // body — a macro-generated import, out of scope — so the boundary stays clean.
    let constitution = Constitution::new("macro-invoke-guard").boundary(
        ModuleBoundary::in_crate("macroinvoke")
            .module("crate")
            .must_not_import("crate::ghost")
            .because("a use inside a macro invocation is macro-generated and out of scope"),
    );
    let outcome = check(&constitution, &fixture("module-macro-invoke"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "a use inside a macro invocation must not create a false positive, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn an_undeclared_orphan_submodule_file_is_not_a_false_module_violation() {
    // Regression: src/kernel/orphan.rs is an undeclared submodule (kernel.rs has no
    // `mod orphan;`), so Rust never compiles it. Its `use crate::ghost::Thing;` must not
    // be observed — only files reachable via `mod` declarations are governed.
    let constitution = Constitution::new("orphan-sub-guard").boundary(
        ModuleBoundary::in_crate("orphansub")
            .module("crate::kernel")
            .must_not_import("crate::ghost")
            .because("an undeclared orphan submodule file is not compiled and not governed"),
    );
    let outcome = check(&constitution, &fixture("module-orphan-sub"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "an undeclared orphan submodule file must not create a false positive, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn restrict_to_flags_a_dependency_outside_the_allowlist() {
    for allowed in [vec![], vec!["other"]] {
        let constitution = Constitution::new("restrict").boundary(
            CrateBoundary::crate_("example-core")
                .restrict_dependencies_to(allowed.clone())
                .because("example-core may depend on only the allowlist"),
        );
        let outcome = check(&constitution, &fixture("violating"));
        match &outcome {
            Outcome::Violations(report) => assert!(
                report
                    .violations
                    .iter()
                    .any(|v| v.finding == "serde" && v.rule == "restrict dependencies to"),
                "serde is outside allowlist {allowed:?}, got {report:?}"
            ),
            other => panic!("expected a violation for allowlist {allowed:?}, got {other:?}"),
        }
        assert_eq!(outcome.exit_code(), 1);
    }
}

#[test]
fn restrict_to_is_clean_when_the_allowlist_covers_the_dependency() {
    let constitution = Constitution::new("restrict").boundary(
        CrateBoundary::crate_("example-core")
            .restrict_dependencies_to(["serde"])
            .because("example-core may depend on only serde"),
    );
    let outcome = check(&constitution, &fixture("violating"));
    assert!(
        matches!(outcome, Outcome::Clean),
        "serde is allowlisted, got {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}

#[test]
fn restrict_to_catches_an_internal_path_dependency() {
    // `core` path-depends on `adapters`; an empty allowlist forbids it (the
    // deny-external rule would not even see an internal dep).
    let forbids = Constitution::new("restrict-internal").boundary(
        CrateBoundary::crate_("core")
            .restrict_dependencies_to::<[&str; 0], &str>([])
            .because("core must depend on nothing"),
    );
    let outcome = check(&forbids, &fixture("layered"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "adapters"),
            "expected adapters to be named, got {report:?}"
        ),
        other => panic!("expected an internal-dep violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);

    let allows = Constitution::new("restrict-internal-ok").boundary(
        CrateBoundary::crate_("core")
            .restrict_dependencies_to(["adapters"])
            .because("core may depend on only adapters"),
    );
    assert!(
        matches!(check(&allows, &fixture("layered")), Outcome::Clean),
        "adapters is allowlisted"
    );
}

#[test]
fn dependency_kind_switches_the_observed_table() {
    // The default (normal) deny-external rule flags the normal dependency `serde`.
    let normal = Constitution::new("kind-normal").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .because("example-core must have no external normal dependencies"),
    );
    assert!(
        matches!(
            check(&normal, &fixture("violating")),
            Outcome::Violations(_)
        ),
        "the normal rule must flag the serde dependency"
    );

    // A Dev-kind rule observes [dev-dependencies] instead; `violating` has none, so
    // the same crate is clean — the kind selection genuinely switches the table.
    let dev = Constitution::new("kind-dev").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .dependency_kind(DependencyKind::Dev)
            .because("example-core must have no external dev-dependencies"),
    );
    assert!(
        matches!(check(&dev, &fixture("violating")), Outcome::Clean),
        "a Dev-kind rule must ignore the normal serde dependency"
    );
}

#[test]
fn forbid_all_workspace_dependencies_catches_an_internal_member() {
    // `core` path-depends on the workspace member `adapters`. The workspace rule
    // catches it (govern-by-default) — exactly what the deny-external rule ignores.
    let forbids = Constitution::new("forbid-workspace").boundary(
        CrateBoundary::crate_("core")
            .forbid_all_workspace_dependencies()
            .because("core must not depend on any other workspace crate"),
    );
    let outcome = check(&forbids, &fixture("layered"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "adapters"),
            "expected adapters to be named, got {report:?}"
        ),
        other => panic!("expected a workspace-dependency violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}

#[test]
fn restrict_workspace_dependencies_to_respects_the_allowlist() {
    // Allowed: clean. Restricted to something else: the unlisted member is flagged.
    let allows = Constitution::new("restrict-workspace-ok").boundary(
        CrateBoundary::crate_("core")
            .restrict_workspace_dependencies_to(["adapters"])
            .because("core may depend on only the adapters workspace crate"),
    );
    assert!(
        matches!(check(&allows, &fixture("layered")), Outcome::Clean),
        "adapters is on the workspace allowlist"
    );

    let forbids = Constitution::new("restrict-workspace-flag").boundary(
        CrateBoundary::crate_("core")
            .restrict_workspace_dependencies_to(["something-else"])
            .because("core may not depend on adapters"),
    );
    let outcome = check(&forbids, &fixture("layered"));
    match &outcome {
        Outcome::Violations(report) => assert!(
            report.violations.iter().any(|v| v.finding == "adapters"),
            "expected adapters to be flagged as outside the allowlist, got {report:?}"
        ),
        other => panic!("expected a workspace-dependency violation, got {other:?}"),
    }
    assert_eq!(outcome.exit_code(), 1);
}
