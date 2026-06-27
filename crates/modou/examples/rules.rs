//! The rule catalog as a *compiled* example — every boundary form the README
//! documents, declared once so CI (`cargo clippy --all-targets`) type-checks the
//! public API and the README's rule snippets cannot silently rot.
//!
//! This is a reference, not a runnable check: it targets illustrative crate and
//! module names, so running it against a real workspace reports a constitution error.
//! For the minimal adoption one-liner, see `adoption.rs`.

use modou::prelude::*;

fn constitution() -> Constitution {
    Constitution::new("rule-catalog")
        // Deny external (registry/git) dependencies, with named exceptions; `.warn()`
        // makes it advisory (reported, never fails CI).
        .boundary(
            CrateBoundary::crate_("core")
                .deny_external_dependencies()
                .allow_external(["serde"])
                .warn()
                .because("core may use serde; observe other externals before enforcing"),
        )
        // Forbid a dependency on named crates (external or an internal workspace path).
        .boundary(
            CrateBoundary::crate_("core")
                .forbid_dependency_on(["adapters"])
                .because("core must not depend on adapters"),
        )
        // Restrict *all* normal dependencies to a closed allowlist (external included).
        .boundary(
            CrateBoundary::crate_("domain")
                .restrict_dependencies_to(["serde", "domain-types"])
                .because("the domain may depend on only serde and its own types"),
        )
        // Restrict only *workspace* dependencies (members from cargo metadata); a new
        // workspace crate is governed by default.
        .boundary(
            CrateBoundary::crate_("backend")
                .restrict_workspace_dependencies_to(["core"])
                .because("a backend may depend on only the core workspace crate"),
        )
        // Forbid every workspace dependency — the empty-allowlist shorthand.
        .boundary(
            CrateBoundary::crate_("leaf")
                .forbid_all_workspace_dependencies()
                .because("leaf must not depend on any other workspace crate"),
        )
        // Point a rule at the dev-dependency table instead of the normal one.
        .boundary(
            CrateBoundary::crate_("backend")
                .forbid_dependency_on(["other-backend"])
                .dependency_kind(DependencyKind::Dev)
                .because("a backend may not pull another backend in as a dev-dependency"),
        )
        // A module boundary over the intra-crate import graph Cargo cannot see:
        // forbid one module from importing another.
        .boundary(
            ModuleBoundary::in_crate("app")
                .module("crate::kernel")
                .must_not_import("crate::projection")
                .because("the kernel must not depend on a projection"),
        )
        // Restrict a module's imports to a closed allowlist (everything else is a
        // violation, so a newly added internal module is governed by default).
        .boundary(
            ModuleBoundary::in_crate("app")
                .module("crate::kernel")
                .restrict_imports_to(["crate::types"])
                .because("the kernel may reach only the shared types module"),
        )
        // Forbid a module from being imported by another — the inbound direction
        // (encapsulation: who may reach *in*).
        .boundary(
            ModuleBoundary::in_crate("app")
                .module("crate::internal")
                .must_not_be_imported_by("crate::http")
                .because("internal is private to the core; the http layer must not reach in"),
        )
}

fn main() -> std::process::ExitCode {
    modou::run(&constitution(), std::env::args())
}
