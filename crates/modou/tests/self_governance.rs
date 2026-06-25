//! Modou governs itself with its own reaction — the strongest robustness statement a
//! governance tool can make. Its invariants are not prose in PROJECT.md alone; they
//! are declared here as a real constitution ([`modou_constitution`]) and run as a
//! `cargo test` gate, so CI fails the moment the law drifts.

use std::path::PathBuf;

use modou::prelude::*;

/// Modou's own manifest (`crates/modou/Cargo.toml`).
fn modou_manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")
}

/// **Modou's self-constitution — the law Modou enforces on itself.**
///
/// The single, legible source of Modou's own invariants, declared in the same Rust
/// DSL adopters use (see the README adoption section). [`modou_governs_itself`] runs
/// it as a real reaction against Modou's own manifest and source, so the dogfooding
/// is not a hope but a non-bypassable gate. Two invariants:
///
/// 1. **Dependency-light** — `serde_json` is the only external dependency (this is
///    also why the source scanner is hand-rolled instead of pulling in `syn`).
/// 2. **Functional core / imperative shell** — the pure `engine` must not import the
///    side-effecting `runner`.
///
/// A wrong boundary here is fixed by a human-reviewed amendment, never by quietly
/// weakening this function to make CI pass (see PROJECT.md and AGENTS.md).
fn modou_constitution() -> Constitution {
    Constitution::new("modou")
        .boundary(
            CrateBoundary::crate_("modou")
                .restrict_dependencies_to(["serde_json"])
                .because(
                    "Modou stays dependency-light: serde_json is the only external \
                     dependency (no syn / proc-macro2, no heavy graph or runtime crates)",
                ),
        )
        .boundary(
            ModuleBoundary::in_crate("modou")
                .module("crate::engine")
                .must_not_import("crate::runner")
                .because("the functional core must not depend on the imperative shell (FCIS)"),
        )
}

#[test]
fn modou_governs_itself() {
    // The whole self-constitution reacts against Modou's own manifest and source. Any
    // drift — a new external dependency, or the engine importing the runner — surfaces
    // here as a `cargo test` failure, with the offending boundary's reason carried in
    // the outcome as the repair hint.
    let outcome = check(&modou_constitution(), &modou_manifest());
    assert!(
        matches!(outcome, Outcome::Clean),
        "Modou's self-constitution drifted: {outcome:?}"
    );
    assert_eq!(outcome.exit_code(), 0);
}
