# constitution-projection Specification

## Purpose

Render the declared constitution as a read-only projection of the Rust source of
truth, so the effective law is legible without reading code or triggering a
violation. Delivered as the runner's `list` command (through `modou::run`, so every
adopter gets it), it serves a steward reviewing an amendment, an operator reading a
CI log, and a tool or agent that wants the declared law. A projection is never a
reaction: `list` observes nothing, claims no target or drift type, and always
exits 0.

## Requirements

### Requirement: List command projects the declared constitution

The runner SHALL provide a `list` command that renders the caller-supplied constitution as a human-readable projection on standard output. For each boundary it SHALL show the severity, the kind, the target (a module boundary SHALL also show its crate), the rule together with its parameters (e.g. an allowlist of crate names, the forbidden crate names, or the forbidden module path), and the boundary's reason. The projection SHALL be derived only from the declared constitution; it MUST NOT invent a field for data the constitution does not hold.

#### Scenario: List renders each boundary

- **WHEN** the runner is invoked as `list` against a constitution holding a crate boundary and a module boundary
- **THEN** standard output names the constitution and, for each boundary, its severity, kind, target, rule with parameters, and reason

#### Scenario: A crate boundary's rule parameters are shown

- **WHEN** a deny-external boundary carries an allowlist, a forbid-dependency-on boundary names crates, or a restrict-to boundary names an allowlist
- **THEN** the projection shows those crate names alongside the rule

#### Scenario: An empty constitution lists cleanly

- **WHEN** the runner is invoked as `list` against a constitution with no boundaries
- **THEN** it prints the constitution name and an empty boundary set and exits `0`, never erroring

### Requirement: List is a projection, not a reaction

The `list` command SHALL observe nothing: it SHALL NOT read `cargo metadata`, SHALL NOT require `--manifest-path`, and SHALL NOT evaluate any boundary against a workspace. It SHALL always exit `0`, because a projection cannot be violated. The `list` command therefore claims no target type or drift type and performs no pass/fail judgment.

#### Scenario: List needs no manifest path

- **WHEN** the runner is invoked as `list` with no `--manifest-path`
- **THEN** it prints the constitution projection and exits `0`, never treating the absent manifest path as a usage error

#### Scenario: List always exits 0

- **WHEN** the runner is invoked as `list` for any valid constitution
- **THEN** it exits `0`, never `1` (it makes no reaction) and never `2` (it reads no workspace)

### Requirement: List honors the format flag

The `list` command SHALL honor `--format`: human-readable text by default, or a JSON document under `--format json` (and `--format=json`). The JSON SHALL faithfully project the constitution — its name and a `boundaries` array whose entries carry the `kind`, `target`, `severity`, `reason`, and the rule with its parameters — so a tool or agent can read the declared law. The `target` SHALL follow the existing report convention: the crate name for a crate boundary and the module path for a module boundary. An unrecognized `--format` value SHALL be a usage error that exits `2`, never a silent fallback, consistent with the `check` command.

#### Scenario: List emits a JSON projection

- **WHEN** the runner is invoked as `list --format json`
- **THEN** standard output is a JSON document carrying the constitution name and a `boundaries` array, each entry with its `kind` (`crate` or `module`), `severity`, `reason`, and rule parameters

#### Scenario: An unknown format to list is a usage error

- **WHEN** the runner is invoked as `list --format` with a value other than `text` or `json`
- **THEN** it prints usage guidance and exits `2`
