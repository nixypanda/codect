//! Named, tuple, and unit structs with every field visibility.

pub struct Public {
    pub id: u32,
    pub(crate) name: String,
    pub(super) label: String,
    pub(in crate::model) tag: String,
    private: u8,
}

struct Tuple(pub u32, pub(crate) String, i64);

pub struct Unit;

pub struct EmptyNamed {}

pub struct Generic<T: Clone, U = u8>
where
    U: Default,
{
    pub first: T,
    second: U,
}
