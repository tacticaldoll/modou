//! The functional core (engine): the pure model, the `check` evaluation, the text
//! and JSON projections, the baseline, and the source scanner. The imperative shell
//! — argument parsing, filesystem, stdout/stderr — lives in the sibling `runner`
//! module. This core MUST NOT depend on the shell (functional core, imperative
//! shell), an invariant Modou enforces on itself in `tests/self_governance.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// The governed shape, declared in Rust (the single source of truth).
#[derive(Debug)]
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

/// One boundary, of either kind. Named `Boundary` (umbrella) with the crate kind as
/// [`CrateBoundary`]: now that a module reaction exists, the v0.1 rename is earned
/// (drift law D2).
#[derive(Debug)]
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
#[derive(Debug)]
pub struct CrateBoundary {
    target: CrateTarget,
    rule: Rule,
    reason: String,
    severity: Severity,
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
}

/// A crate identified by its package name.
#[derive(Debug)]
pub struct CrateTarget {
    /// The crate's package name, as it appears in `cargo metadata`.
    pub package: String,
}

/// What a crate boundary forbids. Each variant is a reaction with an observation
/// source in `cargo metadata`; no variant is named for a reaction that does not
/// exist.
#[derive(Debug)]
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
}

impl Rule {
    /// Each crate rule is the single source of truth for its own behavior: its
    /// label, text and JSON projection, and which declared dependencies it flags
    /// (including its observation source). Every method is one exhaustive match, so
    /// adding a variant is a compile error until it is handled everywhere
    /// (see PROJECT.md). The label in particular feeds the violation `rule` string,
    /// the baseline identity, and the projection — one source, no silent divergence.
    fn label(&self) -> &'static str {
        match self {
            Rule::DenyExternalDependencies { .. } => "deny external dependencies",
            Rule::ForbidDependencyOn { .. } => "forbid dependency on",
            Rule::RestrictDependenciesTo { .. } => "restrict dependencies to",
        }
    }

    /// The human-readable rule text with its parameters, for the text projection.
    fn text(&self) -> String {
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
        }
    }

    /// The JSON parameter fields for the projection. Deny-external's `allowed` is an
    /// optional exception list (emitted only when non-empty); restrict-to's `only` is
    /// the intrinsic closed set (always emitted, as `[]` when empty); forbid lists
    /// its `crates`.
    fn json_params(&self) -> Vec<(&'static str, Value)> {
        match self {
            Rule::DenyExternalDependencies { allowed } if allowed.is_empty() => Vec::new(),
            Rule::DenyExternalDependencies { allowed } => {
                vec![("allowed", serde_json::json!(allowed))]
            }
            Rule::ForbidDependencyOn { crates } => vec![("crates", serde_json::json!(crates))],
            Rule::RestrictDependenciesTo { allowed } => vec![("only", serde_json::json!(allowed))],
        }
    }

    /// The target's declared dependencies that violate this rule. Each rule owns both
    /// its observation source (external-only vs all normal deps) and its filter.
    fn findings(&self, package: &Value) -> Vec<String> {
        match self {
            Rule::DenyExternalDependencies { allowed } => external_normal_dependencies(package)
                .into_iter()
                .filter(|dependency| !allowed.contains(dependency))
                .collect(),
            Rule::ForbidDependencyOn { crates } => normal_dependencies(package)
                .into_iter()
                .filter(|dependency| crates.contains(dependency))
                .collect(),
            Rule::RestrictDependenciesTo { allowed } => normal_dependencies(package)
                .into_iter()
                .filter(|dependency| !allowed.contains(dependency))
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
        }
    }
}

/// A deny-external boundary awaiting an optional allowlist, severity, and reason.
pub struct DenyExternalDraft {
    target: CrateTarget,
    allowed: Vec<String>,
    severity: Severity,
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

    /// Finish the boundary, recording the human-readable `reason` (the repair hint).
    pub fn because(self, reason: &str) -> CrateBoundary {
        CrateBoundary {
            target: self.target,
            rule: Rule::DenyExternalDependencies {
                allowed: self.allowed,
            },
            reason: reason.to_string(),
            severity: self.severity,
        }
    }
}

/// A crate boundary awaiting its severity and reason.
pub struct CrateBoundaryDraft {
    target: CrateTarget,
    rule: Rule,
    severity: Severity,
}

impl CrateBoundaryDraft {
    /// Make this boundary advisory: its violations are reported but do not fail CI.
    pub fn warn(mut self) -> Self {
        self.severity = Severity::Warn;
        self
    }

    /// Finish the boundary, recording the human-readable `reason` (the repair hint).
    pub fn because(self, reason: &str) -> CrateBoundary {
        CrateBoundary {
            target: self.target,
            rule: self.rule,
            reason: reason.to_string(),
            severity: self.severity,
        }
    }
}

/// A boundary over the intra-crate module import graph — the layering Cargo cannot
/// see. Observed from the target crate's source `use` declarations (PROJECT.md).
#[derive(Debug)]
pub struct ModuleBoundary {
    crate_package: String,
    module: String,
    rule: ModuleRule,
    reason: String,
    severity: Severity,
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
#[derive(Debug)]
#[non_exhaustive]
pub enum ModuleRule {
    /// The governed module must not import this module (or anything beneath it).
    MustNotImport {
        /// The forbidden module path (e.g. `"crate::projection"`).
        module: String,
    },
}

impl ModuleRule {
    /// The label feeding the violation `rule` string and the projection — one source.
    fn label(&self) -> &'static str {
        match self {
            ModuleRule::MustNotImport { .. } => "module must not import",
        }
    }

    /// The human-readable rule text with its parameter, for the text projection.
    fn text(&self) -> String {
        match self {
            ModuleRule::MustNotImport { module } => format!("must not import {module}"),
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

/// A module boundary awaiting the forbidden import.
pub struct ModuleTargetDraft {
    crate_package: String,
    module: String,
}

impl ModuleTargetDraft {
    /// Forbid the governed module from importing `module` (or anything beneath it).
    pub fn must_not_import(self, module: &str) -> ModuleBoundaryDraft {
        ModuleBoundaryDraft {
            crate_package: self.crate_package,
            module: self.module,
            forbidden: module.to_string(),
            severity: Severity::Enforce,
        }
    }
}

/// A module boundary awaiting its severity and reason.
pub struct ModuleBoundaryDraft {
    crate_package: String,
    module: String,
    forbidden: String,
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
            rule: ModuleRule::MustNotImport {
                module: self.forbidden,
            },
            reason: reason.to_string(),
            severity: self.severity,
        }
    }
}

/// Which kind of boundary produced a violation — surfaced in the JSON report so a
/// consumer need not reverse-engineer the rule string. Not part of the baseline
/// identity ([`ViolationId`]), so adding it does not invalidate existing baselines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundaryKind {
    /// The violation came from a crate boundary.
    Crate,
    /// The violation came from a module boundary.
    Module,
}

/// One violated boundary. `severity` is the producing boundary's severity, so the
/// exit-code decision and the report can treat enforce and warn findings apart.
/// `baselined` is set when baseline gating records the violation in a baseline; a
/// baselined violation does not fail the reaction.
#[derive(Debug)]
#[non_exhaustive]
pub struct Violation {
    /// Which kind of boundary produced this violation.
    pub kind: BoundaryKind,
    /// The governed target (crate name, or module path for a module boundary).
    pub target: String,
    /// The rule label that was violated.
    pub rule: String,
    /// The offending finding (e.g. the dependency name, or the imported module path).
    pub finding: String,
    /// The boundary's reason — the repair hint.
    pub reason: String,
    /// The producing boundary's severity.
    pub severity: Severity,
    /// Whether this violation is recorded in the active baseline (so it does not fail).
    pub baselined: bool,
}

impl Violation {
    /// The `(target, rule, finding)` identity used to match against a baseline.
    pub fn id(&self) -> ViolationId {
        ViolationId {
            target: self.target.clone(),
            rule: self.rule.clone(),
            finding: self.finding.clone(),
        }
    }
}

/// All violations from one evaluation.
#[derive(Debug)]
#[non_exhaustive]
pub struct Report {
    /// Every violation found in one evaluation.
    pub violations: Vec<Violation>,
}

/// A violation's identity for baseline matching: `(target, rule, finding)`. Reason
/// and severity are excluded so editing them does not turn a known violation new.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ViolationId {
    /// The governed target (crate name or module path).
    pub target: String,
    /// The violated rule's label.
    pub rule: String,
    /// The offending finding.
    pub finding: String,
}

/// A recorded set of accepted violations — a generated observation snapshot, not
/// policy (see PROJECT.md). The gate fails only on violations absent from it.
#[derive(Debug, Default)]
pub(crate) struct Baseline {
    entries: Vec<ViolationId>,
}

impl Baseline {
    /// Build a baseline from the current report's violations.
    pub fn of(report: &Report) -> Self {
        let mut entries: Vec<ViolationId> = report.violations.iter().map(Violation::id).collect();
        entries.sort();
        entries.dedup();
        Baseline { entries }
    }

    /// Whether this baseline records the given violation's identity.
    pub fn contains(&self, violation: &Violation) -> bool {
        let id = violation.id();
        self.entries.iter().any(|entry| entry == &id)
    }

    /// Baseline entries that match no current violation — stale, safe to remove.
    pub fn stale(&self, report: &Report) -> Vec<&ViolationId> {
        let current: Vec<ViolationId> = report.violations.iter().map(Violation::id).collect();
        self.entries
            .iter()
            .filter(|entry| !current.iter().any(|id| id == *entry))
            .collect()
    }

    /// Serialize to the on-disk JSON form: a `version` and sorted `violations`.
    pub fn to_json(&self) -> String {
        let violations: Vec<Value> = self
            .entries
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "target": entry.target,
                    "rule": entry.rule,
                    "finding": entry.finding,
                })
            })
            .collect();
        let doc = serde_json::json!({ "version": 1, "violations": violations });
        serde_json::to_string_pretty(&doc).expect("baseline JSON is serializable")
    }

    /// Parse from the on-disk JSON form. A malformed document or unknown version is
    /// an error, never a silently empty baseline.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let doc: Value = serde_json::from_str(text).map_err(|err| err.to_string())?;
        match doc["version"].as_i64() {
            Some(1) => {}
            Some(other) => return Err(format!("unsupported baseline version {other}")),
            None => return Err("baseline is missing a numeric `version`".to_string()),
        }
        let array = doc["violations"]
            .as_array()
            .ok_or_else(|| "baseline `violations` must be an array".to_string())?;
        let mut entries = Vec::with_capacity(array.len());
        for item in array {
            let field = |name: &str| -> Result<String, String> {
                item[name]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| format!("baseline entry is missing string `{name}`"))
            };
            entries.push(ViolationId {
                target: field("target")?,
                rule: field("rule")?,
                finding: field("finding")?,
            });
        }
        entries.sort();
        entries.dedup();
        Ok(Baseline { entries })
    }
}

/// Mark each violation the baseline records as `baselined`, so it no longer fails
/// the reaction; violations absent from the baseline are left as new.
pub(crate) fn apply_baseline(report: &mut Report, baseline: &Baseline) {
    for violation in &mut report.violations {
        if baseline.contains(violation) {
            violation.baselined = true;
        }
    }
}

/// The reaction's outcome. Exit codes separate architectural drift (1) from a
/// misconfiguration (2), so a mistyped crate name is not reported as drift.
#[derive(Debug)]
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

/// Render the outcome as a JSON document for machine consumption: a faithful
/// projection of [`Outcome`] with each violation's `kind`, the boundary `reason` as
/// the repair hint, and `exit_code` mirroring the process exit. `stale` lists
/// baseline entries matching no current violation (empty outside gate mode).
pub(crate) fn report_json(outcome: &Outcome, stale: &[ViolationId]) -> String {
    let (label, violations, error) = match outcome {
        Outcome::Clean => ("clean", Vec::new(), Value::Null),
        Outcome::Violations(report) => (
            "violations",
            report.violations.iter().map(violation_json).collect(),
            Value::Null,
        ),
        Outcome::ConstitutionError(message) => (
            "constitution_error",
            Vec::new(),
            Value::String(message.clone()),
        ),
    };
    let stale_baseline: Vec<Value> = stale
        .iter()
        .map(
            |id| serde_json::json!({ "target": id.target, "rule": id.rule, "finding": id.finding }),
        )
        .collect();
    let document = serde_json::json!({
        "outcome": label,
        "exit_code": outcome.exit_code(),
        "violations": violations,
        "stale_baseline": stale_baseline,
        "error": error,
    });
    serde_json::to_string_pretty(&document).expect("report JSON is serializable")
}

fn violation_json(violation: &Violation) -> Value {
    serde_json::json!({
        "kind": match violation.kind {
            BoundaryKind::Crate => "crate",
            BoundaryKind::Module => "module",
        },
        "target": violation.target,
        "rule": violation.rule,
        "finding": violation.finding,
        "reason": violation.reason,
        "severity": match violation.severity {
            Severity::Enforce => "enforce",
            Severity::Warn => "warn",
        },
        "baselined": violation.baselined,
    })
}

/// Render the declared constitution as a human-readable projection — the law as
/// code declares it, for a steward reviewing an amendment or an operator reading a
/// CI log. A projection of the Rust source of truth, never a second source and never
/// a reaction. An empty constitution renders its name and `(0 boundaries)`.
pub(crate) fn constitution_text(constitution: &Constitution) -> String {
    let boundaries = constitution.boundaries();
    let noun = if boundaries.len() == 1 {
        "boundary"
    } else {
        "boundaries"
    };
    let mut out = format!(
        "Constitution: {}  ({} {noun})\n",
        constitution.name(),
        boundaries.len()
    );
    for boundary in boundaries {
        let (severity, target, rule, reason) = match boundary {
            Boundary::Crate(b) => (
                b.severity(),
                format!("crate {}", b.target().package),
                b.rule().text(),
                b.reason(),
            ),
            Boundary::Module(b) => (
                b.severity,
                format!("module {} in {}", b.module, b.crate_package),
                b.rule.text(),
                b.reason.as_str(),
            ),
        };
        out.push_str(&format!(
            "\n[{}] {target}\n  rule:   {rule}\n  reason: {reason}\n",
            severity_label(severity)
        ));
    }
    out
}

/// Render the declared constitution as a JSON projection: a `constitution` name and
/// a `boundaries` array. Each entry carries `kind`, `target` (the crate name, or the
/// module path for a module boundary — the same convention as a violation's
/// `target`), `severity`, `reason`, and the rule with its parameters. No field is
/// invented for data the constitution does not hold.
pub(crate) fn constitution_json(constitution: &Constitution) -> String {
    let boundaries: Vec<Value> = constitution
        .boundaries()
        .iter()
        .map(boundary_json)
        .collect();
    let document = serde_json::json!({
        "constitution": constitution.name(),
        "boundaries": boundaries,
    });
    serde_json::to_string_pretty(&document).expect("constitution JSON is serializable")
}

fn boundary_json(boundary: &Boundary) -> Value {
    match boundary {
        Boundary::Crate(b) => {
            let mut object = serde_json::json!({
                "kind": "crate",
                "target": b.target().package,
                "rule": b.rule().label(),
                "severity": severity_label(b.severity()),
                "reason": b.reason(),
            });
            for (key, value) in b.rule().json_params() {
                object[key] = value;
            }
            object
        }
        Boundary::Module(b) => {
            let ModuleRule::MustNotImport { module } = &b.rule;
            serde_json::json!({
                "kind": "module",
                "target": b.module,
                "crate": b.crate_package,
                "rule": b.rule.label(),
                "severity": severity_label(b.severity),
                "reason": b.reason,
                "forbidden": module,
            })
        }
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Enforce => "enforce",
        Severity::Warn => "warn",
    }
}

/// Run the constitution's boundaries against the Cargo workspace at `manifest_path`.
///
/// The spine is **resolve -> observe -> compare -> react**: resolve each target to
/// a workspace package, observe (its dependencies, or its source imports), compare
/// against the rule, and return the outcome. An unresolvable target (or an
/// unreadable workspace) is a constitution error, never a silent pass.
pub fn check(constitution: &Constitution, manifest_path: &Path) -> Outcome {
    let metadata = match cargo_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return Outcome::ConstitutionError(format!(
                "cannot read target workspace at {}: {err}",
                manifest_path.display()
            ));
        }
    };

    let mut violations = Vec::new();
    for boundary in constitution.boundaries() {
        match boundary {
            Boundary::Crate(crate_boundary) => {
                if let Err(error) = check_crate_boundary(&metadata, crate_boundary, &mut violations)
                {
                    return Outcome::ConstitutionError(error);
                }
            }
            Boundary::Module(module_boundary) => {
                if let Err(error) =
                    check_module_boundary(&metadata, module_boundary, &mut violations)
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

fn check_crate_boundary(
    metadata: &Value,
    boundary: &CrateBoundary,
    violations: &mut Vec<Violation>,
) -> Result<(), String> {
    let package = find_package(metadata, &boundary.target.package).ok_or_else(|| {
        format!(
            "target crate '{}' not found in the workspace",
            boundary.target.package
        )
    })?;

    for finding in boundary.rule.findings(package) {
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
    let package = find_package(metadata, &boundary.crate_package).ok_or_else(|| {
        format!(
            "target crate '{}' not found in the workspace",
            boundary.crate_package
        )
    })?;
    let src_dir = package["manifest_path"]
        .as_str()
        .and_then(|manifest| Path::new(manifest).parent())
        .map(|crate_dir| crate_dir.join("src"))
        .ok_or_else(|| format!("cannot locate src for crate '{}'", boundary.crate_package))?;

    let governed = governed_files(&src_dir, &boundary.module);
    if governed.is_empty() {
        return Err(format!(
            "module '{}' not found in crate '{}' source (file-based modules only)",
            boundary.module, boundary.crate_package
        ));
    }

    let rule = boundary.rule.label().to_string();
    let ModuleRule::MustNotImport { module: forbidden } = &boundary.rule;
    let beneath = format!("{forbidden}::");
    for (file, current_module) in governed {
        // A governed file we cannot read is "cannot judge", not "nothing to judge":
        // silently skipping it could hide a real violation. Fail as a scan error
        // (exit 2), never a silent pass.
        let text = std::fs::read_to_string(&file).map_err(|err| {
            format!(
                "cannot read governed source file '{}': {err}",
                file.display()
            )
        })?;
        for import in imported_module_paths(&text, &current_module) {
            if import == *forbidden || import.starts_with(&beneath) {
                violations.push(Violation {
                    kind: BoundaryKind::Module,
                    target: boundary.module.clone(),
                    rule: rule.clone(),
                    finding: import,
                    reason: boundary.reason.clone(),
                    severity: boundary.severity,
                    baselined: false,
                });
            }
        }
    }
    Ok(())
}

fn cargo_metadata(manifest_path: &Path) -> Result<Value, String> {
    let output = Command::new("cargo")
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .output()
        .map_err(|err| err.to_string())?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }

    serde_json::from_slice(&output.stdout).map_err(|err| err.to_string())
}

fn find_package<'a>(metadata: &'a Value, package: &str) -> Option<&'a Value> {
    metadata["packages"]
        .as_array()?
        .iter()
        .find(|candidate| candidate["name"].as_str() == Some(package))
}

/// Names of the target's normal `[dependencies]` that resolve to a registry or git
/// source. Dev/build dependencies and path/internal dependencies are excluded.
///
/// Names are package names, not local renames (`foo = { package = "bar" }` is
/// reported as `bar`), and platform-specific (`[target.'cfg(…)'.dependencies]`) and
/// `optional` deps are included — a declared dependency is governed as declared
/// (PROJECT.md).
fn external_normal_dependencies(package: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(dependencies) = package["dependencies"].as_array() {
        for dependency in dependencies {
            // `kind` is null for normal deps, "dev"/"build" otherwise.
            if dependency["kind"].as_str().is_some() {
                continue;
            }
            // A path/internal dependency has a null `source`; any non-null source is
            // external. Match on presence, not on a fixed `registry+`/`git+` prefix
            // list, so a dependency from an alternative (e.g. `sparse+`) registry
            // cannot slip through unclassified and silently pass the boundary.
            let external = !dependency["source"].is_null();
            if external {
                if let Some(name) = dependency["name"].as_str() {
                    found.push(name.to_string());
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Names of the target's normal `[dependencies]`, regardless of source — internal
/// workspace path dependencies included. Dev/build dependencies are excluded.
/// Used by the forbid and restrict-to rules, which (unlike the external rule) must
/// see internal crate-to-crate dependencies. Same conventions as
/// [`external_normal_dependencies`]: package names (not local renames), and
/// platform-specific / `optional` deps are included (PROJECT.md).
fn normal_dependencies(package: &Value) -> Vec<String> {
    let mut found = Vec::new();
    if let Some(dependencies) = package["dependencies"].as_array() {
        for dependency in dependencies {
            // `kind` is null for normal deps, "dev"/"build" otherwise.
            if dependency["kind"].as_str().is_some() {
                continue;
            }
            if let Some(name) = dependency["name"].as_str() {
                found.push(name.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

/// Internal module paths imported by `source`, normalized to absolute `crate::…`
/// form (`self`/`super` resolved against `current_module`). Only `use` declarations
/// are observed; grouped and glob forms are expanded; paths whose first segment is
/// an external crate are ignored. Bare path expressions and macro-generated imports
/// are out of scope (PROJECT.md). Returns sorted, de-duplicated paths.
pub(crate) fn imported_module_paths(source: &str, current_module: &str) -> Vec<String> {
    let cleaned = strip_comments_and_strings(source);
    let mut paths = Vec::new();
    for tree in use_trees(&cleaned) {
        for leaf in expand_use_tree(&tree) {
            if let Some(absolute) = normalize_module_path(&leaf, current_module) {
                paths.push(absolute);
            }
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

/// Remove comments and string literals — line (`//`), block (`/* */`), normal and
/// byte strings (`"…"`, `b"…"`, honoring `\"`/`\\`), and raw strings (`r"…"`,
/// `r#"…"#`, `br#"…"#`, any number of hashes) — so their contents can never be
/// mistaken for a `use` declaration: a `//` or a `use …;` written inside any of them
/// is ignored. Char literals are recognized minimally so a quote-bearing one (`'"'`)
/// does not open a spurious string; a lifetime (`'a`) is emitted as ordinary text.
/// Bare path expressions and macro-generated imports remain out of scope (PROJECT.md).
/// UTF-8 is preserved: kept bytes are decoded once and never split, because every
/// region boundary cut on is ASCII.
fn strip_comments_and_strings(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // Line comment: drop to end of line.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Block comment: Rust nests these, so track depth and drop through to the
            // `*/` that closes the outermost one — otherwise commented-out code that
            // itself contains a `/* */` would re-expose a `use` after the inner close.
            i += 2;
            let mut depth = 1usize;
            while i + 1 < bytes.len() && depth > 0 {
                if bytes[i] == b'/' && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        } else if let Some((hashes, quote)) = raw_string_prefix(bytes, i) {
            // Raw string `r#*"…"#*`: no escapes; closed by `"` plus the same number
            // of `#`. Drop the whole literal so its text is never scanned.
            i = quote + 1;
            while i < bytes.len() {
                if bytes[i] == b'"' && raw_closing_matches(bytes, i + 1, hashes) {
                    i += 1 + hashes;
                    break;
                }
                i += 1;
            }
        } else if bytes[i] == b'"' {
            // String (or byte-string) literal: drop it, honoring `\"` and `\\`.
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if bytes[i] == b'\'' {
            // A char literal must be skipped whole so a quote it contains (`'"'`)
            // cannot open a spurious string. A lifetime (`'a`) has no closing quote
            // and is emitted as ordinary text.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                // Escaped char literal (`'\n'`, `'\''`, `'\u{…}'`): skip to closing.
                i += 2;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            } else if i + 2 < bytes.len() && bytes[i + 2] == b'\'' {
                // Simple char literal (`'x'`, `'"'`).
                i += 3;
            } else {
                // A lifetime or stray quote.
                out.push(bytes[i]);
                i += 1;
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// If a raw string literal begins at `i` — `r` or `br` at a token boundary, then any
/// number of `#`, then `"` — return `(hash_count, index_of_opening_quote)`. A leading
/// `r`/`b` that is the tail of an identifier is not a prefix.
fn raw_string_prefix(bytes: &[u8], i: usize) -> Option<(usize, usize)> {
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return None;
    }
    let mut j = i;
    if bytes.get(j) == Some(&b'b') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let mut hashes = 0;
    while bytes.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if bytes.get(j) == Some(&b'"') {
        Some((hashes, j))
    } else {
        None
    }
}

/// Whether `hashes` `#` characters start at `at` — the closing delimiter that, with
/// the preceding `"`, terminates a raw string opened with the same number of hashes.
fn raw_closing_matches(bytes: &[u8], at: usize, hashes: usize) -> bool {
    (0..hashes).all(|k| bytes.get(at + k) == Some(&b'#'))
}

/// The path tree of each `use … ;` statement (text between `use` and `;`).
fn use_trees(source: &str) -> Vec<String> {
    let mut trees = Vec::new();
    let mut from = 0;
    while let Some(pos) = keyword_at(source, from, "use") {
        let start = pos + 3;
        match source[start..].find(';') {
            Some(end) => {
                trees.push(source[start..start + end].trim().to_string());
                from = start + end + 1;
            }
            None => break,
        }
    }
    trees
}

/// Index of `keyword` appearing as a standalone word at or after `from`.
fn keyword_at(source: &str, from: usize, keyword: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut search = from;
    while let Some(rel) = source[search..].find(keyword) {
        let pos = search + rel;
        let before_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
        let after = pos + keyword.len();
        let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
        if before_ok && after_ok {
            return Some(pos);
        }
        search = pos + keyword.len();
    }
    None
}

fn is_ident_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

/// Expand a use tree into leaf paths: `a::{b, c::d}` -> `a::b`, `a::c::d`; drop
/// `::*` and ` as alias`; `{self}` resolves to the prefix module.
fn expand_use_tree(tree: &str) -> Vec<String> {
    let tree = tree.trim();
    match tree.find('{') {
        Some(open) => {
            let prefix = tree[..open].trim();
            let inner = brace_content(&tree[open..]);
            let mut out = Vec::new();
            for part in split_top_commas(&inner) {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if part == "self" {
                    let module = prefix.trim_end_matches(':').trim();
                    if !module.is_empty() {
                        out.push(module.to_string());
                    }
                } else {
                    out.extend(expand_use_tree(&format!("{prefix}{part}")));
                }
            }
            out
        }
        None => {
            let leaf = match tree.find(" as ") {
                Some(idx) => &tree[..idx],
                None => tree,
            };
            let leaf = leaf.trim().strip_suffix("::*").unwrap_or(leaf.trim());
            let leaf = leaf.trim_end_matches(':');
            if leaf.is_empty() {
                Vec::new()
            } else {
                vec![leaf.to_string()]
            }
        }
    }
}

/// Content inside the first `{ … }` of `s` (which must start with `{`), honoring
/// nesting.
fn brace_content(s: &str) -> String {
    let mut depth = 0;
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                if depth == 1 {
                    continue;
                }
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        out.push(ch);
    }
    out
}

/// Split on commas at brace depth 0.
fn split_top_commas(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Resolve a use path to an absolute `crate::…` module path, or `None` if it refers
/// to an external crate (first segment is not `crate`/`self`/`super`).
fn normalize_module_path(path: &str, current_module: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    let (first, rest) = segments.split_first()?;
    match *first {
        "crate" => Some(segments.join("::")),
        "self" => {
            let mut out: Vec<&str> = current_module
                .split("::")
                .filter(|s| !s.is_empty())
                .collect();
            out.extend(rest);
            Some(out.join("::"))
        }
        "super" => {
            let mut out: Vec<&str> = current_module
                .split("::")
                .filter(|s| !s.is_empty())
                .collect();
            let mut tail = &segments[..];
            while let Some((segment, next)) = tail.split_first() {
                if *segment != "super" {
                    break;
                }
                out.pop();
                tail = next;
            }
            out.extend(tail.iter().copied());
            if out.is_empty() {
                None
            } else {
                Some(out.join("::"))
            }
        }
        _ => None,
    }
}

/// Every `(file, module path)` whose module is the governed `module` or beneath it.
fn governed_files(src_dir: &Path, module: &str) -> Vec<(PathBuf, String)> {
    let beneath = format!("{module}::");
    rust_files(src_dir)
        .into_iter()
        .filter_map(|file| {
            let relative = file.strip_prefix(src_dir).ok()?;
            let module_path = file_module_path(relative);
            if module_path == module || module_path.starts_with(&beneath) {
                Some((file, module_path))
            } else {
                None
            }
        })
        .collect()
}

/// The module path of a source file from its path relative to `src/`:
/// `lib.rs`/`main.rs`/`mod.rs` contribute no segment; `kernel/foo.rs` ->
/// `crate::kernel::foo`.
fn file_module_path(relative: &Path) -> String {
    let components: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let mut segments = vec![String::from("crate")];
    let last = components.len().saturating_sub(1);
    for (index, component) in components.iter().enumerate() {
        if index == last {
            let stem = component.strip_suffix(".rs").unwrap_or(component);
            if !matches!(stem, "mod" | "lib" | "main") {
                segments.push(stem.to_string());
            }
        } else {
            segments.push(component.clone());
        }
    }
    segments.join("::")
}

/// All `.rs` files under `dir`, recursively.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(rust_files(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                found.push(path);
            }
        }
    }
    found
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
        let json = report_json(&Outcome::Violations(one_enforce_violation()), &[]);
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
            serde_json::from_str(&report_json(&Outcome::Clean, &[])).unwrap();
        assert_eq!(clean["outcome"], "clean");
        assert_eq!(clean["exit_code"], 0);
        assert_eq!(clean["violations"].as_array().unwrap().len(), 0);

        let error: serde_json::Value = serde_json::from_str(&report_json(
            &Outcome::ConstitutionError("boom".into()),
            &[],
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
            serde_json::from_str(&report_json(&Outcome::Violations(report), &stale)).unwrap();
        assert_eq!(doc["exit_code"], 0, "a fully baselined run does not fail");
        assert_eq!(doc["violations"][0]["baselined"], true);
        assert_eq!(doc["stale_baseline"][0]["finding"], "gone");
    }

    #[test]
    fn scanner_expands_groups_and_resolves_relative_imports() {
        let source = r#"
            // a line comment mentioning use crate::ignored::me;
            use crate::a::{b, c::d};
            use super::sibling::X;
            use self::inner::Y;
            use serde::Deserialize;
            use crate::z::*;
        "#;
        let imports = imported_module_paths(source, "crate::kernel");
        assert!(imports.contains(&"crate::a::b".to_string()), "{imports:?}");
        assert!(
            imports.contains(&"crate::a::c::d".to_string()),
            "{imports:?}"
        );
        // `super` from crate::kernel resolves to crate.
        assert!(
            imports.contains(&"crate::sibling::X".to_string()),
            "{imports:?}"
        );
        // `self` resolves against the current module.
        assert!(
            imports.contains(&"crate::kernel::inner::Y".to_string()),
            "{imports:?}"
        );
        // glob keeps the module prefix.
        assert!(imports.contains(&"crate::z".to_string()), "{imports:?}");
        // external first segment is ignored, and commented-out imports are not seen.
        assert!(!imports.iter().any(|p| p.contains("serde")), "{imports:?}");
        assert!(
            !imports.iter().any(|p| p.contains("ignored")),
            "{imports:?}"
        );
    }

    #[test]
    fn scanner_ignores_comments_and_string_literals() {
        // A `//` inside a string must not eat a real `use` later on the same line.
        let url = r#"let u = "http://example.com"; use crate::real::A;"#;
        let imports = imported_module_paths(url, "crate::kernel");
        assert!(
            imports.contains(&"crate::real::A".to_string()),
            "{imports:?}"
        );

        // A `use …;` written inside a string is not a real import.
        let in_string = r#"let s = "use crate::ghost::Z;";"#;
        assert!(
            imported_module_paths(in_string, "crate::kernel").is_empty(),
            "a use inside a string must not be observed"
        );

        // A quote-bearing char literal must not open a spurious string and swallow code.
        let quote_char = r#"let q = '"'; use crate::real::B;"#;
        let imports = imported_module_paths(quote_char, "crate::kernel");
        assert!(
            imports.contains(&"crate::real::B".to_string()),
            "{imports:?}"
        );

        // A lifetime must not break use detection or produce a spurious path.
        let lifetime = "fn f<'a>(x: &'a str) {} use crate::a::b;";
        let imports = imported_module_paths(lifetime, "crate::kernel");
        assert_eq!(imports, vec!["crate::a::b".to_string()], "{imports:?}");
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
            external_normal_dependencies(&package),
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
    fn scanner_handles_nested_block_comments() {
        // Rust nests block comments. Commenting out code that itself contains a
        // `/* */` must not let the inner `*/` re-expose the rest as live code: a
        // `use` inside the (nested) comment must not be observed, while a real `use`
        // after the outer close still is.
        let source = r#"
            /*
            fn old() {
                /* tweak later */
                use crate::legacy::Thing;
            }
            */
            use crate::current::A;
        "#;
        let imports = imported_module_paths(source, "crate::kernel");
        assert!(
            imports.contains(&"crate::current::A".to_string()),
            "the real use after the nested comment must be observed: {imports:?}"
        );
        assert!(
            !imports.iter().any(|p| p.contains("legacy")),
            "a use inside a nested block comment must not be observed: {imports:?}"
        );
    }

    #[test]
    fn scanner_preserves_non_ascii_module_paths() {
        // strip is UTF-8 safe: a non-ASCII module path survives stripping intact.
        let source = "use crate::café::Item;";
        let imports = imported_module_paths(source, "crate::kernel");
        assert!(
            imports.contains(&"crate::café::Item".to_string()),
            "{imports:?}"
        );
    }

    #[test]
    fn scanner_ignores_raw_and_byte_strings() {
        // A `use …;` inside a raw string (any hash count) is not an import.
        for src in [
            r##"let s = r"use crate::ghost::Z;";"##,
            r##"let s = r#"use crate::ghost::Z;"#;"##,
            r##"let s = br#"use crate::ghost::Z;"#;"##,
            r#"let s = b"use crate::ghost::Z;";"#,
        ] {
            assert!(
                imported_module_paths(src, "crate::kernel").is_empty(),
                "a use inside a (raw/byte) string must not be observed: {src}"
            );
        }

        // A `//` and an inner `"#` inside a raw string must not eat a following use.
        // (Two outer hashes so the inner `"#` does not close it.)
        let tricky = r####"let s = r##"http://x "# inside"##; use crate::real::C;"####;
        let imports = imported_module_paths(tricky, "crate::kernel");
        assert!(
            imports.contains(&"crate::real::C".to_string()),
            "{imports:?}"
        );

        // `r` / `b` as ordinary identifiers (not raw-string prefixes) are unaffected.
        let idents = "let r = 1; let b = 2; use crate::real::D;";
        assert_eq!(
            imported_module_paths(idents, "crate::kernel"),
            vec!["crate::real::D".to_string()]
        );
    }

    #[test]
    fn scanner_does_not_panic_on_odd_input() {
        // Truncated / malformed inputs must never panic (robustness over precision).
        for src in [
            "r#\"unterminated raw string",
            "\"unterminated string",
            "/* unterminated block",
            "'",
            "r",
            "use ",
            "use crate::",
            "",
        ] {
            let _ = imported_module_paths(src, "crate::kernel");
        }
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
}
