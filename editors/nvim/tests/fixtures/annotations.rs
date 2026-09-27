/**
 * A documented function whose doc block spans three lines.
 */
pub fn block_doc() -> u32 {
    1
}

#[cfg(
    feature = "serde"
)]
pub fn multi_attr() -> u32 {
    2
}

/* plain block comment */
pub const PLAIN: u32 = 3;

/*! inner block comment */
pub const INNER: u32 = 4;

pub fn first() -> u32 { 5 }
// trailing comment that belongs to first, not to second
pub fn second() -> u32 { 6 }
