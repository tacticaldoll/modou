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

## v0.1

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

Three crate-dependency rules share one observation source (`cargo metadata`). The
external rule takes an optional allowlist; a second forbids a dependency on named
crates — external or an internal workspace path (crate-to-crate layering); a third
restricts the crate's dependencies to a *closed* allowlist (internal and external
alike — "may depend on only these"):

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

```bash
cargo run -p modou -- check --manifest-path path/to/Cargo.toml
```

Exits `0` (clean / warn-only / fully baselined), `1` (enforced violation), or `2`
(constitution/scan error). v0.1 proves the reaction against in-repo fixtures
(`crates/modou/tests/fixtures/`), so the repo is **self-contained**: it references
no external directory.

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

v0.1 detects **crate dependency drift** (via `cargo metadata`) — deny external
dependencies (with an optional allowlist), forbid a dependency on named crates, and
restrict dependencies to a closed allowlist — and **module-boundary drift** (the
intra-crate layering Cargo can't see, observed from `use` declarations). Later
reaction phases — each with its own observation source, each its own OpenSpec
change — are deferred in [`BACKLOG.md`](https://github.com/tacticaldoll/modou/blob/main/BACKLOG.md): capability drift and opt-in
runtime drift. Nothing is named or built before its reaction exists.

## Non-goals (v0.1)

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
