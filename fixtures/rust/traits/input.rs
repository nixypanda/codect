//! Traits with associated types, defaults, supertraits, and constraints.

pub trait Repository<T>: Send + Sync
where
    T: Clone,
{
    type Error: std::error::Error;
    type Output = Vec<T>;

    const MAX: usize;

    fn get(&self, id: Id) -> Option<&T>;

    fn count(&self) -> usize {
        0
    }
}

pub trait Marker {}

unsafe trait Unsafe: Send {}

trait WithGenerics<'a, const N: usize> {
    type Item<'b>: Clone
    where
        'b: 'a;
}
