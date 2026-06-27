// The only `use crate::ghost::…` is inside a macro_rules! body (never invoked). It is a
// macro-generated import, out of scope, so the scanner must not observe it.
macro_rules! make {
    () => {
        use crate::ghost::Thing;
    };
}

pub mod ghost {
    pub struct Thing;
}
