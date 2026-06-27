# Backlog

Forward-looking work, deliberately deferred. Promote an item to an OpenSpec change
when you pick it up. Every future reaction obeys Modou's drift law:

> **No drift type without an observation source. No target type or name without a
> reaction.**

So nothing here is "designed" yet — these are reaction *phases* with their
observation sources named, not APIs. Each is unified with v0.1 at the **principle**
layer (reaction), never in one universal type.

## Deferred

Ordered by readiness, not just value: each later item needs the earlier ones or a
new prerequisite.

The amendment flow (shape vs. policy drift) shipped as harness convention —
CODEOWNERS + steward review, not a Modou feature; see the amendment-flow decision in
`PROJECT.md` and `AGENTS.md`.

### Feature-aware crate boundaries
Observation source: the existing `cargo metadata`, read feature-aware — the optional-dep
and feature→dependency tables, and/or `cargo metadata` resolved under specific feature
sets. Catches a boundary bypassed via `#[cfg(feature = "…")]`: e.g. "even with `test-utils`
enabled, `domain-core` must not depend on an HTTP crate." A refinement of the
crate-dependency reaction (same reaction, richer observation), not a new drift type.
Nearer-term than the phases below: it breaks no non-goal (no AST, no new dependency) — the
cost is feature-unification subtlety (cargo resolves features workspace-globally), to be
pinned when picked up.

### API-surface drift
Observation source: `cargo rustdoc --output-format json` — a standard compiler artifact
parsed with `serde_json`, no AST work by Modou, in the same "borrow the ecosystem's
standard product" spirit as `cargo metadata`. Catches a public interface leaking a
forbidden type: a `pub fn` returning `sqlx::Pool` / `reqwest::Client` from a crate whose
constitution forbids exposing it. Reaction: CI fail. Cost to weigh: rustdoc JSON is
nightly-only and its format is unstable (pin a format version; gate on a nightly
toolchain) — a real prerequisite, but it breaks no Modou non-goal.

### Capability drift
Observation source: capability declarations and their effect / risk / idempotency /
grant metadata. Catches e.g. a write capability missing risk metadata, or a
non-idempotent capability entering durable execution. Requires capability
declarations to exist first (they do not yet). Governance, not capability routing.

*The observation-source cost (explored and reweighed at the v0.1 baseline).* To be
real drift — declared intent vs. observed reality — the declarations must be
**anchored to the real effectful code**, not hand-written for the check; otherwise it
is a lint over a data structure, not a reaction, and does not earn the "drift" name
(no observable fact, no enforcement). Every anchoring costs a non-goal: a
`#[modou::capability(...)]` proc-macro (breaks *no procedural macros*, pulls
`syn`/`quote`); a `syn` AST scan of source (breaks dependency-light, which
`tests/self_governance.rs` enforces); or running a project-provided emitter (breaks
*Modou does not run the app*). The only no-cost form — capabilities hand-passed to
`check` as in-process data — is a lint, not drift (nothing independent can drift). So
this stays a named phase until the steward consciously amends whichever non-goal buys
the observation source (see the note below). Reweighed for the first release: not
worth breaking a core non-goal to ship it in v0.1. *A further, heavier candidate
observation source* — scanning compiler intermediate representation (HIR/MIR) or the
compiled symbol table to catch a hidden `std::fs` / `std::thread` call inside a module
declared "pure" — needs `rustc` internals (a rustc driver, unstable IR), breaking
dependency-light hardest of all; it is the costliest amendment, listed only for
completeness.

### Runtime drift — *opt-in*
Observation source: execution events, grant checks, the audit log. Catches e.g. an
unauthorized capability call, a runtime action missing an audit record, or a
non-idempotent action being retried / durably executed. Opt-in; never a workflow
engine or app runner. A concrete opt-in form is a lightweight `modou::assert_boundary!(obj)`
placed at an architectural entry point, checking an object's origin at runtime (via
`TypeId`) and panicking with a Modou-format JSON log on a violation — reaching `dyn Trait`
and DI-injected objects that static analysis cannot. Note a *declarative* `macro_rules!`
macro would not break the *no procedural macros* non-goal (unlike `#[modou::capability]`);
the runtime-panic layer is still a runtime policy surface to weigh.

> A tempting form for the two phases above — `#[modou::capability(...)]`
> proc-macros plus a runtime audit layer that panics — **conflicts with current
> non-goals** (no procedural macros, no runtime policy engine). Adopting it requires
> the steward to consciously amend those non-goals first (an amendment, per above).
> Until then these stay phases with their observation sources named, not built.

## Deferred, not a reaction phase

Forward-looking items that are not new reactions (so they sit outside the readiness
chain above):

- **A `version` field on the report / `list` JSON.** The baseline JSON carries a
  `version` because Modou reads it back; the report and constitution projections are
  one-way output with no consumer that needs versioning yet. Adding one now would be
  speculative defense (see the self-description decision in `PROJECT.md`); revisit
  when a real consumer needs to detect
  schema changes.

- **Governing inline-module targets.** A boundary's target is file-based; an inline
  `mod name { … }` is reachable for import attribution but owns no file, so targeting one
  is a self-describing constitution error (see the scanner decision in `PROJECT.md`).
  Supporting inline modules as targets is deferred, not a reaction phase: it expands the
  reaction surface for a niche case and is only *partial* until an inline module's
  file-backed children (`mod sub;` at brace depth > 0) are also walked. Revisit if a real
  adopter needs to govern a layer that lives inline rather than in its own file.

- **Editor / LSP reaction (shift-left).** The same observation (source `use` scan) and the
  same constitution, surfaced through a Language Server so an illegal `use` is red-lined in
  the editor as it is typed — extending the reaction *surface* from CI (and later runtime)
  to edit-time. Not a new drift type or observation source, so it sits outside the
  readiness chain; it is a (large) integration. High value for the agent-education thesis
  (an instant reaction the moment an AI writes a boundary-crossing import), weighed against
  keeping a long-running LSP server dependency-light.

- **Domain isolation over sub-graphs.** Grouping workspace crates into named domains and
  forbidding cross-domain dependencies — e.g. service A's crates must not depend on service
  B's internal crate. Observation source is the existing `cargo metadata` graph, and much
  is already expressible with `forbid_dependency_on` / `restrict_workspace_dependencies_to`;
  the only new part is the grouping. Explicitly **bounded**: a domain is a named *set of
  crate targets*, not the universal graph API that stays off the roadmap.

- **Baseline burn-down (debt decay).** A CI gate mode failing a PR unless it reduces the
  baselined-violation count by a threshold (e.g. `--require-baseline-reduction`), forcing
  technical debt to decay rather than merely freeze. Observation = the baseline and current
  report (both already observed); adds no drift type. **In tension** with two decisions, to
  resolve before building: the baseline is "a snapshot, not policy" (`PROJECT.md`), and
  Modou avoids becoming a "governance platform" — a reduction target makes the baseline
  carry policy. A bounded opt-in flag may fit; a debt-scheduling system does not.

## Explicitly not on the roadmap

A universal `ProposedLink` / `GraphKind` / one graph API across phases; an app
runner; an agent framework; expressing the constitution in TOML / YAML. Each phase
keeps its own observation source.
