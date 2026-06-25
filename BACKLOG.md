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
worth breaking a core non-goal to ship it in v0.1.

### Runtime drift — *opt-in*
Observation source: execution events, grant checks, the audit log. Catches e.g. an
unauthorized capability call, a runtime action missing an audit record, or a
non-idempotent action being retried / durably executed. Opt-in; never a workflow
engine or app runner.

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

## Explicitly not on the roadmap

A universal `ProposedLink` / `GraphKind` / one graph API across phases; an app
runner; an agent framework; expressing the constitution in TOML / YAML. Each phase
keeps its own observation source.
