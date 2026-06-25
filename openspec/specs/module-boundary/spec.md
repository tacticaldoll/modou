# module-boundary Specification

## Purpose

Govern the intra-crate module import graph that Cargo cannot see — the
differentiated value over `cargo tree` / `cargo-deny`. A module boundary forbids
one module from importing another ("the kernel must not import a projection"),
observed from the target crate's source `use` declarations (use-only, file-based;
see the scanner decision in `PROJECT.md`). Module violations flow through severity
and the baseline exactly like crate violations.

## Requirements
### Requirement: Module boundary declared in Rust

A module boundary SHALL be declared in Rust, targeting a crate and a module path within it and forbidding an import of another module path. It SHALL be declared as `ModuleBoundary::in_crate("app").module("crate::kernel").must_not_import("crate::projection").because("…")`, and SHALL carry a severity (default enforce, `warn` available) like a crate boundary. The umbrella `Boundary` SHALL accept both crate and module boundaries.

#### Scenario: Module boundary holds its target, module, and forbidden import

- **WHEN** a developer declares `ModuleBoundary::in_crate("app").module("crate::kernel").must_not_import("crate::projection").because("…")`
- **THEN** the constitution holds a module boundary on crate `app`, governing module `crate::kernel`, forbidding imports of `crate::projection`, with a non-empty reason

### Requirement: Module imports observed from source use declarations

The system SHALL observe module imports by scanning the target crate's source `use` declarations. It SHALL resolve `crate`, `self`, and `super` paths to absolute module paths, expand grouped (`{a, b}`) and glob (`::*`) forms, and ignore paths whose first segment is an external crate. Text inside comments and string literals SHALL NOT be treated as a `use` declaration: it is removed before scanning, so neither a `//` inside a string nor a `use …;` written inside a string affects the result. Bare path expressions and macro-generated imports SHALL be out of scope (see the scanner decision in `PROJECT.md`); the rule enforces only what real `use` declarations observe. Comments and string literals — normal, byte, and raw — SHALL be removed before scanning. Modules SHALL be file-based, and a governed module path that matches no source file SHALL be a constitution error (exit 2), never a silent pass. A governed source file that exists but cannot be read SHALL likewise be a scan error (exit 2), never silently skipped — an unreadable file is "cannot judge", not "nothing to judge", and skipping it could hide a real violation.

#### Scenario: A grouped use of crate paths is observed

- **WHEN** a file in the governed module declares `use crate::projection::{A, B};`
- **THEN** both `crate::projection::A` and `crate::projection::B` are observed as imports of `crate::projection`

#### Scenario: An external import is ignored

- **WHEN** a file declares `use serde::Deserialize;`
- **THEN** the system does not treat it as an internal module import

#### Scenario: A use written inside a string literal is not observed

- **WHEN** a file contains a string literal whose text is `use crate::projection::Thing;`, and no real `use` of that path
- **THEN** the system does not observe an import of `crate::projection`

#### Scenario: A string containing `//` does not hide a real use

- **WHEN** a file declares a string literal containing `//` followed, later on the same line, by a real `use crate::projection::Thing;`
- **THEN** the system observes the import `crate::projection::Thing`

#### Scenario: An unknown governed module is a constitution error

- **WHEN** a module boundary governs a module path that matches no source file in the crate
- **THEN** the system reports a constitution error and exits 2

#### Scenario: An unreadable governed source file is a scan error

- **WHEN** a governed module resolves to a source file that exists but cannot be read
- **THEN** the system reports a scan error naming the file and exits 2, rather than skipping the file

### Requirement: Forbidden module import is a violation

The system SHALL emit a violation when a file in the governed module imports the forbidden module or any module beneath it. The violation SHALL name the governed module as its target and the offending import path as its finding, and SHALL react according to its severity (enforce fails, warn is advisory) and any baseline, exactly as a crate violation does.

#### Scenario: Kernel importing projection violates

- **WHEN** a file in `crate::kernel` declares `use crate::projection::Thing;` and the boundary forbids importing `crate::projection`
- **THEN** the system emits a violation naming `crate::kernel` and the import `crate::projection::Thing`, and exits 1 at enforce severity

#### Scenario: The allowed direction is clean

- **WHEN** the boundary forbids `crate::kernel` from importing `crate::projection`, and only `crate::projection` imports `crate::kernel`
- **THEN** the system reports no violation for that boundary

