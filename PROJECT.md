# Project Contract

Modou's orientation layer for humans and AI agents. Keep it short and concrete.

## Purpose

Modou is a Rust-native **reactive governance** framework. It does not run the
app or instruct the agent; it lets developers and agents propose change, then uses
compiler, CI, and (later) runtime *reactions* to keep architectural shape from
drifting. The source of truth is Rust code; TOML, Markdown, and reports are
projections of it.

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
  allowlist). Module rule: `MustNotImport`.
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
  macro-free; its partial coverage — bare path expressions and macro-generated
  imports are out of scope — is acceptable because the drift law only enforces what
  is observed. Comments and string literals (normal, byte, and raw) are stripped so
  their text is never mistaken for a `use`.
- **The amendment flow is harness convention, not a Modou feature.** Modou cannot
  tell shape drift from policy drift (not an observable fact), and generating PRs or
  gating merges is orchestration it must not own. Instead the constitution
  (`crates/modou/src/constitution.rs`) and the living specs (`openspec/specs/`) are
  assigned to the steward in `.github/CODEOWNERS`; once the repository enables
  required Code Owner review on the default branch, changing the law requires steward
  approval, so an agent cannot silently weaken a boundary to make CI pass. Until that
  branch-protection setting is on, the designation only auto-requests review.
- **Modou governs itself.** Modou's own invariants are enforced by its own reaction,
  not just documented — `tests/self_governance.rs` runs `check` against Modou's own
  manifest and source: a `restrict_dependencies_to(["serde_json"])` crate boundary
  keeps it dependency-light (CI fails the moment a new external dependency creeps
  in), and a `module("crate::engine").must_not_import("crate::runner")` module
  boundary enforces the functional-core / imperative-shell split. A governance tool
  earns trust by eating its own dog food.
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
