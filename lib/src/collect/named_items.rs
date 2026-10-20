//! Collection payloads are owned by identity; spelling indexes are lookup surfaces.

use std::collections::{HashMap, HashSet};
use std::ops::Index;

use crate::hir::{HirEnum, HirFunction, HirFunctionSig, HirStruct, HirTrait, HirTypeAlias};
use crate::ids::DefId;

pub(crate) trait NamedItem {
    fn id(&self) -> DefId;
    fn name(&self) -> &str;
}

macro_rules! named_item {
    ($($item:ty),* $(,)?) => {$(
        impl NamedItem for $item {
            fn id(&self) -> DefId { self.id }
            fn name(&self) -> &str { &self.name }
        }
    )*};
}

named_item!(HirEnum, HirFunction, HirFunctionSig, HirStruct, HirTrait, HirTypeAlias);

#[derive(Debug, Clone)]
pub(crate) struct NamedItems<T> {
    items: HashMap<DefId, T>,
    names: HashMap<String, DefId>,
    hidden: HashSet<DefId>,
}

impl<T: NamedItem> NamedItems<T> {
    pub(crate) fn new() -> Self {
        Self { items: HashMap::new(), names: HashMap::new(), hidden: HashSet::new() }
    }

    pub(crate) fn hide_from_root(&mut self, ids: &HashSet<DefId>) {
        self.hidden.extend(ids);
        self.names.retain(|_, id| !ids.contains(id));
    }

    pub(crate) fn insert(&mut self, name: String, item: T) -> Option<T> {
        let id = item.id();
        if !self.hidden.contains(&id) { self.names.insert(name, id); }
        self.items.insert(id, item)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&T> { self.items.get(self.names.get(name)?) }
    pub(crate) fn get_id(&self, id: DefId) -> Option<&T> { self.items.get(&id) }
    pub(crate) fn contains_key(&self, name: &str) -> bool { self.names.contains_key(name) }
    pub(crate) fn values(&self) -> impl Iterator<Item = &T> { self.items.values() }
    pub(crate) fn values_mut(&mut self) -> impl Iterator<Item = &mut T> { self.items.values_mut() }
    pub(crate) fn keys(&self) -> impl Iterator<Item = &String> { self.names.keys() }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &T)> {
        self.names.iter().filter_map(|(name, id)| Some((name, self.items.get(id)?)))
    }

    pub(crate) fn entry(&mut self, name: String) -> NamedEntry<'_, T> { NamedEntry { map: self, name } }

    pub(crate) fn remove_id(&mut self, id: DefId) -> Option<T> {
        self.names.retain(|_, target| *target != id);
        self.items.remove(&id)
    }
}

pub(crate) struct NamedEntry<'a, T> {
    map: &'a mut NamedItems<T>,
    name: String,
}

impl<T: NamedItem> NamedEntry<'_, T> {
    pub(crate) fn or_insert_with(self, make: impl FnOnce() -> T) {
        if !self.map.contains_key(&self.name) { self.map.insert(self.name, make()); }
    }
}

impl<T: NamedItem> IntoIterator for NamedItems<T> {
    type Item = (String, T);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        // At the ID-owned declaration boundary, names are diagnostic data only.
        self.items.into_values().map(|item| (item.name().to_string(), item)).collect::<Vec<_>>().into_iter()
    }
}

impl<'a, T: NamedItem> IntoIterator for &'a NamedItems<T> {
    type Item = (&'a String, &'a T);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter { self.iter().collect::<Vec<_>>().into_iter() }
}

impl<T: NamedItem> Index<&str> for NamedItems<T> {
    type Output = T;
    fn index(&self, name: &str) -> &T { self.get(name).expect("indexed collected item") }
}

impl<T: NamedItem> Index<&String> for NamedItems<T> {
    type Output = T;
    fn index(&self, name: &String) -> &T { &self[name.as_str()] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CrateId, LocalDefId};

    #[derive(Clone, Debug, PartialEq)]
    struct Item { id: DefId, name: String, value: u32 }
    impl NamedItem for Item {
        fn id(&self) -> DefId { self.id }
        fn name(&self) -> &str { &self.name }
    }
    fn item(id: u32, value: u32) -> Item {
        Item { id: DefId::new(CrateId(0), LocalDefId(id)), name: "same".into(), value }
    }

    #[test]
    fn macro_a_13_named_items_refreshes_every_alias_without_duplicating_payload() {
        let mut items = NamedItems::new();
        items.insert("first".into(), item(1, 1));
        items.insert("second".into(), item(1, 2));
        assert_eq!(items.values().count(), 1);
        assert_eq!(items["first"].value, 2);
        assert_eq!(items["second"].value, 2);
        items.values_mut().next().unwrap().value = 3;
        assert_eq!(items["first"].value, 3);
        assert_eq!(items.remove_id(item(1, 0).id), Some(item(1, 3)));
        assert!(!items.contains_key("first"));
        assert!(!items.contains_key("second"));
    }

    #[test]
    fn macro_a_13_named_items_keeps_hidden_same_spelling_id_owned() {
        let mut items = NamedItems::new();
        items.hide_from_root(&HashSet::from([item(2, 0).id]));
        items.insert("same".into(), item(1, 1));
        items.insert("same".into(), item(2, 2));
        assert_eq!(items["same"].id, item(1, 0).id);
        assert_eq!(items.get_id(item(2, 0).id), Some(&item(2, 2)));
        assert_eq!(items.into_iter().count(), 2);
    }
}
