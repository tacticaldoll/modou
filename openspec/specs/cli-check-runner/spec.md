# cli-check-runner Specification

## Purpose

Govern the `modou check` command-line contract — the runner that turns the
Rust-declared constitution into a CI reaction. It fixes the flag surface
(`--manifest-path`), the usage-error handling, and how the process exit code and
report mirror the reaction outcome (0 clean or warn-only, 1 enforce violation,
2 constitution/usage error), so a CI gate has a stable, non-bypassable contract.
## Requirements
### Requirement: Check command interface

The runner SHALL provide a `check` command that accepts the target Cargo
workspace via `--manifest-path <path>`, also accepting the `--manifest-path=<path>`
form. The runner SHALL evaluate the Rust-declared constitution against the
workspace at that path and translate the resulting outcome into a process exit
code. The runner MUST NOT require any flag other than `--manifest-path` to perform
a check.

#### Scenario: Check evaluates the target at the given manifest path

- **WHEN** the runner is invoked as `modou check --manifest-path <path>` where the path is a readable Cargo workspace
- **THEN** the runner evaluates the constitution against that workspace and exits with the code that mirrors the outcome

#### Scenario: The equals form of the flag is accepted

- **WHEN** the runner is invoked as `modou check --manifest-path=<path>`
- **THEN** the runner uses `<path>` as the target workspace, identically to the space-separated form

### Requirement: Process exit code mirrors the reaction outcome

The runner SHALL exit `0` when no enforce-severity boundary is violated, `1` when one or more enforce-severity boundaries are violated, and `2` for a constitution or scan error. Violations of warn-severity boundaries SHALL be reported but SHALL NOT by themselves cause a non-zero exit, so a warn-only run exits `0`. On any non-zero exit the runner SHALL print a human-readable report or error message. The runner MUST NOT exit `0` when it could not evaluate the constitution.

#### Scenario: Clean target exits 0

- **WHEN** the checked workspace satisfies every boundary
- **THEN** the runner reports that no boundary was violated and exits `0`

#### Scenario: Enforce violation exits 1 with a report

- **WHEN** one or more enforce-severity boundaries are violated in the checked workspace
- **THEN** the runner prints a violation report and exits `1`

#### Scenario: Warn-only violations exit 0 with an advisory

- **WHEN** the only violations are of warn-severity boundaries
- **THEN** the runner prints the violations as advisories and exits `0`

#### Scenario: Constitution error exits 2 with a message

- **WHEN** the constitution cannot be evaluated against the workspace (e.g. an unresolvable target or an unreadable workspace)
- **THEN** the runner prints a constitution error message and exits `2`, never `0`

### Requirement: Missing manifest path is a usage error

When `--manifest-path` is not supplied, the runner SHALL print usage guidance to
standard error and exit `2`. It MUST NOT exit `0` (no silent pass) and MUST NOT
exit `1` (a usage mistake is not architectural drift). The runner collapses
"cannot evaluate" cases — usage errors and constitution/scan errors alike — onto
exit `2`, so a CI gate reads `0` as ok, `1` as drift, and any other non-zero code
as "could not judge".

#### Scenario: No manifest path supplied

- **WHEN** the runner is invoked as `modou check` with no `--manifest-path`
- **THEN** the runner prints usage guidance to standard error and exits `2`

### Requirement: Baseline flags

The runner SHALL accept two mutually exclusive baseline flags: `--baseline <file>` selects gate mode (suppress baselined violations, fail only on new ones) and `--write-baseline <file>` records the current violations as a baseline. Each SHALL also accept the `=<file>` form. Supplying both SHALL be a usage error that exits 2. In gate mode the process exit code SHALL reflect the gated outcome — 0 when the only violations are baselined or warn, 1 on a new enforce-severity violation. A baseline file that cannot be read or parsed SHALL be treated as a scan error and exit 2.

#### Scenario: Write-baseline records and exits 0

- **WHEN** the runner is invoked with `--write-baseline <file>` against a workspace with violations
- **THEN** the runner writes the baseline file and exits 0

#### Scenario: Gate against a baseline that covers all violations exits 0

- **WHEN** the runner is invoked with `--baseline <file>` and every enforce violation is recorded in that file
- **THEN** the runner exits 0

#### Scenario: Gate fails on a violation not in the baseline

- **WHEN** the runner is invoked with `--baseline <file>` and an enforce violation is absent from that file
- **THEN** the runner exits 1 and reports the new violation

#### Scenario: Supplying both baseline flags is a usage error

- **WHEN** the runner is invoked with both `--baseline` and `--write-baseline`
- **THEN** the runner prints usage guidance and exits 2

#### Scenario: An unreadable baseline file exits 2

- **WHEN** the runner is invoked with `--baseline <file>` and the file is missing or malformed
- **THEN** the runner reports a scan error and exits 2

### Requirement: Machine-readable report format

The runner SHALL accept `--format json` (and `--format=json`) and emit the outcome as a JSON document on standard output; the default format SHALL remain human-readable text, so existing invocations are unchanged. An unrecognized format value SHALL be a usage error that exits 2, never a silent fallback. The JSON SHALL faithfully project the outcome: an `outcome` discriminant (`clean`, `violations`, or `constitution_error`), the `exit_code` mirroring the process exit, a `violations` array, a `stale_baseline` array (empty outside gate mode), and an `error` message (null unless a constitution error). Each violation SHALL carry its `kind` (`crate` or `module`), `target`, `rule`, `finding`, `reason`, `severity`, and `baselined` flag; the `reason` SHALL serve as the repair hint with no separate invented field.

#### Scenario: JSON format emits a parseable violations document

- **WHEN** the runner checks a workspace with an enforced crate violation under `--format json`
- **THEN** standard output is a JSON document with `outcome` `"violations"`, `exit_code` 1, and a violation whose `kind` is `"crate"` naming the offending dependency

#### Scenario: A clean workspace emits a clean JSON document

- **WHEN** the runner checks a clean workspace under `--format json`
- **THEN** standard output is a JSON document with `outcome` `"clean"`, `exit_code` 0, and an empty `violations` array

#### Scenario: The default format is unchanged

- **WHEN** the runner is invoked without `--format`
- **THEN** it prints the human-readable report exactly as before

#### Scenario: An unknown format is a usage error

- **WHEN** the runner is invoked with `--format` set to a value other than `text` or `json`
- **THEN** it prints usage guidance and exits 2

#### Scenario: Gate mode JSON reflects baseline and stale entries

- **WHEN** the runner gates against a baseline under `--format json`
- **THEN** baselined violations carry `baselined: true`, the `exit_code` reflects only new enforce violations, and baseline entries matching no current violation appear in `stale_baseline`

### Requirement: Runner exposed as a reusable library entry point

The `check` runner contract — argument parsing (`--manifest-path`, `--baseline` / `--write-baseline`, `--format`), the baseline gate and write actions, the report rendering, and the exit-code mapping (`0` clean / warn-only / fully baselined, `1` enforce violation, `2` constitution/scan/usage error) — SHALL be provided by the `modou` library as a public entry point. The entry point SHALL accept a caller-supplied constitution and the process arguments and SHALL return the process exit code, evaluating the supplied constitution exactly as the `check` command specifies. An adopting project SHALL obtain the identical runner contract by declaring its own constitution in Rust and invoking this entry point, without reimplementing argument parsing, baseline handling, report rendering, or exit-code mapping. The entry point MUST NOT exit `0` when it could not evaluate the constitution.

#### Scenario: A project runs its own constitution through the library entry point

- **WHEN** a project declares a constitution in Rust and invokes the library runner entry point with that constitution and `check --manifest-path <path>` against a readable workspace
- **THEN** the runner evaluates that project's constitution against the workspace and returns the exit code mirroring the outcome, identically to the `modou` binary

#### Scenario: The entry point honors the baseline and format flags

- **WHEN** the library runner entry point is invoked with `--baseline` / `--write-baseline` or `--format json`
- **THEN** it applies the gate or write action and the report format exactly as specified for the `check` command, and returns the gated exit code

#### Scenario: A usage error from the entry point exits 2

- **WHEN** the library runner entry point is invoked without `--manifest-path`, or with both `--baseline` and `--write-baseline`
- **THEN** it prints usage guidance and returns exit code `2`, never `0` or `1`

#### Scenario: The bundled binary is a thin caller of the entry point

- **WHEN** the `modou` binary is invoked as `modou check …`
- **THEN** it produces the same flags, reports, and exit codes as before, because it routes through the same library entry point with its own sample constitution

### Requirement: Unrecognized arguments are a usage error

The runner SHALL reject any argument it does not recognize — an unknown flag, a misspelled flag, or a stray positional token — by printing usage guidance to standard error and exiting `2`, never silently ignoring it. This SHALL hold for both the `check` and `list` commands, and matches how an unrecognized `--format` value is already handled, so that a typo such as `--write-baselin` fails loud rather than silently changing behavior. A value consumed by a recognized flag (e.g. the path after `--manifest-path`) SHALL NOT be treated as an unrecognized argument. Conversely, a value-taking flag (`--manifest-path`, `--baseline`, `--write-baseline`, `--format`) supplied with no following value SHALL also be a usage error that prints usage guidance and exits `2`, never a silent downgrade to a default or to a plain check.

#### Scenario: A value-taking flag with no value exits 2

- **WHEN** the runner is invoked as `check --manifest-path <path> --format` (or `--baseline` / `--write-baseline`) with no following value
- **THEN** it prints usage guidance and exits `2`, rather than defaulting the format or running an ordinary check

#### Scenario: An unknown flag exits 2

- **WHEN** the runner is invoked as `check --manifest-path <path> --frobnicate`
- **THEN** it prints usage guidance and exits `2`

#### Scenario: A misspelled flag fails loud instead of being ignored

- **WHEN** the runner is invoked as `check --manifest-path <path> --write-baselin <file>` (a misspelling of `--write-baseline`)
- **THEN** it prints usage guidance and exits `2`, rather than running an ordinary check and writing no baseline

#### Scenario: A stray positional token exits 2

- **WHEN** the runner is invoked as `check stray --manifest-path <path>`
- **THEN** it prints usage guidance and exits `2`

#### Scenario: An unknown flag to list exits 2

- **WHEN** the runner is invoked as `list --bogus`
- **THEN** it prints usage guidance and exits `2`

