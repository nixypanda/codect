//! Free functions in every modifier combination.

pub fn free(a: u32) -> u32 {
    a
}

async fn load<T>(value: T) -> Result<T, Error>
where
    T: Clone,
{
    Ok(value)
}

pub const fn constant() -> u8 {
    MIN
}

unsafe fn danger() {}

pub extern "C" fn c_abi() {}

pub fn with_patterns((a, b): (u32, u32), Point { x, y }: Point, mut z: i32) -> i32 {
    z
}
