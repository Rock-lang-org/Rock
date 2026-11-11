use std::collections::BTreeMap;
use std::collections::HashMap;

use super::hir_types::{HirFunction, HirProgram};
use crate::ids::{DefId, InstanceId, TypeId};
use crate::lexer::Span;
use crate::type_context::TypeContext;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InstanceImplOwner {
    Named(DefId),
    BuiltinSlice,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InstanceOrigin {
    Function(DefId),
    ImplMethod {
        owner: InstanceImplOwner,
        method: DefId,
    },
    TraitDefault {
        trait_id: DefId,
        method: DefId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InstanceKey {
    pub origin: InstanceOrigin,
    pub substitution: Vec<TypeId>,
}

impl InstanceKey {
    pub fn new(origin: InstanceOrigin, substitution: Vec<TypeId>) -> Self {
        Self {
            origin,
            substitution,
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstanceSymbols {
    pub source_name: String,
    pub backend_symbol: String,
}

impl InstanceSymbols {
    pub(super) fn pending(source_name: impl Into<String>) -> Self {
        Self::new(source_name, String::new())
    }
    pub fn new(source_name: impl Into<String>, backend_symbol: impl Into<String>) -> Self {
        Self {
            source_name: source_name.into(),
            backend_symbol: backend_symbol.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstanceRecord {
    pub id: InstanceId,
    pub origin: InstanceOrigin,
    pub substitution: Vec<TypeId>,
    pub symbols: InstanceSymbols,
    pub declared: Option<HirFunction>,
    pub provided_by_object: bool,
    pub is_specialization: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedMethodInstance {
    pub receiver_ty: TypeId,
    pub trait_id: DefId,
    pub member_id: DefId,
    pub instance_id: InstanceId,
    pub origin_span: Option<Span>,
}

impl InstanceRecord {
    pub fn new(
        id: InstanceId,
        origin: InstanceOrigin,
        substitution: Vec<TypeId>,
        symbols: InstanceSymbols,
        declared: Option<HirFunction>,
        provided_by_object: bool,
        is_specialization: bool,
    ) -> Self {
        Self {
            id,
            origin,
            substitution,
            symbols,
            declared,
            provided_by_object,
            is_specialization,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PreMirInstanceBody {
    function: HirFunction,
}

impl PreMirInstanceBody {
    pub fn new(function: HirFunction) -> Self {
        Self { function }
    }

    pub fn as_hir(&self) -> &HirFunction {
        &self.function
    }

    pub fn into_hir(self) -> HirFunction {
        self.function
    }
}

#[derive(Debug, Clone, Default)]
pub struct PreMirInstanceBodies {
    bodies: BTreeMap<InstanceId, PreMirInstanceBody>,
}

impl PreMirInstanceBodies {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: InstanceId, function: HirFunction) {
        self.bodies
            .entry(id)
            .or_insert_with(|| PreMirInstanceBody::new(function));
    }

    fn replace(&mut self, id: InstanceId, function: HirFunction) {
        self.bodies.insert(id, PreMirInstanceBody::new(function));
    }

    pub fn get(&self, id: InstanceId) -> Option<&HirFunction> {
        self.bodies.get(&id).map(PreMirInstanceBody::as_hir)
    }

    pub fn take(&mut self, id: InstanceId) -> Option<HirFunction> {
        self.bodies.remove(&id).map(PreMirInstanceBody::into_hir)
    }

    pub fn contains(&self, id: InstanceId) -> bool {
        self.bodies.contains_key(&id)
    }
}

#[derive(Debug, Clone)]
pub struct MonomorphizedProgram {
    pub program: HirProgram,
    pub instances: BTreeMap<InstanceId, InstanceRecord>,
    pub pre_mir_instance_bodies: PreMirInstanceBodies,
    pub generated_drop_instances: BTreeMap<TypeId, GeneratedMethodInstance>,
    pub type_context: TypeContext,
    /// Session-only source locations; products and artifacts never serialize this map.
    pub(crate) source_map: crate::source_map::SemanticSourceMap,
}

pub struct InstanceRegistry {
    next_id: u32,
    by_key: HashMap<InstanceKey, InstanceId>,
    records: BTreeMap<InstanceId, InstanceRecord>,
    pre_mir_bodies: PreMirInstanceBodies,
}

impl InstanceRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            by_key: HashMap::new(),
            records: BTreeMap::new(),
            pre_mir_bodies: PreMirInstanceBodies::new(),
        }
    }

    pub fn intern<F>(&mut self, key: InstanceKey, make_record: F) -> InstanceId
    where
        F: FnOnce(InstanceId) -> InstanceRecord,
    {
        if let Some(id) = self.by_key.get(&key).copied() {
            return id;
        }

        let id = InstanceId(self.next_id);
        self.next_id += 1;
        self.by_key.insert(key, id);
        let record = make_record(id);
        self.records.insert(id, record);
        id
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn record(&self, id: InstanceId) -> Option<&InstanceRecord> {
        self.records.get(&id)
    }

    pub(super) fn set_backend_symbol(&mut self, id: InstanceId, symbol: String) {
        if let Some(record) = self.records.get_mut(&id) {
            record.symbols.backend_symbol = symbol;
        }
    }

    pub fn insert_pre_mir_body(&mut self, id: InstanceId, function: HirFunction) {
        self.pre_mir_bodies.insert(id, function);
    }

    pub fn replace_pre_mir_body(&mut self, id: InstanceId, function: HirFunction) {
        self.pre_mir_bodies.replace(id, function);
    }

    pub fn pre_mir_body(&self, id: InstanceId) -> Option<&HirFunction> {
        self.pre_mir_bodies.get(id)
    }

    pub fn get(&self, key: &InstanceKey) -> Option<InstanceId> {
        self.by_key.get(key).copied()
    }

    pub fn records(&self) -> impl Iterator<Item = &InstanceRecord> {
        self.records.values()
    }

    pub fn into_records(self) -> BTreeMap<InstanceId, InstanceRecord> {
        self.records
    }

    pub fn into_parts(self) -> (BTreeMap<InstanceId, InstanceRecord>, PreMirInstanceBodies) {
        (self.records, self.pre_mir_bodies)
    }
}

impl Default for InstanceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
