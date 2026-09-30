//! Stable-key allocation shared by the language adapters.
//!
//! Stable keys identify declarations inside a projected file; they are not
//! global database IDs (TECHNICAL_DESIGN.md section 5.2). The first use of a
//! base key is returned unchanged, and later collisions take a deterministic
//! source-order ordinal suffix. Byte offsets are never used, so a harmless edit
//! before a declaration does not destabilize its key.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// Allocates unique stable keys within one projected file.
#[derive(Debug, Default)]
pub struct KeyAllocator {
    counts: HashMap<String, usize>,
}

impl KeyAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `base` the first time it is seen, and `base~N` for later
    /// collisions, where `N` counts from one in source order.
    pub fn unique(&mut self, base: String) -> String {
        match self.counts.entry(base.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(1);
                base
            }
            Entry::Occupied(mut entry) => {
                let ordinal = entry.get_mut();
                let key = format!("{base}~{ordinal}");
                *ordinal += 1;
                key
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KeyAllocator;

    #[test]
    fn first_use_is_returned_unchanged() {
        let mut keys = KeyAllocator::new();
        assert_eq!(keys.unique("a::type::Foo".to_owned()), "a::type::Foo");
    }

    #[test]
    fn collisions_take_a_source_order_ordinal() {
        let mut keys = KeyAllocator::new();
        assert_eq!(keys.unique("k".to_owned()), "k");
        assert_eq!(keys.unique("k".to_owned()), "k~1");
        assert_eq!(keys.unique("k".to_owned()), "k~2");
    }

    #[test]
    fn distinct_bases_do_not_collide() {
        let mut keys = KeyAllocator::new();
        assert_eq!(keys.unique("a".to_owned()), "a");
        assert_eq!(keys.unique("b".to_owned()), "b");
        assert_eq!(keys.unique("a".to_owned()), "a~1");
        assert_eq!(keys.unique("b".to_owned()), "b~1");
    }
}
