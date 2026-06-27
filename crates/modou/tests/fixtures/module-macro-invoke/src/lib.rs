// The only `use crate::ghost::…` lives inside a macro INVOCATION body (macro-generated,
// out of scope). The scanner must not observe it.
with_imports! {
    use crate::ghost::Thing;
}

pub mod ghost {
    pub struct Thing;
}
