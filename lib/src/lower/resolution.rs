use crate::hir::{
    HirEnum, HirFunction, HirImplReceiverPattern, HirMethodCallTarget, HirSelectedTraitMember,
    HirStruct, HirVarTarget, HirVariant,
};
use crate::ids::DefId;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LowerResolvedValue {
    pub(crate) name: String,
    pub(crate) ty: Type,
    pub(crate) target: Option<HirVarTarget>,
    pub(crate) is_alias: bool,
    pub(crate) scope_index: Option<usize>,
    pub(crate) should_instantiate: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LowerResolvedStaticMethod {
    pub(crate) value: LowerResolvedValue,
    pub(crate) target: HirMethodCallTarget,
    pub(crate) receiver_pattern: HirImplReceiverPattern,
    pub(crate) owner_generic_params: Vec<GenericParamId>,
    pub(crate) method_generic_params: Vec<GenericParamId>,
}

#[derive(Debug, Clone, PartialEq)]
struct LowerResolvedStaticOwner {
    name: String,
    ty: Type,
}

impl LowerResolvedStaticOwner {
    fn id(&self) -> DefId {
        match &self.ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => *id,
            _ => unreachable!("static method owner must be nominal"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LowerImportTarget {
    pub(crate) short_name: String,
    pub(crate) source: String,
    pub(crate) id: Option<DefId>,
}

#[derive(Debug, Clone)]
pub(crate) struct LowerResolvedEnumVariant {
    pub(crate) owner_name: String,
    pub(crate) owner: HirEnum,
    pub(crate) variant: HirVariant,
}

#[derive(Debug, Clone)]
pub(crate) struct LowerResolvedStructLiteral {
    pub(crate) name: String,
    pub(crate) structure: Option<HirStruct>,
}

pub(crate) struct LowerResolutionContext<'a> {
    lowerer: &'a Lowerer,
}

impl<'a> LowerResolutionContext<'a> {
    pub(crate) fn new(lowerer: &'a Lowerer) -> Self {
        Self { lowerer }
    }

    pub(crate) fn resolve_item_id(&self, name: &str) -> Option<DefId> {
        self.lowerer.resolver.resolve_item_or_alias(name)
    }

    pub(crate) fn resolve_module_alias_or_item_id(&self, name: &str) -> Option<DefId> {
        self.current_module_local_alias_id(name)
            .or_else(|| self.lowerer.resolver.module_aliases.get(name).copied())
            .or_else(|| self.resolve_item_id(name))
    }

    fn current_module_local_alias_id(&self, name: &str) -> Option<DefId> {
        let module_path = self.current_module_prefix()?;
        self.lowerer
            .resolver
            .resolve_module_local_alias(&module_path, name)
    }

    pub(crate) fn resolve_item_id_for_path(&self, name: &str) -> Option<DefId> {
        self.resolve_item_id(name)
    }

    pub(crate) fn canonical_name(&self, id: DefId) -> Option<&'a str> {
        self.lowerer.resolver.canonical_name(id).or_else(|| {
            self.lowerer
                .dependency_resolvers
                .values()
                .find_map(|resolver| resolver.canonical_name(id))
        })
    }

    pub(crate) fn canonical_name_for_item(&self, name: &str) -> Option<String> {
        let id = self.resolve_item_id(name)?;
        self.canonical_name(id).map(ToString::to_string)
    }

    pub(crate) fn try_canonical_owner_path(&self, name: &str) -> Option<String> {
        let def_id = self.resolve_owner_id(name)?;
        self.canonical_name(def_id).map(ToString::to_string)
    }

    pub(crate) fn resolve_owner_id(&self, candidate: &str) -> Option<DefId> {
        self.resolve_module_alias_or_item_id(candidate)
    }

    pub(crate) fn current_module_prefix(&self) -> Option<String> {
        self.lowerer.modules.current_module_prefix()
    }

    pub(crate) fn canonical_name_for_module_alias_or_item(&self, name: &str) -> Option<String> {
        let id = self.resolve_module_alias_or_item_id(name)?;
        self.canonical_name(id).map(ToString::to_string)
    }

    pub(crate) fn resolve_glob_import_targets(
        &self,
        module_path: &[String],
    ) -> Option<Vec<LowerImportTarget>> {
        if module_path.len() != 1 {
            return None;
        }

        let crate_name = &module_path[0];
        let crate_prefix = format!("{}::", crate_name);
        let resolver = self.lowerer.dependency_resolvers.get(crate_name)?;
        let mut targets = resolver
            .export_aliases
            .iter()
            .filter_map(|(alias, id)| {
                let source = resolver.canonical_name(*id)?;
                Some(LowerImportTarget {
                    short_name: alias
                        .strip_prefix(&crate_prefix)
                        .unwrap_or(alias)
                        .to_string(),
                    source: source.to_string(),
                    id: Some(*id),
                })
            })
            .collect::<Vec<_>>();
        targets.sort_by(|left, right| {
            left.short_name
                .cmp(&right.short_name)
                .then_with(|| left.source.cmp(&right.source))
        });

        Some(targets)
    }

    pub(crate) fn qualify_export_source(&self, resolved_prefix: &str, source: &str) -> String {
        let first_segment = source.split("::").next().unwrap_or(source);
        let is_absolute = self.lowerer.modules.has_loaded_root_name(first_segment);

        if is_absolute {
            source.to_string()
        } else {
            format!("{}::{}", resolved_prefix, source)
        }
    }

    pub(crate) fn current_function_owns_import_name(
        &self,
        short_name: &str,
        imported_id: DefId,
    ) -> bool {
        self.resolve_item_id(short_name)
            .and_then(|id| self.lowerer.items.function(id))
            .is_some_and(|existing| {
                existing.id != imported_id
                    && self.lowerer.current_def_ids.contains(&existing.id)
                    && self.resolve_item_id(short_name) == Some(existing.id)
            })
    }

    pub(crate) fn canonicalize_first_path_segment(&self, segments: &[String]) -> Vec<String> {
        let Some((first, rest)) = segments.split_first() else {
            return Vec::new();
        };

        std::iter::once(
            self.canonical_name_for_module_alias_or_item(first)
                .unwrap_or_else(|| first.clone()),
        )
        .chain(rest.iter().cloned())
        .collect()
    }

    pub(crate) fn resolve_enum_variant_path(
        &self,
        segments: &[String],
    ) -> Option<LowerResolvedEnumVariant> {
        if segments.len() != 2 {
            return None;
        }

        let canonical_segments = self.canonicalize_first_path_segment(segments);
        let resolved_first = canonical_segments.first().unwrap_or(&segments[0]);
        let owner = self
            .resolve_enum_type(resolved_first)
            .or_else(|| self.resolve_enum_type(&segments[0]))?;
        let variant = owner
            .variants
            .iter()
            .find(|variant| variant.name == segments[1])?
            .clone();
        let owner_name = self.preferred_nominal_name(owner.id);

        Some(LowerResolvedEnumVariant {
            owner_name,
            owner,
            variant,
        })
    }

    pub(crate) fn resolve_static_method_path(
        &self,
        segments: &[String],
    ) -> Result<Option<LowerResolvedStaticMethod>, String> {
        if segments.len() != 2 {
            return Ok(None);
        }

        let Some(owner) = self.resolve_static_owner_for_first_path_segment(segments) else {
            return Ok(None);
        };
        let owner_id = owner.id();
        let method_name = &segments[1];
        let owner_impls = self
            .lowerer
            .items
            .impl_defs_in_order()
            .map(|(_, imp)| imp)
            .filter(|imp| {
                matches!(
                    &imp.receiver_pattern,
                    HirImplReceiverPattern::Exact(
                        Type::Struct { id, .. } | Type::Enum { id, .. }
                    ) if *id == owner_id
                )
            })
            .collect::<Vec<_>>();
        let has_named_owner_method = owner_impls
            .iter()
            .any(|imp| imp.methods.contains_key(method_name));
        let mut matches = owner_impls
            .iter()
            .filter_map(|imp| {
                imp.methods
                    .get(method_name)
                    .filter(|method| !method.is_method || method.self_receiver.is_some())
                    .map(|method| (*imp, method))
            })
            .collect::<Vec<_>>();
        matches.sort_by_key(|(imp, _)| imp.id);
        matches.dedup_by_key(|(imp, _)| imp.id);
        let (imp, method) = match matches.as_slice() {
            [] => {
                if has_named_owner_method {
                    return Ok(None);
                }
                return Err(format!(
                    "static method `{}` has no matching static implementation",
                    segments.join("::")
                ));
            }
            [(imp, method)] => (*imp, *method),
            _ => {
                return Err(format!(
                    "ambiguous static method `{}`: multiple implementations match",
                    segments.join("::")
                ));
            }
        };
        let owner_name = owner.name;
        let selected_trait = match imp.trait_id {
            Some(trait_id) => {
                let trait_def = self.lowerer.items.trait_def(trait_id).ok_or_else(|| {
                    format!(
                        "static method `{owner_name}::{method_name}` resolved to an unknown trait"
                    )
                })?;
                let member_id = trait_def
                    .methods
                    .get(method_name)
                    .map(|member| member.id)
                    .or_else(|| {
                        trait_def
                            .signatures
                            .get(method_name)
                            .map(|member| member.id)
                    })
                    .ok_or_else(|| {
                        format!(
                            "static method `{owner_name}::{method_name}` is missing trait member authority"
                        )
                    })?;
                Some(HirSelectedTraitMember {
                    trait_id,
                    member_id,
                    trait_args: imp.trait_arg_types.clone(),
                })
            }
            None => None,
        };

        Ok(Some(LowerResolvedStaticMethod {
            value: self.resolved_method_value(&owner_name, method_name, method),
            target: HirMethodCallTarget::impl_method(imp.id, method.id, selected_trait),
            receiver_pattern: imp.receiver_pattern.clone(),
            owner_generic_params: imp.type_generics.iter().map(|param| param.id).collect(),
            method_generic_params: method
                .generic_params
                .iter()
                .map(|param| param.id)
                .filter(|param| param.owner == method.id)
                .collect(),
        }))
    }

    pub(crate) fn resolve_struct_literal_path(
        &self,
        segments: &[String],
    ) -> LowerResolvedStructLiteral {
        let canonical_segments = self.canonicalize_first_path_segment(segments);
        let fallback_name = canonical_segments.join("::");
        let structure = self.resolve_struct_type(&fallback_name).or_else(|| {
            if segments.len() == 1 {
                self.resolve_struct_type(&segments[0])
            } else {
                None
            }
        });
        let name = structure
            .as_ref()
            .map(|structure| self.preferred_nominal_name(structure.id))
            .unwrap_or(fallback_name);

        LowerResolvedStructLiteral { name, structure }
    }

    fn resolve_static_owner_for_first_path_segment(
        &self,
        segments: &[String],
    ) -> Option<LowerResolvedStaticOwner> {
        let first = segments.first()?;
        let canonical_segments = self.canonicalize_first_path_segment(segments);
        let resolved_first = canonical_segments.first().unwrap_or(first);
        let nominal = self
            .resolve_nominal_type(resolved_first)
            .or_else(|| self.resolve_nominal_type(first))?;
        let (id, generic_count, is_struct) = match nominal {
            crate::type_lowering::ResolvedNominalType::Struct(structure) => {
                (structure.id, structure.generic_params.len(), true)
            }
            crate::type_lowering::ResolvedNominalType::Enum(enum_def) => {
                (enum_def.id, enum_def.generic_params.len(), false)
            }
        };
        let args = (0..generic_count)
            .map(|index| {
                Type::Generic(GenericParamId {
                    owner: id,
                    index: index as u32,
                })
            })
            .collect();
        let ty = if is_struct {
            Type::Struct { id, args }
        } else {
            Type::Enum { id, args }
        };

        Some(LowerResolvedStaticOwner {
            name: self.preferred_nominal_name(id),
            ty,
        })
    }

    fn preferred_nominal_name(&self, id: DefId) -> String {
        self.canonical_name(id)
            .map(str::to_string)
            .expect("resolved nominal type must have a canonical resolver name")
    }

    fn resolved_method_value(
        &self,
        type_name: &str,
        method_name: &str,
        method: &HirFunction,
    ) -> LowerResolvedValue {
        let param_types: Vec<Type> = method.params.iter().map(|param| param.ty.clone()).collect();
        LowerResolvedValue {
            name: format!("{}::{}", type_name, method_name),
            ty: Type::function_with_safety(
                param_types,
                method.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(method.is_unsafe),
            ),
            target: Some(HirVarTarget::Function(method.id)),
            is_alias: false,
            scope_index: None,
            should_instantiate: true,
        }
    }

    pub(crate) fn resolve_identifier_value(&self, name: &str) -> Option<LowerResolvedValue> {
        if let Some(binding) = self.lowerer.scope.lookup(name).cloned() {
            if let Some(local_id) = binding.local_id {
                return Some(LowerResolvedValue {
                    name: name.to_string(),
                    ty: binding.ty,
                    target: Some(HirVarTarget::Local(local_id)),
                    is_alias: binding.is_alias,
                    scope_index: self.lowerer.scope.binding_scope_index(name),
                    should_instantiate: false,
                });
            }

            if binding.is_alias {
                if let Some(id) = self.resolve_module_alias_or_item_id(name) {
                    if let Some(target) = self.top_level_var_target_by_id(id) {
                        return Some(LowerResolvedValue {
                            name: self
                                .canonical_name_for_module_alias_or_item(name)
                                .unwrap_or_else(|| name.to_string()),
                            ty: self.top_level_value_type(id).unwrap_or(binding.ty),
                            target: Some(target),
                            is_alias: true,
                            scope_index: self.lowerer.scope.binding_scope_index(name),
                            should_instantiate: true,
                        });
                    }
                }
            }

            if binding.is_top_level {
                if let Some(id) = self.resolve_item_id(name) {
                    if let Some(target) = self.top_level_var_target_by_id(id) {
                        return Some(LowerResolvedValue {
                            name: self
                                .canonical_name_for_item(name)
                                .unwrap_or_else(|| name.to_string()),
                            ty: self.top_level_value_type(id).unwrap_or(binding.ty),
                            target: Some(target),
                            is_alias: false,
                            scope_index: self.lowerer.scope.binding_scope_index(name),
                            should_instantiate: true,
                        });
                    }
                }
            }

            let should_instantiate = self.should_instantiate_type(&binding.ty);
            return Some(LowerResolvedValue {
                name: name.to_string(),
                ty: binding.ty,
                target: None,
                is_alias: binding.is_alias,
                scope_index: self.lowerer.scope.binding_scope_index(name),
                should_instantiate,
            });
        }

        let id = self.resolve_module_alias_or_item_id(name)?;
        let target = self.top_level_var_target_by_id(id)?;
        Some(LowerResolvedValue {
            name: self
                .canonical_name_for_module_alias_or_item(name)
                .unwrap_or_else(|| name.to_string()),
            ty: self.top_level_value_type(id)?,
            target: Some(target),
            is_alias: self.lowerer.resolver.module_aliases.contains_key(name),
            scope_index: None,
            should_instantiate: true,
        })
    }

    pub(crate) fn resolve_qualified_value_path(
        &self,
        segments: &[String],
    ) -> Option<LowerResolvedValue> {
        let canonical_segments = self.canonicalize_first_path_segment(segments);
        let qualified_name = canonical_segments.join("::");

        if let Some(binding) = self.lowerer.scope.lookup(&qualified_name).cloned() {
            if let Some(id) = self.resolve_item_id_for_path(&qualified_name) {
                if let Some(target) = self.top_level_var_target_by_id(id) {
                    return Some(LowerResolvedValue {
                        name: self
                            .canonical_name(id)
                            .map(ToString::to_string)
                            .unwrap_or_else(|| qualified_name.clone()),
                        ty: self.top_level_value_type(id).unwrap_or(binding.ty),
                        target: Some(target),
                        is_alias: binding.is_alias,
                        scope_index: self.lowerer.scope.binding_scope_index(&qualified_name),
                        should_instantiate: true,
                    });
                }
            }

            let should_instantiate = self.should_instantiate_type(&binding.ty);
            let scope_index = self.lowerer.scope.binding_scope_index(&qualified_name);
            return Some(LowerResolvedValue {
                name: qualified_name,
                ty: binding.ty,
                target: None,
                is_alias: binding.is_alias,
                scope_index,
                should_instantiate,
            });
        }

        if let Some(id) = self.resolve_item_id_for_path(&qualified_name) {
            let target = self.top_level_var_target_by_id(id)?;
            return Some(LowerResolvedValue {
                name: self
                    .canonical_name(id)
                    .map(ToString::to_string)
                    .unwrap_or(qualified_name),
                ty: self.top_level_value_type(id)?,
                target: Some(target),
                is_alias: false,
                scope_index: None,
                should_instantiate: true,
            });
        }

        if let Some(prefix) = self.current_module_prefix() {
            let prefixed_name = format!("{prefix}::{qualified_name}");
            if prefixed_name != qualified_name {
                if let Some(id) = self.resolve_item_id_for_path(&prefixed_name) {
                    let target = self.top_level_var_target_by_id(id)?;
                    return Some(LowerResolvedValue {
                        name: self
                            .canonical_name(id)
                            .map(ToString::to_string)
                            .unwrap_or(prefixed_name),
                        ty: self.top_level_value_type(id)?,
                        target: Some(target),
                        is_alias: false,
                        scope_index: None,
                        should_instantiate: true,
                    });
                }
            }
        }

        None
    }

    fn should_instantiate_type(&self, ty: &Type) -> bool {
        let mut generic_params = std::collections::HashSet::new();
        ty.collect_generic_params(&mut generic_params);
        !generic_params.is_empty()
    }

    pub(crate) fn top_level_var_target_by_id(&self, id: DefId) -> Option<HirVarTarget> {
        self.lowerer
            .items
            .function(id)
            .map(|function| HirVarTarget::Function(function.id))
            .or_else(|| {
                self.lowerer
                    .items
                    .extern_def(id)
                    .map(|extern_| HirVarTarget::Extern(extern_.id))
            })
    }

    pub(crate) fn top_level_value_type(&self, id: DefId) -> Option<Type> {
        self.lowerer
            .items
            .function(id)
            .map(|function| {
                Type::function_with_safety(
                    function
                        .params
                        .iter()
                        .map(|param| param.ty.clone())
                        .collect(),
                    function.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(function.is_unsafe),
                )
            })
            .or_else(|| {
                self.lowerer.items.extern_def(id).map(|extern_| {
                    Type::function_with_safety(
                        extern_.params.clone(),
                        extern_.ret.clone(),
                        crate::types::FunctionSafety::from_is_unsafe(extern_.is_unsafe),
                    )
                })
            })
    }

    #[cfg(test)]
    pub(crate) fn canonical_type_name_for_method_lookup(&self, ty: &Type) -> Option<String> {
        match ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => self
                .canonical_name(*id)
                .map(str::to_string)
                .or_else(|| crate::lower::Lowerer::get_type_name_for_method_lookup(ty)),
            Type::Reference { inner, .. }
                if matches!(inner.as_ref(), Type::Slice(_) | Type::Str) =>
            {
                Some(ty.to_string())
            }
            Type::Reference { inner, .. } => self.canonical_type_name_for_method_lookup(inner),
            _ => crate::lower::Lowerer::get_type_name_for_method_lookup(ty),
        }
    }

    pub(crate) fn resolve_trait_id(&self, name: &str) -> Option<DefId> {
        self.resolve_trait_type(name).map(|trait_def| trait_def.id)
    }

    pub(crate) fn resolve_struct_type(&self, name: &str) -> Option<crate::hir::HirStruct> {
        self.resolve_nominal_item_id(name)
            .and_then(|id| self.lowerer.items.structure(id))
            .cloned()
    }

    pub(crate) fn resolve_nominal_type(
        &self,
        name: &str,
    ) -> Option<crate::type_lowering::ResolvedNominalType> {
        let id = self.resolve_nominal_item_id(name)?;
        if let Some(structure) = self.lowerer.items.structure(id).cloned() {
            return Some(crate::type_lowering::ResolvedNominalType::Struct(structure));
        }

        self.lowerer
            .items
            .enumeration(id)
            .cloned()
            .map(crate::type_lowering::ResolvedNominalType::Enum)
    }

    pub(crate) fn struct_segment_looks_unexported(&self, segment: &str) -> bool {
        if self.resolve_struct_type(segment).is_some() {
            return false;
        }

        self.lowerer
            .resolver
            .item_names_by_id
            .iter()
            .any(|(id, name)| {
                name.rsplit("::").next() == Some(segment)
                    && self.lowerer.items.structure(*id).is_some()
            })
    }

    pub(crate) fn resolve_enum_type(&self, name: &str) -> Option<crate::hir::HirEnum> {
        self.resolve_nominal_item_id(name)
            .and_then(|id| self.lowerer.items.enumeration(id))
            .cloned()
    }

    pub(crate) fn resolve_trait_type(&self, name: &str) -> Option<crate::hir::HirTrait> {
        self.resolve_nominal_item_id(name)
            .and_then(|id| self.lowerer.items.trait_def(id))
            .cloned()
    }

    fn resolve_nominal_item_id(&self, name: &str) -> Option<DefId> {
        let id = self.resolve_module_alias_or_item_id(name)?;
        self.canonical_name(id).map(|_| id)
    }
}

#[cfg(test)]
mod tests {
    use crate::collect::resolver::ResolverTables;
    use crate::crate_artifact::ArtifactExport;
    use crate::hir::{
        HirBlock, HirEnum, HirExtern, HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern,
        HirParam, HirStruct, HirTrait, HirVarTarget,
    };
    use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};
    use crate::lower::prelude::PreludeImports;
    use crate::lower::resolution::{LowerImportTarget, LowerResolutionContext};
    use crate::lower::Lowerer;
    use crate::types::Type;

    use std::collections::HashMap;

    fn def_id(crate_id: u32, local_id: u32) -> DefId {
        DefId::new(CrateId(crate_id), LocalDefId(local_id))
    }

    fn test_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn test_enum(id: DefId, name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        }
    }

    fn test_trait(id: DefId, name: &str) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::<HirParam>::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn test_extern(id: DefId, name: &str) -> HirExtern {
        HirExtern {
            id,
            name: name.to_string(),
            params: Vec::new(),
            ret: Type::I64,
            variadic: false,
            is_unsafe: false,
        }
    }

    #[test]
    fn resolution_context_resolves_current_and_dependency_items() {
        let current_id = def_id(0, 10);
        let dep_id = def_id(2, 20);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_paths
            .insert("demo::local".to_string(), current_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(current_id, "demo::local".to_string());

        let mut dep = ResolverTables::default();
        dep.item_paths.insert("dep::value".to_string(), dep_id);
        dep.item_names_by_id
            .insert(dep_id, "dep::value".to_string());
        lowerer.resolver.merge_global_inputs(&dep);
        lowerer.dependency_resolvers.insert("dep".to_string(), dep);

        let resolution = LowerResolutionContext::new(&lowerer);

        assert_eq!(resolution.resolve_item_id("demo::local"), Some(current_id));
        assert_eq!(resolution.resolve_item_id("dep::value"), Some(dep_id));
        assert_eq!(resolution.canonical_name(current_id), Some("demo::local"));
        assert_eq!(resolution.canonical_name(dep_id), Some("dep::value"));
    }

    #[test]
    fn resolution_context_prefers_module_alias_before_root_item_when_requested() {
        let root_id = def_id(0, 30);
        let module_id = def_id(0, 31);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Thing".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Thing".to_string(),
            "demo::helper::Thing".to_string(),
            module_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);

        assert_eq!(resolution.resolve_item_id("Thing"), Some(root_id));
        assert_eq!(
            resolution.resolve_module_alias_or_item_id("Thing"),
            Some(module_id)
        );
        assert_eq!(
            resolution.canonical_name_for_module_alias_or_item("Thing"),
            Some("demo::helper::Thing".to_string())
        );
    }

    #[test]
    fn resolution_context_resolves_nominals_by_module_alias_first() {
        let root_id = def_id(0, 40);
        let module_id = def_id(0, 41);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(root_id, "Thing"));
        lowerer
            .items
            .insert_structure(test_struct(module_id, "demo::helper::Thing"));
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Thing".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Thing".to_string(),
            "demo::helper::Thing".to_string(),
            module_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_struct_type("Thing").unwrap();

        assert_eq!(resolved.id, module_id);
    }

    #[test]
    fn resolution_context_resolves_enum_and_trait_by_id() {
        let enum_id = def_id(0, 50);
        let trait_id = def_id(0, 51);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_enumeration(test_enum(enum_id, "demo::Choice"));
        lowerer
            .items
            .insert_trait_def(test_trait(trait_id, "demo::Show"));
        lowerer
            .resolver
            .item_paths
            .insert("Choice".to_string(), enum_id);
        lowerer
            .resolver
            .item_paths
            .insert("Show".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(enum_id, "demo::Choice".to_string());
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "demo::Show".to_string());

        let resolution = LowerResolutionContext::new(&lowerer);

        assert_eq!(resolution.resolve_enum_type("Choice").unwrap().id, enum_id);
        assert_eq!(resolution.resolve_trait_type("Show").unwrap().id, trait_id);
    }

    #[test]
    fn resolution_context_requires_resolver_ids_for_nominal_types() {
        let struct_id = def_id(0, 52);
        let enum_id = def_id(0, 53);
        let trait_id = def_id(0, 54);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(struct_id, "Widget"));
        lowerer
            .items
            .insert_enumeration(test_enum(enum_id, "Choice"));
        lowerer.items.insert_trait_def(test_trait(trait_id, "Show"));

        let resolution = LowerResolutionContext::new(&lowerer);

        assert!(resolution.resolve_struct_type("Widget").is_none());
        assert!(resolution.resolve_enum_type("Choice").is_none());
        assert!(resolution.resolve_trait_type("Show").is_none());
    }

    #[test]
    fn resolution_context_requires_canonical_names_for_nominal_types() {
        let struct_id = def_id(0, 55);
        let enum_id = def_id(0, 56);
        let trait_id = def_id(0, 57);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(struct_id, "pkg::Widget"));
        lowerer
            .items
            .insert_enumeration(test_enum(enum_id, "pkg::Choice"));
        lowerer
            .items
            .insert_trait_def(test_trait(trait_id, "pkg::Show"));
        lowerer
            .resolver
            .item_paths
            .insert("Widget".to_string(), struct_id);
        lowerer
            .resolver
            .item_paths
            .insert("Choice".to_string(), enum_id);
        lowerer
            .resolver
            .item_paths
            .insert("Show".to_string(), trait_id);

        let resolution = LowerResolutionContext::new(&lowerer);

        assert!(resolution.resolve_struct_type("Widget").is_none());
        assert!(resolution.resolve_enum_type("Choice").is_none());
        assert!(resolution.resolve_trait_type("Show").is_none());
    }

    #[test]
    fn resolution_context_resolves_local_before_top_level_alias() {
        let function_id = def_id(0, 60);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "value"));
        lowerer
            .resolver
            .item_paths
            .insert("value".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "value".to_string());
        lowerer
            .scope
            .define_local("value".to_string(), Type::Bool, true, HirLocalId(9));

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("value").unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Local(HirLocalId(9))));
        assert_eq!(resolved.name, "value");
        assert_eq!(resolved.ty, Type::Bool);
    }

    #[test]
    fn resolution_context_keeps_nested_local_function_value_shadow_local() {
        let function_id = def_id(0, 98);
        let local_id = HirLocalId(10);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "value"));
        lowerer
            .resolver
            .item_paths
            .insert("value".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "value".to_string());
        lowerer.scope.define(
            "value".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer.scope.push();
        lowerer.scope.define_local(
            "value".to_string(),
            Type::function(Vec::new(), Type::Bool),
            false,
            local_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("value").unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Local(local_id)));
        assert_eq!(resolved.ty, Type::function(Vec::new(), Type::Bool));
    }

    #[test]
    fn resolution_context_resolves_alias_to_top_level_target() {
        let function_id = def_id(0, 61);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "demo::helper::value"));
        lowerer.resolver.insert_module_alias_with_name(
            "value".to_string(),
            "demo::helper::value".to_string(),
            function_id,
        );
        lowerer.scope.define_alias(
            "value".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("value").unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
        assert_eq!(resolved.name, "demo::helper::value");
    }

    #[test]
    fn resolution_context_resolves_scoped_function_alias_through_resolver_metadata() {
        let function_id = def_id(0, 92);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "answer"));
        lowerer
            .resolver
            .item_paths
            .insert("answer".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "answer".to_string());
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("answer").unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
        assert_eq!(resolved.name, "answer");
    }

    #[test]
    fn resolution_context_requires_resolver_ids_for_scoped_alias_targets() {
        let function_id = def_id(0, 97);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "answer"));
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("answer").unwrap();

        assert_eq!(resolved.target, None);
        assert_eq!(resolved.name, "answer");
    }

    #[test]
    fn resolution_context_resolves_scoped_extern_alias_through_resolver_metadata() {
        let extern_id = def_id(0, 93);
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_extern(test_extern(extern_id, "puts"));
        lowerer
            .resolver
            .item_paths
            .insert("puts".to_string(), extern_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(extern_id, "puts".to_string());
        lowerer.scope.define_alias(
            "puts".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_identifier_value("puts").unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Extern(extern_id)));
        assert_eq!(resolved.name, "puts");
    }

    #[test]
    fn resolution_context_resolves_qualified_function_after_first_segment_alias() {
        let function_id = def_id(0, 70);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "demo::helper::answer"));
        lowerer
            .resolver
            .item_paths
            .insert("demo::helper::answer".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "demo::helper::answer".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Helper".to_string(),
            "demo::helper".to_string(),
            def_id(0, 71),
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution
            .resolve_qualified_value_path(&["Helper".to_string(), "answer".to_string()])
            .unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
        assert_eq!(resolved.name, "demo::helper::answer");
    }

    #[test]
    fn resolution_context_resolves_qualified_function_through_resolver_metadata() {
        let function_id = def_id(0, 72);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "pkg::answer"));
        lowerer
            .resolver
            .item_paths
            .insert("pkg::answer".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "pkg::answer".to_string());

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution
            .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
            .unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
        assert_eq!(resolved.name, "pkg::answer");
    }

    #[test]
    fn resolution_context_requires_resolver_ids_for_qualified_values() {
        let function_id = def_id(0, 96);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "pkg::answer"));

        let resolution = LowerResolutionContext::new(&lowerer);

        assert!(resolution
            .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
            .is_none());
    }

    #[test]
    fn resolution_context_resolves_static_method_target_by_canonical_owner() {
        let struct_id = def_id(0, 74);
        let method_id = def_id(0, 75);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(struct_id, "String"));
        lowerer.resolver.insert_import_alias_with_name(
            "String".to_string(),
            "stdlib::string::String".to_string(),
            struct_id,
        );
        let impl_id = def_id(0, 76);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("stdlib::string::String".to_string()),
                type_name: "stdlib::string::String".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("from".to_string(), test_function(method_id, "from"))]),
            })
            .unwrap();
        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution
            .resolve_static_method_path(&["String".to_string(), "from".to_string()])
            .unwrap()
            .unwrap();

        assert_eq!(resolved.value.name, "stdlib::string::String::from");
        assert_eq!(
            resolved.value.target,
            Some(HirVarTarget::Function(method_id))
        );
        assert_eq!(resolved.target.impl_id(), Some(impl_id));
        assert_eq!(resolved.target.method_id(), Some(method_id));
    }

    #[test]
    fn resolution_context_rejects_receiver_method_without_explicit_receiver_mode_as_static() {
        let struct_id = def_id(0, 174);
        let method_id = def_id(0, 175);
        let impl_id = def_id(0, 176);
        let mut method = test_function(method_id, "value");
        method.is_method = true;
        method.self_receiver = None;
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(struct_id, "Box"));
        lowerer
            .resolver
            .item_paths
            .insert("Box".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "Box".to_string());
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("value".to_string(), method)]),
            })
            .unwrap();

        assert!(LowerResolutionContext::new(&lowerer)
            .resolve_static_method_path(&["Box".to_string(), "value".to_string()])
            .unwrap()
            .is_none());
    }

    #[test]
    fn resolution_context_resolves_struct_literal_target_by_module_alias() {
        let root_id = def_id(0, 76);
        let module_id = def_id(0, 77);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(root_id, "Thing"));
        lowerer
            .items
            .insert_structure(test_struct(module_id, "demo::helper::Thing"));
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Thing".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Thing".to_string(),
            "demo::helper::Thing".to_string(),
            module_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution.resolve_struct_literal_path(&["Thing".to_string()]);

        assert_eq!(resolved.name, "demo::helper::Thing");
        assert_eq!(resolved.structure.unwrap().id, module_id);
    }

    #[test]
    fn resolution_context_resolves_scoped_qualified_function_through_resolver_metadata() {
        let function_id = def_id(0, 73);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_function(test_function(function_id, "pkg::answer"));
        lowerer
            .resolver
            .item_paths
            .insert("pkg::answer".to_string(), function_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(function_id, "pkg::answer".to_string());
        lowerer.scope.define_alias(
            "pkg::answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let resolution = LowerResolutionContext::new(&lowerer);
        let resolved = resolution
            .resolve_qualified_value_path(&["pkg".to_string(), "answer".to_string()])
            .unwrap();

        assert_eq!(resolved.target, Some(HirVarTarget::Function(function_id)));
        assert_eq!(resolved.name, "pkg::answer");
    }

    #[test]
    fn resolution_context_resolves_artifact_root_glob_targets_from_dependency_resolver_exports() {
        let export_id = def_id(3, 5);
        let mut lowerer = Lowerer::new_for_test();
        let mut dep = ResolverTables::default();
        dep.insert_export_alias_with_name(
            "answer".to_string(),
            "dep::answer".to_string(),
            export_id,
        );
        lowerer.dependency_resolvers.insert("dep".to_string(), dep);

        let resolution = LowerResolutionContext::new(&lowerer);
        let targets = resolution
            .resolve_glob_import_targets(&["dep".to_string()])
            .unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].short_name, "answer");
        assert_eq!(targets[0].source, "dep::answer");
        assert_eq!(targets[0].id, Some(export_id));
    }

    #[test]
    fn resolution_context_preserves_namespaced_dependency_export_aliases_for_root_globs() {
        let show_println_id = def_id(3, 6);
        let debug_fmt_id = def_id(3, 7);
        let mut lowerer = Lowerer::new_for_test();
        let mut dep = ResolverTables::default();
        dep.insert_export_alias_with_name(
            "Show::println".to_string(),
            "dep::Show::println".to_string(),
            show_println_id,
        );
        dep.insert_export_alias_with_name(
            "dep::Debug::fmt".to_string(),
            "dep::Debug::fmt".to_string(),
            debug_fmt_id,
        );
        lowerer.dependency_resolvers.insert("dep".to_string(), dep);

        let resolution = LowerResolutionContext::new(&lowerer);
        let targets = resolution
            .resolve_glob_import_targets(&["dep".to_string()])
            .unwrap();

        assert_eq!(
            targets,
            vec![
                LowerImportTarget {
                    short_name: "Debug::fmt".to_string(),
                    source: "dep::Debug::fmt".to_string(),
                    id: Some(debug_fmt_id),
                },
                LowerImportTarget {
                    short_name: "Show::println".to_string(),
                    source: "dep::Show::println".to_string(),
                    id: Some(show_println_id),
                },
            ]
        );
    }

    #[test]
    fn resolution_context_resolves_owner_path_by_id() {
        let owner_id = def_id(0, 81);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .resolver
            .item_paths
            .insert("demo::Box".to_string(), owner_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(owner_id, "demo::Box".to_string());

        let resolution = LowerResolutionContext::new(&lowerer);

        assert_eq!(
            resolution.try_canonical_owner_path("demo::Box"),
            Some("demo::Box".to_string())
        );
    }

    #[test]
    fn resolution_context_does_not_resolve_qualified_missing_path_through_prelude_suffix() {
        let drop_id = def_id(7, 42);
        let mut lowerer = Lowerer::new_for_test();
        *lowerer.prelude = PreludeImports::with_exports(
            true,
            [(
                "Drop".to_string(),
                ArtifactExport {
                    source: "stdlib::drop::Drop".to_string(),
                    id: drop_id,
                },
            )]
            .into_iter()
            .collect(),
        );
        lowerer.resolver.insert_import_alias_with_name(
            "Drop".to_string(),
            "stdlib::drop::Drop".to_string(),
            drop_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);

        assert_eq!(resolution.resolve_item_id("Drop"), Some(drop_id));
        assert_eq!(resolution.resolve_item_id("missing::Drop"), None);
    }

    #[test]
    fn prelude_imports_preserves_prelude_sync_same_export_alias() {
        let export_id = def_id(0, 90);
        let other_id = def_id(0, 91);
        let mut resolver = ResolverTables::default();

        assert!(
            !crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
                &resolver, "map", export_id
            )
        );

        resolver.insert_import_alias_with_name(
            "map".to_string(),
            "stdlib::prelude::map".to_string(),
            export_id,
        );
        assert!(
            !crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
                &resolver, "map", export_id
            )
        );

        resolver.insert_import_alias_with_name(
            "filter".to_string(),
            "demo::filter".to_string(),
            other_id,
        );
        assert!(
            crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
                &resolver, "filter", export_id
            )
        );

        resolver.item_paths.insert("fold".to_string(), other_id);
        assert!(
            crate::lower::prelude::PreludeImports::export_short_name_is_user_owned(
                &resolver, "fold", export_id
            )
        );
    }

    #[test]
    fn resolution_context_does_not_mark_import_alias_struct_unexported() {
        let string_id = def_id(0, 94);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(string_id, "stdlib::string::String"));
        lowerer.resolver.insert_import_alias_with_name(
            "String".to_string(),
            "stdlib::string::String".to_string(),
            string_id,
        );

        let resolution = LowerResolutionContext::new(&lowerer);

        assert!(!resolution.struct_segment_looks_unexported("String"));
    }

    #[test]
    fn resolution_context_marks_qualified_only_struct_unexported() {
        let hidden_id = def_id(0, 95);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(hidden_id, "pkg::Hidden"));
        lowerer
            .resolver
            .item_names_by_id
            .insert(hidden_id, "pkg::Hidden".to_string());

        let resolution = LowerResolutionContext::new(&lowerer);

        assert!(resolution.struct_segment_looks_unexported("Hidden"));
    }
}
