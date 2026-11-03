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
        // All operators are now handled via std infix declarations and method calls
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
mod tests;
