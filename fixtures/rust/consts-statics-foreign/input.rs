//! Module-level and associated constants and statics, and foreign blocks.

pub const MAX: usize = 100;

const PRIVATE: u8 = 0;

pub static VERSION: &str = "1";

pub static mut COUNTER: u64 = 0;

static INTERNAL: i32 = 0;

impl Limits {
    pub const DEFAULT: u32 = 1;
    static LIMIT: i32 = 0;
}

trait Config {
    const TIMEOUT: u64;
}

extern "C" {
    pub fn strlen(s: *const c_char) -> usize;
    static mut GLOBAL: i32;
    pub static REGISTRY: u32;
}
