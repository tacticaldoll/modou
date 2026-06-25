//! The constitution — the governed shape, declared in Rust (the single source of
//! truth). It lives in its own file so it can be protected as the law:
//! `.github/CODEOWNERS` routes any change here to the steward (an amendment), so an
//! agent cannot silently weaken a boundary to make CI pass. See `AGENTS.md`.
//!
//! This sample drives the in-repo fixtures; a real project declares its own.

use modou::prelude::*;

pub fn constitution() -> Constitution {
    Constitution::new("example").boundary(
        CrateBoundary::crate_("example-core")
            .deny_external_dependencies()
            .because("example-core is a domain-free core and must stay dependency-light"),
    )
}
