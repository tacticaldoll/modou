//! Modou v0.1 runner — the CI reaction.
//!
//! A thin caller of the library entry point [`modou::run`], over this repo's own
//! sample constitution. A real project declares its own constitution in Rust and
//! calls `modou::run` the same way to get the identical `check` contract.
//!
//! Usage:
//!   modou check --manifest-path <path/to/Cargo.toml>
//!                [--baseline <file> | --write-baseline <file>] [--format text|json]
//!
//! Exits 0 (clean / warn-only / fully baselined), 1 (enforced violation), or
//! 2 (constitution/scan error, unreadable baseline, or a usage mistake).

use std::process::ExitCode;

mod constitution;
use constitution::constitution;

fn main() -> ExitCode {
    modou::run(&constitution(), std::env::args())
}
