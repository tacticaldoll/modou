//! Modou v0.1 — Rust-native reactive governance.
//!
//! **Govern by reaction, not instruction.** A [`Constitution`] declared in Rust is
//! the single source of truth. [`check`] runs its boundaries against a Cargo
//! workspace and returns an [`Outcome`]; the runner turns that into an exit code:
//! `0` clean (warn-only / fully baselined), `1` an enforced violation, `2` a
//! constitution/scan error.
//!
//! Two reaction kinds, each with its own observation source:
//! - [`CrateBoundary`] over `cargo metadata` — deny external dependencies (with an
//!   optional allowlist), forbid a dependency on named crates, or restrict to a
//!   closed allowlist.
//! - [`ModuleBoundary`] over the crate's own source — forbid one module from
//!   importing another (intra-crate layering Cargo cannot see). Observed from `use`
//!   declarations only; file-based modules (see PROJECT.md).
//!
//! Each boundary carries a [`Severity`] (`warn` before `enforce`), and violations
//! can be gated against a baseline. No macros, no TOML/Markdown for the
//! constitution, no universal graph API.
//!
//! This crate root is the **facade**: it re-exports the public surface and is split
//! into the functional core (`engine`) and the imperative shell (`runner`). The core
//! must not depend on the shell — an invariant Modou enforces on itself
//! (`tests/self_governance.rs`).

#![deny(missing_docs)]

mod engine;
mod runner;

pub use engine::{
    Boundary, BoundaryKind, Constitution, CrateBoundary, CrateBoundaryBuilder, CrateBoundaryDraft,
    CrateTarget, DenyExternalDraft, ModuleBoundary, ModuleBoundaryBuilder, ModuleBoundaryDraft,
    ModuleRule, ModuleTargetDraft, Outcome, Report, Rule, Severity, Violation, ViolationId, check,
};
pub use runner::run;

/// The public facade for declaring a constitution and running the reaction. Internal
/// projections, the baseline machinery, and the source scanner are crate-private;
/// consumers go through `check` / `run`.
pub mod prelude {
    pub use super::{
        Boundary, BoundaryKind, Constitution, CrateBoundary, ModuleBoundary, Outcome, Report, Rule,
        Severity, Violation, ViolationId, check, run,
    };
}
