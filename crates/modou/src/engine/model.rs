use super::*;
use serde_json::Value;

/// The governed shape, declared in Rust (the single source of truth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constitution {
    name: String,
    boundaries: Vec<Boundary>,
}

impl Constitution {
    /// Begin a constitution for a project (the name is a label, not a path).
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            boundaries: Vec::new(),
        }
    }

    /// Add one boundary — a [`CrateBoundary`] or a [`ModuleBoundary`].
    pub fn boundary(mut self, boundary: impl Into<Boundary>) -> Self {
        self.boundaries.push(boundary.into());
        self
    }

    /// The constitution's name (a label, not a path).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The declared boundaries, in declaration order.
    pub fn boundaries(&self) -> &[Boundary] {
        &self.boundaries
    }
}

/// How strongly a boundary reacts. `Enforce` fails the reaction (exit 1); `Warn`
/// reports the violation as advisory without failing — the first rung of adoption,
/// so a dirty project can observe a boundary before enforcing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Severity {
    /// Violations fail the reaction (exit 1). The default.
    #[default]
    Enforce,
    /// Violations are reported as advisory but do not fail — the first rung of
    /// adoption, observed before a boundary is enforced.
    Warn,
}

/// Which dependency table a crate rule observes. Defaults to `Normal`. Mirrors
/// cargo's fixed set (normal / dev / build), so it is intentionally not
/// `#[non_exhaustive]` — unlike [`Rule`], this enum will not grow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DependencyKind {
    /// The normal `[dependencies]` table. The default.
    #[default]
    Normal,
    /// The `[dev-dependencies]` table.
    Dev,
    /// The `[build-dependencies]` table.
    Build,
}

/// One boundary, of either kind. Named `Boundary` (umbrella) with the crate kind as
/// [`CrateBoundary`]: now that a module reaction exists, the v0.1 rename is earned
/// (drift law D2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Boundary {
    /// A rule on a crate target, observed via `cargo metadata`.
    Crate(CrateBoundary),
    /// A rule on an intra-crate module, observed from source `use` declarations.
    Module(ModuleBoundary),
}

impl From<CrateBoundary> for Boundary {
    fn from(boundary: CrateBoundary) -> Self {
        Boundary::Crate(boundary)
    }
}

impl From<ModuleBoundary> for Boundary {
    fn from(boundary: ModuleBoundary) -> Self {
        Boundary::Module(boundary)
    }
}

/// A boundary attached to one crate target, with a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateBoundary {
    pub(crate) target: CrateTarget,
    pub(crate) rule: Rule,
    pub(crate) reason: String,
    pub(crate) severity: Severity,
    pub(crate) kind: DependencyKind,
}

impl CrateBoundary {
    /// Begin a crate boundary for the crate named `package`.
    pub fn crate_(package: &str) -> CrateBoundaryBuilder {
        CrateBoundaryBuilder {
            target: CrateTarget {
                package: package.to_string(),
            },
        }
    }

    /// The crate this boundary governs.
    pub fn target(&self) -> &CrateTarget {
        &self.target
    }

    /// The rule the boundary enforces.
    pub fn rule(&self) -> &Rule {
        &self.rule
    }

    /// The human-readable reason recorded with the boundary (the repair hint).
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The boundary's severity (`enforce` or `warn`).
    pub fn severity(&self) -> Severity {
        self.severity
    }

    /// The dependency table this boundary observes (`Normal` by default).
    pub fn dependency_kind(&self) -> DependencyKind {
        self.kind
    }
}

/// A crate identified by its package name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateTarget {
    /// The crate's package name, as it appears in `cargo metadata`.
    pub package: String,
}

/// What a crate boundary forbids. Each variant is a reaction with an observation
/// source in `cargo metadata`; no variant is named for a reaction that does not
/// exist.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Rule {
    /// Deny external (registry/git) dependencies, except any named in `allowed`.
    DenyExternalDependencies {
        /// External crate names permitted despite the deny rule.
        allowed: Vec<String>,
    },
    /// Forbid a normal dependency on any of these crates (external or internal).
    ForbidDependencyOn {
        /// The forbidden crate names.
        crates: Vec<String>,
    },
    /// Restrict normal dependencies to a closed allowlist: any normal dependency
    /// (external or internal) whose name is not in `allowed` is a violation. An
    /// empty allowlist forbids every normal dependency.
    RestrictDependenciesTo {
        /// The closed allowlist of permitted dependency names.
        allowed: Vec<String>,
    },
    /// Restrict the target's dependencies on *other workspace members* to a closed
    /// allowlist: any normal dependency on a workspace member not in `allowed` is a
    /// violation, while external dependencies are ignored. Workspace membership is
    /// observed from `cargo metadata`, so a newly added member is governed by default.
    /// An empty allowlist forbids every workspace dependency.
    RestrictWorkspaceDependenciesTo {
        /// The closed allowlist of permitted workspace-member names.
        allowed: Vec<String>,
    },
}

impl Rule {
    /// Each crate rule is the single source of truth for its own behavior: its
    /// label, text and JSON projection, and which declared dependencies it flags
    /// (including its observation source). Every method is one exhaustive match, so
    /// adding a variant is a compile error until it is handled everywhere
    /// (see PROJECT.md). The label in particular feeds the violation `rule` string,
    /// the baseline identity, and the projection — one source, no silent divergence.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Rule::DenyExternalDependencies { .. } => "deny external dependencies",
            Rule::ForbidDependencyOn { .. } => "forbid dependency on",
            Rule::RestrictDependenciesTo { .. } => "restrict dependencies to",
            Rule::RestrictWorkspaceDependenciesTo { .. } => "restrict workspace dependencies to",
        }
    }

    /// The human-readable rule text with its parameters, for the text projection.
    pub(crate) fn text(&self) -> String {
        match self {
            Rule::DenyExternalDependencies { allowed } if allowed.is_empty() => {
                "deny external dependencies".to_string()
            }
            Rule::DenyExternalDependencies { allowed } => {
                format!("deny external dependencies (allow: {})", allowed.join(", "))
            }
            Rule::ForbidDependencyOn { crates } => {
                format!("forbid dependency on: {}", crates.join(", "))
            }
            Rule::RestrictDependenciesTo { allowed } if allowed.is_empty() => {
                "restrict dependencies to nothing".to_string()
            }
            Rule::RestrictDependenciesTo { allowed } => {
                format!("restrict dependencies to: {}", allowed.join(", "))
            }
            Rule::RestrictWorkspaceDependenciesTo { allowed } if allowed.is_empty() => {
                "forbid all workspace dependencies".to_string()
            }
            Rule::RestrictWorkspaceDependenciesTo { allowed } => {
                format!("restrict workspace dependencies to: {}", allowed.join(", "))
            }
        }
    }

    /// The JSON parameter fields for the projection. Deny-external's `allowed` is an
    /// optional exception list (emitted only when non-empty); restrict-to's `only` is
    /// the intrinsic closed set (always emitted, as `[]` when empty); forbid lists
    /// its `crates`; the workspace-scoped restrict-to uses `only_workspace`, distinct
    /// from `only` so the projection says which dependency surface it governs.
    pub(crate) fn json_params(&self) -> Vec<(&'static str, Value)> {
        match self {
            Rule::DenyExternalDependencies { allowed } if allowed.is_empty() => Vec::new(),
            Rule::DenyExternalDependencies { allowed } => {
                vec![("allowed", serde_json::json!(allowed))]
            }
            Rule::ForbidDependencyOn { crates } => vec![("crates", serde_json::json!(crates))],
            Rule::RestrictDependenciesTo { allowed } => vec![("only", serde_json::json!(allowed))],
            Rule::RestrictWorkspaceDependenciesTo { allowed } => {
                vec![("only_workspace", serde_json::json!(allowed))]
            }
        }
    }

    /// The target's declared dependencies that violate this rule. Each rule owns both
    /// its observation source (external-only / all normal / workspace-only) and its
    /// filter. `workspace_members` is all workspace member names, observed from
    /// `cargo metadata`; only the workspace-scoped rule consults it. (It includes the
    /// target crate itself, harmlessly: no crate depends on itself.)
    pub(crate) fn findings(
        &self,
        package: &Value,
        workspace_members: &[String],
        kind: DependencyKind,
    ) -> Vec<String> {
        match self {
            Rule::DenyExternalDependencies { allowed } => external_dependencies(package, kind)
                .into_iter()
                .filter(|dependency| !allowed.contains(dependency))
                .collect(),
            Rule::ForbidDependencyOn { crates } => dependencies(package, kind)
                .into_iter()
                .filter(|dependency| crates.contains(dependency))
                .collect(),
            Rule::RestrictDependenciesTo { allowed } => dependencies(package, kind)
                .into_iter()
                .filter(|dependency| !allowed.contains(dependency))
                .collect(),
            Rule::RestrictWorkspaceDependenciesTo { allowed } => dependencies(package, kind)
                .into_iter()
                .filter(|dependency| {
                    workspace_members.contains(dependency) && !allowed.contains(dependency)
                })
                .collect(),
        }
    }
}

/// Fluent builder: `CrateBoundary::crate_("x").deny_external_dependencies().because("…")`
/// or `CrateBoundary::crate_("x").forbid_dependency_on(["y"]).because("…")`.
pub struct CrateBoundaryBuilder {
    target: CrateTarget,
}

impl CrateBoundaryBuilder {
    /// Deny external dependencies. Chain [`DenyExternalDraft::allow_external`] to
    /// name exceptions, and [`DenyExternalDraft::warn`] to make it advisory, before
    /// [`DenyExternalDraft::because`].
    pub fn deny_external_dependencies(self) -> DenyExternalDraft {
        DenyExternalDraft {
            target: self.target,
            allowed: Vec::new(),
            severity: Severity::Enforce,
            kind: DependencyKind::Normal,
        }
    }

    /// Forbid a normal dependency on any of `crates`, whether it resolves to an
    /// external source or to an internal workspace path (crate-to-crate layering).
    pub fn forbid_dependency_on<I, S>(self, crates: I) -> CrateBoundaryDraft
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CrateBoundaryDraft {
            target: self.target,
            rule: Rule::ForbidDependencyOn {
                crates: crates.into_iter().map(Into::into).collect(),
            },
            severity: Severity::Enforce,
            kind: DependencyKind::Normal,
        }
    }

    /// Restrict this crate's normal dependencies to a closed allowlist: any normal
    /// dependency (external or internal) not named in `allowed` is a violation. An
    /// empty allowlist forbids every normal dependency.
    pub fn restrict_dependencies_to<I, S>(self, allowed: I) -> CrateBoundaryDraft
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CrateBoundaryDraft {
            target: self.target,
            rule: Rule::RestrictDependenciesTo {
                allowed: allowed.into_iter().map(Into::into).collect(),
            },
            severity: Severity::Enforce,
            kind: DependencyKind::Normal,
        }
    }

    /// Restrict this crate's dependencies on *other workspace members* to a closed
    /// allowlist: any normal dependency on a workspace member not named in `allowed`
    /// is a violation; external dependencies are ignored. Workspace members are
    /// derived from `cargo metadata`, so a newly added member is governed by default.
    /// Unlike [`restrict_dependencies_to`](Self::restrict_dependencies_to), which
    /// governs *all* normal dependencies (external included), this governs only the
    /// workspace surface.
    pub fn restrict_workspace_dependencies_to<I, S>(self, allowed: I) -> CrateBoundaryDraft
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        CrateBoundaryDraft {
            target: self.target,
            rule: Rule::RestrictWorkspaceDependenciesTo {
                allowed: allowed.into_iter().map(Into::into).collect(),
            },
            severity: Severity::Enforce,
            kind: DependencyKind::Normal,
        }
    }

    /// Forbid this crate from depending on *any* other workspace member — the
    /// empty-allowlist shorthand for
    /// [`restrict_workspace_dependencies_to`](Self::restrict_workspace_dependencies_to).
    pub fn forbid_all_workspace_dependencies(self) -> CrateBoundaryDraft {
        self.restrict_workspace_dependencies_to(Vec::<String>::new())
    }
}

/// A deny-external boundary awaiting an optional allowlist, severity, and reason.
pub struct DenyExternalDraft {
    target: CrateTarget,
    allowed: Vec<String>,
    severity: Severity,
    kind: DependencyKind,
}

impl DenyExternalDraft {
    /// Allow these external dependencies as named exceptions to the deny rule.
    pub fn allow_external<I, S>(mut self, crates: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.allowed.extend(crates.into_iter().map(Into::into));
        self
    }

    /// Make this boundary advisory: its violations are reported but do not fail CI.
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warn;
        self
    }

    /// Observe a different dependency table (`Dev` or `Build`); the default is `Normal`.
    pub fn dependency_kind(mut self, kind: DependencyKind) -> Self {
        self.kind = kind;
        self
    }

    /// Finish the boundary, recording the human-readable `reason` (the repair hint).
    pub fn because(self, reason: &str) -> CrateBoundary {
        CrateBoundary {
            target: self.target,
            rule: Rule::DenyExternalDependencies {
                allowed: self.allowed,
            },
            reason: reason.to_string(),
            severity: self.severity,
            kind: self.kind,
        }
    }
}

/// A crate boundary awaiting its severity and reason.
pub struct CrateBoundaryDraft {
    target: CrateTarget,
    rule: Rule,
    severity: Severity,
    kind: DependencyKind,
}

impl CrateBoundaryDraft {
    /// Make this boundary advisory: its violations are reported but do not fail CI.
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warn;
        self
    }

    /// Observe a different dependency table (`Dev` or `Build`); the default is `Normal`.
    pub fn dependency_kind(mut self, kind: DependencyKind) -> Self {
        self.kind = kind;
        self
    }

    /// Finish the boundary, recording the human-readable `reason` (the repair hint).
    pub fn because(self, reason: &str) -> CrateBoundary {
        CrateBoundary {
            target: self.target,
            rule: self.rule,
            reason: reason.to_string(),
            severity: self.severity,
            kind: self.kind,
        }
    }
}

/// A boundary over the intra-crate module import graph — the layering Cargo cannot
/// see. Observed from the target crate's source `use` declarations (PROJECT.md).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleBoundary {
    pub(crate) crate_package: String,
    pub(crate) module: String,
    pub(crate) rule: ModuleRule,
    pub(crate) reason: String,
    pub(crate) severity: Severity,
}

impl ModuleBoundary {
    /// Begin a module boundary within the crate named `package`.
    pub fn in_crate(package: &str) -> ModuleBoundaryBuilder {
        ModuleBoundaryBuilder {
            crate_package: package.to_string(),
        }
    }
}

/// What a module boundary forbids.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModuleRule {
    /// The governed module must not import this module (or anything beneath it).
    MustNotImport {
        /// The forbidden module path (e.g. `"crate::projection"`).
        module: String,
    },
    /// The governed module may import only these modules (each "or beneath"), plus its
    /// own subtree; any other internal import is a violation. An empty allowlist permits
    /// only the module's own subtree.
    RestrictImportsTo {
        /// The closed allowlist of importable module paths (e.g. `["crate::types"]`).
        allowed: Vec<String>,
    },
    /// The governed (protected) module must not be imported by this module (or anything
    /// beneath it) — an inbound encapsulation rule, the mirror of `MustNotImport`. A
    /// module within the protected module's own subtree is never an importer.
    MustNotBeImportedBy {
        /// The forbidden importer module path (e.g. `"crate::http"`).
        importer: String,
    },
}

impl ModuleRule {
    /// The label feeding the violation `rule` string and the projection — one source.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            ModuleRule::MustNotImport { .. } => "module must not import",
            ModuleRule::RestrictImportsTo { .. } => "restrict imports to",
            ModuleRule::MustNotBeImportedBy { .. } => "module must not be imported by",
        }
    }

    /// The human-readable rule text with its parameter, for the text projection.
    pub(crate) fn text(&self) -> String {
        match self {
            ModuleRule::MustNotImport { module } => format!("must not import {module}"),
            ModuleRule::RestrictImportsTo { allowed } if allowed.is_empty() => {
                "restrict imports to nothing".to_string()
            }
            ModuleRule::RestrictImportsTo { allowed } => {
                format!("restrict imports to: {}", allowed.join(", "))
            }
            ModuleRule::MustNotBeImportedBy { importer } => {
                format!("must not be imported by {importer}")
            }
        }
    }

    /// The JSON parameter fields for the projection. `must_not_import` names its single
    /// `forbidden` path; `restrict_imports_to` emits its closed set as `only` (always,
    /// as `[]` when empty), matching the crate-level restrict-to vocabulary;
    /// `must_not_be_imported_by` names its declared forbidden `importer`.
    pub(crate) fn json_params(&self) -> Vec<(&'static str, Value)> {
        match self {
            ModuleRule::MustNotImport { module } => {
                vec![("forbidden", serde_json::json!(module))]
            }
            ModuleRule::RestrictImportsTo { allowed } => {
                vec![("only", serde_json::json!(allowed))]
            }
            ModuleRule::MustNotBeImportedBy { importer } => {
                vec![("importer", serde_json::json!(importer))]
            }
        }
    }
}

/// Fluent builder for a [`ModuleBoundary`].
pub struct ModuleBoundaryBuilder {
    crate_package: String,
}

impl ModuleBoundaryBuilder {
    /// The module whose imports are governed (e.g. `"crate::kernel"`).
    pub fn module(self, module: &str) -> ModuleTargetDraft {
        ModuleTargetDraft {
            crate_package: self.crate_package,
            module: module.to_string(),
        }
    }
}

/// A module boundary awaiting its module rule.
pub struct ModuleTargetDraft {
    crate_package: String,
    module: String,
}

impl ModuleTargetDraft {
    /// Forbid the governed module from importing `module` (or anything beneath it).
    pub fn must_not_import(self, module: &str) -> ModuleBoundaryDraft {
        self.with_rule(ModuleRule::MustNotImport {
            module: module.to_string(),
        })
    }

    /// Restrict the governed module's internal imports to a closed allowlist: any
    /// internal `use` reaching a module that is neither within the governed module's
    /// own subtree nor within an allowlist entry (each matched "or beneath") is a
    /// violation. An empty allowlist permits only the module's own subtree. Governs
    /// new internal modules by default, the module-level mirror of the crate-level
    /// [`restrict_dependencies_to`](CrateBoundaryBuilder::restrict_dependencies_to).
    pub fn restrict_imports_to<I, S>(self, allowed: I) -> ModuleBoundaryDraft
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.with_rule(ModuleRule::RestrictImportsTo {
            allowed: allowed.into_iter().map(Into::into).collect(),
        })
    }

    /// Forbid the governed (protected) module from being imported by `importer` (or
    /// anything beneath it) — an inbound encapsulation rule, the mirror of
    /// [`must_not_import`](Self::must_not_import). A module within the protected module's
    /// own subtree is never treated as an importer.
    pub fn must_not_be_imported_by(self, importer: &str) -> ModuleBoundaryDraft {
        self.with_rule(ModuleRule::MustNotBeImportedBy {
            importer: importer.to_string(),
        })
    }

    fn with_rule(self, rule: ModuleRule) -> ModuleBoundaryDraft {
        ModuleBoundaryDraft {
            crate_package: self.crate_package,
            module: self.module,
            rule,
            severity: Severity::Enforce,
        }
    }
}

/// A module boundary awaiting its severity and reason.
pub struct ModuleBoundaryDraft {
    crate_package: String,
    module: String,
    rule: ModuleRule,
    severity: Severity,
}

impl ModuleBoundaryDraft {
    /// Make this boundary advisory: its violations are reported but do not fail CI.
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warn;
        self
    }

    /// Finish the boundary, recording the human-readable `reason` (the repair hint).
    pub fn because(self, reason: &str) -> ModuleBoundary {
        ModuleBoundary {
            crate_package: self.crate_package,
            module: self.module,
            rule: self.rule,
            reason: reason.to_string(),
            severity: self.severity,
        }
    }
}
