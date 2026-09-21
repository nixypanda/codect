//! Comments and formatting must not affect the projection.

pub   struct    Spaced


{
    pub      id   :    u32,   // trailing comment
    /* block */ name:String,
}

pub fn spaced  (  a : u32 , b:u32 )->u32
{
    a+b
}
