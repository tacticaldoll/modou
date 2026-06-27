// The crate root does a bare external `use` of the `serde` crate. There is NO
// `mod serde;` declaration, so the sibling `src/serde.rs` file is not a module.
use serde::Deserialize;

pub struct X;
