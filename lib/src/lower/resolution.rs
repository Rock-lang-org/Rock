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
mod tests;
