use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirGenericBounds {
    bounds: HashMap<GenericParamId, Vec<TraitBound>>,
    #[serde(default)]
    pub predicates: Vec<crate::types::Predicate>,
}

impl HirGenericBounds {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_bounds(bounds: HashMap<GenericParamId, Vec<TraitBound>>) -> Self {
        Self {
            bounds,
            predicates: Vec::new(),
        }
    }
}

impl FromIterator<(GenericParamId, Vec<TraitBound>)> for HirGenericBounds {
    fn from_iter<T: IntoIterator<Item = (GenericParamId, Vec<TraitBound>)>>(iter: T) -> Self {
        Self::from_bounds(iter.into_iter().collect())
    }
}

impl From<HashMap<GenericParamId, Vec<TraitBound>>> for HirGenericBounds {
    fn from(bounds: HashMap<GenericParamId, Vec<TraitBound>>) -> Self {
        Self::from_bounds(bounds)
    }
}

impl Extend<(GenericParamId, Vec<TraitBound>)> for HirGenericBounds {
    fn extend<T: IntoIterator<Item = (GenericParamId, Vec<TraitBound>)>>(&mut self, iter: T) {
        self.bounds.extend(iter);
    }
}

impl std::ops::Deref for HirGenericBounds {
    type Target = HashMap<GenericParamId, Vec<TraitBound>>;

    fn deref(&self) -> &Self::Target {
        &self.bounds
    }
}

impl std::ops::DerefMut for HirGenericBounds {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.bounds
    }
}

impl<'a> IntoIterator for &'a HirGenericBounds {
    type Item = (&'a GenericParamId, &'a Vec<TraitBound>);
    type IntoIter = std::collections::hash_map::Iter<'a, GenericParamId, Vec<TraitBound>>;

    fn into_iter(self) -> Self::IntoIter {
        self.bounds.iter()
    }
}

impl<'a> IntoIterator for &'a mut HirGenericBounds {
    type Item = (&'a GenericParamId, &'a mut Vec<TraitBound>);
    type IntoIter = std::collections::hash_map::IterMut<'a, GenericParamId, Vec<TraitBound>>;

    fn into_iter(self) -> Self::IntoIter {
        self.bounds.iter_mut()
    }
}

impl IntoIterator for HirGenericBounds {
    type Item = (GenericParamId, Vec<TraitBound>);
    type IntoIter = std::collections::hash_map::IntoIter<GenericParamId, Vec<TraitBound>>;

    fn into_iter(self) -> Self::IntoIter {
        self.bounds.into_iter()
    }
}
