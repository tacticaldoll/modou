//! Adopting Modou: declare your own constitution in Rust and get the entire `check`
//! contract — the flags, the baseline gate, the JSON report, and the `0`/`1`/`2`
//! exit-code mapping — from one library call.
//!
//! This is the README's "Adopting Modou" snippet as a *compiled* example, so the
//! adoption surface (the prelude, the builders, `modou::run`) cannot silently rot:
//! CI builds it via `cargo clippy --all-targets`. Run it against a workspace with
//!
//! ```text
//! cargo run --example adoption -- check --manifest-path path/to/Cargo.toml
//! ```
//!
//! In your own project this is your binary's `main`; `my-core` is your crate.

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
