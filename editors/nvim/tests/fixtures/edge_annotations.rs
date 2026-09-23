#[doc = "a ] b"]
pub fn bracket_in_string() -> u32 { 1 }

#[doc = "a [ b"]
pub fn open_bracket_in_string() -> u32 { 1 }

impl S {
    fn a(&self) {}
    #[inline]
    pub fn b(&self) {}
    /// doc for c
    pub fn c(&self) {}
}

/**
 * A doc block

 * with a blank interior line.
 */
pub fn blank_interior() -> u32 { 1 }

/* outer
 /* inner */
 end */
pub fn nested_block() -> u32 { 2 }

pub fn trailing_owner() -> u32 { 3 }
// trailing comment that belongs to trailing_owner
pub fn after_trailing() -> u32 { 4 }
