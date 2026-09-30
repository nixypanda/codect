//! A small bounded cache used for derived, pure, per-path work.

use std::collections::{BTreeMap, VecDeque};

use base::RepoPath;

/// How many entries a [`BoundedCache`] retains before the oldest is dropped.
pub(crate) const CAP: usize = 32;

/// A small insertion-ordered map that evicts its oldest entry past a cap.
///
/// Eviction is by insertion order rather than true recency: entries are read
/// without `&mut self` from `view`, and for navigation the insertion order is
/// the access order.
#[derive(Clone)]
pub struct BoundedCache<V> {
    entries: BTreeMap<RepoPath, V>,
    order: VecDeque<RepoPath>,
}

impl<V> std::fmt::Debug for BoundedCache<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BoundedCache")
            .field("len", &self.entries.len())
            .finish()
    }
}

impl<V> Default for BoundedCache<V> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            order: VecDeque::new(),
        }
    }
}

impl<V> BoundedCache<V> {
    pub fn get(&self, path: &RepoPath) -> Option<&V> {
        self.entries.get(path)
    }

    pub fn contains_key(&self, path: &RepoPath) -> bool {
        self.entries.contains_key(path)
    }

    pub fn insert(&mut self, path: RepoPath, value: V) {
        if self.entries.insert(path.clone(), value).is_none() {
            self.order.push_back(path);
        }
        while self.order.len() > CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}
