//! The projections: the text and JSON renderings of an [`Outcome`] and a
//! [`Constitution`]. A projection is a faithful, self-describing view of the model
//! for humans (text) and machines (JSON) — it adds no policy and makes no decision
//! (PROJECT.md). All JSON serialization funnels through [`pretty_json`], the single
//! place the infallibility-vs-`Result` decision is recorded.

use super::*;
use serde_json::Value;

/// Serialize an owned [`Value`] to pretty JSON. Infallible by construction, not by
/// hope: a `Value`'s `Serialize` impl never errors, its map keys are always strings,
/// and it cannot hold a non-finite float (`json!(f64::NAN)` yields `Null`), so the
/// only two documented `to_string_pretty` failure modes are both unreachable; the
/// sink is an in-memory `String`, so there is no I/O error path either. The `expect`
/// is therefore a proof annotation, not unhandled error. We deliberately keep it
/// over `-> Result<String, _>` plumbing into the callers: that would defend an
/// impossible state, which PROJECT.md's minimalism bound rules out (fail-loud is for
/// observable misconfiguration, not for facts that cannot occur). This is the single
/// place that decision lives — change it here, with reasoning, not site by site.
pub(crate) fn pretty_json(document: &Value) -> String {
    serde_json::to_string_pretty(document).expect("a serde_json::Value is always serializable")
}

/// Render the outcome as a JSON document for machine consumption: a faithful
/// projection of [`Outcome`] with each violation's `kind`, the boundary `reason` as
/// the repair hint, and `exit_code` mirroring the process exit. `stale` lists
/// baseline entries matching no current violation (empty outside gate mode).
pub(crate) fn report_json(
    outcome: &Outcome,
    stale: &[ViolationId],
    coverage: Option<&Coverage>,
) -> String {
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
    let mut document = serde_json::json!({
        "outcome": label,
        "exit_code": outcome.exit_code(),
        "violations": violations,
        "stale_baseline": stale_baseline,
        "error": error,
    });
    if let Some(coverage) = coverage {
        document["coverage"] = serde_json::json!({
            "workspace_crates": coverage.total,
            "uncovered": coverage.uncovered,
        });
    }
    pretty_json(&document)
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
            Boundary::Crate(b) => {
                let rule = match dependency_kind_label(b.dependency_kind()) {
                    Some(kind) => format!("{} ({kind} dependencies)", b.rule().text()),
                    None => b.rule().text(),
                };
                (
                    b.severity(),
                    format!("crate {}", b.target().package),
                    rule,
                    b.reason(),
                )
            }
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
    pretty_json(&document)
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
            if let Some(kind) = dependency_kind_label(b.dependency_kind()) {
                object["dependency_kind"] = serde_json::json!(kind);
            }
            object
        }
        Boundary::Module(b) => {
            let mut object = serde_json::json!({
                "kind": "module",
                "target": b.module,
                "crate": b.crate_package,
                "rule": b.rule.label(),
                "severity": severity_label(b.severity),
                "reason": b.reason,
            });
            for (key, value) in b.rule.json_params() {
                object[key] = value;
            }
            object
        }
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Enforce => "enforce",
        Severity::Warn => "warn",
    }
}

/// The projection label for a non-default dependency kind, or `None` for `Normal` so
/// the common projection is unchanged.
fn dependency_kind_label(kind: DependencyKind) -> Option<&'static str> {
    match kind {
        DependencyKind::Normal => None,
        DependencyKind::Dev => Some("dev"),
        DependencyKind::Build => Some("build"),
    }
}
