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

The system SHALL observe module imports by scanning the target crate's source `use` declarations. It SHALL resolve `crate`, `self`, and `super` paths to absolute `crate::…` module paths, expand grouped (`{a, b}`) and glob (`::*`) forms, and ignore paths whose first segment is an external crate. A first segment that names a crate-root module SHALL be resolved to `crate::…` **only when the importing file is the crate root**: there a sibling `mod` is in scope and shadows the extern prelude, so a bare `use foo::…` is the local module. In a submodule a bare first segment reaches only the extern prelude — it is an external crate, or a compile error — and SHALL be treated as external, even when a crate-root module of that name exists. The crate-root module names used for this resolution SHALL be observed from the crate's own source as **declared modules** — a `mod name;` or `mod name { … }` declaration in the crate-root file(s) — not from the mere existence of a like-named source file: an undeclared orphan source file (e.g. a stray `src/foo.rs` that no `mod foo;` declares) does NOT make its name a crate-root module, because Rust does not bring an undeclared file into scope and a bare `use foo::…` then resolves through the extern prelude. A path written with a leading `::` (`use ::name::…`) is the explicit external/global form and SHALL be treated as external even when its first segment matches a crate-root module. Text inside comments and string literals SHALL NOT be treated as a `use` (or `mod`) declaration: it is removed before scanning, so neither a `//` inside a string nor a `use …;` written inside a string affects the result. Bare path expressions and macro-generated imports SHALL be out of scope (see the scanner decision in `PROJECT.md`); the rule enforces only what real `use` declarations observe. In particular, a `use` written inside a macro body — a `macro_rules!` definition OR a macro invocation (`ident! {…}` / `(…)` / `[…]`) — is a macro-generated import: the `macro_rules!` definition (its name and balanced body) and any macro invocation's balanced `{}`/`()`/`[]` body are removed before scanning, so such a `use` SHALL NOT be observed. Comments and string literals — normal, byte, and raw — SHALL be removed before scanning. Modules SHALL be file-based **and reachable from the crate root via `mod` declarations**: a source file that no `mod` declaration brings into scope — an undeclared orphan, at the crate root or anywhere in a subtree — is not a module of the crate, is not governed, and its imports SHALL NOT be observed, matching the compiler (which never compiles it). A governed module path that matches no reachable source file SHALL be a constitution error (exit 2), never a silent pass. A governed source file that exists but cannot be read SHALL likewise be a scan error (exit 2), never silently skipped — an unreadable file is "cannot judge", not "nothing to judge", and skipping it could hide a real violation. A governed source directory that cannot be traversed SHALL likewise be a scan error (exit 2), naming the directory, never silently skipped — the same "cannot judge, not nothing to judge" rule, because a skipped subtree could hide a real violation.

#### Scenario: A grouped use of crate paths is observed

- **WHEN** a file in the governed module declares `use crate::projection::{A, B};`
- **THEN** both `crate::projection::A` and `crate::projection::B` are observed as imports of `crate::projection`

#### Scenario: A root-relative bare use of a declared local module is observed

- **WHEN** a file at the crate root declares `use kernel::Thing;` and the crate root declares `mod kernel;` (so `kernel` is a crate-root module of the target crate)
- **THEN** the system observes the import `crate::kernel::Thing`, rather than dropping it as an external crate

#### Scenario: An undeclared orphan source file does not create a crate-root module

- **WHEN** a file at the crate root declares `use serde::Deserialize;`, `serde` is an external crate, and a source file `src/serde.rs` exists that no `mod serde;` declaration brings into scope
- **THEN** the system treats the import as external and does NOT observe `crate::serde::Deserialize`, because an undeclared orphan file is not a crate-root module

#### Scenario: An undeclared orphan submodule file is not governed

- **WHEN** a crate declares `mod kernel;`, the file `src/kernel/orphan.rs` exists that `kernel` never declares with `mod orphan;`, that orphan file contains `use crate::projection::Thing;`, and a boundary governs `crate::kernel` forbidding `crate::projection`
- **THEN** the system reports no violation, because only files reachable from the crate root via `mod` declarations are modules of the crate — the orphan file is not compiled, is not governed, and its import is not observed

#### Scenario: A bare use in a submodule is external even when it matches a crate-root module

- **WHEN** a file in a submodule (not the crate root) declares `use serde::Deserialize;` and `serde` is also a crate-root module of the target crate
- **THEN** the system treats the import as external and does not observe `crate::serde::Deserialize`, because a submodule's bare first segment reaches only the extern prelude

#### Scenario: A leading-colon path is external even when its head is a crate-root module

- **WHEN** a file declares `use ::serde::Deserialize;` and `serde` is also a crate-root module of the target crate
- **THEN** the system treats the import as external and does not observe `crate::serde::Deserialize`, because the leading `::` is the explicit external/global form

#### Scenario: An external import is ignored

- **WHEN** a file declares `use serde::Deserialize;` and `serde` is not a crate-root module of the target crate
- **THEN** the system does not treat it as an internal module import

#### Scenario: A use written inside a string literal is not observed

- **WHEN** a file contains a string literal whose text is `use crate::projection::Thing;`, and no real `use` of that path
- **THEN** the system does not observe an import of `crate::projection`

#### Scenario: A use written inside a macro_rules body is not observed

- **WHEN** a file declares `macro_rules! m { () => { use crate::projection::Thing; }; }` and no real `use` of that path outside the macro
- **THEN** the system does not observe an import of `crate::projection`, because the `macro_rules!` body is a macro-generated import and is removed before scanning

#### Scenario: A use written inside a macro invocation body is not observed

- **WHEN** a file declares `some_macro! { use crate::projection::Thing; }` and no real `use` of that path outside the macro
- **THEN** the system does not observe an import of `crate::projection`, because a macro invocation body is a macro-generated import and is removed before scanning

#### Scenario: A string containing `//` does not hide a real use

- **WHEN** a file declares a string literal containing `//` followed, later on the same line, by a real `use crate::projection::Thing;`
- **THEN** the system observes the import `crate::projection::Thing`

#### Scenario: An unknown governed module is a constitution error

- **WHEN** a module boundary governs a module path that matches no reachable source file in the crate
- **THEN** the system reports a constitution error and exits 2

#### Scenario: An unreadable governed source file is a scan error

- **WHEN** a governed module resolves to a source file that exists but cannot be read
- **THEN** the system reports a scan error naming the file and exits 2, rather than skipping the file

#### Scenario: An unreadable governed source directory is a scan error

- **WHEN** a governed module's source subtree contains a directory that cannot be traversed
- **THEN** the system reports a scan error naming the directory and exits 2, rather than skipping the subtree

### Requirement: Forbidden module import is a violation

The system SHALL emit a violation when a file in the governed module imports the forbidden module or any module beneath it. The violation SHALL name the governed module as its target and the offending import path as its finding, and SHALL react according to its severity (enforce fails, warn is advisory) and any baseline, exactly as a crate violation does.

#### Scenario: Kernel importing projection violates

- **WHEN** a file in `crate::kernel` declares `use crate::projection::Thing;` and the boundary forbids importing `crate::projection`
- **THEN** the system emits a violation naming `crate::kernel` and the import `crate::projection::Thing`, and exits 1 at enforce severity

#### Scenario: The allowed direction is clean

- **WHEN** the boundary forbids `crate::kernel` from importing `crate::projection`, and only `crate::projection` imports `crate::kernel`
- **THEN** the system reports no violation for that boundary

### Requirement: Module imports restricted to a closed allowlist

A module boundary SHALL support a closed-allowlist rule restricting which internal modules the governed module may import: `ModuleBoundary::in_crate(p).module(m).restrict_imports_to([...]).because(...)`. Any internal `use` from the governed module to a module that is neither within the governed module's own subtree (`m` or beneath, i.e. `m` or a path beginning `m::`) nor within an allowlist entry (an entry or beneath, i.e. the entry or a path beginning `entry::`) SHALL be a violation; an empty allowlist forbids every outward internal import (only the module's own subtree is permitted). The "or beneath" test SHALL be `::`-delimited, so an allowlist entry `crate::types` does not cover a sibling `crate::types_extra`. Relative imports (`self::`/`super::`) SHALL be resolved to absolute paths before the check. External imports SHALL remain out of scope. The rule SHALL carry severity (default enforce, `warn` available) and flow through the baseline exactly as `must_not_import`. Like the crate-level restrict-to, its JSON projection SHALL use the key `only`. Declaring the rule on `crate` itself SHALL be a constitution error (exit 2), self-describing and distinct from a violation, because the crate root has no outward internal edge to observe.

#### Scenario: An internal import outside the allowlist violates

- **WHEN** the governed module `crate::kernel` declares `use crate::io::Sink;` and the boundary is `restrict_imports_to(["crate::types"])`
- **THEN** the system emits a violation naming `crate::kernel` and the import `crate::io::Sink`, and exits 1 at enforce severity

#### Scenario: An allowlisted import is clean

- **WHEN** `crate::kernel` imports only `crate::types::Id` and the boundary is `restrict_imports_to(["crate::types"])`
- **THEN** the system reports no violation for that boundary

#### Scenario: The governed module's own subtree is allowed without listing

- **WHEN** `crate::kernel` declares `use crate::kernel::detail::Thing;` and the boundary is `restrict_imports_to(["crate::types"])`
- **THEN** the system reports no violation, because a module importing its own subtree is not an outward edge

#### Scenario: An empty allowlist forbids every outward internal import

- **WHEN** `crate::kernel` imports `crate::types::Id` and the boundary is `restrict_imports_to([])`
- **THEN** the system emits a violation for `crate::types::Id`, because the empty allowlist permits only the module's own subtree

#### Scenario: A prefix-colliding sibling of an allowlist entry violates

- **WHEN** `crate::kernel` declares `use crate::types_extra::Y;` and the boundary is `restrict_imports_to(["crate::types"])`
- **THEN** the system emits a violation, because the "or beneath" test is `::`-delimited: `crate::types_extra` is neither `crate::types` nor beneath `crate::types::`

#### Scenario: An external import is never flagged, even under an empty allowlist

- **WHEN** `crate::kernel` declares `use serde::Deserialize;` and the boundary is `restrict_imports_to([])`
- **THEN** the system reports no violation, because external imports are out of scope (only internal `crate::…` edges are observed)

#### Scenario: A `super::`-reaching-outward import is governed

- **WHEN** `crate::kernel::inner` declares `use super::super::other::Thing;` (resolving to `crate::other::Thing`, outside the governed subtree and the allowlist) and the boundary is `restrict_imports_to(["crate::types"])` on `crate::kernel`
- **THEN** the system emits a violation, because relative imports are resolved to absolute paths and an outward edge is governed regardless of how it was written

#### Scenario: Importing the governed module itself or via `self::` is clean

- **WHEN** `crate::kernel` declares `use self::detail::Thing;` (resolving to `crate::kernel::detail::Thing`) and the boundary is `restrict_imports_to(["crate::types"])`
- **THEN** the system reports no violation, because the import is within the governed module's own subtree

#### Scenario: A raw-identifier allowlist entry is canonicalized

- **WHEN** the boundary is `restrict_imports_to(["crate::r#type"])` and `crate::kernel` declares `use crate::type::Thing;`
- **THEN** the system reports no violation, because allowlist entries are canonicalized (`r#type` and `type` are one module) exactly like the governed and forbidden paths

#### Scenario: Declaring the rule on `crate` itself is a constitution error

- **WHEN** a boundary declares `restrict_imports_to([...])` on `crate` (the crate root)
- **THEN** the system emits a self-describing constitution error and exits 2 — naming `crate` and that it has no outward internal edge — distinct from a boundary violation, never a silent pass

### Requirement: A module may forbid being imported by another module

A module boundary SHALL support an inbound rule: `ModuleBoundary::in_crate(p).module(m).must_not_be_imported_by(x).because(...)` declares that the protected module `m` must not be imported by module `x` or anything beneath it. The system SHALL observe this from the crate's source `use` declarations across all reachable files: a file whose enclosing module is `x` or beneath `x` that imports `m` or anything beneath `m` SHALL be a violation, naming `m` as the target and the offending importing module as the finding. The "or beneath" test SHALL be `::`-delimited on both sides, so a forbidden importer `crate::http` does not match a sibling `crate::httpx`, and a protected `crate::internal` does not match a sibling `crate::internal_util`. A file whose enclosing module is `m` or beneath `m` SHALL NOT be treated as an importer — a module importing its own subtree is not an inbound edge — so the rule never flags `m`'s own files even when `x` is an ancestor of `m`. External imports SHALL remain out of scope. The finding SHALL be the importing module path, deduplicated so one offending importer yields one violation per protected target; the rule SHALL carry severity (default enforce, `warn` available) and flow through the baseline like the other module rules. The protected module `m` SHALL be a reachable file-based module, with the same inline/unknown constitution-error handling as other module targets. Declaring the rule with `m` = `crate` (the crate root) SHALL be a constitution error (exit 2), self-describing and distinct from a violation, because every internal import is then "m or beneath" and the rule could never react as an inbound rule.

#### Scenario: An import from the forbidden importer violates

- **WHEN** `crate::internal` is protected by `must_not_be_imported_by("crate::http")` and a file in `crate::http` declares `use crate::internal::Secret;`
- **THEN** the system emits a violation naming `crate::internal` as the target and the offending importer `crate::http`, and exits 1 at enforce severity

#### Scenario: An import from outside the forbidden importer is clean

- **WHEN** the same boundary holds and only `crate::core` (not beneath `crate::http`) imports `crate::internal`
- **THEN** the system reports no violation for that boundary

#### Scenario: The rule applies beneath the importer

- **WHEN** a file in `crate::http::v1` declares `use crate::internal::Secret;` and the boundary is `must_not_be_imported_by("crate::http")`
- **THEN** the system emits a violation naming the importer `crate::http::v1`, because it is beneath the forbidden importer

#### Scenario: The rule applies beneath the protected module

- **WHEN** a file in `crate::http` declares `use crate::internal::deep::Thing;` and the boundary is `must_not_be_imported_by("crate::http")` protecting `crate::internal`
- **THEN** the system emits a violation, because the imported path is beneath the protected module

#### Scenario: A prefix-colliding importer sibling is clean

- **WHEN** a file in `crate::httpx` declares `use crate::internal::Secret;` and the boundary is `must_not_be_imported_by("crate::http")`
- **THEN** the system reports no violation, because `crate::httpx` is neither `crate::http` nor beneath `crate::http::`

#### Scenario: A prefix-colliding protected sibling is clean

- **WHEN** a file in `crate::http` declares `use crate::internal_util::X;` and the boundary protects `crate::internal` via `must_not_be_imported_by("crate::http")`
- **THEN** the system reports no violation, because `crate::internal_util` is neither `crate::internal` nor beneath `crate::internal::`

#### Scenario: The protected module's own subtree is not an importer

- **WHEN** `crate::a::b` is protected by `must_not_be_imported_by("crate::a")` and a file in `crate::a::b` declares `use crate::a::b::detail::Thing;`
- **THEN** the system reports no violation, because a file within the protected module is not an inbound importer even though it is beneath the forbidden importer `crate::a`

#### Scenario: An external import is ignored

- **WHEN** a file in `crate::http` declares `use serde::Deserialize;` and the boundary is `must_not_be_imported_by("crate::http")` protecting `crate::internal`
- **THEN** the system reports no violation, because external imports are out of scope

#### Scenario: Forbidding the crate root as importer forbids every outside importer

- **WHEN** `crate::internal` is protected by `must_not_be_imported_by("crate")` and `crate::http` imports `crate::internal`
- **THEN** the system emits a violation naming `crate::http`, because every module outside `crate::internal`'s own subtree is beneath `crate`; `crate::internal`'s own files remain clean

#### Scenario: Protecting the crate root is a constitution error

- **WHEN** a boundary declares `must_not_be_imported_by(x)` on `crate` (the crate root)
- **THEN** the system emits a self-describing constitution error and exits 2 — distinct from a boundary violation, never a silent pass — because every internal import would match and the rule could never react as an inbound rule

### Requirement: A boundary reports each violation once

A module boundary SHALL report each distinct violation at most once: its violations SHALL be deduplicated by identity `(target, rule, finding)`. When the governed module's subtree spans multiple source files that produce the same finding — a parent and a child file importing the same path, or a module backed by both `lib.rs` and `main.rs` (which both resolve to `crate`) — the system SHALL emit a single violation, not one per file. Deduplication SHALL be performed per boundary at the point findings are produced, so a duplicate arising from any other source is not silently suppressed.

#### Scenario: A finding produced by two files in the governed subtree is reported once

- **WHEN** the governed module `crate::kernel` spans `kernel.rs` and `kernel/sub.rs`, both declaring `use crate::forbidden::Thing;`, and the boundary forbids `crate::forbidden`
- **THEN** the system emits exactly one violation for `crate::forbidden::Thing`, not two

#### Scenario: Identical findings collapse but distinct findings are kept

- **WHEN** one file in the governed subtree imports `crate::forbidden::A` and another imports both `crate::forbidden::A` and `crate::forbidden::B`, under a boundary forbidding `crate::forbidden`
- **THEN** the system emits exactly two violations — `crate::forbidden::A` (once) and `crate::forbidden::B` — never a duplicate of `A`

### Requirement: Raw identifiers in module paths are canonicalized

The system SHALL treat a raw identifier (`r#name`) and its plain form (`name`) as the same module segment when observing module declarations, module file paths, and `use` paths, and when comparing them against a boundary's declared module and forbidden paths. Because Rust resolves `mod r#name;` to the source file `name.rs`, the file-derived path and the declaration must canonicalize to the same module identity; a boundary MAY be declared with either form and SHALL match the observed module regardless of which form the source uses. A module whose name is a raw identifier SHALL therefore be governable, and a forbidden import written with a raw identifier SHALL be observed.

#### Scenario: A raw-identifier module is governed and its imports observed

- **WHEN** a crate declares `mod r#type;` (resolving to `src/type.rs`), that file imports `use crate::r#mod::Thing;`, and a boundary governs `crate::type` forbidding `crate::mod`
- **THEN** the module `crate::type` is found (not an unknown-module constitution error) and the import `crate::mod::Thing` is observed as a violation, the raw and plain forms having been canonicalized to one identity

### Requirement: Imports are attributed to their enclosing inline module

The system SHALL attribute each `use` declaration to the module that lexically encloses it, including an inline `mod name { … }` submodule, rather than to the containing file's module. A `self`/`super` path SHALL be resolved against that enclosing module, and a bare first segment inside an inline submodule SHALL be treated as external even when the file itself is the crate root, matching how the compiler resolves it. A `mod name;` declaration with no inline body does not enclose any `use` and SHALL NOT change attribution.

#### Scenario: A self import inside an inline submodule resolves against that submodule

- **WHEN** the crate-root file declares `mod inner { use self::leaf::Thing; }`
- **THEN** the import is observed as `crate::inner::leaf::Thing`, not `crate::leaf::Thing`, because it is attributed to the enclosing inline module `crate::inner`

### Requirement: Module declarations inside macro bodies are not observed

The system SHALL NOT observe a `mod` declaration written inside a macro body — a `macro_rules!` definition or a macro invocation (`ident! {…}` / `(…)` / `[…]`) — as a real module of the crate, the same out-of-scope rule already applied to a `use` inside a macro body. The macro body SHALL be removed before scanning for `mod` declarations, so a `mod` token inside a `()`/`[]`-delimited macro invocation is not mistaken for a crate-root module declaration.

#### Scenario: A mod inside a macro invocation is not a declared module

- **WHEN** the crate-root file declares `some_macro!( mod ghost; );` and no real `mod ghost;` outside the macro
- **THEN** the system does not treat `crate::ghost` as a declared, reachable module, so a bare `use ghost::…` elsewhere stays external

### Requirement: A governed target is a file-based module

A module boundary's governed target SHALL be a file-based module — one backed by a source file reachable from the crate root via `mod` declarations. An inline module (declared with a body, `mod name { … }`, rather than its own file) is reachable for import attribution but owns no source file, and SHALL NOT be a governable target. When a boundary targets a module path that is reachable but file-less (inline), the system SHALL report a constitution error (exit 2) that is self-describing — naming the inline cause and the file-based-target rule — distinct from the unknown-module error used when the path is not reachable at all (e.g. a typo). Both are constitution errors and exit 2; neither is a silent pass.

#### Scenario: An inline module target is a self-describing constitution error

- **WHEN** a crate-root file declares `mod kernel { use crate::projection::Thing; }` and a boundary governs `crate::kernel` forbidding `crate::projection`
- **THEN** the system reports a constitution error (exit 2) explaining that `crate::kernel` is declared inline and owns no source file, so module boundaries — which govern file-based modules — cannot target it, rather than reporting it as an unknown module

#### Scenario: A genuinely unknown module is still reported as not found

- **WHEN** a boundary governs a module path that is not reachable in the crate at all (e.g. a typo)
- **THEN** the system reports a constitution error (exit 2) that the module was not found among the crate's reachable modules

### Requirement: Path-remapped modules are out of scope

The system SHALL treat a module declared with a `#[path = "…"]` attribute as out of scope: its imports SHALL NOT be observed, and it SHALL NOT be a governable target. Module identity is observed from the conventional file path (`lib.rs`/`main.rs` → `crate`, `kernel/foo.rs` → `crate::kernel::foo`), and a `#[path]` attribute relocates the file away from that path, so a remapped module is not mapped to its declaration — a stated partial-coverage bound of the same family as inline and macro-generated items (see the scanner decision in `PROJECT.md`). The drift law enforces only what is observed; closing this gap would require reading attributes (an AST-class amendment), so it is stated, not silently relied upon.

#### Scenario: A path-remapped module's imports are not observed

- **WHEN** a crate declares `#[path = "weird/place.rs"] mod foo;` and `weird/place.rs` contains `use crate::other::Thing;`
- **THEN** the system does not observe an import of `crate::other` for `crate::foo`, because the module's file is not at its conventional path

#### Scenario: A path-remapped module is not a governable target

- **WHEN** a boundary targets `crate::foo`, declared via `#[path = "weird/place.rs"] mod foo;`
- **THEN** the system reports a constitution error (exit 2), because no conventionally-pathed source file backs the target — never a silent pass

