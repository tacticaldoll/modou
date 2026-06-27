# Project Contract

Modou's orientation layer for humans and AI agents. Keep it short and concrete.

## Purpose

Modou is a Rust-native **reactive governance** framework. It does not run the
app or instruct the agent; it lets developers and agents propose change, then uses
compiler, CI, and (later) runtime *reactions* to keep architectural shape from
drifting. The source of truth is Rust code; TOML, Markdown, and reports are
projections of it.

*Why reaction, not instruction:* architectural intent once lived in human
understanding and review, but an AI agent writes fluent code without holding it, so
the intent is crystallized into a non-bypassable reaction rather than entrusted to
instruction — neither the agent nor Modou needs to understand for the law to hold;
the understanding is front-loaded into the human-authored constitution. Modou is to
architectural boundaries what `cargo-deny` is to the supply chain.

## Core Contract

The behavior protected first: **a declared boundary reacts.** A boundary declared
in Rust must produce a real, non-bypassable reaction when violated — for v0.1, a CI
failure with a non-zero exit and an explanatory report. The reaction MUST never
silently pass, and MUST distinguish a boundary violation (exit 1) from a
constitution error / misconfiguration (exit 2).

## How drift is judged

Drift is a **policy-aware diff**, not an AI judgment:

1. **Declare** the intended shape in Rust (the constitution).
2. **Observe** the real shape from the project (e.g. `cargo metadata`).
3. **Compare** observed against declared, per the rule.
4. **Classify**: pass / warning / violation.

Three rules keep this honest and stop the tool from becoming an opinionated linter
or a governance platform:

- **No declared shape, no drift.** If the constitution does not declare it, Modou
  does not report it.
- **No observable fact, no enforcement.** What cannot be observed from the codebase
  may at most be reported — never hard-failed.
- **No target type or drift type without a reaction and an observation source.**
  Names are not claimed for reactions that do not yet exist.

## Division of labor

> **Human owns the invariant. Modou owns the reaction. Agent owns the change.**

- The **human steward** decides the few shapes that must not drift, and accepts or
  rejects amendments to them.
- **Modou** observes the codebase, compares it against the constitution, reacts.
- The **agent** makes changes and repairs violations using the report. It must not
  edit the constitution to pass; a wrong boundary is fixed by a human-reviewed
  amendment, not by silently weakening the law.

## Keep the constitution small

The constitution is human-owned but **tiny**: only invariants that are high-value
(expensive if they drift), stable, observable, and reactive. Declaring one should
cost about as much as writing a doc comment — not an enterprise governance document.
Do not encode the whole architecture; encode only the few boundaries that must not
break.

## Terminology

- **Constitution** — the governed shape, declared in Rust; the single source of
  truth. A label, not a path.
- **Boundary** — the umbrella over `CrateBoundary` (a rule on a crate target) and
  `ModuleBoundary` (a rule on an intra-crate module). Each carries a human reason.
- **Rule** — what a boundary forbids. Crate rules: `DenyExternalDependencies` (with
  an optional allowlist), `ForbidDependencyOn`, `RestrictDependenciesTo` (a closed
  allowlist), `RestrictWorkspaceDependenciesTo` (a closed allowlist over workspace
  members only). Module rules: `MustNotImport` (forbid one outward import),
  `RestrictImportsTo` (a closed outward allowlist), `MustNotBeImportedBy` (forbid an
  inbound importer — who may reach *in*).
- **Reaction** — the control surface: a compiler / CI / runtime response. *Govern by
  reaction, not instruction.*
- **Drift** — a divergence between declared and observed shape.
- **Shape drift** — the code violates the constitution; repair the code.
- **Policy drift** — the constitution itself is outdated; propose an amendment.
- **Amendment** — a human-reviewed change to the constitution; agents propose, the
  steward decides.
- **Constitution error** — a misconfiguration (e.g. an unresolvable target),
  reported distinctly from a boundary violation.

## Decisions

The project's load-bearing decisions — the *why*, where the specs and code carry the
*what*. These were previously kept as per-decision ADRs under `docs/adr/`; they were
dissolved into this contract at the v0.1 baseline. Record future significant
decisions here.

- **Spec-driven development (OpenSpec).** The source of truth is `openspec/`; work
  flows explore → propose → apply → sync → archive. See `AGENTS.md`.
- **Baseline is a generated snapshot, not policy.** A baseline records accepted
  violations so a dirty project can adopt a boundary and gate only on *new* drift; it
  is a projection of the report, never the constitution.
- **Module imports are observed by scanning source `use` declarations**, not by
  parsing a full AST. A hand-rolled scanner keeps Modou dependency-light and
  macro-free; its partial coverage — bare path expressions, macro-generated
  imports, and `#[path = "…"]`-remapped modules are out of scope — is acceptable
  because the drift law only enforces what is observed. (A `#[path]` attribute moves a
  `mod name;` to a non-conventional file; the token scanner maps modules by their
  conventional path, so a remapped module's imports are not observed and the module is
  not governable — the same stated partial-coverage bound as inline and macro-generated
  items. Closing it would require reading attributes, an AST-class amendment, not a
  silent trade.) Comments and string literals (normal, byte, and raw) are stripped so
  their text is never mistaken for a `use`. A module's identity is derived in three
  places — its file path, its `mod` declaration, and a `use` path that names it — and
  these MUST stay in lockstep, since a divergence both fails to govern a real module
  and silently hides its imports (a false negative, the one thing the core contract
  forbids). Two consequences fall out and stay token-level, not parser-level, to keep
  the hand-rolled scanner: a raw identifier is canonicalized (`mod r#type;` compiles to
  `type.rs`, so `r#type` and `type` are one module), and a `use` is attributed to the
  inline `mod { … }` that encloses it (so `self`/`super` resolve correctly) — and macro
  bodies are stripped before scanning for `mod` declarations too, not just `use`s, so
  the out-of-scope rule for macro-generated items is symmetric. Adopting a real parser
  (`syn`) would resolve all of this for free but would break the dependency-light
  self-constitution; that is an amendment, not a silent trade. A boundary's governed
  *target* is file-based: an inline `mod name { … }` is reachable for import attribution
  but owns no file, so it cannot be a target — a boundary on one fails loud with a
  self-describing constitution error (exit 2), distinct from an unknown-module typo,
  never a silent pass. Governing inline modules as targets is a deliberate non-goal here
  (it would expand the reaction surface and is only partial while inline modules' own
  file-backed children are not walked); if ever wanted it is a separate amendment.
- **The module rule set mirrors the crate level, and a rule that could never react is
  a constitution error.** Beyond `must_not_import` (forbid one outward edge), a module
  may `restrict_imports_to` a closed outward allowlist — the govern-by-default shape the
  crate level already chose, so a newly added module is caught without editing a denylist
  — and `must_not_be_imported_by` an inbound importer, the complementary direction
  (encapsulation: who may reach *in*), observed from the same `use` scan. Two principles
  fall out. First, a rule whose target makes it un-reactive is a self-describing
  constitution error (exit 2), never a silent no-op: `restrict_imports_to` on `crate`
  (the root has no outward internal edge) and `must_not_be_imported_by` on `crate` (every
  internal import is then "the protected module or beneath") could never fire, so they
  fail loud — the same "no observable reaction → fail, never silently pass" rule as an
  inline target. Second, a boundary deduplicates its violations **per boundary at the
  point findings are produced** (a module subtree spans multiple files — and `lib.rs` +
  `main.rs` both resolve to `crate` — so one forbidden import can be found twice), *not*
  by a blanket report-wide pass: a report-wide dedup is a suppressor that would silently
  swallow an unexpected duplicate from any other source, against the fail-loud contract.
- **The amendment flow is harness convention, not a Modou feature.** Modou cannot
  tell shape drift from policy drift (not an observable fact), and generating PRs or
  gating merges is orchestration it must not own. Instead Modou's self-governing
  constitution (`crates/modou/tests/self_governance.rs`), the bundled demo constitution
  (`crates/modou/src/constitution.rs`), the supply-chain policy (`deny.toml`), and the
  living specs (`openspec/specs/`) are assigned to the steward in `.github/CODEOWNERS`
  — the protected path is the boundary whose edit would turn CI green after drift, not
  only a showcase of it; once the repository enables
  required Code Owner review on the default branch, changing the law requires steward
  approval, so an agent cannot silently weaken a boundary to make CI pass. Until that
  branch-protection setting is on, the designation only auto-requests review.
- **Modou governs itself.** Modou's own invariants are enforced by its own reaction,
  not just documented — `tests/self_governance.rs` runs `check` against Modou's own
  manifest and source: a `restrict_dependencies_to(["serde_json"])` crate boundary
  keeps it dependency-light (CI fails the moment a new external dependency creeps
  in), and a `module("crate::engine").must_not_import("crate::runner")` module
  boundary enforces the functional-core / imperative-shell split. A governance tool
  earns trust by eating its own dog food. Because this test *is* Modou's real
  self-law, it is steward-owned in `.github/CODEOWNERS` (alongside `deny.toml`), so an
  agent cannot relax a self-imposed boundary — e.g. widen the `serde_json`-only
  allowlist — to turn its own CI green; that is an amendment, not a silent edit.
- **Projections and the CLI are self-describing; misconfiguration fails loud.** A
  projection must say what it means without a side-channel (e.g. the restrict-to JSON
  key `only`, distinct from deny-external's `allowed`), and an unrecognized flag is a
  usage error, never silently ignored. This is a convention for humans and agents,
  not a Modou-enforced reaction (it has no observation source), and it is bounded by
  minimalism — *not* a licence for defensive over-foolproofing or enforcing
  unobservable facts. Two standing consequences: dependency rules match the **package
  name**, not a local alias; and a crate's **declared** normal dependencies are
  governed as declared, including `[target.'cfg(…)'.dependencies]` and `optional`
  ones (dev/build remain out of scope).
- **The public API is CLI-first; its frozen shape is chosen, not defaulted.** Modou's
  primary consumer is the CLI/CI reaction (an exit code and a human-readable report),
  with `modou::run` / `check` as a secondary library surface. Two deliberate freeze
  choices follow, recorded so a later change is a conscious one, not a silent regret.
  First, `Outcome::ConstitutionError` carries a **human-readable `String`**, not a
  structured error enum: the kinds are modeled internally (single-source message
  constructors) but collapsed at the boundary, because no consumer yet needs to *match*
  on the kind — adding speculative structure is the over-foolproofing the minimalism
  bound rejects (the same reasoning as the deferred report `version` field). If a real
  consumer ever needs matchable kinds, swapping the payload type is a breaking change,
  acceptable at a minor bump pre-1.0 and gated by amendment after. Second, the public
  *value* types (`Constitution`, the boundaries and rules, `Violation`, `Report`,
  `Outcome`) uniformly derive `Clone, PartialEq, Eq` so a library consumer can compare
  and store them — adding a derive is non-breaking, so this is a free win, not a freeze
  risk. Enums that are expected to grow are `#[non_exhaustive]` (`Rule`, `ModuleRule`,
  `Boundary`, `BoundaryKind`, `Outcome`); `DependencyKind` is deliberately *not* (it
  mirrors cargo's fixed normal/dev/build set, so users may match it exhaustively).
  `Severity` is `#[non_exhaustive]` even though it is a two-rung ladder today: keeping
  it open costs users an exhaustive match but lets a future rung (e.g. an off/info
  severity) ship without a break, and *removing* `#[non_exhaustive]` later is itself
  non-breaking — so open is the reversible default.

## Change Prioritization

When comparing possible changes, prefer the one that protects the core contract
earliest:

1. The core contract: a declared boundary produces a real, non-bypassable reaction
   and never silently passes.
2. Reaction fidelity and precision: low false positives; drift distinguished from
   misconfiguration.
3. Specified feature completeness for capabilities already declared in OpenSpec.
4. Operator and developer ergonomics.
5. New reaction phases (module, composition, capability, runtime) and integrations.

Do not claim a target type or name for a reaction that does not yet exist. Keep
enabling changes small and separate.
