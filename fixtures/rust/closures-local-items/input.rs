//! Closures, local items, and local declarations must not appear.

pub fn host() {
    fn local() {}
    struct Local;
    let closure = |x: i32| x + 1;
    let _ = closure(1);
}

pub const OUTER: u32 = {
    let inner = 1;
    inner
};
