# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1] - 2026-06-27

### Added

- **Workspace-aware crate rule** — `restrict_workspace_dependencies_to([...])` and the
  `forbid_all_workspace_dependencies()` shorthand govern only the target's dependencies
  on other workspace members, with membership derived from `cargo metadata`, so a newly
  added workspace crate is governed by default.
- **Module closed-allowlist rule** — `restrict_imports_to([...])` governs which internal
  modules the target module may import: anything off the allowlist (and outside its own
  subtree) is a violation. The module-level mirror of `restrict_dependencies_to`, JSON
  key `only`. Declaring it on `crate` is a self-describing constitution error (exit 2).
- **Module inbound rule** — `must_not_be_imported_by(x)` governs the complementary
  direction (encapsulation — "who may reach in"), naming the offending importer.
  Declaring it on `crate` is a self-describing constitution error (exit 2).
- **Workspace coverage reporting** — `check` reports how many workspace crates have no
  boundary (an always-on line and a `coverage` field under `--format json`); the opt-in
  `--warn-uncovered` flag raises each uncovered crate to a warn-severity advisory.
  Coverage never changes the exit code.
- **Selectable dependency kind** — a crate boundary may observe `[dev-dependencies]` or
  `[build-dependencies]` via `.dependency_kind(kind)`; the default `Normal` leaves
  existing constitutions unaffected.

### Changed

- `check` defaults to the nearest `Cargo.toml` (cargo-style) when `--manifest-path` is
  omitted, instead of erroring; the cannot-evaluate → exit `2` invariant is preserved.
- All public value types (`Constitution`, the boundaries and rules, `Violation`,
  `Report`, `Outcome`) uniformly derive `Clone, PartialEq, Eq`.
- The text `check` report is emitted on stderr as a single stream; stdout is reserved
  for the `--format json` document and the `list` projection.
- Module violations are emitted in a deterministic order, independent of filesystem
  directory-read order.
- `check` reads `cargo metadata` once per run.
- Coverage is emitted only when the constitution evaluates successfully (omitted on a
  constitution error).
- The "no `Cargo.toml` found" scan error names the directory the search started from.

### Fixed

- A boundary deduplicates its violations by identity `(target, rule, finding)`, per
  boundary at the point findings are produced — a module subtree spanning several files
  (or a lib+bin `crate`) no longer double-reports.
- Only files reachable from the crate root via `mod` declarations are governed; an
  undeclared orphan file is not compiled by Rust, so it is no longer treated as a module
  and its imports are not observed.
- A root-relative bare `use foo::…` resolves to a crate-root module only when the
  importing file is the crate root; in a submodule, and for any leading-`::` path, the
  import stays external — matching the compiler.
- Raw identifiers (`r#name`) are canonicalized across a module's file path, its `mod`
  declaration, and `use` paths, so `mod r#type;` (compiled to `type.rs`) is governable.
- A `use` or `mod` written inside a macro body (a `macro_rules!` definition or a macro
  invocation) is out of scope — the body is stripped before scanning.
- A `use` is attributed to the inline `mod { … }` that encloses it, so `self`/`super`
  resolve against that submodule rather than the file's module.
- Comment, string, and char-literal stripping is robust, including an escaped `'\''`
  and Unicode identifiers that share a keyword prefix (e.g. `use貓`).
- An unreadable governed source directory is a scan error (exit 2), not a silent skip.
- A module boundary targeting an inline `mod name { … }` (reachable but file-less) fails
  with a self-describing constitution error, distinct from the unknown-module (typo) error.
- `list` rejects check-only flags (`--manifest-path`, `--baseline`, `--write-baseline`,
  `--warn-uncovered`) as a usage error instead of silently exiting 0.
- In gate mode a constitution error is reported before the baseline is read, so a missing
  or unreadable baseline cannot mask it.

### Security

- `.github/CODEOWNERS` protects `crates/modou/tests/self_governance.rs` (Modou's
  self-governing constitution) and `deny.toml`, not only the demo constitution — the
  boundary whose edit would turn Modou's own CI green after drift is steward-owned.

## [0.1.0] - 2026-06-25

### Added

- Initial release. A Rust-declared constitution with crate-dependency boundaries
  (deny-external with an optional allowlist, forbid-dependency-on, restrict-to) and
  module-import boundaries, boundary severity (`warn` / `enforce`), a baseline gate,
  the `check` and `list` commands with `--format json`, the reusable `modou::run`
  library entry point, and the `0` / `1` / `2` exit-code reaction contract.
