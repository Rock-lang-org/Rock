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

mod accepted;
mod accepted_program;
mod accepted_validation;
mod codegen_concrete;
mod definition_indexes;
mod definitions;
mod expr;
mod generic_bounds;
pub(crate) mod language_items;
mod method_authority;
mod method_target;
mod program;
mod receiver_shape;
mod type_ids;
mod typevar_substitution;

pub use accepted::{
    AcceptedHirBlock, AcceptedHirExpr, AcceptedHirFunction, AcceptedHirImpl, AcceptedHirProgram,
    AcceptedHirTrait,
};
use accepted_validation::type_contains_backend_forbidden_sentinel;
pub use codegen_concrete::hir_function_is_codegen_concrete;
pub(crate) use codegen_concrete::{
    function_is_object_provided_candidate, function_requires_downstream_specialization,
    hir_function_signature_is_codegen_concrete, impl_requires_downstream_specialization,
};
use definition_indexes::{display_aliases_for_id, ordered_owned_ids, sorted_definition_ids};
pub use definition_indexes::{
    HirAssociatedTypeLocation, HirDefinitionIndexes, HirFieldLocation, HirMethodLocation,
    HirVariantLocation,
};
pub use definitions::{
    HirAssociatedTypeDecl, HirAssociatedTypeDef, HirEnum, HirField, HirFunction, HirFunctionFor,
    HirFunctionSig, HirImpl, HirImplFor, HirImplOwner, HirParam, HirStruct, HirTrait, HirTraitFor,
    HirVariant, HirVariantFields,
};
pub use expr::{
    BinOp, HirBlock, HirBlockFor, HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirExpr,
    HirExprFor, HirExprKind, HirExprKindFor, HirLiteralPattern, HirMatchArm, HirMatchArmFor,
    HirPattern, HirStmt, HirStmtFor, HirStructLiteralField, HirStructLiteralFieldFor,
    HirStructPatternField, HirVarRef, HirVarTarget, UnaryOp,
};
pub use generic_bounds::HirGenericBounds;
pub use method_target::{
    HirMethodCallTarget, HirSelectedMethodTarget, HirSelectedTraitMember, HirStaticMethodTarget,
    HirTraitDispatchKind, HirTypeBinding,
};
pub(crate) use receiver_shape::{
    function_has_receiver, validate_effective_trait_member_receiver_shape,
    validate_effective_trait_method_receiver_shape, validate_impl_receiver_pattern_bindings,
};
pub use type_ids::{collect_hir_type_ids, HirTypeIds, HirTypeLocation};
pub use typevar_substitution::substitute_typevars_in_function;

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

impl HirExtern {
    /// The extern's callable type: its parameter types, return type and safety.
    pub fn fn_type(&self) -> Type {
        Type::function_with_safety(
            self.params.clone(),
            self.ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(self.is_unsafe),
        )
    }
}

#[cfg(test)]
mod tests;
