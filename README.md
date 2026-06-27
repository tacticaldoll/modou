# 墨斗 / Modou

**語意引路,墨斗定界。** — *Let semantics guide. Let Modou mark the line.*

> A carpenter's 墨斗 (ink-line) snaps one straight reference line; anything off it
> is visibly off. Modou snaps the architectural line, then reacts when the code
> crosses it. **Govern by reaction, not instruction.**

Modou is a Rust-native reactive governance framework. It does not run your app
and it does not instruct your agent. Developers and agents propose change; Modou
uses compiler, CI, and (later) runtime *reactions* to keep architectural shape from
drifting.

## Thesis

- Governance is the framework.
- Reaction is the control surface.
- **Rust is the constitution** — TOML, Markdown, and reports are projections, not
  the source of truth. The compiler/CI reaction comes first.

## Why reaction, not instruction

Architectural intent — "the core must not depend on adapters" — used to live in
human understanding and code review. An AI agent writes fluent, locally-plausible
code without holding that intent, so it erodes the shape it does not understand, and
*instructing* it ("keep the core clean") cannot bind an agent that has no
understanding to follow. Modou's answer is not to give the agent understanding: it
crystallizes the human's intent into a **non-bypassable reaction**, so neither the
agent nor Modou needs to understand for the law to hold — the understanding is
front-loaded into the human-authored constitution. Modou is to architectural
boundaries what `cargo-deny` is to the supply chain: the same govern-by-reaction
discipline, on the layer Cargo cannot see.

## How drift is detected

Drift is a **policy-aware diff**, not an AI judgment: **declare** the intended shape
in Rust -> **observe** the real shape from the project -> **compare** ->
**classify** (pass / warning / violation). Modou never guesses whether a change
is "reasonable"; it only reacts when observed reality diverges from the declared
constitution.

## Division of labor

> **Human owns the invariant. Modou owns the reaction. Agent owns the change.**

The human steward keeps a *small* set of must-not-drift boundaries; Modou reacts
to violations; the agent repairs using the report. A wrong boundary is changed by a
human-reviewed amendment — never by weakening the constitution to make CI pass.

## A declared boundary

A Rust-declared boundary plus one CI reaction.

```rust
use modou::prelude::*;

fn constitution() -> Constitution {
    Constitution::new("example").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .because("example-core must stay dependency-light"),
    )
}
```

Four crate-dependency rules share one observation source (`cargo metadata`). The
external rule takes an optional allowlist; a second forbids a dependency on named
crates — external or an internal workspace path (crate-to-crate layering); a third
restricts the crate's dependencies to a *closed* allowlist (internal and external
alike — "may depend on only these"); a fourth restricts only the crate's
**workspace** dependencies, deriving the members from `cargo metadata` so a newly
added crate is governed by default:

```rust
CrateBoundary::crate_("example-core")
    .deny_external_dependencies()
    .allow_external(["serde"])
    .because("example-core may use serde, nothing else external");

CrateBoundary::crate_("core")
    .forbid_dependency_on(["adapters"])
    .because("core must not depend on adapters");

CrateBoundary::crate_("domain")
    .restrict_dependencies_to(["serde", "domain-types"])
    .because("the domain may depend on only serde and its own types");

CrateBoundary::crate_("backend")
    .restrict_workspace_dependencies_to(["core"])
    .because("a backend may depend on only the core workspace crate");
```

The two restrict rules differ in **scope**, on purpose: `restrict_dependencies_to`
governs *all* normal dependencies — external crates (`serde`, `tokio`) included — so
anything off its allowlist is a violation. `restrict_workspace_dependencies_to`
governs *only* dependencies on other workspace members and ignores external crates;
`forbid_all_workspace_dependencies()` is the empty-allowlist shorthand. Because
workspace membership is observed rather than hand-listed, adding a new workspace
crate cannot silently slip past the rule.

By default a crate rule observes the normal `[dependencies]` table. Append
`.dependency_kind(DependencyKind::Dev)` (or `Build`) to point any crate rule at the
`[dev-dependencies]` or `[build-dependencies]` table instead — so "a backend may not
pull another backend in as a normal dependency, but a dev-dependency is fine" is
expressible directly:

```rust
CrateBoundary::crate_("backend")
    .forbid_dependency_on(["other-backend"])
    .dependency_kind(DependencyKind::Dev)
    .because("a backend may not pull another backend in as a dev-dependency");
```

A boundary defaults to *enforce* (a violation fails CI). Mark it `.warn()` to make
it advisory — its violations are reported but do not fail — so a dirty project can
observe a boundary before ratcheting it to enforce:

```rust
CrateBoundary::crate_("legacy")
    .deny_external_dependencies()
    .warn()
    .because("legacy is not clean yet; observe before enforcing");
```

A module boundary governs the intra-crate import graph Cargo cannot see — observed
from the crate's own `use` declarations (see the scanner decision in
[PROJECT.md](https://github.com/tacticaldoll/modou/blob/main/PROJECT.md#decisions)):

```rust
ModuleBoundary::in_crate("app")
    .module("crate::kernel")
    .must_not_import("crate::projection")
    .because("the kernel must not depend on a projection");
```

Three module rules share that one `use`-scan observation. `must_not_import` forbids one
outward import; `restrict_imports_to(["crate::types"])` is the closed-allowlist mirror of
the crate-level restrict rule (anything off the allowlist is a violation, so a new
internal module is governed by default); and `must_not_be_imported_by("crate::http")`
governs the inbound direction — encapsulation, "who may reach in" — naming the offending
importer. A rule whose target could never react (a `restrict_imports_to` or
`must_not_be_imported_by` on `crate` itself) is a self-describing constitution error, not
a silent pass.

```bash
cargo run -p modou -- check --manifest-path path/to/Cargo.toml
```

`--manifest-path` is optional: omit it and `check` resolves the nearest `Cargo.toml`
by walking up from the current directory, like `cargo` itself.

Exits `0` (clean / warn-only / fully baselined), `1` (enforced violation), or `2`
(constitution/scan error — including when no `Cargo.toml` can be found). The reaction
is proven against in-repo fixtures (`crates/modou/tests/fixtures/`), so the repo is
**self-contained**: it references no external directory.

`check` also reports **workspace coverage** — how many workspace crates have no
boundary at all — as an always-on line (and a `coverage` field under `--format
json`), so a fully-covered clean run reads differently from one where crates are
simply unchecked. Add `--warn-uncovered` to surface each ungoverned crate as a
warn-severity advisory; like all advisories, it never changes the exit code:

```bash
cargo run -p modou -- check --manifest-path path/to/Cargo.toml --warn-uncovered
```

A dirty project can adopt a boundary without first fixing every violation: record
the current ones as a baseline, then gate only on *new* ones. The baseline is a
generated snapshot (a projection), not policy — see
[PROJECT.md](https://github.com/tacticaldoll/modou/blob/main/PROJECT.md#decisions).

```bash
# pin current violations
cargo run -p modou -- check --manifest-path path/to/Cargo.toml --write-baseline modou-baseline.json
# fail CI only on violations new since that baseline
cargo run -p modou -- check --manifest-path path/to/Cargo.toml --baseline modou-baseline.json
```

For an AI repair loop, `--format json` prints the outcome as a structured document
on stdout (human text stays the default). Each violation carries its `kind`
(`crate` / `module`), and the boundary's `reason` is the repair hint:

```bash
cargo run -p modou -- check --manifest-path path/to/Cargo.toml --format json
```

To see the law itself — every boundary's target, rule, severity, and reason —
`list` prints the declared constitution. It is a projection of the Rust source, not
a reaction: it observes nothing, needs no `--manifest-path`, and always exits `0`
(`--format json` is also accepted). Useful in a CI log or when a steward reviews an
amendment:

```bash
cargo run -p modou -- list
```

## Adopting Modou in your project

The constitution is Rust, so you declare yours in your own code and get the entire
`check` contract — the flags above, the baseline gate, the JSON report, and the
`0` / `1` / `2` exit-code mapping — from one library call. Depend on `modou`, then
expose a tiny binary:

```rust
use modou::prelude::*;

fn constitution() -> Constitution {
    Constitution::new("my-project").boundary(
        CrateBoundary::crate_("my-core")
            .deny_external_dependencies()
            .because("my-core must stay dependency-light"),
    )
}

fn main() -> std::process::ExitCode {
    modou::run(&constitution(), std::env::args())
}
```

This exact snippet ships as a compiled example —
[`examples/adoption.rs`](https://github.com/tacticaldoll/modou/blob/main/crates/modou/examples/adoption.rs) —
so the adoption surface cannot silently rot (`cargo run --example adoption -- check
--manifest-path path/to/Cargo.toml`).

Now `your-binary check --manifest-path path/to/Cargo.toml [...]` reacts against
*your* constitution with the identical contract — no argument parsing, baseline
handling, or exit-code logic to reimplement. The bundled `modou` binary is itself
just this one-liner over the repo's sample constitution.

> **Note:** the published `modou` binary (`cargo install modou`) is a *demo* bound
> to that sample constitution — it governs a crate named `example-core` and will
> report a constitution error on any other project. Modou is consumed as a
> **library**: declare your own constitution and expose your own binary as above.

See [`docs/adoption.md`](https://github.com/tacticaldoll/modou/blob/main/docs/adoption.md) for a copy-paste quick start and the full
walkthrough — dependency setup, CI wiring, gradual adoption via baselines, and
protecting your constitution.

## Roadmap

Modou detects **crate dependency drift** (via `cargo metadata`) — deny external
dependencies (with an optional allowlist), forbid a dependency on named crates,
restrict dependencies to a closed allowlist, and restrict only the *workspace*
dependencies (members derived from `cargo metadata`, so new crates are governed by
default) — and **module-boundary drift** (the intra-crate layering Cargo can't see,
observed from `use` declarations) — forbid one module from importing another, restrict a
module's imports to a closed allowlist, or forbid a module from being imported by another.
Each crate rule can target normal, dev, or build dependencies, and `check` reports
workspace coverage. Later reaction phases — each
with its own observation source, each its own OpenSpec change — are deferred in
[`BACKLOG.md`](https://github.com/tacticaldoll/modou/blob/main/BACKLOG.md): capability drift and opt-in
runtime drift. Nothing is named or built before its reaction exists.

## Non-goals

Not a schema crate, document generator, app framework, agent framework, universal
graph registry, or runtime policy engine. No TOML/Markdown for the constitution, no
`SHAPE.md`, no in-tool amendment system (the amendment flow is harness convention —
CODEOWNERS + steward review, see [PROJECT.md](https://github.com/tacticaldoll/modou/blob/main/PROJECT.md#decisions)),
no procedural macros, no multi-crate split, no universal graph API — until a real
reaction earns them.

## Drift law

**No target type or name without a reaction.** A name is not claimed for a reaction
that does not yet exist. The crate boundary was named `Boundary` while it was the
only kind; once the module reaction landed, the earned rename followed —
`Boundary -> CrateBoundary`, with `Boundary` now the umbrella over `CrateBoundary`
and `ModuleBoundary`.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
