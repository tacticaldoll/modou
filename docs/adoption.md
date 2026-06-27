# Adopting Modou

How a downstream project uses Modou to protect its own architecture.

The key idea: **the constitution is Rust code, not a config file.** You do not
"configure" Modou — you declare your boundaries in your own Rust and turn them into
a CI reaction with one library call, `modou::run`. The bundled `modou` binary is
itself just that pattern applied to this repo's sample constitution.

## Quick start (copy-paste)

A governance crate is an ordinary Rust binary that depends on `modou`. There is no
`modou init` — `cargo` already scaffolds and wires everything, and you keep full
ownership of the files. Run this from your workspace root:

```bash
# 1. Create the governance crate. Inside a workspace, `cargo new` also appends it
#    to `[workspace] members`, so there is no manifest to hand-edit.
cargo new --bin governance

# 2. Depend on modou (from crates.io; for local dev see "Depend on modou" below).
cargo add modou --manifest-path governance/Cargo.toml

# 3. Declare your constitution. Start from the skeleton below and uncomment the
#    boundaries you actually want to protect — Modou never guesses them for you.
cat > governance/src/main.rs <<'RS'
use modou::prelude::*;

fn constitution() -> Constitution {
    Constitution::new("my-project")
    // .boundary(
    //     CrateBoundary::crate_("my-core")
    //         .deny_external_dependencies()
    //         .because("my-core must stay dependency-light"),
    // )
}

fn main() -> std::process::ExitCode {
    modou::run(&constitution(), std::env::args())
}
RS

# 4. React.
cargo run -p governance -- check --manifest-path Cargo.toml
```

The sections below walk through each piece in detail.

## 1. Depend on `modou`

Add a small crate to your workspace whose only job is to declare and run your
constitution (an `xtask`-style binary works well). Depend on `modou` from crates.io,
or via git / a path for local development:

```toml
# my-project/governance/Cargo.toml
[package]
name = "governance"
edition = "2021"

[dependencies]
modou = "0.3"
# git:   modou = { git = "https://github.com/tacticaldoll/modou", package = "modou" }
# local: modou = { path = "../../modou/crates/modou" }
```

## 2. Declare your constitution and call `modou::run`

```rust
// my-project/governance/src/main.rs
use modou::prelude::*;

fn constitution() -> Constitution {
    Constitution::new("my-project")
        // Crate dependency drift (observed via `cargo metadata`).
        .boundary(
            CrateBoundary::crate_("my-core")
                .deny_external_dependencies()
                .allow_external(["serde"])
                .because("my-core must stay dependency-light"),
        )
        // Internal crate-to-crate layering.
        .boundary(
            CrateBoundary::crate_("my-core")
                .forbid_dependency_on(["my-adapters"])
                .because("the core must not depend on adapters"),
        )
        // A closed allowlist: depend on only these, internal and external alike.
        .boundary(
            CrateBoundary::crate_("my-domain")
                .restrict_dependencies_to(["serde", "my-types"])
                .because("the domain may depend on only serde and its own types"),
        )
        // Intra-crate module layering Cargo cannot see (observed from `use`).
        .boundary(
            ModuleBoundary::in_crate("my-app")
                .module("crate::domain")
                .must_not_import("crate::http")
                .because("the domain must not import the HTTP layer"),
        )
}

fn main() -> std::process::ExitCode {
    modou::run(&constitution(), std::env::args())
}
```

That single `modou::run` call gives you the whole `check` contract — flag parsing,
the baseline gate, the JSON report, and the exit-code mapping. There is no runner
logic to reimplement.

A boundary defaults to *enforce*. Mark it `.warn()` to make it advisory (reported
but not failing) so a dirty crate can be observed before it is ratcheted to enforce.

## 3. Run it against your project

```bash
cargo run -p governance -- check --manifest-path Cargo.toml
```

The exit code is the reaction:

- `0` — no boundary violated (or only warn / fully baselined violations)
- `1` — an enforce-severity violation; the report names the offending crate/module,
  the rule, the finding, and your `because(...)` reason
- `2` — a constitution or scan error (e.g. a target crate that does not exist, or no
  `Cargo.toml` to evaluate) — distinct from drift, never a silent pass

`--manifest-path` is optional: omit it and `check` resolves the nearest `Cargo.toml`
(cargo-style). `check` also prints a workspace-coverage line — how many workspace
crates have no boundary at all; add `--warn-uncovered` to raise each as a
warn-severity advisory (it never changes the exit code).

## See the law you declared

`list` prints the constitution as code declares it — every boundary's severity,
kind, target, rule with its parameters, and reason. It is a projection of the Rust
source, not a reaction: it observes nothing, needs no `--manifest-path`, and always
exits `0`. Use it in a CI log, or when a steward reviews an amendment to see the
effective law without reading the source:

```bash
cargo run -p governance -- list
# Constitution: my-project  (3 boundaries)
#
# [enforce] crate my-core
#   rule:   deny external dependencies (allow: serde)
#   reason: my-core must stay dependency-light
# ...

cargo run -p governance -- list --format json   # the same projection for tooling
```

## 4. Wire it into CI

The core contract is a real, non-bypassable reaction. A non-zero exit fails the job:

```yaml
# .github/workflows/governance.yml
- name: Modou reaction
  run: cargo run -p governance -- check --manifest-path Cargo.toml
```

## 5. Gradual adoption and machine consumption

Both come for free through the same entry point.

```bash
# Adopt on a dirty project: pin current violations, then fail only on new ones.
cargo run -p governance -- check --manifest-path Cargo.toml --write-baseline modou-baseline.json
cargo run -p governance -- check --manifest-path Cargo.toml --baseline modou-baseline.json

# Structured output for an AI repair loop: each violation carries its `kind`
# (`crate` / `module`); the boundary `reason` is the repair hint.
cargo run -p governance -- check --manifest-path Cargo.toml --format json
```

The baseline is a generated snapshot (a projection), not policy — see
[PROJECT.md](../PROJECT.md#decisions).

## Protecting the constitution (your project's choice)

Modou does not police edits to your constitution; that is a harness convention. If
you want the law to be hard to weaken, route your constitution file (e.g.
`governance/src/main.rs`) to a steward via `.github/CODEOWNERS` and enable required
Code Owner review on your default branch, so any change to a boundary requires review
— an *amendment*, not a silent relaxation to make CI pass.
This mirrors how Modou governs its own constitution; see the amendment-flow decision
in [PROJECT.md](../PROJECT.md#decisions) and [`AGENTS.md`](../AGENTS.md).

## What you are not signing up for

Declaring boundaries in Rust does not pull in a runtime, an app framework, or proc
macros. Modou only reads `cargo metadata` and your source `use` declarations,
compares them against what you declared, and reacts. It does not run your app or
instruct your agent — see the non-goals in [`README.md`](../README.md) and the
contract in [`PROJECT.md`](../PROJECT.md).
