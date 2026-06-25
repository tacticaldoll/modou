# violation-baseline Specification

## Purpose

Let a dirty project adopt a boundary without first fixing every pre-existing
violation. A baseline is a generated snapshot of accepted violations (a projection,
not policy — see the baseline decision in `PROJECT.md`); the gate fails only on violations new since the baseline,
suppresses recorded ones, and reports stale entries so the baseline can only
ratchet down. This is the `gate` rung of the adoption ramp above `warn`.

## Requirements
### Requirement: Baseline records accepted violations

A baseline SHALL record a set of accepted violations, each identified by the triple `(target, rule, finding)`. The baseline SHALL be a generated artifact (a projection), never the constitution: boundaries, rules, and severity remain declared in Rust. The identity SHALL exclude the boundary's reason and severity, so editing a reason or moving a boundary between warn and enforce SHALL NOT make an existing violation count as new. The baseline SHALL serialize as JSON carrying a `version` and a list of `violations` sorted by their identity, so the file is stable and diffable.

#### Scenario: A baseline round-trips through JSON

- **WHEN** a baseline of known violations is written to JSON and read back
- **THEN** the parsed baseline contains the same `(target, rule, finding)` entries

#### Scenario: A malformed or unknown-version baseline is an error

- **WHEN** a baseline file is malformed or declares an unknown `version`
- **THEN** the system reports an error rather than silently treating the baseline as empty

### Requirement: Gate suppresses baselined violations and fails only on new ones

In gate mode the system SHALL classify each current violation against the baseline: a violation whose identity is in the baseline SHALL be suppressed and SHALL NOT cause failure, while a violation not in the baseline SHALL react according to its severity. The reaction SHALL fail (exit 1) only when a non-baselined enforce-severity violation exists; warn-severity and baselined violations SHALL NOT fail.

#### Scenario: A pre-existing violation in the baseline does not fail

- **WHEN** every current enforce-severity violation is present in the baseline
- **THEN** the gate does not fail (exit 0)

#### Scenario: A new enforce violation fails

- **WHEN** an enforce-severity violation is present that is not in the baseline
- **THEN** the gate fails (exit 1) and reports that violation as new

### Requirement: Stale baseline entries are reported but do not fail

The system SHALL report any baseline entry that matches no current violation as stale, so the baseline can be ratcheted down as violations are fixed. A stale entry SHALL NOT cause the reaction to fail.

#### Scenario: A fixed violation leaves a stale baseline entry

- **WHEN** the baseline contains an entry that matches no current violation
- **THEN** the system reports that entry as stale and does not fail on account of it

### Requirement: Writing a baseline records the current violations

The system SHALL provide a write action that records the current violations as a baseline and SHALL exit 0, since recording is not judging. The write action SHALL refuse to write when the constitution cannot be evaluated (a constitution or scan error), exiting 2, because a baseline cannot be pinned from an unevaluable constitution.

#### Scenario: Write records current violations and exits 0

- **WHEN** the write action runs against a workspace with violations
- **THEN** the system writes those violations as a baseline and exits 0

#### Scenario: Write refuses on a constitution error

- **WHEN** the write action runs but the constitution cannot be evaluated
- **THEN** the system reports the error and exits 2 without writing a baseline

