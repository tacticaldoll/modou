use super::*;
use serde_json::Value;

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
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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
        pretty_json(&doc)
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
