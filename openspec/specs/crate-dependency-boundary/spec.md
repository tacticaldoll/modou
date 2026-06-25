# crate-dependency-boundary Specification

## Purpose

Detect crate-dependency drift: compare each declared `Boundary` against the
observed dependencies of its target crate (via `cargo metadata`) and react with a
distinct CI exit code — clean, boundary violation, or constitution error — never a
silent pass. This is Modou's first reaction and the proof of its core contract: a
declared boundary in Rust produces a real, non-bypassable reaction when violated.
## Requirements
### Requirement: Constitution declared in Rust

The constitution SHALL be expressed as Rust code and is the single source of
truth. A `Constitution` SHALL carry a name and a list of `Boundary` values. The
system MUST NOT require TOML, YAML, Markdown, or any generated policy file to
declare or run a boundary.

#### Scenario: Boundary declared in Rust

- **WHEN** a developer writes `Constitution::new("example").boundary(Boundary::crate_("example-core").deny_external_dependencies().because("..."))`
- **THEN** the constitution holds one boundary targeting the crate `example-core` with rule `DenyExternalDependencies` and a non-empty reason

### Requirement: Target resolution

For each boundary, the system SHALL resolve the named `CrateTarget` to a real
package in the target Cargo workspace before evaluating its rule. If the target
crate cannot be found in the workspace, the system SHALL treat this as a
**constitution error** (a misconfiguration) — failing loud with a distinct exit
code rather than a silent pass, and distinct from a boundary violation so a
mistyped crate name is not reported as architectural drift.

#### Scenario: Target resolves to a workspace package

- **WHEN** a boundary targets `example-core` and that package exists in the workspace metadata
- **THEN** the system observes that package's dependencies for comparison

#### Scenario: Unresolvable target is a constitution error

- **WHEN** a boundary targets a crate name that is not present in the workspace metadata
- **THEN** the system emits a constitution error reporting the unresolved target and exits with code 2 (distinct from a boundary violation, which is exit 1)

### Requirement: External dependency classification

The `DenyExternalDependencies` rule SHALL classify the target crate's normal
`[dependencies]` by source: a dependency resolving to a registry or git source
is external; a dependency resolving to a workspace path is internal and
allowed. For v0.1 the rule SHALL consider only normal `[dependencies]`;
`[dev-dependencies]` and `[build-dependencies]` are out of scope.

#### Scenario: External dependency violates the boundary

- **WHEN** the target crate declares a normal dependency whose source is a registry or git source
- **THEN** the system emits a violation naming that dependency

#### Scenario: Internal path dependency is allowed

- **WHEN** the target crate declares only dependencies that resolve to workspace paths
- **THEN** the system reports no external-dependency violation for that boundary

#### Scenario: Dev and build dependencies are ignored

- **WHEN** the target crate declares a registry dependency only under `[dev-dependencies]` or `[build-dependencies]`
- **THEN** the system does not emit an external-dependency violation in v0.1

### Requirement: CI reaction

The system SHALL distinguish three outcomes by exit code so a CI gate can tell
architectural drift from misconfiguration: **exit 0** when no boundary is
violated; **exit 1** when one or more boundaries are violated; **exit 2** for a
constitution or scan error (e.g. an unresolvable target or an unreadable target
workspace). For exit 1 and exit 2 the system SHALL print a human-readable report.

#### Scenario: Clean target passes

- **WHEN** the target crate has no external dependencies
- **THEN** the system reports that the boundary is satisfied and exits 0

#### Scenario: Violation fails CI

- **WHEN** one or more boundaries are violated
- **THEN** the system prints a report and exits 1

#### Scenario: Misconfiguration is distinguishable from violation

- **WHEN** evaluation cannot proceed because a target is unresolvable or the workspace cannot be read
- **THEN** the system prints a constitution/scan error and exits 2, never exit 0 (no silent pass) and never exit 1 (not reported as a boundary violation)

### Requirement: Human-readable violation report

A violation report SHALL identify the target crate, the rule, the offending
finding (e.g. the dependency name), and a human-readable reason supplied with
the boundary, and SHALL state that the reaction failed.

#### Scenario: Report explains the violation

- **WHEN** the target crate `example-core` declares the external dependency `serde`
- **THEN** the report names the target `example-core`, the rule "deny external dependencies", the finding `serde`, the boundary's reason, and indicates CI failed

### Requirement: Multiple boundaries

A constitution MAY declare more than one boundary. When it does, the system SHALL
evaluate every boundary and aggregate all resulting violations into a single
report. A constitution error on any boundary (an unresolvable target) SHALL
supersede any boundary violation found in the same run: the run reports a
constitution error (exit 2), not a violation (exit 1), because a boundary that
could not be evaluated makes the run's verdict untrustworthy. The order in which
boundaries are declared SHALL NOT change the outcome class.

#### Scenario: Violations across boundaries are aggregated

- **WHEN** a constitution declares two boundaries targeting two different crates that each declare an external dependency
- **THEN** the report contains a violation for each crate, and the system exits 1

#### Scenario: A constitution error supersedes a violation

- **WHEN** a constitution declares one boundary that is violated and one boundary whose target crate does not exist in the workspace
- **THEN** the system reports a constitution error and exits 2, not a violation (exit 1)

#### Scenario: Order of boundaries does not change the outcome class

- **WHEN** the same set of boundaries is evaluated in any order and at least one target is unresolvable
- **THEN** the system reports a constitution error (exit 2) regardless of declaration order

### Requirement: External dependency allowlist

The `DenyExternalDependencies` rule SHALL support an optional allowlist of crate names. An external dependency whose name is in the allowlist SHALL NOT be reported as a violation; an external dependency not in the allowlist SHALL still be a violation. A rule declared with no allowlist SHALL behave exactly as v0.1 (deny every external dependency). The allowlist SHALL apply only to the same normal `[dependencies]` the rule already classifies.

#### Scenario: An allowed external dependency is not a violation

- **WHEN** the target crate declares the external dependency `serde` and the boundary allows `serde`
- **THEN** the system reports no violation for that dependency

#### Scenario: A non-allowed external dependency still violates

- **WHEN** the target crate declares the external dependency `serde` and the boundary's allowlist does not contain `serde`
- **THEN** the system emits a violation naming `serde`

### Requirement: Forbid dependency on named crates

A boundary SHALL support a rule that forbids a dependency on specific named crates. A normal `[dependencies]` entry whose name matches a forbidden name SHALL be a violation, whether that dependency resolves to an external source or to an internal workspace path. This enables both deny-specific-crate and crate → crate layering ("core must not depend on adapters"). `[dev-dependencies]` and `[build-dependencies]` SHALL remain out of scope, as in v0.1.

#### Scenario: A forbidden external crate is a violation

- **WHEN** the target crate declares the external dependency `serde` and the boundary forbids `serde`
- **THEN** the system emits a violation naming `serde` and exits 1

#### Scenario: A forbidden internal crate is a violation (layering)

- **WHEN** the target crate `core` declares a workspace path dependency on `adapters` and the boundary forbids `adapters`
- **THEN** the system emits a violation naming `adapters`, even though that dependency is internal and the external rule would ignore it

#### Scenario: A crate that is not depended on is clean

- **WHEN** the boundary forbids a crate the target does not depend on
- **THEN** the system reports no violation for that boundary

### Requirement: Boundary severity

A boundary SHALL carry a severity that controls how its violations react: `enforce` or `warn`. A boundary declared without an explicit severity SHALL default to `enforce`, preserving prior behavior. The violations of a `warn`-severity boundary SHALL still be observed and reported, but SHALL NOT by themselves cause the reaction to fail; a `warn` boundary is the advisory rung of adoption, observed before it is enforced. Severity SHALL be declared in Rust alongside the boundary, never in a separate file.

#### Scenario: A warn boundary's violation is reported but does not fail

- **WHEN** a `warn`-severity boundary is violated and no `enforce`-severity boundary is violated
- **THEN** the system reports the violation but the reaction does not fail (the exit code is 0)

#### Scenario: An enforce boundary still fails

- **WHEN** an `enforce`-severity boundary is violated
- **THEN** the system reports the violation and the reaction fails (exit 1), regardless of any warn boundaries

#### Scenario: A boundary defaults to enforce

- **WHEN** a boundary is declared without an explicit severity and is violated
- **THEN** the reaction fails (exit 1), exactly as before this capability existed

### Requirement: Restrict dependencies to an allowlist

A boundary SHALL support a rule that restricts the target crate's normal `[dependencies]` to a closed allowlist of crate names. Every normal dependency whose name is not in the allowlist SHALL be a violation, whether it resolves to an external source or to an internal workspace path. An empty allowlist SHALL forbid every normal dependency (stricter than the deny-external rule, which still permits internal path dependencies). `[dev-dependencies]` and `[build-dependencies]` SHALL remain out of scope, as for the other crate rules. The rule SHALL carry severity and react through the report, baseline, and projection exactly as the existing crate rules do.

#### Scenario: A dependency outside the allowlist is a violation

- **WHEN** the target crate declares a normal dependency `serde` and the boundary restricts dependencies to `["other"]`
- **THEN** the system emits a violation naming `serde` and exits 1

#### Scenario: A dependency inside the allowlist is clean

- **WHEN** the target crate's only normal dependency is `serde` and the boundary restricts dependencies to `["serde"]`
- **THEN** the system reports no violation for that boundary

#### Scenario: An internal path dependency outside the allowlist is a violation

- **WHEN** the target crate `core` declares a workspace path dependency on `adapters` and the boundary restricts dependencies to an allowlist that omits `adapters`
- **THEN** the system emits a violation naming `adapters`, even though it is internal and the deny-external rule would ignore it

#### Scenario: An empty allowlist forbids every dependency

- **WHEN** the target crate declares any normal dependency and the boundary restricts dependencies to `[]`
- **THEN** the system emits a violation for that dependency

