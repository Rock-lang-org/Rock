//! An insert-only map that is cheap to clone for inference probes.
//!
//! A probe clones the engine, attempts a unification and is either committed
//! or dropped. Copying whole substitution and span tables for every probe
//! dominated large-crate inference, so entries live in a shared base plus a
//! small private delta: a clone copies only the delta. The delta is folded
//! into the base when nothing else shares it, so folding never copies the base.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

/// Delta size at which an unshared map folds it into the base.
const FOLD_THRESHOLD: usize = 32;

#[derive(Debug, Clone)]
pub(crate) struct LayeredMap<K, V> {
    base: Arc<HashMap<K, V>>,
    delta: HashMap<K, V>,
}

impl<K, V> Default for LayeredMap<K, V> {
    fn default() -> Self {
        Self {
            base: Arc::default(),
            delta: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, V: Clone> LayeredMap<K, V> {
    pub(crate) fn get(&self, key: &K) -> Option<&V> {
        self.delta.get(key).or_else(|| self.base.get(key))
    }

    pub(crate) fn contains_key(&self, key: &K) -> bool {
        self.delta.contains_key(key) || self.base.contains_key(key)
    }

    pub(crate) fn insert(&mut self, key: K, value: V) {
        self.delta.insert(key, value);
        self.fold_if_unshared();
    }

    /// Inserts `value` unless `key` already has one.
    pub(crate) fn insert_if_absent(&mut self, key: K, value: V) {
        if !self.contains_key(&key) {
            self.insert(key, value);
        }
    }

    /// A mutable entry, created with `V::default()` when absent.
    pub(crate) fn entry_or_default(&mut self, key: K) -> &mut V
    where
        V: Default,
    {
        if !self.delta.contains_key(&key) {
            let value = self.base.get(&key).cloned().unwrap_or_default();
            self.delta.insert(key.clone(), value);
            // Fold here too, or probes keep cloning a delta that only grows.
            self.fold_if_unshared();
        }
        if self.delta.contains_key(&key) {
            return self.delta.get_mut(&key).expect("entry was just ensured");
        }
        // Folding only happens when the base is unshared.
        Arc::get_mut(&mut self.base)
            .and_then(|base| base.get_mut(&key))
            .expect("folded entry lives in the unshared base")
    }

    /// Every entry once; a delta entry hides the base entry it overrides.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.base
            .iter()
            .filter(|(key, _)| !self.delta.contains_key(key))
            .chain(self.delta.iter())
    }

    fn fold_if_unshared(&mut self) {
        if self.delta.len() < FOLD_THRESHOLD {
            return;
        }
        if let Some(base) = Arc::get_mut(&mut self.base) {
            base.extend(self.delta.drain());
        }
    }
}

impl<K: Eq + Hash + Clone, V: Clone> FromIterator<(K, V)> for LayeredMap<K, V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(entries: I) -> Self {
        Self {
            base: Arc::new(entries.into_iter().collect()),
            delta: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone, V: Clone> std::ops::Index<&K> for LayeredMap<K, V> {
    type Output = V;

    fn index(&self, key: &K) -> &V {
        self.get(key).expect("LayeredMap key must be present")
    }
}

#[cfg(test)]
#[path = "layered_map/tests.rs"]
mod tests;
