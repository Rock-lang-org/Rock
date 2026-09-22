//! HIR - High-level Intermediate Representation
//!
//! The HIR is a desugared, typed representation of the program.
//! It removes syntactic sugar, resolves names, and carries type information.

use std::{
    collections::{HashMap, HashSet},
    hash::Hash,
};

use serde::{Deserialize, Serialize};

use crate::ids::{AssocTypeId, DefId, FieldId, HirLocalId, InstanceId, TypeVarId, VariantId};
use crate::language_items::LanguageItems;
use crate::lexer::Span;
use crate::types::ReceiverMode;
use crate::types::{GenericParamId, TraitBound, Type};

mod sealed {
    pub trait Sealed {}
}

pub trait HirPhase: sealed::Sealed + Clone + std::fmt::Debug {
    type MethodAuthority: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de>;
    type ResidualAuthority: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de>;
    type ImplReceiverAuthority: Clone + std::fmt::Debug + Serialize + for<'de> Deserialize<'de>;

    fn method_authority(authority: &Self::MethodAuthority) -> Option<&HirMethodCallTarget>;
    fn residual_authority(authority: &Self::ResidualAuthority) -> Option<&HirCallTarget>;
    fn method_authority_mut(
        authority: &mut Self::MethodAuthority,
    ) -> Option<&mut HirMethodCallTarget>;
    fn residual_authority_mut(
        authority: &mut Self::ResidualAuthority,
    ) -> Option<&mut HirCallTarget>;
    fn impl_receiver_pattern(
        authority: &Self::ImplReceiverAuthority,
    ) -> Option<&HirImplReceiverPattern>;
    fn unresolved_receiver_parts(authority: &Self::ImplReceiverAuthority) -> Option<&[Type]>;
    fn visit_impl_receiver_types(
        authority: &Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&Type),
    );
    fn visit_impl_receiver_types_mut(
        authority: &mut Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&mut Type),
    );
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnresolvedHir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptedHir;

impl sealed::Sealed for UnresolvedHir {}
impl sealed::Sealed for AcceptedHir {}

impl HirPhase for UnresolvedHir {
    type MethodAuthority = Option<HirMethodCallTarget>;
    type ResidualAuthority = Option<HirCallTarget>;
    type ImplReceiverAuthority = HirImplReceiverPattern;

    fn method_authority(authority: &Self::MethodAuthority) -> Option<&HirMethodCallTarget> {
        authority.as_ref()
    }

    fn residual_authority(authority: &Self::ResidualAuthority) -> Option<&HirCallTarget> {
        authority.as_ref()
    }

    fn method_authority_mut(
        authority: &mut Self::MethodAuthority,
    ) -> Option<&mut HirMethodCallTarget> {
        authority.as_mut()
    }

    fn residual_authority_mut(
        authority: &mut Self::ResidualAuthority,
    ) -> Option<&mut HirCallTarget> {
        authority.as_mut()
    }

    fn impl_receiver_pattern(
        authority: &Self::ImplReceiverAuthority,
    ) -> Option<&HirImplReceiverPattern> {
        Some(authority)
    }

    fn unresolved_receiver_parts(_authority: &Self::ImplReceiverAuthority) -> Option<&[Type]> {
        None
    }

    fn visit_impl_receiver_types(
        authority: &Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&Type),
    ) {
        match authority {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                visitor(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => visitor(element),
        }
    }

    fn visit_impl_receiver_types_mut(
        authority: &mut Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&mut Type),
    ) {
        match authority {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                visitor(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => visitor(element),
        }
    }
}

impl HirPhase for AcceptedHir {
    type MethodAuthority = HirMethodCallTarget;
    type ResidualAuthority = HirCallTarget;
    type ImplReceiverAuthority = HirImplReceiverPattern;

    fn method_authority(authority: &Self::MethodAuthority) -> Option<&HirMethodCallTarget> {
        Some(authority)
    }

    fn residual_authority(authority: &Self::ResidualAuthority) -> Option<&HirCallTarget> {
        Some(authority)
    }

    fn method_authority_mut(
        authority: &mut Self::MethodAuthority,
    ) -> Option<&mut HirMethodCallTarget> {
        Some(authority)
    }

    fn residual_authority_mut(
        authority: &mut Self::ResidualAuthority,
    ) -> Option<&mut HirCallTarget> {
        Some(authority)
    }

    fn impl_receiver_pattern(
        authority: &Self::ImplReceiverAuthority,
    ) -> Option<&HirImplReceiverPattern> {
        Some(authority)
    }

    fn unresolved_receiver_parts(_authority: &Self::ImplReceiverAuthority) -> Option<&[Type]> {
        None
    }

    fn visit_impl_receiver_types(
        authority: &Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&Type),
    ) {
        match authority {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                visitor(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => visitor(element),
        }
    }

    fn visit_impl_receiver_types_mut(
        authority: &mut Self::ImplReceiverAuthority,
        visitor: &mut dyn FnMut(&mut Type),
    ) {
        match authority {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                visitor(ty)
            }
            HirImplReceiverPattern::SliceFamily { element } => visitor(element),
        }
    }
}

mod type_ids;

mod accepted;
pub(crate) mod language_items;

pub use accepted::{
    AcceptedHirBlock, AcceptedHirExpr, AcceptedHirFunction, AcceptedHirImpl, AcceptedHirProgram,
    AcceptedHirTrait,
};
pub use type_ids::{collect_hir_type_ids, HirTypeIds, HirTypeLocation};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirMethodLocation {
    TraitDefault {
        trait_id: DefId,
        method_id: DefId,
        method_name: String,
    },
    ImplMethod {
        impl_id: DefId,
        method_id: DefId,
        method_name: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirFieldLocation {
    pub owner: DefId,
    pub field_id: FieldId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirVariantLocation {
    pub owner: DefId,
    pub variant_id: VariantId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirAssociatedTypeLocation {
    pub owner: DefId,
    pub assoc_type_id: AssocTypeId,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HirDefinitionIndexes {
    pub functions_by_id: HashMap<DefId, String>,
    pub structs_by_id: HashMap<DefId, String>,
    pub enums_by_id: HashMap<DefId, String>,
    pub traits_by_id: HashMap<DefId, String>,
    pub impls_by_id: HashMap<DefId, usize>,
    pub impl_owner_ids_by_id: HashMap<DefId, DefId>,
    pub externs_by_id: HashMap<DefId, usize>,
    pub type_aliases_by_id: HashMap<DefId, String>,
    pub methods_by_id: HashMap<DefId, HirMethodLocation>,
    /// For a trait implementation, the concrete method selected for each trait
    /// member. This is the post-conformance authority used by later compiler
    /// phases; they must not reconstruct the relationship from method names.
    pub effective_trait_methods: HashMap<(DefId, DefId), DefId>,
    pub fields_by_id: HashMap<(DefId, FieldId), HirFieldLocation>,
    pub variants_by_id: HashMap<(DefId, VariantId), HirVariantLocation>,
    pub associated_type_decls_by_id: HashMap<(DefId, AssocTypeId), HirAssociatedTypeLocation>,
    pub associated_type_defs_by_id: HashMap<(DefId, AssocTypeId), HirAssociatedTypeLocation>,
}

/// Canonical target domain for an implementation.
///
/// This index is rebuilt from HIR after lowering and artifact loading. It is
/// deliberately separate from legacy display aliases stored on `HirImpl` so
/// later phases can match implementations without interpreting owner names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirImplReceiverPattern {
    Exact(Type),
    SliceFamily {
        element: Type,
    },
    /// A normalized constructor-level trait target.
    Constructor(Type),
}

pub type HirImplTarget = HirImplReceiverPattern;

#[cfg(test)]
impl From<Vec<Type>> for HirImplReceiverPattern {
    fn from(mut parts: Vec<Type>) -> Self {
        match parts.len() {
            0 => Self::Exact(Type::Unit),
            1 => Self::Exact(parts.pop().expect("one receiver test part")),
            _ => Self::Exact(Type::Tuple(parts)),
        }
    }
}

#[derive(Clone, Debug)]
struct TraitDefaultMethodCandidate {
    trait_id: DefId,
    trait_name: String,
    method_name: String,
}

/// Display and serialization aliases for HIR definitions.
///
/// Production compiler phases should use ID-keyed maps, resolved targets, or
/// `HirDefinitionIndexes`. These tables remain for diagnostics, debug output,
/// artifact compatibility, and tests that need to inspect display aliases.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HirNameTables {
    pub functions_by_name: HashMap<String, DefId>,
    pub structs_by_name: HashMap<String, DefId>,
    pub enums_by_name: HashMap<String, DefId>,
    pub traits_by_name: HashMap<String, DefId>,
    pub externs_by_name: HashMap<String, DefId>,
    pub type_aliases_by_name: HashMap<String, DefId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HirProgramOrder {
    pub impls: Vec<DefId>,
    pub externs: Vec<DefId>,
}

pub type HirLanguageItems = LanguageItems<DefId>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(
    serialize = "P: Serialize, P::MethodAuthority: Serialize, P::ResidualAuthority: Serialize, P::ImplReceiverAuthority: Serialize",
    deserialize = "P: Deserialize<'de>, P::MethodAuthority: Deserialize<'de>, P::ResidualAuthority: Deserialize<'de>, P::ImplReceiverAuthority: Deserialize<'de>"
))]
pub struct HirProgramFor<P: HirPhase> {
    pub functions: HashMap<DefId, HirFunctionFor<P>>,
    pub structs: HashMap<DefId, HirStruct>,
    pub enums: HashMap<DefId, HirEnum>,
    pub traits: HashMap<DefId, HirTraitFor<P>>,
    pub impls: HashMap<DefId, HirImplFor<P>>,
    pub externs: HashMap<DefId, HirExtern>,
    #[serde(default)]
    pub type_aliases: HashMap<DefId, HirTypeAlias>,
    #[serde(default)]
    pub names: HirNameTables,
    #[serde(default)]
    pub order: HirProgramOrder,
    #[serde(default)]
    pub language_items: HirLanguageItems,
    #[serde(skip, default)]
    pub indexes: HirDefinitionIndexes,
}

pub type HirProgram = HirProgramFor<UnresolvedHir>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirTypeAlias {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub ty: Type,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirExtern {
    pub id: DefId,
    pub name: String,
    pub params: Vec<Type>,
    pub ret: Type,
    pub variadic: bool,
    #[serde(default)]
    pub is_unsafe: bool,
}

impl HirProgram {
    pub fn from_id_parts_with_names(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        language_items: HirLanguageItems,
    ) -> Self {
        let mut canonical_names_by_id = HashMap::new();
        canonical_names_by_id.extend(
            functions
                .iter()
                .map(|(id, function)| (*id, function.name.clone())),
        );
        canonical_names_by_id.extend(
            structs
                .iter()
                .map(|(id, structure)| (*id, structure.name.clone())),
        );
        canonical_names_by_id.extend(
            enums
                .iter()
                .map(|(id, enum_def)| (*id, enum_def.name.clone())),
        );
        canonical_names_by_id.extend(
            traits
                .iter()
                .map(|(id, trait_def)| (*id, trait_def.name.clone())),
        );

        Self::from_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            names,
            language_items,
            &canonical_names_by_id,
        )
    }

    pub fn from_id_parts_with_names_and_canonical_names(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        language_items: HirLanguageItems,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Self {
        let impl_order = sorted_definition_ids(&impls);
        let extern_order = sorted_definition_ids(&externs);

        let mut program = Self {
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases: HashMap::new(),
            names,
            order: HirProgramOrder {
                impls: impl_order,
                externs: extern_order,
            },
            language_items,
            indexes: HirDefinitionIndexes::default(),
        };
        program.rebuild_indexes_with_canonical_names(&canonical_names_by_id);
        program
    }

    pub fn rebuild_indexes(&mut self) {
        self.rebuild_indexes_with_canonical_names(&HashMap::new());
    }

    pub fn rebuild_indexes_with_canonical_names(
        &mut self,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) {
        let effective_trait_methods = std::mem::take(&mut self.indexes.effective_trait_methods);
        self.indexes = HirDefinitionIndexes::from_parts(
            &self.functions,
            &self.structs,
            &self.enums,
            &self.traits,
            &self.impls,
            &self.externs,
            &self.type_aliases,
            &self.names,
            &self.order,
            canonical_names_by_id,
        );
        self.indexes
            .effective_trait_methods
            .extend(effective_trait_methods);
    }

    pub fn validate_method_authorities(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for function in self.functions.values() {
            validate_method_authorities_in_block(self, &function.body, &mut errors);
        }
        for imp in self.impls.values() {
            if let Err(error) = validate_impl_receiver_pattern_bindings(imp, &imp.receiver_pattern)
            {
                errors.push(error);
            }
            if let Some(trait_id) = imp.trait_id {
                let Some(trait_def) = self.traits.get(&trait_id) else {
                    errors.push(format!(
                        "implementation {:?} references unknown trait {trait_id:?}",
                        imp.id
                    ));
                    continue;
                };
                let member_ids = trait_def.methods.values().map(|method| method.id).chain(
                    trait_def
                        .signatures
                        .iter()
                        .filter(|(name, _)| !trait_def.methods.contains_key(*name))
                        .map(|(_, signature)| signature.id),
                );
                for member_id in member_ids {
                    let Some(method_id) = self.effective_trait_method(imp.id, member_id) else {
                        errors.push(format!(
                            "implementation {:?} has no effective body for trait member {member_id:?}",
                            imp.id
                        ));
                        continue;
                    };
                    if !imp.methods.values().any(|method| method.id == method_id) {
                        errors.push(format!(
                            "implementation {:?} maps trait member {member_id:?} to method {method_id:?} outside the impl",
                            imp.id
                        ));
                        continue;
                    }
                    if let Err(error) = validate_effective_trait_method_receiver_shape(
                        trait_def, imp, member_id, method_id,
                    ) {
                        errors.push(error);
                    }
                }
            }
            for function in imp.methods.values() {
                validate_method_authorities_in_block(self, &function.body, &mut errors);
            }
        }
        for (&(impl_id, member_id), &method_id) in &self.indexes.effective_trait_methods {
            let Some(imp) = self.impls.get(&impl_id) else {
                errors.push(format!(
                    "effective trait method relation references unknown impl {impl_id:?}"
                ));
                continue;
            };
            let Some(trait_id) = imp.trait_id else {
                errors.push(format!(
                    "effective trait method relation references inherent impl {impl_id:?}"
                ));
                continue;
            };
            let member_exists = self.traits.get(&trait_id).is_some_and(|trait_def| {
                trait_def
                    .methods
                    .values()
                    .any(|method| method.id == member_id)
                    || trait_def
                        .signatures
                        .values()
                        .any(|signature| signature.id == member_id)
            });
            if !member_exists {
                errors.push(format!(
                    "effective trait method relation references member {member_id:?} outside trait {trait_id:?}"
                ));
            }
            if !imp.methods.values().any(|method| method.id == method_id) {
                errors.push(format!(
                    "effective trait method relation references method {method_id:?} outside impl {impl_id:?}"
                ));
            }
        }
        for trait_def in self.traits.values() {
            for function in trait_def.methods.values() {
                validate_method_authorities_in_block(self, &function.body, &mut errors);
            }
        }
        errors
    }

    fn type_normalization_env(&self) -> crate::type_services::normalize::TypeNormalizationEnv {
        let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
        for structure in self.structs.values() {
            normalization_env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for enumeration in self.enums.values() {
            normalization_env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        crate::type_lowering::register_type_aliases(
            &mut normalization_env,
            self.type_aliases.values(),
        );
        for trait_def in self.traits.values() {
            for param in &trait_def.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
            if let Some(target) = &trait_def.target {
                normalization_env.register_generic_kind(target.id, target.kind.clone());
            }
            for associated_type in &trait_def.associated_types {
                normalization_env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
            for function in trait_def.methods.values() {
                for param in &function.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for param in &signature.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        for function in self.functions.values() {
            for param in &function.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for imp in self.impls.values() {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
            for function in imp.methods.values() {
                for param in &function.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }

        normalization_env
    }

    pub(crate) fn validate_accepted_types(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let normalization_env = self.type_normalization_env();
        let mut predicate_rows = Vec::new();
        for trait_def in self.traits.values() {
            for (index, predicate) in trait_def.predicates.iter().enumerate() {
                predicate_rows.push((trait_def.id, index, predicate));
            }
            for signature in trait_def.signatures.values() {
                for (index, predicate) in signature.generic_bounds.predicates.iter().enumerate() {
                    predicate_rows.push((signature.id, index, predicate));
                }
            }
            for method in trait_def.methods.values() {
                for (index, predicate) in method.generic_bounds.predicates.iter().enumerate() {
                    predicate_rows.push((method.id, index, predicate));
                }
            }
        }
        for function in self.functions.values() {
            for (index, predicate) in function.generic_bounds.predicates.iter().enumerate() {
                predicate_rows.push((function.id, index, predicate));
            }
        }
        for imp in self.impls.values() {
            for (index, predicate) in imp.bounds.predicates.iter().enumerate() {
                predicate_rows.push((imp.id, index, predicate));
            }
        }
        predicate_rows.sort_by_key(|(owner, index, _)| (*owner, *index));
        for (owner, index, predicate) in predicate_rows {
            let crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } = predicate;
            let Some(required_trait) = self.traits.get(trait_id) else {
                errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] references unknown trait {trait_id:?}"
                ));
                continue;
            };
            let expected_subject_kind = required_trait
                .target
                .as_ref()
                .map(|target| target.kind.clone())
                .unwrap_or(crate::type_services::kind::Kind::Type);
            let normalizer =
                crate::type_services::normalize::TypeNormalizer::new(&normalization_env);
            match normalizer.kind_of(subject) {
                Ok(actual) if actual != expected_subject_kind => errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] subject has kind {actual}, expected {expected_subject_kind} for trait '{}'",
                    required_trait.name
                )),
                Err(error) => errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] subject kind is invalid: {error}"
                )),
                _ => {}
            }
            if args.len() != required_trait.generic_params.len() {
                errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] supplies {} arguments to trait '{}', expected {}",
                    args.len(),
                    required_trait.name,
                    required_trait.generic_params.len()
                ));
                continue;
            }
            for (arg_index, (arg, param)) in
                args.iter().zip(&required_trait.generic_params).enumerate()
            {
                match crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                    .kind_of(arg)
                {
                    Ok(actual) if actual != param.kind => errors.push(format!(
                        "accepted HIR predicate {owner:?}[{index}] argument {arg_index} has kind {actual}, expected {}",
                        param.kind
                    )),
                    Err(error) => errors.push(format!(
                        "accepted HIR predicate {owner:?}[{index}] argument {arg_index} kind is invalid: {error}"
                    )),
                    _ => {}
                }
            }
        }

        let mut type_context = crate::type_context::TypeContext::new();
        let type_ids = collect_hir_type_ids(self, &mut type_context);
        let mut locations = type_ids.iter().collect::<Vec<_>>();
        locations.sort_by_key(|(location, _)| format!("{location:?}"));
        for (location, type_id) in locations {
            let ty = type_context.type_for(type_id);
            if type_contains_backend_forbidden_sentinel(&ty) {
                errors.push(format!(
                    "accepted HIR contains unresolved type at {location:?}: {ty:?}"
                ));
            }
            let normalizer =
                crate::type_services::normalize::TypeNormalizer::new(&normalization_env);
            if let Err(error) = normalizer.validate_canonical(&ty) {
                errors.push(format!(
                    "accepted HIR contains noncanonical type at {location:?}: {error}"
                ));
            }
            let expected_kind = match location {
                HirTypeLocation::TypeAliasBody { .. } => None,
                HirTypeLocation::AssociatedTypeDef {
                    impl_id,
                    assoc_type,
                } => self.impls.get(impl_id).and_then(|imp| {
                    imp.associated_types
                        .iter()
                        .find(|associated| associated.id == *assoc_type)
                        .map(|associated| associated.kind.clone())
                }),
                location if hir_type_location_requires_runtime_type(location) => {
                    Some(crate::type_services::kind::Kind::Type)
                }
                _ => None,
            };
            if let Some(expected_kind) = expected_kind {
                match crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                    .kind_of(&ty)
                {
                    Ok(actual_kind) if actual_kind != expected_kind => errors.push(format!(
                        "accepted HIR type at {location:?} has kind {actual_kind}, expected {expected_kind}"
                    )),
                    Err(error) => errors.push(format!(
                        "accepted HIR cannot determine kind at {location:?}: {error}"
                    )),
                    _ => {}
                }
            }
            validate_projection_ambiguities(self, &ty, &format!("{location:?}"), &mut errors);
        }

        errors
    }

    pub fn function_by_id(&self, id: DefId) -> Option<(&str, &HirFunction)> {
        self.functions.get(&id).map(|function| {
            let name = self
                .indexes
                .functions_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (name, function)
        })
    }

    pub fn struct_by_id(&self, id: DefId) -> Option<(&str, &HirStruct)> {
        self.structs.get(&id).map(|structure| {
            let name = self
                .indexes
                .structs_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(structure.name.as_str());
            (name, structure)
        })
    }

    pub fn enum_by_id(&self, id: DefId) -> Option<(&str, &HirEnum)> {
        self.enums.get(&id).map(|enumeration| {
            let name = self
                .indexes
                .enums_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(enumeration.name.as_str());
            (name, enumeration)
        })
    }

    pub fn trait_by_id(&self, id: DefId) -> Option<(&str, &HirTrait)> {
        self.traits.get(&id).map(|trt| {
            let name = self
                .indexes
                .traits_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(trt.name.as_str());
            (name, trt)
        })
    }

    pub fn impl_by_id(&self, id: DefId) -> Option<(usize, &HirImpl)> {
        let index = *self.indexes.impls_by_id.get(&id)?;
        self.impls.get(&id).map(|imp| (index, imp))
    }

    pub fn extern_by_id(&self, id: DefId) -> Option<(usize, &HirExtern)> {
        let index = *self.indexes.externs_by_id.get(&id)?;
        self.externs.get(&id).map(|ext| (index, ext))
    }

    pub fn functions_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirFunction)> {
        self.functions.iter().map(|(id, function)| {
            let name = self
                .indexes
                .functions_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (*id, name, function)
        })
    }

    pub fn structs_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirStruct)> {
        self.structs.iter().map(|(id, structure)| {
            let name = self
                .indexes
                .structs_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(structure.name.as_str());
            (*id, name, structure)
        })
    }

    pub fn enums_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirEnum)> {
        self.enums.iter().map(|(id, enumeration)| {
            let name = self
                .indexes
                .enums_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(enumeration.name.as_str());
            (*id, name, enumeration)
        })
    }

    pub fn traits_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirTrait)> {
        self.traits.iter().map(|(id, trt)| {
            let name = self
                .indexes
                .traits_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(trt.name.as_str());
            (*id, name, trt)
        })
    }

    pub fn impls_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirImpl)> {
        self.indexes
            .impls_by_id
            .iter()
            .filter_map(|(id, index)| self.impls.get(id).map(|imp| (*id, *index, imp)))
    }

    pub fn externs_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirExtern)> {
        self.indexes
            .externs_by_id
            .iter()
            .filter_map(|(id, index)| self.externs.get(id).map(|ext| (*id, *index, ext)))
    }

    /// Display/compatibility aliases for a canonical function owner.
    ///
    /// Semantic consumers should use `function_by_id` when they already have a
    /// `DefId`; this helper is only for display, artifact compatibility, and
    /// alias metadata derived from a known owner.
    pub fn function_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.function_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.functions_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical struct owner.
    pub fn struct_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.struct_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.structs_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical enum owner.
    pub fn enum_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.enum_by_id(id) else {
            return Vec::new();
        };

        display_aliases_for_id(&self.names.enums_by_name, id, Some(primary_name))
    }

    /// Display/compatibility aliases for a canonical nominal owner.
    pub fn nominal_display_aliases(&self, id: DefId) -> Vec<&str> {
        if self.structs.contains_key(&id) {
            self.struct_display_aliases(id)
        } else if self.enums.contains_key(&id) {
            self.enum_display_aliases(id)
        } else {
            Vec::new()
        }
    }

    /// Resolve a display alias to an existing nominal owner.
    ///
    /// This is a compatibility helper for HIR metadata that still stores a
    /// display owner string. Prefer `Type::Struct { id, .. }`,
    /// `Type::Enum { id, .. }`, or explicit owner sidecars when available.
    pub fn nominal_owner_id_for_display_alias(&self, name: &str) -> Option<DefId> {
        self.names
            .structs_by_name
            .get(name)
            .copied()
            .filter(|id| self.structs.contains_key(id))
            .or_else(|| {
                self.names
                    .enums_by_name
                    .get(name)
                    .copied()
                    .filter(|id| self.enums.contains_key(id))
            })
    }

    pub fn impl_owner_id(&self, impl_id: DefId) -> Option<DefId> {
        self.indexes.impl_owner_ids_by_id.get(&impl_id).copied()
    }

    pub fn impl_receiver_pattern(&self, impl_id: DefId) -> Option<&HirImplReceiverPattern> {
        self.impls.get(&impl_id).map(|imp| &imp.receiver_pattern)
    }

    pub fn effective_trait_method(&self, impl_id: DefId, trait_method_id: DefId) -> Option<DefId> {
        self.indexes
            .effective_trait_methods
            .get(&(impl_id, trait_method_id))
            .copied()
    }

    pub fn impls_in_order(&self) -> impl Iterator<Item = (DefId, &HirImpl)> {
        ordered_owned_ids(&self.impls, &self.order.impls)
            .into_iter()
            .filter_map(|id| self.impls.get(&id).map(|imp| (id, imp)))
    }

    pub fn externs_in_order(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        ordered_owned_ids(&self.externs, &self.order.externs)
            .into_iter()
            .filter_map(|id| self.externs.get(&id).map(|ext| (id, ext)))
    }
}

fn hir_type_location_requires_runtime_type(location: &HirTypeLocation) -> bool {
    matches!(
        location,
        HirTypeLocation::FunctionReturn { .. }
            | HirTypeLocation::FunctionParam { .. }
            | HirTypeLocation::ExternReturn { .. }
            | HirTypeLocation::ExternParam { .. }
            | HirTypeLocation::StructField { .. }
            | HirTypeLocation::EnumVariantNamedField { .. }
            | HirTypeLocation::EnumVariantPositionalField { .. }
            | HirTypeLocation::TraitSignatureReturn { .. }
            | HirTypeLocation::TraitSignatureParam { .. }
            | HirTypeLocation::Block { .. }
            | HirTypeLocation::LetStmt { .. }
            | HirTypeLocation::Expr { .. }
            | HirTypeLocation::ClosureCapture { .. }
            | HirTypeLocation::ClosureParam { .. }
            | HirTypeLocation::CastTarget { .. }
            | HirTypeLocation::TryOutput { .. }
            | HirTypeLocation::TryResidual { .. }
            | HirTypeLocation::TryReturn { .. }
    )
}

impl HirProgramFor<AcceptedHir> {
    #[cfg(test)]
    pub(crate) fn rebuild_indexes(&mut self) {
        self.rebuild_indexes_with_canonical_names(&HashMap::new());
    }

    pub(crate) fn from_accepted_id_parts_with_names_and_canonical_names(
        functions: HashMap<DefId, HirFunctionFor<AcceptedHir>>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTraitFor<AcceptedHir>>,
        impls: HashMap<DefId, HirImplFor<AcceptedHir>>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Self {
        let impl_order = sorted_definition_ids(&impls);
        let extern_order = sorted_definition_ids(&externs);
        let mut program = Self {
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            type_aliases: HashMap::new(),
            names,
            order: HirProgramOrder {
                impls: impl_order,
                externs: extern_order,
            },
            language_items: HirLanguageItems::default(),
            indexes: HirDefinitionIndexes::default(),
        };
        program.rebuild_indexes_with_canonical_names(canonical_names_by_id);
        program
    }

    pub fn struct_by_id(&self, id: DefId) -> Option<(&str, &HirStruct)> {
        let structure = self.structs.get(&id)?;
        let name = self
            .indexes
            .structs_by_id
            .get(&id)
            .map(String::as_str)
            .unwrap_or(structure.name.as_str());
        Some((name, structure))
    }

    pub fn enum_by_id(&self, id: DefId) -> Option<(&str, &HirEnum)> {
        let enumeration = self.enums.get(&id)?;
        let name = self
            .indexes
            .enums_by_id
            .get(&id)
            .map(String::as_str)
            .unwrap_or(enumeration.name.as_str());
        Some((name, enumeration))
    }

    pub fn rebuild_indexes_with_canonical_names(
        &mut self,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) {
        let effective_trait_methods = std::mem::take(&mut self.indexes.effective_trait_methods);
        self.indexes = HirDefinitionIndexes::from_parts(
            &self.functions,
            &self.structs,
            &self.enums,
            &self.traits,
            &self.impls,
            &self.externs,
            &self.type_aliases,
            &self.names,
            &self.order,
            canonical_names_by_id,
        );
        self.indexes.effective_trait_methods = effective_trait_methods;
    }

    pub fn function_by_id(&self, id: DefId) -> Option<(&str, &HirFunctionFor<AcceptedHir>)> {
        self.functions.get(&id).map(|function| {
            let name = self
                .indexes
                .functions_by_id
                .get(&id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (name, function)
        })
    }

    #[cfg(test)]
    pub(crate) fn test_function_by_name(
        &self,
        name: &str,
    ) -> Option<(DefId, &HirFunctionFor<AcceptedHir>)> {
        let id = *self.names.functions_by_name.get(name)?;
        self.functions.get(&id).map(|function| (id, function))
    }

    #[cfg(test)]
    pub(crate) fn test_struct_by_name(&self, name: &str) -> Option<(DefId, &HirStruct)> {
        let id = *self.names.structs_by_name.get(name)?;
        self.structs.get(&id).map(|structure| (id, structure))
    }

    #[cfg(test)]
    pub(crate) fn test_enum_by_name(&self, name: &str) -> Option<(DefId, &HirEnum)> {
        let id = *self.names.enums_by_name.get(name)?;
        self.enums.get(&id).map(|enumeration| (id, enumeration))
    }

    pub fn functions_by_id(
        &self,
    ) -> impl Iterator<Item = (DefId, &str, &HirFunctionFor<AcceptedHir>)> {
        self.functions.iter().map(|(id, function)| {
            let name = self
                .indexes
                .functions_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(function.name.as_str());
            (*id, name, function)
        })
    }

    pub fn function_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.function_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.functions_by_name, id, Some(primary_name))
    }

    pub fn struct_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.struct_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.structs_by_name, id, Some(primary_name))
    }

    pub fn enum_display_aliases(&self, id: DefId) -> Vec<&str> {
        let Some((primary_name, _)) = self.enum_by_id(id) else {
            return Vec::new();
        };
        display_aliases_for_id(&self.names.enums_by_name, id, Some(primary_name))
    }

    pub fn structs_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirStruct)> {
        self.structs.iter().map(|(id, structure)| {
            let name = self
                .indexes
                .structs_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(structure.name.as_str());
            (*id, name, structure)
        })
    }

    pub fn enums_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirEnum)> {
        self.enums.iter().map(|(id, enumeration)| {
            let name = self
                .indexes
                .enums_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(enumeration.name.as_str());
            (*id, name, enumeration)
        })
    }

    pub fn traits_by_id(&self) -> impl Iterator<Item = (DefId, &str, &HirTraitFor<AcceptedHir>)> {
        self.traits.iter().map(|(id, trait_def)| {
            let name = self
                .indexes
                .traits_by_id
                .get(id)
                .map(String::as_str)
                .unwrap_or(trait_def.name.as_str());
            (*id, name, trait_def)
        })
    }

    pub fn impls_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirImplFor<AcceptedHir>)> {
        self.indexes
            .impls_by_id
            .iter()
            .filter_map(|(id, index)| self.impls.get(id).map(|imp| (*id, *index, imp)))
    }

    pub fn externs_by_id(&self) -> impl Iterator<Item = (DefId, usize, &HirExtern)> {
        self.indexes
            .externs_by_id
            .iter()
            .filter_map(|(id, index)| self.externs.get(id).map(|ext| (*id, *index, ext)))
    }

    pub fn impl_receiver_pattern(&self, impl_id: DefId) -> Option<&HirImplReceiverPattern> {
        self.impls.get(&impl_id).map(|imp| &imp.receiver_pattern)
    }

    pub fn effective_trait_method(&self, impl_id: DefId, member_id: DefId) -> Option<DefId> {
        self.indexes
            .effective_trait_methods
            .get(&(impl_id, member_id))
            .copied()
    }

    pub fn impls_in_order(&self) -> impl Iterator<Item = (DefId, &HirImplFor<AcceptedHir>)> {
        ordered_owned_ids(&self.impls, &self.order.impls)
            .into_iter()
            .filter_map(|id| self.impls.get(&id).map(|imp| (id, imp)))
    }

    pub fn externs_in_order(&self) -> impl Iterator<Item = (DefId, &HirExtern)> {
        ordered_owned_ids(&self.externs, &self.order.externs)
            .into_iter()
            .filter_map(|id| self.externs.get(&id).map(|ext| (id, ext)))
    }
}

pub(crate) fn validate_effective_trait_method_receiver_shape(
    trait_def: &HirTrait,
    imp: &HirImpl,
    member_id: DefId,
    method_id: DefId,
) -> Result<(), String> {
    let mut member_receiver_shape = None;
    for shape in trait_def
        .methods
        .values()
        .filter(|method| method.id == member_id)
        .map(function_has_receiver)
        .chain(
            trait_def
                .signatures
                .values()
                .filter(|signature| signature.id == member_id)
                .map(|signature| signature.self_receiver.is_some()),
        )
    {
        if let Some(existing) = member_receiver_shape {
            if existing != shape {
                return Err(format!(
                    "effective trait member {member_id:?} has conflicting receiver/static representations"
                ));
            }
        } else {
            member_receiver_shape = Some(shape);
        }
    }
    let Some(member_has_receiver) = member_receiver_shape else {
        return Err(format!(
            "effective trait member {member_id:?} is not declared by trait {:?}",
            trait_def.id
        ));
    };
    let Some(method) = imp.methods.values().find(|method| method.id == method_id) else {
        return Err(format!(
            "effective trait member {member_id:?} maps to method {method_id:?} outside impl {:?}",
            imp.id
        ));
    };
    validate_effective_trait_member_receiver_shape(
        member_id,
        method_id,
        member_has_receiver,
        function_has_receiver(method),
    )
}

pub(crate) fn validate_effective_trait_member_receiver_shape(
    member_id: DefId,
    method_id: DefId,
    member_has_receiver: bool,
    method_has_receiver: bool,
) -> Result<(), String> {
    if member_has_receiver != method_has_receiver {
        let member_shape = if member_has_receiver {
            "receiver"
        } else {
            "static"
        };
        let method_shape = if method_has_receiver {
            "receiver"
        } else {
            "static"
        };
        return Err(format!(
            "effective trait member {member_id:?} maps to method {method_id:?} with receiver/static mismatch: trait member is {member_shape}, impl method is {method_shape}"
        ));
    }

    Ok(())
}

pub(crate) fn function_has_receiver(function: &HirFunction) -> bool {
    function.is_method || function.self_receiver.is_some()
}

pub(crate) fn validate_impl_receiver_pattern_bindings<P: HirPhase>(
    imp: &HirImplFor<P>,
    pattern: &HirImplReceiverPattern,
) -> Result<(), String> {
    let mut actual = HashSet::new();
    match pattern {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
            ty.collect_generic_params(&mut actual)
        }
        HirImplReceiverPattern::SliceFamily { element } => {
            element.collect_generic_params(&mut actual)
        }
    }
    for trait_arg in &imp.trait_arg_types {
        trait_arg.collect_generic_params(&mut actual);
    }
    let expected = (0..imp.type_generics.len())
        .map(|index| GenericParamId {
            owner: imp.id,
            index: index as u32,
        })
        .collect::<HashSet<_>>();
    let invalid_expected = expected
        .iter()
        .any(|param| param.owner != imp.id || param.index as usize >= imp.type_generics.len());
    let invalid_actual = actual
        .iter()
        .any(|param| param.owner != imp.id || param.index as usize >= imp.type_generics.len());

    if !invalid_expected && !invalid_actual && expected == actual {
        Ok(())
    } else {
        Err(format!(
            "implementation {:?} typed receiver/trait argument generic bindings are incomplete or invalid for {:?}: required {expected:?}, found {actual:?}",
            imp.id, imp.type_generics
        ))
    }
}

fn validate_method_target_authority(
    program: &HirProgram,
    target: &HirMethodCallTarget,
    errors: &mut Vec<String>,
) {
    let mut seen_bindings = HashSet::new();
    for binding in target
        .owner_substitution
        .iter()
        .chain(target.method_substitution.iter())
    {
        if !seen_bindings.insert(binding.param) {
            errors.push(format!(
                "method authority contains duplicate generic binding {:?}",
                binding.param
            ));
        }
        if type_contains_backend_forbidden_sentinel(&binding.ty) {
            errors.push(format!(
                "method authority binding {:?} is unresolved: {:?}",
                binding.param, binding.ty
            ));
        }
    }
    if target
        .trait_args()
        .iter()
        .any(type_contains_backend_forbidden_sentinel)
    {
        errors.push("method authority contains an unresolved trait argument".to_string());
    }

    let expected_bindings = match &target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        } => {
            let Some(imp) = program.impls.get(impl_id) else {
                errors.push(format!(
                    "method authority references unknown impl {impl_id:?}"
                ));
                return;
            };
            if !imp.methods.values().any(|method| method.id == *method_id) {
                errors.push(format!(
                    "method authority references method {method_id:?} outside impl {impl_id:?}"
                ));
            }
            let owner_params = (0..imp.type_generics.len())
                .map(|index| GenericParamId {
                    owner: *impl_id,
                    index: index as u32,
                })
                .collect::<Vec<_>>();
            let method_params: Vec<GenericParamId> = imp
                .methods
                .values()
                .find(|method| method.id == *method_id)
                .map(|method| {
                    method
                        .generic_params
                        .iter()
                        .map(|param| param.id)
                        .filter(|param| {
                            !owner_params.contains(param)
                                && selected_trait
                                    .as_ref()
                                    .is_none_or(|selected| param.owner != selected.trait_id)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if let Some(selected_trait) = selected_trait {
                if imp.trait_id != Some(selected_trait.trait_id) {
                    errors.push(format!(
                        "method authority impl {impl_id:?} does not implement trait {:?}",
                        selected_trait.trait_id
                    ));
                }
                if program.effective_trait_method(*impl_id, selected_trait.member_id)
                    != Some(*method_id)
                {
                    errors.push(format!(
                        "method authority has no effective mapping from member {:?} to method {method_id:?}",
                        selected_trait.member_id
                    ));
                }
                if let Some(trait_def) = program.traits.get(&selected_trait.trait_id) {
                    if selected_trait.trait_args.len() != trait_def.generic_params.len() {
                        errors.push(format!(
                            "method authority trait argument arity mismatch for {:?}: expected {}, found {}",
                            selected_trait.trait_id,
                            trait_def.generic_params.len(),
                            selected_trait.trait_args.len()
                        ));
                    }
                    let owner_substitution = target
                        .owner_substitution
                        .iter()
                        .map(|binding| (binding.param, binding.ty.clone()))
                        .collect::<HashMap<_, _>>();
                    let normalization_env = program.type_normalization_env();
                    let expected_trait_args = imp
                        .trait_arg_types
                        .iter()
                        .map(|arg| {
                            crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                                .normalize(&arg.substitute_generics(&owner_substitution))
                        })
                        .collect::<Result<Vec<_>, _>>();
                    let expected_trait_args = match expected_trait_args {
                        Ok(args) => args,
                        Err(error) => {
                            errors.push(format!(
                                "cannot normalize method authority trait arguments for impl {impl_id:?}: {error}"
                            ));
                            return;
                        }
                    };
                    if expected_trait_args != selected_trait.trait_args {
                        errors.push(format!(
                            "method authority trait arguments do not match impl {impl_id:?}: expected {expected_trait_args:?}, found {:?}",
                            selected_trait.trait_args
                        ));
                    }
                }
            } else if imp.trait_id.is_some() {
                errors.push(format!(
                    "trait impl method authority for impl {impl_id:?} has no selected trait identity"
                ));
            }
            Some((owner_params, method_params))
        }
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => {
            let Some(trait_def) = program.traits.get(trait_id) else {
                errors.push(format!(
                    "method authority references unknown trait {trait_id:?}"
                ));
                return;
            };
            if target.trait_args().len() != trait_def.generic_params.len() {
                errors.push(format!(
                    "method authority trait argument arity mismatch for {trait_id:?}: expected {}, found {}",
                    trait_def.generic_params.len(),
                    target.trait_args().len()
                ));
            }
            if !trait_def
                .methods
                .values()
                .any(|method| method.id == *member_id)
                && !trait_def
                    .signatures
                    .values()
                    .any(|signature| signature.id == *member_id)
            {
                errors.push(format!(
                    "method authority references unknown member {member_id:?} on trait {trait_id:?}"
                ));
            }
            let is_constructor_trait = trait_def.target.as_ref().is_some_and(|target| {
                !matches!(target.kind, crate::type_services::kind::Kind::Type)
            });
            let mut owner_params = if is_constructor_trait {
                trait_def
                    .generic_params
                    .iter()
                    .map(|param| param.id)
                    .chain(trait_def.target.iter().map(|target| target.id))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            owner_params
                .sort_by_key(|param| (param.owner.crate_id.0, param.owner.local.0, param.index));
            let method_params: Vec<GenericParamId> = trait_def
                .methods
                .values()
                .find(|method| method.id == *member_id)
                .map(|method| {
                    method
                        .generic_params
                        .iter()
                        .map(|param| param.id)
                        .filter(|param| {
                            if is_constructor_trait {
                                !owner_params.contains(param)
                            } else {
                                param.owner == method.id
                            }
                        })
                        .collect()
                })
                .or_else(|| {
                    trait_def
                        .signatures
                        .values()
                        .find(|signature| signature.id == *member_id)
                        .map(|signature| {
                            signature
                                .generic_params
                                .iter()
                                .map(|param| param.id)
                                .filter(|param| {
                                    if is_constructor_trait {
                                        !owner_params.contains(param)
                                    } else {
                                        param.owner == signature.id
                                    }
                                })
                                .collect()
                        })
                })
                .unwrap_or_default();
            Some((owner_params, method_params))
        }
    };

    if let Some((expected_owner, expected_method)) = expected_bindings {
        let actual_owner = target
            .owner_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<Vec<_>>();
        let actual_method = target
            .method_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<Vec<_>>();
        if actual_owner != expected_owner {
            errors.push(format!(
                "method authority {:?} owner bindings are incomplete or non-canonical: expected {expected_owner:?}, found {actual_owner:?}",
                target.target
            ));
        }
        if actual_method != expected_method {
            errors.push(format!(
                "method authority {:?} method bindings are incomplete or non-canonical: expected {expected_method:?}, found {actual_method:?}",
                target.target
            ));
        }
    }
}

fn validate_method_authorities_in_block(
    program: &HirProgram,
    block: &HirBlock,
    errors: &mut Vec<String>,
) {
    for stmt in &block.stmts {
        match stmt {
            HirStmtFor::Let { value, .. }
            | HirStmtFor::Expr(value)
            | HirStmtFor::Return(Some(value))
            | HirStmtFor::Break(Some(value)) => {
                validate_method_authorities_in_expr(program, value, errors)
            }
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
        }
    }
}

fn validate_method_authorities_in_expr(
    program: &HirProgram,
    expr: &HirExpr,
    errors: &mut Vec<String>,
) {
    match &expr.kind {
        HirExprKindFor::MethodCall(receiver, _, args, self_receiver, target) => {
            validate_method_authorities_in_expr(program, receiver, errors);
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
            match target {
                Some(target) => {
                    validate_method_target_authority(program, target, errors);
                    let expected = method_target_self_receiver(program, target);
                    if expected == Some(None) {
                        errors.push("method-call authority references a static method".to_string());
                    }
                    if expected != Some(*self_receiver) {
                        errors.push(format!(
                            "method authority receiver mode mismatch: expected {expected:?}, found {self_receiver:?}"
                        ));
                    }
                }
                None => {
                    errors.push("accepted HIR method call has no selected authority".to_string())
                }
            }
        }
        HirExprKindFor::Call(callee, args, target) => {
            let callee_is_authorized_method = matches!(
                (&callee.kind, target),
                (
                    HirExprKindFor::ResolvedVar(HirVarRef {
                        target: HirVarTarget::Function(callee_id),
                        ..
                    }),
                    Some(HirCallTarget::StaticMethod(target))
                ) if target.method.method_id() == Some(*callee_id)
            );
            if !callee_is_authorized_method {
                validate_method_authorities_in_expr(program, callee, errors);
            }
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
            if let Some(HirCallTarget::StaticMethod(target)) = target {
                validate_method_target_authority(program, &target.method, errors);
                if method_target_self_receiver(program, &target.method) != Some(None) {
                    errors.push("static method authority references a receiver method".to_string());
                }
                if type_contains_backend_forbidden_sentinel(&target.owner_ty) {
                    errors.push("static method authority has an unresolved owner type".to_string());
                }
            }
            if let Some(HirCallTarget::Function(id)) = target {
                if program.indexes.methods_by_id.contains_key(id) {
                    errors.push(format!(
                        "accepted HIR static method call {id:?} ({:?}) has no selected static-method authority",
                        callee.kind
                    ));
                }
            }
            if matches!(target, Some(HirCallTarget::Instance(_))) {
                errors.push(
                    "accepted HIR contains a pre-monomorphization instance call target".to_string(),
                );
            }
            if target.is_none() && matches!(&callee.kind, HirExprKindFor::FieldAccess(_, _, None)) {
                errors.push("accepted HIR contains an unresolved field method call".to_string());
            }
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            branch_target,
            branch_self_receiver,
            from_residual_target,
            ..
        } => {
            validate_method_authorities_in_expr(program, expr, errors);
            match branch_method {
                Some(target) => {
                    validate_method_target_authority(program, target, errors);
                    let expected = method_target_self_receiver(program, target);
                    if expected != Some(*branch_self_receiver) {
                        errors.push(format!(
                            "Try branch receiver mode mismatch: expected {expected:?}, found {branch_self_receiver:?}"
                        ));
                    }
                }
                None => errors.push("accepted Try expression has no branch authority".to_string()),
            }
            if branch_target.is_some() {
                errors.push(
                    "accepted Try expression contains a pre-monomorphization instance target"
                        .to_string(),
                );
            }
            match from_residual_target {
                Some(HirCallTarget::StaticMethod(target)) => {
                    validate_method_target_authority(program, &target.method, errors);
                    if type_contains_backend_forbidden_sentinel(&target.owner_ty) {
                        errors.push(
                            "Try from-residual authority has an unresolved owner type".to_string(),
                        );
                    }
                }
                Some(_) => errors.push(
                    "accepted Try expression has a non-static from-residual authority".to_string(),
                ),
                None => errors
                    .push("accepted Try expression has no from-residual authority".to_string()),
            }
        }
        HirExprKindFor::FieldAccess(base, field_name, location) => {
            validate_method_authorities_in_expr(program, base, errors);
            if location.is_none() {
                errors.push(format!(
                    "accepted HIR contains unresolved field access '{field_name}'"
                ));
            }
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values) => {
            for value in values {
                validate_method_authorities_in_expr(program, value, errors);
            }
        }
        HirExprKindFor::ArrayRepeat(value, _) => {
            validate_method_authorities_in_expr(program, value, errors);
        }
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields {
                validate_method_authorities_in_expr(program, &field.value, errors);
            }
        }
        HirExprKindFor::EnumVariant(_, _, args, _) | HirExprKindFor::Intrinsic { args, .. } => {
            for arg in args {
                validate_method_authorities_in_expr(program, arg, errors);
            }
        }
        HirExprKindFor::TupleIndex(inner, _)
        | HirExprKindFor::UnaryOp(_, inner)
        | HirExprKindFor::Ref(_, inner)
        | HirExprKindFor::Deref(inner)
        | HirExprKindFor::Cast(inner, _) => {
            validate_method_authorities_in_expr(program, inner, errors)
        }
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            validate_method_authorities_in_expr(program, left, errors);
            validate_method_authorities_in_expr(program, right, errors);
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            validate_method_authorities_in_expr(program, condition, errors);
            validate_method_authorities_in_block(program, then_branch, errors);
            if let Some(else_branch) = else_branch {
                validate_method_authorities_in_block(program, else_branch, errors);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            validate_method_authorities_in_expr(program, scrutinee, errors);
            for arm in arms {
                if let Some(guard) = &arm.guard {
                    validate_method_authorities_in_expr(program, guard, errors);
                }
                validate_method_authorities_in_block(program, &arm.body, errors);
            }
        }
        HirExprKindFor::While { condition, body } => {
            validate_method_authorities_in_expr(program, condition, errors);
            validate_method_authorities_in_block(program, body, errors);
        }
        HirExprKindFor::For { iter, body, .. } => {
            validate_method_authorities_in_expr(program, iter, errors);
            validate_method_authorities_in_block(program, body, errors);
        }
        HirExprKindFor::Lambda { body, .. }
        | HirExprKindFor::Block(body)
        | HirExprKindFor::Loop(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            validate_method_authorities_in_block(program, body, errors)
        }
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_) => {}
        HirExprKindFor::ResolvedVar(reference) => {
            if matches!(reference.target, HirVarTarget::Instance(_)) {
                errors.push(
                    "accepted HIR contains a pre-monomorphization instance variable target"
                        .to_string(),
                );
            }
            if matches!(reference.target, HirVarTarget::Function(id) if program.indexes.methods_by_id.contains_key(&id))
            {
                errors.push(format!(
                    "accepted HIR callable retains raw method DefId {:?}",
                    reference.target
                ));
            }
        }
    }
}

fn method_target_self_receiver(
    program: &HirProgram,
    target: &HirMethodCallTarget,
) -> Option<Option<ReceiverMode>> {
    match target.target {
        HirSelectedMethodTarget::ImplMethod {
            impl_id, method_id, ..
        } => program.impls.get(&impl_id).and_then(|imp| {
            imp.methods
                .values()
                .find(|method| method.id == method_id)
                .map(|method| method.self_receiver)
        }),
        HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            ..
        } => program.traits.get(&trait_id).and_then(|trait_def| {
            trait_def
                .methods
                .values()
                .find(|method| method.id == member_id)
                .map(|method| method.self_receiver)
                .or_else(|| {
                    trait_def
                        .signatures
                        .values()
                        .find(|signature| signature.id == member_id)
                        .map(|signature| signature.self_receiver)
                })
        }),
    }
}

fn type_contains_backend_forbidden_sentinel(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::TypeVar(_) | Type::Error)
    })
}

fn projection_input_is_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

fn projection_impl_matches(
    imp: &HirImpl,
    base_ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    assoc_type_id: AssocTypeId,
) -> bool {
    if imp.trait_id != Some(trait_id)
        || imp.trait_arg_types.len() != trait_args.len()
        || !imp
            .associated_types
            .iter()
            .any(|associated| associated.id == assoc_type_id)
    {
        return false;
    }
    let Some(mut substitution) =
        crate::selection::receiver_pattern_substitution(&imp.receiver_pattern, base_ty)
    else {
        return false;
    };
    imp.trait_arg_types
        .iter()
        .zip(trait_args)
        .all(|(expected, actual)| {
            crate::selection::type_pattern_matches(expected, actual, &mut substitution)
        })
}

fn validate_projection_ambiguities(
    program: &HirProgram,
    ty: &Type,
    location: &str,
    errors: &mut Vec<String>,
) {
    match ty {
        Type::Projection {
            ty: base_ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            if assoc_type.owner == *trait_id
                && projection_input_is_concrete(base_ty)
                && trait_args.iter().all(projection_input_is_concrete)
            {
                let mut matching = program
                    .impls
                    .values()
                    .filter(|imp| {
                        projection_impl_matches(
                            imp,
                            base_ty,
                            *trait_id,
                            trait_args,
                            assoc_type.assoc_type_id,
                        )
                    })
                    .map(|imp| imp.id)
                    .collect::<Vec<_>>();
                matching.sort();
                matching.dedup();
                if matching.len() > 1 {
                    errors.push(format!(
                        "accepted HIR contains multiple matching typed projection implementations at {location}: {matching:?}"
                    ));
                }
            }
            validate_projection_ambiguities(program, base_ty, location, errors);
            for arg in trait_args {
                validate_projection_ambiguities(program, arg, location, errors);
            }
        }
        Type::Slice(inner) | Type::Pointer(inner) | Type::Array(inner, _) => {
            validate_projection_ambiguities(program, inner, location, errors)
        }
        Type::Reference { inner, .. } => {
            validate_projection_ambiguities(program, inner, location, errors)
        }
        Type::Tuple(types) => {
            for ty in types {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for ty in params {
                validate_projection_ambiguities(program, ty, location, errors);
            }
            validate_projection_ambiguities(program, ret, location, errors);
            for capture in captures {
                validate_projection_ambiguities(program, &capture.ty, location, errors);
            }
        }
        Type::Struct { args, .. } | Type::Enum { args, .. } => {
            for ty in args {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Apply { constructor, args } => {
            validate_projection_ambiguities(program, constructor, location, errors);
            for ty in args {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Lambda { body, .. } => {
            validate_projection_ambiguities(program, body, location, errors)
        }
        _ => {}
    }
}

fn display_aliases_for_id<'a>(
    names_by_name: &'a HashMap<String, DefId>,
    owner_id: DefId,
    primary_name: Option<&'a str>,
) -> Vec<&'a str> {
    let mut names = primary_name.into_iter().collect::<Vec<_>>();
    let mut aliases = names_by_name
        .iter()
        .filter_map(|(name, id)| (*id == owner_id).then_some(name.as_str()))
        .collect::<Vec<_>>();
    aliases.sort();

    for alias in aliases {
        if !names.contains(&alias) {
            names.push(alias);
        }
    }

    names
}

pub fn hir_function_is_codegen_concrete<P: HirPhase>(function: &HirFunctionFor<P>) -> bool {
    hir_function_signature_is_codegen_concrete(function)
        && hir_block_is_codegen_concrete(&function.body)
}

pub(crate) fn hir_function_signature_is_codegen_concrete<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    function
        .params
        .iter()
        .all(|param| hir_type_is_codegen_concrete(&param.ty))
        && hir_type_is_codegen_concrete(&function.ret_type)
}

pub(crate) fn function_requires_downstream_specialization<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    !function.generic_params.is_empty()
}

pub(crate) fn function_is_object_provided_candidate<P: HirPhase>(
    function: &HirFunctionFor<P>,
) -> bool {
    !function_requires_downstream_specialization(function)
        && hir_function_is_codegen_concrete(function)
}

pub(crate) fn impl_requires_downstream_specialization<P: HirPhase>(imp: &HirImplFor<P>) -> bool {
    !imp.type_generics.is_empty()
        || !imp.trait_generics.is_empty()
        || imp
            .methods
            .values()
            .any(|method| !method.generic_params.is_empty())
}

fn hir_type_is_codegen_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

fn hir_block_is_codegen_concrete<P: HirPhase>(block: &HirBlockFor<P>) -> bool {
    hir_type_is_codegen_concrete(&block.ty) && block.stmts.iter().all(hir_stmt_is_codegen_concrete)
}

fn hir_stmt_is_codegen_concrete<P: HirPhase>(stmt: &HirStmtFor<P>) -> bool {
    match stmt {
        HirStmtFor::Let { ty, value, .. } => {
            hir_type_is_codegen_concrete(ty) && hir_expr_is_codegen_concrete(value)
        }
        HirStmtFor::Expr(expr) | HirStmtFor::Return(Some(expr)) | HirStmtFor::Break(Some(expr)) => {
            hir_expr_is_codegen_concrete(expr)
        }
        HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => true,
    }
}

fn hir_method_target_is_codegen_concrete(target: &HirMethodCallTarget) -> bool {
    target
        .trait_args()
        .iter()
        .all(crate::type_services::facts::TypeFacts::is_concrete_type_argument)
        && target
            .owner_substitution
            .iter()
            .chain(target.method_substitution.iter())
            .all(|binding| {
                crate::type_services::facts::TypeFacts::is_concrete_type_argument(&binding.ty)
            })
}

fn hir_call_target_is_codegen_concrete(target: &HirCallTarget) -> bool {
    match target {
        HirCallTarget::StaticMethod(target) => {
            crate::type_services::facts::TypeFacts::is_concrete_type_argument(&target.owner_ty)
                && hir_method_target_is_codegen_concrete(&target.method)
        }
        _ => true,
    }
}

fn hir_expr_is_codegen_concrete<P: HirPhase>(expr: &HirExprFor<P>) -> bool {
    if !hir_type_is_codegen_concrete(&expr.ty) {
        return false;
    }

    match &expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => true,
        HirExprKindFor::ArrayLiteral(elems) | HirExprKindFor::TupleLiteral(elems) => {
            elems.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::ArrayRepeat(value, _) => hir_expr_is_codegen_concrete(value),
        HirExprKindFor::StructLiteral(_, _, fields) => fields
            .iter()
            .all(|field| hir_expr_is_codegen_concrete(&field.value)),
        HirExprKindFor::EnumVariant(_, _, args, _) | HirExprKindFor::Intrinsic { args, .. } => {
            args.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::FieldAccess(base, _, _)
        | HirExprKindFor::TupleIndex(base, _)
        | HirExprKindFor::Deref(base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::UnaryOp(_, base)
        | HirExprKindFor::Cast(base, _) => hir_expr_is_codegen_concrete(base),
        HirExprKindFor::BinOp(_, base, index) | HirExprKindFor::Assign(base, index) => {
            hir_expr_is_codegen_concrete(base) && hir_expr_is_codegen_concrete(index)
        }
        HirExprKindFor::Call(func, args, target) => {
            target
                .as_ref()
                .is_none_or(hir_call_target_is_codegen_concrete)
                && hir_expr_is_codegen_concrete(func)
                && args.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::MethodCall(recv, _, args, _, target) => {
            P::method_authority(target).is_none_or(hir_method_target_is_codegen_concrete)
                && hir_expr_is_codegen_concrete(recv)
                && args.iter().all(hir_expr_is_codegen_concrete)
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            hir_expr_is_codegen_concrete(expr)
                && P::method_authority(branch_method)
                    .is_none_or(hir_method_target_is_codegen_concrete)
                && P::residual_authority(from_residual_target)
                    .is_none_or(hir_call_target_is_codegen_concrete)
                && hir_type_is_codegen_concrete(output_ty)
                && hir_type_is_codegen_concrete(residual_ty)
                && hir_type_is_codegen_concrete(return_ty)
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            hir_expr_is_codegen_concrete(condition)
                && hir_block_is_codegen_concrete(then_branch)
                && else_branch
                    .as_ref()
                    .is_none_or(|block| hir_block_is_codegen_concrete(block))
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            hir_expr_is_codegen_concrete(scrutinee)
                && arms.iter().all(|arm| {
                    hir_pattern_is_codegen_concrete(&arm.pattern)
                        && arm.guard.as_ref().is_none_or(hir_expr_is_codegen_concrete)
                        && hir_block_is_codegen_concrete(&arm.body)
                })
        }
        HirExprKindFor::While { condition, body } => {
            hir_expr_is_codegen_concrete(condition) && hir_block_is_codegen_concrete(body)
        }
        HirExprKindFor::For { iter, body, .. } => {
            hir_expr_is_codegen_concrete(iter) && hir_block_is_codegen_concrete(body)
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => hir_block_is_codegen_concrete(body),
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            params
                .iter()
                .all(|param| hir_type_is_codegen_concrete(&param.ty))
                && captures
                    .iter()
                    .all(|capture| hir_type_is_codegen_concrete(&capture.ty))
                && hir_block_is_codegen_concrete(body)
        }
    }
}

fn hir_pattern_is_codegen_concrete(pattern: &HirPattern) -> bool {
    match pattern {
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => true,
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            patterns.iter().all(hir_pattern_is_codegen_concrete)
        }
        HirPattern::Struct(_, _, type_args, fields) => {
            type_args.iter().all(hir_type_is_codegen_concrete)
                && fields
                    .iter()
                    .all(|field| hir_pattern_is_codegen_concrete(&field.pattern))
        }
        HirPattern::Enum(_, _, _, patterns) => patterns.iter().all(hir_pattern_is_codegen_concrete),
    }
}

fn sorted_definition_ids<T>(definitions: &HashMap<DefId, T>) -> Vec<DefId> {
    let mut ids = definitions.keys().copied().collect::<Vec<_>>();
    ids.sort();
    ids
}

impl HirDefinitionIndexes {
    fn from_parts<P: HirPhase>(
        _functions: &HashMap<DefId, HirFunctionFor<P>>,
        structs: &HashMap<DefId, HirStruct>,
        enums: &HashMap<DefId, HirEnum>,
        traits: &HashMap<DefId, HirTraitFor<P>>,
        impls: &HashMap<DefId, HirImplFor<P>>,
        externs: &HashMap<DefId, HirExtern>,
        type_aliases: &HashMap<DefId, HirTypeAlias>,
        names: &HirNameTables,
        order: &HirProgramOrder,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Self {
        let mut indexes = Self::default();

        index_owned_named_definitions(
            &mut indexes.functions_by_id,
            _functions,
            &names.functions_by_name,
            |function| function.name.as_str(),
            canonical_names_by_id,
            "function",
        );
        index_owned_named_definitions(
            &mut indexes.structs_by_id,
            structs,
            &names.structs_by_name,
            |structure| structure.name.as_str(),
            canonical_names_by_id,
            "struct",
        );
        index_owned_named_definitions(
            &mut indexes.enums_by_id,
            enums,
            &names.enums_by_name,
            |enumeration| enumeration.name.as_str(),
            canonical_names_by_id,
            "enum",
        );
        index_owned_named_definitions(
            &mut indexes.traits_by_id,
            traits,
            &names.traits_by_name,
            |trait_def| trait_def.name.as_str(),
            canonical_names_by_id,
            "trait",
        );
        index_owned_named_definitions(
            &mut indexes.type_aliases_by_id,
            type_aliases,
            &names.type_aliases_by_name,
            |alias| alias.name.as_str(),
            canonical_names_by_id,
            "type alias",
        );

        for structure in structs.values() {
            for field in &structure.fields {
                insert_unique_child(
                    &mut indexes.fields_by_id,
                    (structure.id, field.id),
                    HirFieldLocation {
                        owner: structure.id,
                        field_id: field.id,
                        name: field.name.clone(),
                    },
                    "field",
                );
            }
        }

        for enumeration in enums.values() {
            for variant in &enumeration.variants {
                insert_unique_child(
                    &mut indexes.variants_by_id,
                    (enumeration.id, variant.id),
                    HirVariantLocation {
                        owner: enumeration.id,
                        variant_id: variant.id,
                        name: variant.name.clone(),
                    },
                    "variant",
                );

                if let HirVariantFields::Named(fields) = &variant.fields {
                    for field in fields {
                        insert_unique_child(
                            &mut indexes.fields_by_id,
                            (enumeration.id, field.id),
                            HirFieldLocation {
                                owner: enumeration.id,
                                field_id: field.id,
                                name: field.name.clone(),
                            },
                            "field",
                        );
                    }
                }
            }
        }

        for trait_def in traits.values() {
            for associated_type in &trait_def.associated_types {
                insert_unique_child(
                    &mut indexes.associated_type_decls_by_id,
                    (trait_def.id, associated_type.id),
                    HirAssociatedTypeLocation {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                        name: associated_type.name.clone(),
                    },
                    "associated type declaration",
                );
            }
        }

        for (index, id) in ordered_owned_ids(impls, &order.impls).iter().enumerate() {
            let imp = &impls[id];
            insert_unique(&mut indexes.impls_by_id, *id, index, "impl");
            for associated_type in &imp.associated_types {
                insert_unique_child(
                    &mut indexes.associated_type_defs_by_id,
                    (imp.id, associated_type.id),
                    HirAssociatedTypeLocation {
                        owner: imp.id,
                        assoc_type_id: associated_type.id,
                        name: associated_type.name.clone(),
                    },
                    "associated type definition",
                );
            }
            for (method_name, method) in &imp.methods {
                insert_unique(
                    &mut indexes.methods_by_id,
                    method.id,
                    HirMethodLocation::ImplMethod {
                        impl_id: imp.id,
                        method_id: method.id,
                        method_name: method_name.clone(),
                    },
                    "method",
                );
            }
        }

        for (index, id) in ordered_owned_ids(externs, &order.externs)
            .iter()
            .enumerate()
        {
            let ext = &externs[id];
            insert_unique(&mut indexes.externs_by_id, ext.id, index, "extern");
        }

        index_trait_default_methods(
            &mut indexes.methods_by_id,
            traits,
            &names.traits_by_name,
            canonical_names_by_id,
        );

        indexes
    }
}

fn ordered_owned_ids<T>(definitions: &HashMap<DefId, T>, preferred_order: &[DefId]) -> Vec<DefId> {
    let mut seen = HashSet::new();
    let mut ids = preferred_order
        .iter()
        .copied()
        .filter(|id| definitions.contains_key(id) && seen.insert(*id))
        .collect::<Vec<_>>();

    let mut missing = definitions
        .keys()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    missing.sort();
    ids.extend(missing);
    ids
}

fn index_trait_default_methods<P: HirPhase>(
    output: &mut HashMap<DefId, HirMethodLocation>,
    traits: &HashMap<DefId, HirTraitFor<P>>,
    traits_by_name: &HashMap<String, DefId>,
    canonical_names_by_id: &HashMap<DefId, String>,
) {
    let mut grouped: HashMap<DefId, Vec<TraitDefaultMethodCandidate>> = HashMap::new();
    let mut trait_ids = traits.keys().copied().collect::<Vec<_>>();
    trait_ids.sort();

    for trait_id in trait_ids {
        let trait_def = &traits[&trait_id];
        let mut trait_names = traits_by_name
            .iter()
            .filter_map(|(name, name_id)| (name_id == &trait_id).then(|| name.clone()))
            .collect::<Vec<_>>();
        if !trait_names.iter().any(|name| name == &trait_def.name) {
            trait_names.push(trait_def.name.clone());
        }
        trait_names.sort();
        trait_names.dedup();

        let mut method_names = trait_def.methods.keys().cloned().collect::<Vec<_>>();
        method_names.sort();

        for trait_name in trait_names {
            for method_name in &method_names {
                let method = &trait_def.methods[method_name];
                grouped
                    .entry(method.id)
                    .or_default()
                    .push(TraitDefaultMethodCandidate {
                        trait_id: trait_def.id,
                        trait_name: trait_name.clone(),
                        method_name: method_name.clone(),
                    });
            }
        }
    }

    let mut method_ids = grouped.keys().copied().collect::<Vec<_>>();
    method_ids.sort();

    for method_id in method_ids {
        let candidates = grouped.remove(&method_id).unwrap_or_default();
        let selected =
            select_trait_default_method_candidate(method_id, candidates, canonical_names_by_id);
        insert_unique(
            output,
            method_id,
            HirMethodLocation::TraitDefault {
                trait_id: selected.trait_id,
                method_id,
                method_name: selected.method_name,
            },
            "method",
        );
    }
}

fn select_trait_default_method_candidate(
    method_id: DefId,
    candidates: Vec<TraitDefaultMethodCandidate>,
    canonical_names_by_id: &HashMap<DefId, String>,
) -> TraitDefaultMethodCandidate {
    if candidates.len() == 1 {
        return candidates.into_iter().next().unwrap();
    }

    let canonical_matches = candidates
        .iter()
        .filter(|candidate| {
            canonical_names_by_id
                .get(&candidate.trait_id)
                .is_some_and(|canonical_name| canonical_name == &candidate.trait_name)
        })
        .cloned()
        .collect::<Vec<_>>();
    if canonical_matches.len() == 1 {
        return canonical_matches.into_iter().next().unwrap();
    }

    if let Some(candidate) = single_qualified_trait_default_candidate(&candidates) {
        return candidate.clone();
    }

    panic!(
        "duplicate HIR trait default method DefId {:?}: {:?}",
        method_id, candidates
    );
}

fn single_qualified_trait_default_candidate(
    candidates: &[TraitDefaultMethodCandidate],
) -> Option<&TraitDefaultMethodCandidate> {
    let first = candidates.first()?;
    if candidates.iter().any(|candidate| {
        candidate.trait_id != first.trait_id || candidate.method_name != first.method_name
    }) {
        return None;
    }

    let mut qualified_candidates = candidates
        .iter()
        .filter(|candidate| candidate.trait_name.contains("::"));
    let qualified_candidate = qualified_candidates.next()?;
    if qualified_candidates.next().is_none() {
        Some(qualified_candidate)
    } else {
        None
    }
}

fn index_owned_named_definitions<T, NameOf>(
    output: &mut HashMap<DefId, String>,
    definitions: &HashMap<DefId, T>,
    names_by_name: &HashMap<String, DefId>,
    name_of: NameOf,
    canonical_names_by_id: &HashMap<DefId, String>,
    kind: &str,
) where
    NameOf: Fn(&T) -> &str,
{
    for (id, definition) in definitions {
        let default_name = name_of(definition);
        let mut stored_names = names_by_name
            .iter()
            .filter_map(|(name, name_id)| (name_id == id).then(|| name.clone()))
            .collect::<Vec<_>>();
        stored_names.sort();
        stored_names.dedup();

        let mut names = stored_names.clone();
        if !names.iter().any(|name| name == default_name) {
            names.push(default_name.to_string());
        }
        names.sort();
        names.dedup();

        let selected = if let Some(canonical_name) = canonical_names_by_id.get(id) {
            if names.iter().any(|name| name == canonical_name) {
                canonical_name.clone()
            } else if let Some(short_canonical_name) = canonical_name.rsplit("::").next() {
                if names.iter().any(|name| name == short_canonical_name) {
                    short_canonical_name.to_string()
                } else if names.len() == 1 {
                    names[0].clone()
                } else if let Some(canonical_path_name) = single_canonical_path_name(&names) {
                    canonical_path_name.clone()
                } else if let Some(mangled_name) = single_mangled_name(&names) {
                    mangled_name.clone()
                } else {
                    panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
                }
            } else if names.len() == 1 {
                names[0].clone()
            } else if let Some(canonical_path_name) = single_canonical_path_name(&names) {
                canonical_path_name.clone()
            } else {
                panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
            }
        } else if names.len() == 1 {
            names[0].clone()
        } else if stored_names.len() == 1 {
            stored_names[0].clone()
        } else {
            panic!("duplicate HIR {} DefId {:?}: {:?}", kind, id, names);
        };

        insert_unique(output, *id, selected, kind);
    }
}

fn single_canonical_path_name(names: &[String]) -> Option<&String> {
    let mut canonical_path_names = names.iter().filter(|name| name.contains("::"));
    let canonical_path_name = canonical_path_names.next()?;
    if canonical_path_names.next().is_none() {
        Some(canonical_path_name)
    } else {
        None
    }
}

fn single_mangled_name(names: &[String]) -> Option<&String> {
    let mut mangled_names = names.iter().filter(|name| name.contains('_'));
    let mangled_name = mangled_names.next()?;
    if mangled_names.next().is_none() {
        Some(mangled_name)
    } else {
        None
    }
}

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

fn insert_unique<T>(map: &mut HashMap<DefId, T>, id: DefId, value: T, kind: &str) {
    assert!(
        map.insert(id, value).is_none(),
        "duplicate HIR {} DefId: {:?}",
        kind,
        id
    );
}

fn insert_unique_child<K, T>(map: &mut HashMap<K, T>, key: K, value: T, kind: &str)
where
    K: Copy + Eq + Hash + std::fmt::Debug,
    T: PartialEq,
{
    if let Some(existing) = map.get(&key) {
        assert!(
            existing == &value,
            "duplicate HIR {} identity: {:?}",
            kind,
            key
        );
        return;
    }

    map.insert(key, value);
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirFunctionFor<P: HirPhase> {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub generic_bounds: HirGenericBounds,
    pub params: Vec<HirParam>,
    pub ret_type: Type,
    pub body: HirBlockFor<P>,
    pub is_curried: bool,
    pub is_method: bool,
    pub self_receiver: Option<ReceiverMode>,
    pub is_unsafe: bool,
}

pub type HirFunction = HirFunctionFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirParam {
    pub name: String,
    pub local_id: HirLocalId,
    pub ty: Type,
    pub mutable: bool,
    pub is_ref: bool, // parameter is passed by reference
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirClosureCapture {
    pub name: String,
    pub local_id: HirLocalId,
    pub kind: HirClosureCaptureKind,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirClosureCaptureKind {
    SharedBorrow,
    MutableBorrow,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirMethodCallTarget {
    pub target: HirSelectedMethodTarget,
    pub owner_substitution: Vec<HirTypeBinding>,
    pub method_substitution: Vec<HirTypeBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirSelectedMethodTarget {
    ImplMethod {
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    },
    TraitMethod {
        trait_id: DefId,
        member_id: DefId,
        trait_args: Vec<Type>,
        dispatch: HirTraitDispatchKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirSelectedTraitMember {
    pub trait_id: DefId,
    pub member_id: DefId,
    pub trait_args: Vec<Type>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirTraitDispatchKind {
    TraitBound,
    CurrentTrait,
    UnresolvedGeneric,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirTypeBinding {
    pub param: GenericParamId,
    pub ty: Type,
}

impl HirMethodCallTarget {
    pub fn impl_method(
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    ) -> Self {
        Self {
            target: HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        }
    }

    pub fn trait_method(
        trait_id: DefId,
        member_id: DefId,
        trait_args: Vec<Type>,
        dispatch: HirTraitDispatchKind,
    ) -> Self {
        Self {
            target: HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        }
    }

    pub fn impl_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { impl_id, .. } => Some(impl_id),
            HirSelectedMethodTarget::TraitMethod { .. } => None,
        }
    }

    pub fn trait_id(&self) -> Option<DefId> {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => {
                selected_trait.as_ref().map(|selected| selected.trait_id)
            }
            HirSelectedMethodTarget::TraitMethod { trait_id, .. } => Some(*trait_id),
        }
    }

    pub fn method_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { method_id, .. } => Some(method_id),
            HirSelectedMethodTarget::TraitMethod { member_id, .. } => Some(member_id),
        }
    }

    pub fn trait_args(&self) -> &[Type] {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait
                .as_ref()
                .map(|selected| selected.trait_args.as_slice())
                .unwrap_or_default(),
            HirSelectedMethodTarget::TraitMethod { trait_args, .. } => trait_args,
        }
    }

    pub fn trait_args_mut(&mut self) -> Option<&mut Vec<Type>> {
        match &mut self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait
                .as_mut()
                .map(|selected| &mut selected.trait_args),
            HirSelectedMethodTarget::TraitMethod { trait_args, .. } => Some(trait_args),
        }
    }

    pub fn set_impl_method(
        &mut self,
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    ) {
        self.target = HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        };
    }

    pub fn for_each_type_mut(&mut self, mut visit: impl FnMut(&mut Type)) {
        if let Some(trait_args) = self.trait_args_mut() {
            for ty in trait_args {
                visit(ty);
            }
        }
        for binding in self
            .owner_substitution
            .iter_mut()
            .chain(self.method_substitution.iter_mut())
        {
            visit(&mut binding.ty);
        }
    }

    pub fn for_each_def_id_mut(&mut self, mut visit: impl FnMut(&mut DefId)) {
        match &mut self.target {
            HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => {
                visit(impl_id);
                visit(method_id);
                if let Some(selected_trait) = selected_trait {
                    visit(&mut selected_trait.trait_id);
                    visit(&mut selected_trait.member_id);
                }
            }
            HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                ..
            } => {
                visit(trait_id);
                visit(member_id);
            }
        }
        for binding in self
            .owner_substitution
            .iter_mut()
            .chain(self.method_substitution.iter_mut())
        {
            visit(&mut binding.param.owner);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirStaticMethodTarget {
    pub owner_ty: Type,
    pub method: HirMethodCallTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStruct {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub fields: Vec<HirField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirField {
    pub id: FieldId,
    pub name: String,
    pub ty: Type,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirEnum {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub variants: Vec<HirVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirVariant {
    pub id: VariantId,
    pub name: String,
    pub fields: HirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirVariantFields {
    Named(Vec<HirField>),
    Positional(Vec<Type>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirTraitFor<P: HirPhase> {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub target: Option<crate::types::GenericParamDecl>,
    pub predicates: Vec<crate::types::Predicate>,
    pub associated_types: Vec<HirAssociatedTypeDecl>,
    pub methods: HashMap<String, HirFunctionFor<P>>,
    pub signatures: HashMap<String, HirFunctionSig>,
}

pub type HirTrait = HirTraitFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirAssociatedTypeDecl {
    pub id: AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirFunctionSig {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub params: Vec<Type>,
    pub ret: Type,
    pub generic_bounds: HirGenericBounds,
    pub self_receiver: Option<ReceiverMode>,
    pub is_unsafe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirImplFor<P: HirPhase> {
    pub id: DefId,
    pub owner: HirImplOwner,
    pub type_name: String,
    pub type_generics: Vec<crate::types::GenericParamDecl>,
    pub receiver_pattern: P::ImplReceiverAuthority,
    pub trait_name: Option<String>,
    pub trait_id: Option<DefId>,
    pub trait_generics: Vec<crate::types::GenericParamDecl>,
    pub trait_arg_types: Vec<Type>,
    pub associated_types: Vec<HirAssociatedTypeDef>,
    pub bounds: HirGenericBounds,
    pub methods: HashMap<String, HirFunctionFor<P>>,
}

pub type HirImpl = HirImplFor<UnresolvedHir>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirImplOwner {
    Named(String),
    BuiltinSlice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirAssociatedTypeDef {
    pub id: AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
    pub ty: Type,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirBlockFor<P: HirPhase> {
    pub stmts: Vec<HirStmtFor<P>>,
    pub ty: Type,
}

pub type HirBlock = HirBlockFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirStmtFor<P: HirPhase> {
    Let {
        name: String,
        local_id: HirLocalId,
        ty: Type,
        value: HirExprFor<P>,
        mutable: bool,
    },
    Expr(HirExprFor<P>),
    Return(Option<HirExprFor<P>>),
    Break(Option<HirExprFor<P>>),
    Continue,
}

pub type HirStmt = HirStmtFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirExprFor<P: HirPhase> {
    pub kind: HirExprKindFor<P>,
    pub ty: Type,
    pub span: Span,
}

pub type HirExpr = HirExprFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStructLiteralFieldFor<P: HirPhase> {
    pub name: String,
    pub value: HirExprFor<P>,
    pub field: Option<HirFieldLocation>,
}

pub type HirStructLiteralField = HirStructLiteralFieldFor<UnresolvedHir>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirVarTarget {
    Local(HirLocalId),
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirCallTarget {
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Local(HirLocalId),
    Intrinsic(String),
    StaticMethod(HirStaticMethodTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirVarRef {
    pub name: String,
    pub target: HirVarTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStructPatternField {
    pub name: String,
    pub field: Option<HirFieldLocation>,
    pub pattern: HirPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirExprKindFor<P: HirPhase> {
    /// Integer literal
    IntLiteral(i64),
    /// Float literal
    FloatLiteral(f64),
    /// Bool literal
    BoolLiteral(bool),
    /// String literal
    StringLiteral(String),
    /// Char literal
    CharLiteral(char),
    /// Array literal
    ArrayLiteral(Vec<HirExprFor<P>>),
    /// Fixed array initialized by evaluating one value and copying it `len` times
    ArrayRepeat(Box<HirExprFor<P>>, usize),
    /// Tuple literal
    TupleLiteral(Vec<HirExprFor<P>>),
    /// Unit value
    Unit,
    /// Variable reference
    Var(String),
    /// Resolved function or extern reference
    ResolvedVar(HirVarRef),
    /// Struct field access
    FieldAccess(Box<HirExprFor<P>>, String, Option<HirFieldLocation>),
    /// Tuple index access
    TupleIndex(Box<HirExprFor<P>>, u32),
    /// Binary operation
    BinOp(BinOp, Box<HirExprFor<P>>, Box<HirExprFor<P>>),
    /// Unary operation
    UnaryOp(UnaryOp, Box<HirExprFor<P>>),
    /// Function call
    Call(
        Box<HirExprFor<P>>,
        Vec<HirExprFor<P>>,
        Option<HirCallTarget>,
    ),
    /// Method call
    MethodCall(
        Box<HirExprFor<P>>,
        String,
        Vec<HirExprFor<P>>,
        Option<ReceiverMode>,
        P::MethodAuthority,
    ),
    /// Try short-circuit expression lowered through Try and FromResidual traits
    Try {
        expr: Box<HirExprFor<P>>,
        branch_method: P::MethodAuthority,
        branch_target: Option<HirCallTarget>,
        branch_self_receiver: Option<ReceiverMode>,
        from_residual_target: P::ResidualAuthority,
        output_ty: Type,
        residual_ty: Type,
        return_ty: Type,
        control_flow_enum: DefId,
        break_variant: HirVariantLocation,
        continue_variant: HirVariantLocation,
    },
    /// Struct construction
    StructLiteral(String, Option<DefId>, Vec<HirStructLiteralFieldFor<P>>),
    /// Enum variant construction
    EnumVariant(
        String,
        String,
        Vec<HirExprFor<P>>,
        Option<HirVariantLocation>,
    ),
    /// If expression
    If {
        condition: Box<HirExprFor<P>>,
        then_branch: HirBlockFor<P>,
        else_branch: Option<HirBlockFor<P>>,
    },
    /// Match expression
    Match {
        scrutinee: Box<HirExprFor<P>>,
        arms: Vec<HirMatchArmFor<P>>,
    },
    /// While loop
    While {
        condition: Box<HirExprFor<P>>,
        body: HirBlockFor<P>,
    },
    /// For loop (desugared into while over iterator)
    For {
        var: String,
        local_id: HirLocalId,
        iter: Box<HirExprFor<P>>,
        body: HirBlockFor<P>,
    },
    /// Infinite loop
    Loop(HirBlockFor<P>),
    /// Block expression
    Block(HirBlockFor<P>),
    /// Explicit unsafe block; this marker is erased when converting to accepted HIR.
    UnsafeBlock(HirBlockFor<P>),
    /// Lambda/closure
    Lambda {
        params: Vec<HirParam>,
        body: HirBlockFor<P>,
        captures: Vec<HirClosureCapture>,
    },
    /// Reference creation
    Ref(bool, Box<HirExprFor<P>>), // mutable, expr
    /// Dereference
    Deref(Box<HirExprFor<P>>),
    /// Type cast
    Cast(Box<HirExprFor<P>>, Type),
    /// Assignment (for mutable variables, field assignment)
    Assign(Box<HirExprFor<P>>, Box<HirExprFor<P>>),
    /// Compiler intrinsic (maps directly to LLVM operation)
    /// e.g., ~I64Add, ~F64Mul
    Intrinsic {
        name: String,
        args: Vec<HirExprFor<P>>,
    },
}

pub type HirExprKind = HirExprKindFor<UnresolvedHir>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn from_str(s: &str) -> Option<BinOp> {
        // All operators are now handled via stdlib infix declarations and method calls
        // Short-circuit operators (&&, ||) are kept as BinOp for special handling
        match s {
            "&&" => Some(BinOp::And),
            "||" => Some(BinOp::Or),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirMatchArmFor<P: HirPhase> {
    pub pattern: HirPattern,
    pub guard: Option<HirExprFor<P>>,
    pub body: HirBlockFor<P>,
}

pub type HirMatchArm = HirMatchArmFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirPattern {
    /// Wildcard _
    Wildcard,
    /// Bind a name
    Binding {
        name: String,
        local_id: HirLocalId,
        mutable: bool,
    },
    /// Literal pattern
    Literal(HirLiteralPattern),
    /// Tuple pattern
    Tuple(Vec<HirPattern>),
    /// Struct pattern
    Struct(String, Option<DefId>, Vec<Type>, Vec<HirStructPatternField>),
    /// Enum variant pattern
    Enum(String, String, Option<HirVariantLocation>, Vec<HirPattern>),
    /// Or pattern (not yet used)
    Or(Vec<HirPattern>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirLiteralPattern {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
}

/// Substitute only the generic identity or TypeVars reachable from the original
/// self type with a concrete type when injecting trait default methods into
/// concrete impl types.
pub fn substitute_typevars_in_function(
    func: &mut HirFunction,
    source_ty: &Type,
    concrete_ty: &Type,
) {
    let mut target_ids: HashSet<TypeVarId> = HashSet::new();
    let mut target_generic_ids: HashSet<crate::types::GenericParamId> = HashSet::new();
    collect_typevar_ids(source_ty, &mut target_ids);
    collect_generic_param_ids(source_ty, &mut target_generic_ids);

    for param in &mut func.params {
        substitute_typevar_in_type(&mut param.ty, &target_ids, &target_generic_ids, concrete_ty);
        substitute_generic_param_in_type(&mut param.ty, &target_generic_ids, concrete_ty);
    }
    substitute_typevar_in_type(
        &mut func.ret_type,
        &target_ids,
        &target_generic_ids,
        concrete_ty,
    );
    substitute_generic_param_in_type(&mut func.ret_type, &target_generic_ids, concrete_ty);
    substitute_typevars_in_block(
        &mut func.body,
        &target_ids,
        &target_generic_ids,
        concrete_ty,
    );
}

fn collect_typevar_ids(ty: &Type, target_ids: &mut HashSet<TypeVarId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::TypeVar(id) = nested {
            target_ids.insert(*id);
        }
    });
}

fn collect_generic_param_ids(ty: &Type, target_ids: &mut HashSet<crate::types::GenericParamId>) {
    crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
        if let Type::Generic(id) = nested {
            target_ids.insert(*id);
        }
    });
}

fn substitute_typevars_in_pattern(
    pattern: &mut HirPattern,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    match pattern {
        HirPattern::Tuple(patterns) | HirPattern::Or(patterns) => {
            for pattern in patterns {
                substitute_typevars_in_pattern(
                    pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Struct(_, _, type_args, field_patterns) => {
            for ty in type_args {
                substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            }
            for field in field_patterns {
                substitute_typevars_in_pattern(
                    &mut field.pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Enum(_, _, _, patterns) => {
            for pattern in patterns {
                substitute_typevars_in_pattern(
                    pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirPattern::Wildcard | HirPattern::Binding { .. } | HirPattern::Literal(_) => {}
    }
}

struct TargetTypeVarFolder<'a> {
    target_ids: &'a HashSet<TypeVarId>,
    concrete_ty: &'a Type,
}

impl crate::type_services::visit::TypeFolder for TargetTypeVarFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) if self.target_ids.contains(&id) => self.concrete_ty.clone(),
            other => crate::type_services::visit::fold_type_children(other, self),
        }
    }
}

fn substitute_typevar_in_type(
    ty: &mut Type,
    target_ids: &HashSet<TypeVarId>,
    _target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut TargetTypeVarFolder {
            target_ids,
            concrete_ty,
        },
    );
}

struct TargetGenericFolder<'a> {
    target_ids: &'a HashSet<crate::types::GenericParamId>,
    concrete_ty: &'a Type,
}

impl crate::type_services::visit::TypeFolder for TargetGenericFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::Generic(id) if self.target_ids.contains(&id) => self.concrete_ty.clone(),
            other => crate::type_services::visit::fold_type_children(other, self),
        }
    }
}

fn substitute_generic_param_in_type(
    ty: &mut Type,
    target_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    crate::type_services::visit::fold_type_in_place(
        ty,
        &mut TargetGenericFolder {
            target_ids,
            concrete_ty,
        },
    );
}

fn substitute_typevars_in_block(
    block: &mut HirBlock,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    substitute_typevar_in_type(&mut block.ty, target_ids, target_generic_ids, concrete_ty);
    substitute_generic_param_in_type(&mut block.ty, target_generic_ids, concrete_ty);
    for stmt in &mut block.stmts {
        substitute_typevars_in_stmt(stmt, target_ids, target_generic_ids, concrete_ty);
    }
}

fn substitute_typevars_in_stmt(
    stmt: &mut HirStmt,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    match stmt {
        HirStmtFor::Let { ty, value, .. } => {
            substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
            substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            substitute_typevars_in_expr_with_targets(
                value,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Expr(expr) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Return(Some(expr)) | HirStmtFor::Break(Some(expr)) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {}
    }
}

fn substitute_typevars_in_method_target_with_targets(
    target: &mut HirMethodCallTarget,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    target.for_each_type_mut(|ty| {
        substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
        substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
    });
}

fn substitute_typevars_in_call_target_with_targets(
    target: &mut HirCallTarget,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    if let HirCallTarget::StaticMethod(target) = target {
        substitute_typevar_in_type(
            &mut target.owner_ty,
            target_ids,
            target_generic_ids,
            concrete_ty,
        );
        substitute_generic_param_in_type(&mut target.owner_ty, target_generic_ids, concrete_ty);
        substitute_typevars_in_method_target_with_targets(
            &mut target.method,
            target_ids,
            target_generic_ids,
            concrete_ty,
        );
    }
}

fn substitute_typevars_in_expr_with_targets(
    expr: &mut HirExpr,
    target_ids: &HashSet<TypeVarId>,
    target_generic_ids: &HashSet<crate::types::GenericParamId>,
    concrete_ty: &Type,
) {
    substitute_typevar_in_type(&mut expr.ty, target_ids, target_generic_ids, concrete_ty);
    substitute_generic_param_in_type(&mut expr.ty, target_generic_ids, concrete_ty);
    match &mut expr.kind {
        HirExprKindFor::IntLiteral(_)
        | HirExprKindFor::FloatLiteral(_)
        | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_)
        | HirExprKindFor::CharLiteral(_)
        | HirExprKindFor::Unit
        | HirExprKindFor::Var(_)
        | HirExprKindFor::ResolvedVar(_) => {}
        HirExprKindFor::ArrayLiteral(elems) | HirExprKindFor::TupleLiteral(elems) => {
            for e in elems.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    e,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::ArrayRepeat(value, _) => {
            substitute_typevars_in_expr_with_targets(
                value,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::FieldAccess(base, _, _)
        | HirExprKindFor::Deref(base)
        | HirExprKindFor::Ref(_, base)
        | HirExprKindFor::UnaryOp(_, base) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::TupleIndex(base, _) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::BinOp(_, base, idx) | HirExprKindFor::Assign(base, idx) => {
            substitute_typevars_in_expr_with_targets(
                base,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_expr_with_targets(
                idx,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
        }
        HirExprKindFor::Call(func, args, target) => {
            substitute_typevars_in_expr_with_targets(
                func,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = target {
                substitute_typevars_in_call_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::MethodCall(recv, _, args, _, target) => {
            substitute_typevars_in_expr_with_targets(
                recv,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = target {
                substitute_typevars_in_method_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::Try {
            expr,
            branch_method,
            from_residual_target,
            output_ty,
            residual_ty,
            return_ty,
            ..
        } => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            if let Some(target) = branch_method {
                substitute_typevars_in_method_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            if let Some(target) = from_residual_target {
                substitute_typevars_in_call_target_with_targets(
                    target,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
            for ty in [output_ty, residual_ty, return_ty] {
                substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
            }
        }
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    &mut field.value,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::EnumVariant(_, _, args, _) => {
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::If {
            condition,
            then_branch,
            else_branch,
        } => {
            substitute_typevars_in_expr_with_targets(
                condition,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(then_branch, target_ids, target_generic_ids, concrete_ty);
            if let Some(eb) = else_branch {
                substitute_typevars_in_block(eb, target_ids, target_generic_ids, concrete_ty);
            }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            substitute_typevars_in_expr_with_targets(
                scrutinee,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            for arm in arms.iter_mut() {
                substitute_typevars_in_pattern(
                    &mut arm.pattern,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
                if let Some(g) = &mut arm.guard {
                    substitute_typevars_in_expr_with_targets(
                        g,
                        target_ids,
                        target_generic_ids,
                        concrete_ty,
                    );
                }
                substitute_typevars_in_block(
                    &mut arm.body,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
        HirExprKindFor::While { condition, body } => {
            substitute_typevars_in_expr_with_targets(
                condition,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::For { iter, body, .. } => {
            substitute_typevars_in_expr_with_targets(
                iter,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Loop(body)
        | HirExprKindFor::Block(body)
        | HirExprKindFor::UnsafeBlock(body) => {
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Lambda {
            params,
            body,
            captures,
        } => {
            for p in params.iter_mut() {
                substitute_typevar_in_type(&mut p.ty, target_ids, target_generic_ids, concrete_ty);
                substitute_generic_param_in_type(&mut p.ty, target_generic_ids, concrete_ty);
            }
            for capture in captures.iter_mut() {
                substitute_typevar_in_type(
                    &mut capture.ty,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
                substitute_generic_param_in_type(&mut capture.ty, target_generic_ids, concrete_ty);
            }
            substitute_typevars_in_block(body, target_ids, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Cast(expr, ty) => {
            substitute_typevars_in_expr_with_targets(
                expr,
                target_ids,
                target_generic_ids,
                concrete_ty,
            );
            substitute_typevar_in_type(ty, target_ids, target_generic_ids, concrete_ty);
            substitute_generic_param_in_type(ty, target_generic_ids, concrete_ty);
        }
        HirExprKindFor::Intrinsic { args, .. } => {
            for a in args.iter_mut() {
                substitute_typevars_in_expr_with_targets(
                    a,
                    target_ids,
                    target_generic_ids,
                    concrete_ty,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::hir::{
        AcceptedHirProgram, HirAssociatedTypeDecl, HirAssociatedTypeDef, HirAssociatedTypeLocation,
        HirBlock, HirCallTarget, HirEnum, HirExpr, HirExprKind, HirExtern, HirField,
        HirFieldLocation, HirFunction, HirImpl, HirImplOwner, HirImplReceiverPattern,
        HirMethodCallTarget, HirMethodLocation, HirNameTables, HirParam, HirPattern, HirProgram,
        HirSelectedMethodTarget, HirSelectedTraitMember, HirStmt, HirStruct, HirStructPatternField,
        HirTrait, HirTraitDispatchKind, HirTypeBinding, HirVarRef, HirVarTarget, HirVariant,
        HirVariantFields, HirVariantLocation,
    };
    use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
    use crate::types::{GenericParamDecl, GenericParamId, Type};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn field_id(index: u32) -> FieldId {
        FieldId(index)
    }

    fn variant_id(index: u32) -> VariantId {
        VariantId(index)
    }

    fn assoc_type_id(index: u32) -> AssocTypeId {
        AssocTypeId(index)
    }

    #[test]
    fn hir_var_target_can_identify_local_bindings() {
        let local = crate::ids::HirLocalId(7);
        let target = HirVarTarget::Local(local);

        assert_eq!(target, HirVarTarget::Local(crate::ids::HirLocalId(7)));
    }

    #[test]
    fn hir_call_target_can_identify_direct_function_calls() {
        let function_id = def_id(44);
        let target = HirCallTarget::Function(function_id);

        assert_eq!(target, HirCallTarget::Function(function_id));
    }

    #[test]
    fn hir_var_target_can_reference_monomorphized_instance() {
        let target = HirVarTarget::Instance(crate::ids::InstanceId(9));

        assert_eq!(target, HirVarTarget::Instance(crate::ids::InstanceId(9)));
    }

    #[test]
    fn method_selection_target_has_explicit_validated_variants() {
        let impl_id = DefId::new(CrateId(0), LocalDefId(90));
        let method_id = DefId::new(CrateId(0), LocalDefId(91));
        let trait_id = DefId::new(CrateId(0), LocalDefId(92));
        let member_id = DefId::new(CrateId(0), LocalDefId(93));

        let inherent = HirMethodCallTarget {
            target: HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait: None,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        };
        assert_eq!(inherent.impl_id(), Some(impl_id));
        assert_eq!(inherent.method_id(), Some(method_id));

        let trait_method = HirMethodCallTarget {
            target: HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args: vec![Type::I64],
                dispatch: HirTraitDispatchKind::TraitBound,
            },
            owner_substitution: vec![HirTypeBinding {
                param: GenericParamId {
                    owner: impl_id,
                    index: 0,
                },
                ty: Type::I64,
            }],
            method_substitution: Vec::new(),
        };
        assert_eq!(trait_method.trait_id(), Some(trait_id));
        assert_eq!(trait_method.method_id(), Some(member_id));
        assert_eq!(trait_method.trait_args(), &[Type::I64]);
    }

    #[test]
    fn hir_reference_shapes_carry_resolved_targets() {
        let owner = def_id(10);
        let field = field_id(0);
        let variant = variant_id(0);

        let var = HirExprKind::ResolvedVar(HirVarRef {
            name: "make_box".to_string(),
            target: HirVarTarget::Function(def_id(11)),
        });
        let struct_literal = HirExprKind::StructLiteral("Box".to_string(), Some(owner), Vec::new());
        let struct_pattern = HirPattern::Struct(
            "Box".to_string(),
            Some(owner),
            Vec::new(),
            vec![HirStructPatternField {
                name: "value".to_string(),
                field: Some(HirFieldLocation {
                    owner,
                    field_id: field,
                    name: "value".to_string(),
                }),
                pattern: HirPattern::Wildcard,
            }],
        );
        let enum_pattern = HirPattern::Enum(
            "Option".to_string(),
            "Some".to_string(),
            Some(HirVariantLocation {
                owner,
                variant_id: variant,
                name: "Some".to_string(),
            }),
            Vec::new(),
        );

        match var {
            HirExprKind::ResolvedVar(reference) => assert_eq!(reference.name, "make_box"),
            _ => panic!("expected resolved var"),
        }
        match struct_literal {
            HirExprKind::StructLiteral(_, Some(id), _) => assert_eq!(id, owner),
            _ => panic!("expected resolved struct literal"),
        }
        match struct_pattern {
            HirPattern::Struct(_, Some(id), _, fields) => {
                assert_eq!(id, owner);
                assert_eq!(fields[0].field.as_ref().unwrap().field_id, field);
            }
            _ => panic!("expected resolved struct pattern"),
        }
        match enum_pattern {
            HirPattern::Enum(_, _, Some(location), _) => assert_eq!(location.variant_id, variant),
            _ => panic!("expected resolved enum pattern"),
        }
    }

    fn empty_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::Unit,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn test_function(id: DefId, name: &str) -> HirFunction {
        empty_function(id, name)
    }

    fn empty_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: field_id(0),
                name: "value".to_string(),
                ty: Type::I32,
                public: true,
            }],
        }
    }

    fn empty_enum(id: DefId, name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        }
    }

    fn trait_with_default_method(id: DefId, name: &str, method_id: DefId) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([(
                "default_method".to_string(),
                empty_function(method_id, "default_method"),
            )]),
            signatures: HashMap::new(),
        }
    }

    fn impl_with_method(id: DefId, method_id: DefId) -> HirImpl {
        HirImpl {
            id,
            owner: HirImplOwner::Named("Thing".to_string()),
            type_name: "Thing".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id(0),
                name: "Item".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I32,
            }],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([(
                "impl_method".to_string(),
                empty_function(method_id, "impl_method"),
            )]),
        }
    }

    fn program(
        functions: HashMap<DefId, HirFunction>,
        structs: HashMap<DefId, HirStruct>,
        enums: HashMap<DefId, HirEnum>,
        traits: HashMap<DefId, HirTrait>,
        impls: HashMap<DefId, HirImpl>,
        externs: HashMap<DefId, HirExtern>,
        names: HirNameTables,
        canonical_names: HashMap<DefId, String>,
    ) -> HirProgram {
        HirProgram::from_id_parts_with_names_and_canonical_names(
            functions,
            structs,
            enums,
            traits,
            impls,
            externs,
            names,
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        )
    }

    #[test]
    fn accepted_hir_rejects_raw_method_callee_without_call_target() {
        let function_id = def_id(70);
        let impl_id = def_id(71);
        let method_id = def_id(72);
        let mut function = empty_function(function_id, "main");
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "method".to_string(),
                        target: HirVarTarget::Function(method_id),
                    }),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                None,
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let mut program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );
        program.indexes.methods_by_id.insert(
            method_id,
            HirMethodLocation::ImplMethod {
                impl_id,
                method_id,
                method_name: "method".to_string(),
            },
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| error.contains("raw method DefId")));
    }

    #[test]
    fn constructor_trait_selection_accepted_hir_rejects_targetless_constructor_call() {
        let function_id = def_id(73);
        let impl_id = def_id(74);
        let method_id = def_id(75);
        let constructor_id = def_id(76);
        let mut function = empty_function(function_id, "main");
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "Option::pure".to_string(),
                        target: HirVarTarget::Function(method_id),
                    }),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Function(method_id)),
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let mut imp = impl_with_method(impl_id, method_id);
        imp.receiver_pattern = HirImplReceiverPattern::Constructor(Type::Constructor {
            id: constructor_id,
            flavor: crate::types::NominalTypeKind::Enum,
        });
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, imp)]),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| { error.contains("has no selected static-method authority") }));
    }

    #[test]
    fn accepted_hir_rejects_missing_try_branch_receiver_authority() {
        let function_id = def_id(80);
        let trait_id = def_id(81);
        let member_id = def_id(82);
        let target = HirMethodCallTarget {
            target: HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args: Vec::new(),
                dispatch: HirTraitDispatchKind::TraitBound,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        };
        let mut function = empty_function(function_id, "main");
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Try {
                expr: Box::new(HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }),
                branch_method: Some(target.clone()),
                branch_target: None,
                branch_self_receiver: None,
                from_residual_target: Some(HirCallTarget::StaticMethod(
                    crate::hir::HirStaticMethodTarget {
                        owner_ty: Type::Unit,
                        method: target,
                    },
                )),
                output_ty: Type::Unit,
                residual_ty: Type::Unit,
                return_ty: Type::Unit,
                control_flow_enum: def_id(83),
                break_variant: HirVariantLocation {
                    owner: def_id(83),
                    variant_id: variant_id(0),
                    name: "Break".to_string(),
                },
                continue_variant: HirVariantLocation {
                    owner: def_id(83),
                    variant_id: variant_id(1),
                    name: "Continue".to_string(),
                },
            },
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Try".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "branch".to_string(),
                crate::hir::HirFunctionSig {
                    id: member_id,
                    name: "branch".to_string(),
                    generic_params: Vec::new(),
                    params: vec![Type::Unit],
                    ret: Type::Unit,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Move),
                    is_unsafe: false,
                },
            )]),
        };
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                traits_by_name: HashMap::from([("Try".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| error.contains("Try branch receiver mode mismatch")));
    }

    #[test]
    fn accepted_hir_program_requires_validated_construction() {
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        let accepted = crate::hir::AcceptedHirProgram::try_from(program)
            .expect("empty program has valid method authority");

        assert!(accepted.program().functions.is_empty());
    }

    #[test]
    fn hir_rejects_static_trait_member_mapped_to_receiver_impl_method() {
        let trait_id = def_id(84);
        let member_id = def_id(85);
        let impl_id = def_id(86);
        let method_id = def_id(87);
        let trait_def = trait_with_default_method(trait_id, "Protocol", member_id);
        let mut imp = impl_with_method(impl_id, method_id);
        imp.trait_id = Some(trait_id);
        imp.trait_name = Some("Protocol".to_string());
        let method = imp.methods.get_mut("impl_method").expect("fixture method");
        method.is_method = true;
        method.self_receiver = Some(crate::types::ReceiverMode::Move);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::from([(impl_id, imp)]),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );
        program
            .indexes
            .effective_trait_methods
            .insert((impl_id, member_id), method_id);

        let errors = program.validate_method_authorities();

        assert!(errors.iter().any(|error| {
            error.contains("effective trait member")
                && error.contains(&format!("{member_id:?}"))
                && error.contains(&format!("{method_id:?}"))
                && error.contains("receiver/static mismatch")
        }));
    }

    #[test]
    fn accepted_hir_rejects_method_call_without_trait_or_impl_identity() {
        let trait_id = def_id(85);
        let method_id = def_id(86);
        let mut method = empty_function(method_id, "default");
        method.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::MethodCall(
                Box::new(HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }),
                "missing".to_string(),
                Vec::new(),
                None,
                None,
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Defaults".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("default".to_string(), method)]),
            signatures: HashMap::new(),
        };
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Defaults".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| error.contains("method call has no selected authority")));
    }

    #[test]
    fn hir_function_with_error_type_is_not_codegen_concrete() {
        let mut function = empty_function(def_id(87), "broken");
        function.ret_type = Type::Error;
        function.body.ty = Type::Error;

        assert!(!crate::hir::hir_function_is_codegen_concrete(&function));
        assert!(!crate::hir::function_is_object_provided_candidate(
            &function
        ));
        let program = program(
            HashMap::from([(function.id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("broken".to_string(), def_id(87))]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );
        assert!(crate::hir::AcceptedHirProgram::try_from(program).is_err());
    }

    #[test]
    fn accepted_hir_rejects_unresolved_type_locations() {
        let function_id = def_id(89);
        let unresolved = Type::TypeVar(crate::ids::TypeVarId(0));
        let mut function = empty_function(function_id, "broken");
        function.ret_type = unresolved.clone();
        function.body.ty = unresolved.clone();
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Unit,
            ty: unresolved,
            span: crate::lexer::Span::test(),
        }));
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("broken".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = crate::hir::AcceptedHirProgram::try_from(program)
            .expect_err("accepted HIR must reject every unresolved location");
        let unresolved_errors = errors
            .iter()
            .filter(|error| error.contains("accepted HIR contains unresolved type at"))
            .collect::<Vec<_>>();

        assert_eq!(unresolved_errors.len(), 3);
        assert!(unresolved_errors
            .iter()
            .any(|error| error.contains("FunctionReturn")));
        assert!(unresolved_errors
            .iter()
            .any(|error| error.contains("Block")));
        assert!(unresolved_errors.iter().any(|error| error.contains("Expr")));
    }

    #[test]
    fn accepted_hir_rejects_unsaturated_constructor_in_return_type() {
        let function_id = def_id(190);
        let struct_id = def_id(191);
        let mut function = empty_function(function_id, "broken");
        function.ret_type = Type::Constructor {
            id: struct_id,
            flavor: crate::types::NominalTypeKind::Struct,
        };
        let structure = HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: vec![crate::types::GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        };
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::from([(struct_id, structure)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

        assert!(errors.iter().any(|error| {
            error.contains("FunctionReturn")
                && error.contains("has kind Type -> Type, expected Type")
        }));
    }

    #[test]
    fn kind_check_rejects_unsaturated_constructors_in_runtime_locations() {
        let function_id = def_id(194);
        let box_id = def_id(195);
        let holder_id = def_id(196);
        let constructor = Type::Constructor {
            id: box_id,
            flavor: crate::types::NominalTypeKind::Struct,
        };
        let box_struct = HirStruct {
            id: box_id,
            name: "Box".to_string(),
            generic_params: vec![crate::types::GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: box_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        };
        let holder = HirStruct {
            id: holder_id,
            name: "Holder".to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "value".to_string(),
                ty: constructor.clone(),
                public: false,
            }],
        };
        let mut function = empty_function(function_id, "broken");
        function.params.push(HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: constructor.clone(),
            mutable: false,
            is_ref: false,
        });
        function.ret_type = constructor.clone();
        function.body.stmts.push(HirStmt::Let {
            name: "local".to_string(),
            local_id: crate::ids::HirLocalId(1),
            ty: constructor,
            value: HirExpr {
                kind: HirExprKind::Unit,
                ty: Type::Unit,
                span: crate::lexer::Span::test(),
            },
            mutable: false,
        });
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::from([(box_id, box_struct), (holder_id, holder)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

        for location in ["StructField", "FunctionParam", "FunctionReturn", "LetStmt"] {
            assert!(
                errors.iter().any(|error| error.contains(location)
                    && error.contains("has kind Type -> Type, expected Type")),
                "missing {location} error in {errors:?}"
            );
        }
    }

    #[test]
    fn accepted_hir_rejects_noncanonical_saturated_application() {
        let function_id = def_id(192);
        let struct_id = def_id(193);
        let mut function = empty_function(function_id, "broken");
        function.ret_type = Type::Apply {
            constructor: Box::new(Type::Constructor {
                id: struct_id,
                flavor: crate::types::NominalTypeKind::Struct,
            }),
            args: vec![Type::I64],
        };
        let structure = HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: vec![crate::types::GenericParamDecl::type_param(
                crate::types::GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        };
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::from([(struct_id, structure)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        let errors = crate::hir::AcceptedHirProgram::try_from(program).unwrap_err();

        assert!(errors.iter().any(|error| {
            error.contains("FunctionReturn") && error.contains("noncanonical type")
        }));
    }

    #[test]
    fn accepted_hir_rejects_unresolved_ordinary_field_access() {
        let function_id = def_id(88);
        let mut function = empty_function(function_id, "main");
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::FieldAccess(
                Box::new(HirExpr {
                    kind: HirExprKind::Unit,
                    ty: Type::Unit,
                    span: crate::lexer::Span::test(),
                }),
                "missing".to_string(),
                None,
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| error.contains("unresolved field access")));
    }

    #[test]
    fn accepted_hir_rejects_pre_mono_instance_targets() {
        let function_id = def_id(89);
        let instance_id = crate::ids::InstanceId(3);
        let mut function = empty_function(function_id, "main");
        function.body.stmts.push(HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "specialized".to_string(),
                        target: HirVarTarget::Instance(instance_id),
                    }),
                    ty: Type::function(Vec::new(), Type::Unit),
                    span: crate::lexer::Span::test(),
                }),
                Vec::new(),
                Some(HirCallTarget::Instance(instance_id)),
            ),
            ty: Type::Unit,
            span: crate::lexer::Span::test(),
        }));
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = program.validate_method_authorities();

        assert!(errors
            .iter()
            .any(|error| error.contains("pre-monomorphization instance")));
    }

    #[test]
    fn accepted_hir_rejects_incomplete_impl_receiver_pattern_bindings() {
        let owner_id = def_id(90);
        let impl_id = def_id(91);
        let method_id = def_id(92);
        let mut imp = impl_with_method(impl_id, method_id);
        imp.type_generics = GenericParamDecl::type_params(impl_id, ["T"]);
        imp.receiver_pattern = crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: owner_id,
            args: Vec::new(),
        });
        let program = program(
            HashMap::new(),
            HashMap::from([(owner_id, empty_struct(owner_id, "Thing"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, imp)]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Thing".to_string(), owner_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );
        let errors = program.validate_method_authorities();

        assert!(errors.iter().any(|error| {
            error.contains("receiver/trait argument generic bindings")
                && error.contains(&format!("{impl_id:?}"))
        }));
    }

    #[test]
    fn accepted_hir_allows_impl_generic_bound_by_trait_argument() {
        let impl_id = def_id(193);
        let method_id = def_id(194);
        let receiver_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let trait_arg_param = GenericParamId {
            owner: impl_id,
            index: 1,
        };
        let mut imp = impl_with_method(impl_id, method_id);
        imp.type_generics = GenericParamDecl::type_params(impl_id, ["T", "U"]);
        imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Generic(receiver_param));
        imp.trait_arg_types = vec![Type::Generic(trait_arg_param)];

        assert_eq!(
            super::validate_impl_receiver_pattern_bindings(&imp, &imp.receiver_pattern),
            Ok(())
        );
    }

    #[test]
    fn accepted_hir_rejects_ambiguous_concrete_projection_impls() {
        let owner_id = def_id(93);
        let trait_id = def_id(94);
        let generic_impl_id = def_id(95);
        let concrete_impl_id = def_id(96);
        let function_id = def_id(97);
        let assoc_type_id = AssocTypeId(0);
        let owner_ty = Type::Struct {
            id: owner_id,
            args: Vec::new(),
        };
        let projection_ty = Type::Projection {
            ty: Box::new(owner_ty.clone()),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: Vec::new(),
        };
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Project".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        };
        let projection_impl = |id, receiver_pattern| HirImpl {
            id,
            owner: HirImplOwner::Named("Owner".to_string()),
            type_name: "Owner".to_string(),
            type_generics: (id == generic_impl_id)
                .then(|| GenericParamDecl::type_params(id, ["T"]))
                .unwrap_or_default(),
            receiver_pattern,
            trait_name: Some("Project".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: vec![HirAssociatedTypeDef {
                id: assoc_type_id,
                name: "Output".to_string(),
                kind: crate::type_services::kind::Kind::Type,
                ty: Type::I64,
            }],
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        };
        let function = HirFunction {
            id: function_id,
            name: "project".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: projection_ty.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: projection_ty,
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let program = program(
            HashMap::from([(function_id, function)]),
            HashMap::from([(owner_id, empty_struct(owner_id, "Owner"))]),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::from([
                (
                    generic_impl_id,
                    projection_impl(
                        generic_impl_id,
                        HirImplReceiverPattern::Exact(Type::Generic(GenericParamId {
                            owner: generic_impl_id,
                            index: 0,
                        })),
                    ),
                ),
                (
                    concrete_impl_id,
                    projection_impl(concrete_impl_id, HirImplReceiverPattern::Exact(owner_ty)),
                ),
            ]),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("project".to_string(), function_id)]),
                structs_by_name: HashMap::from([("Owner".to_string(), owner_id)]),
                traits_by_name: HashMap::from([("Project".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let errors = AcceptedHirProgram::try_from(program).unwrap_err();

        assert!(errors.iter().any(|error| {
            error.contains("multiple matching typed projection implementations")
                && error.contains(&format!("{generic_impl_id:?}"))
                && error.contains(&format!("{concrete_impl_id:?}"))
        }));
    }

    #[test]
    fn hir_program_owns_functions_by_def_id_and_names_are_views() {
        let canonical_id = def_id(10);
        let canonical_names = HashMap::from([(canonical_id, "main".to_string())]);

        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(canonical_id, test_function(canonical_id, "main"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("main".to_string(), canonical_id),
                    ("alias_main".to_string(), canonical_id),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        );

        assert_eq!(program.functions.len(), 1);
        assert!(program.functions.contains_key(&canonical_id));
        assert_eq!(program.names.functions_by_name["main"], canonical_id);
        assert_eq!(program.names.functions_by_name["alias_main"], canonical_id);
        assert_eq!(
            program.names.functions_by_name.get("alias_main"),
            Some(&canonical_id)
        );
    }

    #[test]
    fn hir_program_can_be_built_from_id_keyed_declarations() {
        let function_id = def_id(11);
        let function = test_function(function_id, "main");
        let function_names = HashMap::from([
            ("main".to_string(), function_id),
            ("alias_main".to_string(), function_id),
        ]);

        let program = HirProgram::from_id_parts_with_names(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: function_names,
                ..crate::hir::HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
        );

        assert_eq!(program.functions.len(), 1);
        assert!(program.functions.contains_key(&function_id));
        assert_eq!(program.names.functions_by_name["alias_main"], function_id);
    }

    #[test]
    fn hir_program_lists_display_aliases_by_def_id() {
        let function_id = def_id(40);
        let struct_id = def_id(41);
        let enum_id = def_id(42);
        let canonical_names = HashMap::from([
            (function_id, "main".to_string()),
            (struct_id, "module::Foo".to_string()),
            (enum_id, "module::Choice".to_string()),
        ]);

        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, test_function(function_id, "main"))]),
            HashMap::from([(struct_id, empty_struct(struct_id, "Foo"))]),
            HashMap::from([(enum_id, empty_enum(enum_id, "Choice"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("main".to_string(), function_id),
                    ("alias_main".to_string(), function_id),
                ]),
                structs_by_name: HashMap::from([
                    ("module::Foo".to_string(), struct_id),
                    ("AliasFoo".to_string(), struct_id),
                ]),
                enums_by_name: HashMap::from([
                    ("module::Choice".to_string(), enum_id),
                    ("AliasChoice".to_string(), enum_id),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &canonical_names,
        );

        assert_eq!(
            program.function_display_aliases(function_id),
            vec!["main", "alias_main"]
        );
        assert_eq!(
            program.struct_display_aliases(struct_id),
            vec!["module::Foo", "AliasFoo"]
        );
        assert_eq!(
            program.enum_display_aliases(enum_id),
            vec!["module::Choice", "AliasChoice"]
        );
        assert_eq!(
            program.nominal_display_aliases(struct_id),
            vec!["module::Foo", "AliasFoo"]
        );
        assert_eq!(
            program.nominal_display_aliases(enum_id),
            vec!["module::Choice", "AliasChoice"]
        );
    }

    #[test]
    fn hir_program_resolves_nominal_display_aliases_only_to_existing_owners() {
        let struct_id = def_id(43);
        let enum_id = def_id(44);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::from([(struct_id, empty_struct(struct_id, "Foo"))]),
            HashMap::from([(enum_id, empty_enum(enum_id, "Choice"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([("Foo".to_string(), struct_id)]),
                enums_by_name: HashMap::from([("Choice".to_string(), enum_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );
        program
            .names
            .structs_by_name
            .insert("AliasFoo".to_string(), struct_id);
        program
            .names
            .enums_by_name
            .insert("AliasChoice".to_string(), enum_id);
        program
            .names
            .structs_by_name
            .insert("Stale".to_string(), stale_id);

        assert_eq!(
            program.nominal_owner_id_for_display_alias("AliasFoo"),
            Some(struct_id)
        );
        assert_eq!(
            program.nominal_owner_id_for_display_alias("AliasChoice"),
            Some(enum_id)
        );
        assert_eq!(program.nominal_owner_id_for_display_alias("Stale"), None);
        assert!(program.nominal_display_aliases(stale_id).is_empty());
    }

    #[test]
    fn hir_program_display_aliases_ignore_stale_name_table_ids() {
        let stale_id = def_id(100);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );
        program
            .names
            .functions_by_name
            .insert("stale_function".to_string(), stale_id);
        program
            .names
            .structs_by_name
            .insert("stale_struct".to_string(), stale_id);
        program
            .names
            .enums_by_name
            .insert("stale_enum".to_string(), stale_id);

        assert!(program.function_display_aliases(stale_id).is_empty());
        assert!(program.struct_display_aliases(stale_id).is_empty());
        assert!(program.enum_display_aliases(stale_id).is_empty());
    }

    #[test]
    fn hir_name_tables_do_not_own_semantic_bodies() {
        let id = def_id(30);
        let program = program(
            HashMap::from([(id, test_function(id, "answer"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("answer".to_string(), id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let by_name_id = program.names.functions_by_name["answer"];
        let by_name = &program.functions[&by_name_id];
        let (_, by_id) = program.function_by_id(id).unwrap();

        assert_eq!(by_name_id, id);
        assert!(std::ptr::eq(by_name, by_id));
    }

    #[test]
    fn hir_program_keeps_one_function_payload_for_multiple_aliases() {
        let id = def_id(11);
        let program = program(
            HashMap::from([(id, test_function(id, "canonical"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("canonical".to_string(), id),
                    ("alias".to_string(), id),
                ]),
                ..HirNameTables::default()
            },
            HashMap::from([(id, "canonical".to_string())]),
        );

        assert_eq!(program.functions.len(), 1);
        assert_eq!(program.names.functions_by_name["alias"], id);
    }

    #[test]
    fn hir_program_rebuild_indexes_uses_owned_functions_when_name_table_is_stale() {
        let function_id = def_id(12);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::from([(function_id, test_function(function_id, "main"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.names.functions_by_name.clear();
        program
            .names
            .functions_by_name
            .insert("stale".to_string(), stale_id);

        program.rebuild_indexes();

        assert_eq!(program.indexes.functions_by_id[&function_id], "main");
        assert!(program.function_by_id(function_id).is_some());
        assert_eq!(program.functions_by_id().count(), 1);
        assert!(!program.indexes.functions_by_id.contains_key(&stale_id));
    }

    #[test]
    fn hir_program_rebuild_indexes_uses_owned_traits_for_default_methods_without_name_table() {
        let trait_id = def_id(14);
        let method_id = def_id(15);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                trait_id,
                trait_with_default_method(trait_id, "Show", method_id),
            )]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.names.traits_by_name.clear();
        program.rebuild_indexes();

        assert_eq!(
            program.indexes.methods_by_id[&method_id],
            HirMethodLocation::TraitDefault {
                trait_id,
                method_id,
                method_name: "default_method".to_string(),
            }
        );
    }

    #[test]
    fn hir_program_rebuild_indexes_ignores_stale_trait_names_for_default_methods() {
        let trait_id = def_id(16);
        let method_id = def_id(17);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                trait_id,
                trait_with_default_method(trait_id, "Show", method_id),
            )]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Show".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.names.traits_by_name.clear();
        program
            .names
            .traits_by_name
            .insert("stale".to_string(), stale_id);
        program.rebuild_indexes();

        assert!(program.indexes.methods_by_id.contains_key(&method_id));
        assert!(!program.indexes.traits_by_id.contains_key(&stale_id));
    }

    #[test]
    fn hir_program_rebuild_indexes_uses_owned_impls_when_order_is_stale() {
        let impl_id = def_id(18);
        let method_id = def_id(19);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, impl_with_method(impl_id, method_id))]),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        program.order.impls = vec![stale_id];
        program.rebuild_indexes();

        assert_eq!(program.indexes.impls_by_id[&impl_id], 0);
        assert!(program.impl_by_id(impl_id).is_some());
        assert!(!program.indexes.impls_by_id.contains_key(&stale_id));
        assert_eq!(
            program.indexes.methods_by_id[&method_id],
            HirMethodLocation::ImplMethod {
                impl_id,
                method_id,
                method_name: "impl_method".to_string(),
            }
        );
    }

    #[test]
    fn hir_method_locations_use_owner_ids_not_names_or_indexes() {
        let trait_id = def_id(20);
        let trait_method_id = def_id(21);
        let impl_id = def_id(22);
        let impl_method_id = def_id(23);

        let traits = HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Display".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([(
                    "fmt".to_string(),
                    test_function(trait_method_id, "fmt"),
                )]),
                signatures: HashMap::new(),
            },
        )]);
        let impls = HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Widget".to_string()),
                type_name: "Widget".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Display".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
            },
        )]);

        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            traits,
            impls,
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert_eq!(
            program.indexes.methods_by_id.get(&trait_method_id),
            Some(&HirMethodLocation::TraitDefault {
                trait_id,
                method_id: trait_method_id,
                method_name: "fmt".to_string(),
            })
        );
        assert_eq!(
            program.indexes.methods_by_id.get(&impl_method_id),
            Some(&HirMethodLocation::ImplMethod {
                impl_id,
                method_id: impl_method_id,
                method_name: "fmt".to_string(),
            })
        );
        assert_eq!(
            program.effective_trait_method(impl_id, trait_method_id),
            None
        );
    }

    #[test]
    fn hir_index_build_does_not_reconstruct_effective_trait_methods_by_name() {
        let trait_id = def_id(120);
        let trait_method_id = def_id(121);
        let impl_id = def_id(122);
        let impl_method_id = def_id(123);
        let traits = HashMap::from([(
            trait_id,
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Display".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([(
                    "fmt".to_string(),
                    test_function(trait_method_id, "fmt"),
                )]),
                signatures: HashMap::new(),
            },
        )]);
        let impls = HashMap::from([(
            impl_id,
            HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Widget".to_string()),
                type_name: "Widget".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: Vec::new().into(),
                trait_name: Some("Display".to_string()),
                trait_id: Some(trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("fmt".to_string(), test_function(impl_method_id, "fmt"))]),
            },
        )]);

        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            traits,
            impls,
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Display".to_string(), trait_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );
        program.rebuild_indexes();

        assert!(program.indexes.effective_trait_methods.is_empty());
    }

    #[test]
    fn hir_program_impls_in_order_uses_owned_impls_when_order_is_stale() {
        let preferred_id = def_id(18);
        let missing_id = def_id(19);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                (missing_id, impl_with_method(missing_id, def_id(20))),
                (preferred_id, impl_with_method(preferred_id, def_id(21))),
            ]),
            HashMap::new(),
            HirNameTables::default(),
            HashMap::new(),
        );

        program.order.impls = vec![preferred_id, stale_id];
        let ids = program
            .impls_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![preferred_id, missing_id]);
    }

    #[test]
    fn hir_program_does_not_reconstruct_impl_owner_id_from_display_alias() {
        let struct_id = def_id(80);
        let impl_id = def_id(81);
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::from([(struct_id, empty_struct(struct_id, "Box"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                impl_id,
                HirImpl {
                    id: impl_id,
                    owner: HirImplOwner::Named("AliasBox".to_string()),
                    type_name: "AliasBox".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: HirImplReceiverPattern::Exact(Type::Unit),
                    trait_name: None,
                    trait_id: None,
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: Vec::new(),
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([
                    ("module::Box".to_string(), struct_id),
                    ("AliasBox".to_string(), struct_id),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(struct_id, "module::Box".to_string())]),
        );

        assert_eq!(program.impl_owner_id(impl_id), None);
        assert_eq!(
            program.impl_receiver_pattern(impl_id),
            Some(&HirImplReceiverPattern::Exact(Type::Unit))
        );
    }

    #[test]
    fn hir_program_rebuild_indexes_uses_owned_externs_when_order_is_stale() {
        let extern_id = def_id(20);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "puts".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.order.externs = vec![stale_id];
        program.rebuild_indexes();

        assert_eq!(program.indexes.externs_by_id[&extern_id], 0);
        assert!(program.extern_by_id(extern_id).is_some());
        assert!(!program.indexes.externs_by_id.contains_key(&stale_id));
    }

    #[test]
    fn hir_program_externs_in_order_uses_owned_externs_when_order_is_stale() {
        let preferred_id = def_id(20);
        let missing_id = def_id(21);
        let stale_id = def_id(99);
        let mut program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                (
                    missing_id,
                    HirExtern {
                        id: missing_id,
                        name: "missing".to_string(),
                        params: vec![Type::Str],
                        ret: Type::I32,
                        variadic: false,
                        is_unsafe: false,
                    },
                ),
                (
                    preferred_id,
                    HirExtern {
                        id: preferred_id,
                        name: "preferred".to_string(),
                        params: vec![Type::Str],
                        ret: Type::I32,
                        variadic: false,
                        is_unsafe: false,
                    },
                ),
            ]),
            HirNameTables {
                externs_by_name: HashMap::from([
                    ("missing".to_string(), missing_id),
                    ("preferred".to_string(), preferred_id),
                ]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.order.externs = vec![preferred_id, stale_id];
        let ids = program
            .externs_in_order()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();

        assert_eq!(ids, vec![preferred_id, missing_id]);
    }

    #[test]
    fn hir_program_externs_keep_owned_ids_after_serde_roundtrip_and_rebuild() {
        let extern_id = def_id(13);
        let default_id = DefId::new(CrateId(0), LocalDefId(0));
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                extern_id,
                HirExtern {
                    id: extern_id,
                    name: "puts".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                externs_by_name: HashMap::from([("puts".to_string(), extern_id)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        let bytes = bincode::serialize(&program).unwrap();
        let mut decoded: HirProgram = bincode::deserialize(&bytes).unwrap();
        decoded.rebuild_indexes();

        assert!(decoded.extern_by_id(extern_id).is_some());
        assert!(!decoded.indexes.externs_by_id.contains_key(&default_id));
    }

    #[test]
    fn hir_program_indexes_same_named_fields_by_owner_and_field_id() {
        let left_owner = def_id(10);
        let right_owner = def_id(11);
        let program = program(
            HashMap::new(),
            HashMap::from([
                (
                    left_owner,
                    HirStruct {
                        id: left_owner,
                        name: "Left".to_string(),
                        generic_params: Vec::new(),
                        fields: vec![HirField {
                            id: field_id(0),
                            name: "value".to_string(),
                            ty: Type::I32,
                            public: true,
                        }],
                    },
                ),
                (
                    right_owner,
                    HirStruct {
                        id: right_owner,
                        name: "Right".to_string(),
                        generic_params: Vec::new(),
                        fields: vec![HirField {
                            id: field_id(0),
                            name: "value".to_string(),
                            ty: Type::I64,
                            public: true,
                        }],
                    },
                ),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                structs_by_name: HashMap::from([
                    ("Left".to_string(), left_owner),
                    ("Right".to_string(), right_owner),
                ]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert_eq!(
            program.indexes.fields_by_id[&(left_owner, field_id(0))],
            HirFieldLocation {
                owner: left_owner,
                field_id: field_id(0),
                name: "value".to_string(),
            }
        );
        assert_eq!(
            program.indexes.fields_by_id[&(right_owner, field_id(0))],
            HirFieldLocation {
                owner: right_owner,
                field_id: field_id(0),
                name: "value".to_string(),
            }
        );
        assert_eq!(program.indexes.fields_by_id.len(), 2);
    }

    #[test]
    fn hir_program_indexes_same_named_variants_by_owner_and_variant_id() {
        let left_owner = def_id(20);
        let right_owner = def_id(21);
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                (
                    left_owner,
                    HirEnum {
                        id: left_owner,
                        name: "LeftChoice".to_string(),
                        generic_params: Vec::new(),
                        variants: vec![HirVariant {
                            id: variant_id(0),
                            name: "Some".to_string(),
                            fields: HirVariantFields::Unit,
                        }],
                    },
                ),
                (
                    right_owner,
                    HirEnum {
                        id: right_owner,
                        name: "RightChoice".to_string(),
                        generic_params: Vec::new(),
                        variants: vec![HirVariant {
                            id: variant_id(0),
                            name: "Some".to_string(),
                            fields: HirVariantFields::Unit,
                        }],
                    },
                ),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                enums_by_name: HashMap::from([
                    ("LeftChoice".to_string(), left_owner),
                    ("RightChoice".to_string(), right_owner),
                ]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert_eq!(
            program.indexes.variants_by_id[&(left_owner, variant_id(0))],
            HirVariantLocation {
                owner: left_owner,
                variant_id: variant_id(0),
                name: "Some".to_string(),
            }
        );
        assert_eq!(
            program.indexes.variants_by_id[&(right_owner, variant_id(0))],
            HirVariantLocation {
                owner: right_owner,
                variant_id: variant_id(0),
                name: "Some".to_string(),
            }
        );
        assert_eq!(program.indexes.variants_by_id.len(), 2);
    }

    #[test]
    fn hir_program_indexes_associated_type_decl_and_def_by_owner_and_assoc_type_id() {
        let trait_owner = def_id(30);
        let impl_owner = def_id(31);
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                trait_owner,
                HirTrait {
                    target: None,
                    predicates: Vec::new(),
                    id: trait_owner,
                    name: "Iterable".to_string(),
                    generic_params: Vec::new(),
                    associated_types: vec![HirAssociatedTypeDecl {
                        id: assoc_type_id(0),
                        name: "Item".to_string(),
                        kind: crate::type_services::kind::Kind::Type,
                    }],
                    methods: HashMap::new(),
                    signatures: HashMap::new(),
                },
            )]),
            HashMap::from([(
                impl_owner,
                HirImpl {
                    id: impl_owner,
                    owner: HirImplOwner::Named("List".to_string()),
                    type_name: "List".to_string(),
                    type_generics: Vec::new(),
                    receiver_pattern: Vec::new().into(),
                    trait_name: Some("Iterable".to_string()),
                    trait_id: Some(trait_owner),
                    trait_generics: Vec::new(),
                    trait_arg_types: Vec::new(),
                    associated_types: vec![HirAssociatedTypeDef {
                        id: assoc_type_id(0),
                        name: "Item".to_string(),
                        kind: crate::type_services::kind::Kind::Type,
                        ty: Type::I32,
                    }],
                    bounds: std::collections::HashMap::new().into(),
                    methods: HashMap::new(),
                },
            )]),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([("Iterable".to_string(), trait_owner)]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert_eq!(
            program.indexes.associated_type_decls_by_id[&(trait_owner, assoc_type_id(0))],
            HirAssociatedTypeLocation {
                owner: trait_owner,
                assoc_type_id: assoc_type_id(0),
                name: "Item".to_string(),
            }
        );
        assert_eq!(
            program.indexes.associated_type_defs_by_id[&(impl_owner, assoc_type_id(0))],
            HirAssociatedTypeLocation {
                owner: impl_owner,
                assoc_type_id: assoc_type_id(0),
                name: "Item".to_string(),
            }
        );
        assert_eq!(program.indexes.associated_type_decls_by_id.len(), 1);
        assert_eq!(program.indexes.associated_type_defs_by_id.len(), 1);
    }

    #[test]
    fn hir_extern_carries_canonical_def_id() {
        let extern_fn = HirExtern {
            id: DefId::new(CrateId(0), LocalDefId(9)),
            name: "puts".to_string(),
            params: vec![Type::Str],
            ret: Type::I32,
            variadic: false,
            is_unsafe: false,
        };

        assert_eq!(extern_fn.id, DefId::new(CrateId(0), LocalDefId(9)));
    }

    #[test]
    fn hir_extern_serde_roundtrips_id() {
        let extern_fn = HirExtern {
            id: DefId::new(CrateId(1), LocalDefId(9)),
            name: "printf".to_string(),
            params: vec![Type::Pointer(Box::new(Type::I8))],
            ret: Type::I32,
            variadic: true,
            is_unsafe: false,
        };

        let bytes = bincode::serialize(&extern_fn).unwrap();
        let decoded: HirExtern = bincode::deserialize(&bytes).unwrap();

        assert_eq!(decoded.id, extern_fn.id);
        assert_eq!(decoded.name, extern_fn.name);
        assert_eq!(decoded.params, extern_fn.params);
        assert_eq!(decoded.ret, extern_fn.ret);
        assert_eq!(decoded.variadic, extern_fn.variadic);
    }

    #[test]
    fn hir_program_serde_skips_derived_indexes() {
        let program = program(
            HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(
                def_id(2),
                HirExtern {
                    id: def_id(2),
                    name: "puts".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
                externs_by_name: HashMap::from([("puts".to_string(), def_id(2))]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert!(!program.indexes.functions_by_id.is_empty());
        assert!(!program.indexes.externs_by_id.is_empty());

        let bytes = bincode::serialize(&program).unwrap();
        let decoded: HirProgram = bincode::deserialize(&bytes).unwrap();

        assert!(decoded.indexes.functions_by_id.is_empty());
        assert!(decoded.indexes.externs_by_id.is_empty());
        assert_eq!(decoded.externs[&def_id(2)].id, def_id(2));
    }

    #[test]
    fn hir_program_indexes_definitions_by_def_id() {
        let program = program(
            HashMap::from([(def_id(1), empty_function(def_id(1), "main"))]),
            HashMap::from([(def_id(2), empty_struct(def_id(2), "Thing"))]),
            HashMap::from([(def_id(3), empty_enum(def_id(3), "Choice"))]),
            HashMap::from([(
                def_id(4),
                trait_with_default_method(def_id(4), "Show", def_id(8)),
            )]),
            HashMap::from([(def_id(5), impl_with_method(def_id(5), def_id(9)))]),
            HashMap::from([(
                def_id(6),
                HirExtern {
                    id: def_id(6),
                    name: "puts".to_string(),
                    params: vec![Type::Str],
                    ret: Type::I32,
                    variadic: false,
                    is_unsafe: false,
                },
            )]),
            HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), def_id(1))]),
                structs_by_name: HashMap::from([("Thing".to_string(), def_id(2))]),
                enums_by_name: HashMap::from([("Choice".to_string(), def_id(3))]),
                traits_by_name: HashMap::from([("Show".to_string(), def_id(4))]),
                externs_by_name: HashMap::from([("puts".to_string(), def_id(6))]),
                type_aliases_by_name: HashMap::new(),
            },
            HashMap::new(),
        );

        assert_eq!(program.indexes.functions_by_id[&def_id(1)], "main");
        assert_eq!(program.indexes.structs_by_id[&def_id(2)], "Thing");
        assert_eq!(program.indexes.enums_by_id[&def_id(3)], "Choice");
        assert_eq!(program.indexes.traits_by_id[&def_id(4)], "Show");
        assert_eq!(program.indexes.impls_by_id[&def_id(5)], 0);
        assert_eq!(program.indexes.externs_by_id[&def_id(6)], 0);
        assert_eq!(
            program.indexes.methods_by_id[&def_id(8)],
            HirMethodLocation::TraitDefault {
                trait_id: def_id(4),
                method_id: def_id(8),
                method_name: "default_method".to_string(),
            }
        );
        assert_eq!(
            program.indexes.methods_by_id[&def_id(9)],
            HirMethodLocation::ImplMethod {
                impl_id: def_id(5),
                method_id: def_id(9),
                method_name: "impl_method".to_string(),
            }
        );
    }

    #[test]
    fn hir_program_indexes_trait_default_aliases_by_canonical_trait_name() {
        let trait_id = def_id(4);
        let method_id = def_id(8);
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("show".to_string(), empty_function(method_id, "show"))]),
            signatures: HashMap::new(),
        };
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([
                    ("Alias".to_string(), trait_id),
                    ("module::Show".to_string(), trait_id),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(trait_id, "module::Show".to_string())]),
        );

        assert_eq!(
            program.indexes.methods_by_id[&method_id],
            HirMethodLocation::TraitDefault {
                trait_id,
                method_id,
                method_name: "show".to_string(),
            }
        );
    }

    #[test]
    fn hir_indexes_distinguish_multiple_inherited_default_method_ids() {
        let first_method_id = def_id(8);
        let second_method_id = def_id(9);
        let program = program(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([
                (
                    def_id(4),
                    HirTrait {
                        target: None,
                        predicates: Vec::new(),
                        id: def_id(4),
                        name: "First".to_string(),
                        generic_params: Vec::new(),
                        associated_types: Vec::new(),
                        methods: HashMap::from([(
                            "value".to_string(),
                            empty_function(first_method_id, "value"),
                        )]),
                        signatures: HashMap::new(),
                    },
                ),
                (
                    def_id(5),
                    HirTrait {
                        target: None,
                        predicates: Vec::new(),
                        id: def_id(5),
                        name: "Second".to_string(),
                        generic_params: Vec::new(),
                        associated_types: Vec::new(),
                        methods: HashMap::from([(
                            "value".to_string(),
                            empty_function(second_method_id, "value"),
                        )]),
                        signatures: HashMap::new(),
                    },
                ),
            ]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([
                    ("First".to_string(), def_id(4)),
                    ("Second".to_string(), def_id(5)),
                ]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        assert_eq!(
            program.indexes.methods_by_id[&first_method_id],
            HirMethodLocation::TraitDefault {
                trait_id: def_id(4),
                method_id: first_method_id,
                method_name: "value".to_string(),
            }
        );
        assert_eq!(
            program.indexes.methods_by_id[&second_method_id],
            HirMethodLocation::TraitDefault {
                trait_id: def_id(5),
                method_id: second_method_id,
                method_name: "value".to_string(),
            }
        );
    }

    #[test]
    fn hir_program_rebuild_indexes() {
        let mut program = program(
            HashMap::from([
                (def_id(1), empty_function(def_id(1), "main")),
                (def_id(2), empty_function(def_id(2), "helper")),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("main".to_string(), def_id(1)),
                    ("helper".to_string(), def_id(2)),
                ]),
                ..HirNameTables::default()
            },
            HashMap::new(),
        );

        program.indexes.functions_by_id.remove(&def_id(2));
        assert!(!program.indexes.functions_by_id.contains_key(&def_id(2)));

        program.rebuild_indexes();

        assert_eq!(program.indexes.functions_by_id[&def_id(2)], "helper");
    }

    #[test]
    fn hir_program_rebuild_indexes_with_canonical_names() {
        let mut program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(def_id(1), empty_function(def_id(1), "canonical"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([
                    ("canonical".to_string(), def_id(1)),
                    ("alias".to_string(), def_id(1)),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(def_id(1), "canonical".to_string())]),
        );

        program
            .indexes
            .functions_by_id
            .insert(def_id(1), "alias".to_string());
        assert_eq!(program.indexes.functions_by_id[&def_id(1)], "alias");

        program.rebuild_indexes_with_canonical_names(&HashMap::from([(
            def_id(1),
            "canonical".to_string(),
        )]));

        assert_eq!(program.indexes.functions_by_id[&def_id(1)], "canonical");
    }

    #[test]
    fn hir_program_uses_canonical_names_with_aliases() {
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(def_id(1), empty_function(def_id(1), "canonical"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(def_id(1), "canonical".to_string())]),
        );

        assert_eq!(program.indexes.functions_by_id[&def_id(1)], "canonical");
    }

    #[test]
    fn hir_program_resolves_aliased_function_by_def_id() {
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([(def_id(1), empty_function(def_id(1), "dep::canonical"))]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(def_id(1), "dep::canonical".to_string())]),
        );

        let (name, function) = program.function_by_id(def_id(1)).unwrap();

        assert_eq!(name, "dep::canonical");
        assert_eq!(function.id, def_id(1));
    }

    #[test]
    fn hir_program_iterates_canonical_functions_once_per_def_id() {
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::from([
                (def_id(1), empty_function(def_id(1), "dep::canonical")),
                (def_id(2), empty_function(def_id(2), "other")),
            ]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                functions_by_name: HashMap::from([("alias".to_string(), def_id(1))]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(def_id(1), "dep::canonical".to_string())]),
        );

        let mut functions = program.functions_by_id().collect::<Vec<_>>();
        functions.sort_by_key(|(_, name, _)| (*name).to_string());

        assert_eq!(functions.len(), 2);
        assert_eq!(functions[0].0, def_id(1));
        assert_eq!(functions[0].1, "dep::canonical");
        assert_eq!(functions[1].0, def_id(2));
        assert_eq!(functions[1].1, "other");
    }

    #[test]
    fn hir_program_uses_table_candidates_when_canonical_name_is_for_another_kind() {
        let trait_def = HirTrait {
            target: None,
            predicates: Vec::new(),
            id: def_id(1),
            name: "Animal".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        };
        let program = HirProgram::from_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(def_id(1), trait_def)]),
            HashMap::new(),
            HashMap::new(),
            HirNameTables {
                traits_by_name: HashMap::from([
                    ("Animal".to_string(), def_id(1)),
                    ("dep::Animal".to_string(), def_id(1)),
                ]),
                ..HirNameTables::default()
            },
            crate::hir::HirLanguageItems::default(),
            &HashMap::from([(def_id(1), "Dog".to_string())]),
        );

        assert_eq!(program.indexes.traits_by_id[&def_id(1)], "dep::Animal");
    }

    #[test]
    fn substitute_typevars_in_function_updates_method_target_trait_args() {
        let trait_id = def_id(100);
        let method_id = def_id(101);
        let impl_id = def_id(102);
        let generic = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let source_ty = Type::Generic(generic);
        let mut function = HirFunction {
            id: method_id,
            name: "call".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: source_ty.clone(),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::I64,
            body: HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::MethodCall(
                        Box::new(HirExpr {
                            kind: HirExprKind::Var("self".to_string()),
                            ty: source_ty.clone(),
                            span: crate::lexer::Span::test(),
                        }),
                        "target".to_string(),
                        Vec::new(),
                        None,
                        Some(HirMethodCallTarget::impl_method(
                            impl_id,
                            method_id,
                            Some(HirSelectedTraitMember {
                                trait_id,
                                member_id: method_id,
                                trait_args: vec![source_ty.clone()],
                            }),
                        )),
                    ),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        };

        super::substitute_typevars_in_function(&mut function, &source_ty, &Type::I64);

        let HirStmt::Expr(expr) = &function.body.stmts[0] else {
            panic!("function body should contain method call");
        };
        let HirExprKind::MethodCall(_, _, _, _, Some(target)) = &expr.kind else {
            panic!("function body should contain method target");
        };
        assert_eq!(target.trait_args(), &[Type::I64]);
    }
}
