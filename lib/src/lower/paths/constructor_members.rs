//! Constructor static member selection and instantiation.

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::lower::Lowerer;
use crate::types::Type;
use std::collections::{HashMap, HashSet};

impl Lowerer {
    pub(super) fn constructor_target_for_path_segment(
        &mut self,
        segment: &ast::IdentOrType,
    ) -> Option<Type> {
        match segment {
            ast::IdentOrType::Type(ast::ParseType::Type(inner)) if inner.generics.is_empty() => {
                Some(crate::type_lowering::TypeLowerer::lower_constructor(
                    self,
                    &ast::ParseType::Type(inner.clone()),
                ))
            }
            ast::IdentOrType::Type(parse_type) => Some(
                crate::type_lowering::TypeLowerer::lower_parse_type(self, parse_type),
            ),
            ast::IdentOrType::Ident(ident) => {
                let nominal = crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_nominal_type(&ident.name)?;
                match nominal {
                    crate::type_lowering::ResolvedNominalType::Struct(structure) => {
                        if structure.generic_params.is_empty() {
                            Some(Type::Struct {
                                id: structure.id,
                                args: Vec::new(),
                            })
                        } else {
                            Some(Type::Constructor {
                                id: structure.id,
                                flavor: crate::types::NominalTypeKind::Struct,
                            })
                        }
                    }
                    crate::type_lowering::ResolvedNominalType::Enum(enumeration) => {
                        if enumeration.generic_params.is_empty() {
                            Some(Type::Enum {
                                id: enumeration.id,
                                args: Vec::new(),
                            })
                        } else {
                            Some(Type::Constructor {
                                id: enumeration.id,
                                flavor: crate::types::NominalTypeKind::Enum,
                            })
                        }
                    }
                }
            }
        }
    }

    pub(super) fn select_constructor_static_member(
        &mut self,
        target_ty: &Type,
        required_trait_id: Option<crate::ids::DefId>,
        method_name: &str,
        span: Span,
    ) -> Result<Option<crate::selection::SelectedConstructorMember>, String> {
        let target_kind = self.engine.kind_of(target_ty).map_err(|error| {
            format!(
                "invalid constructor target `{}`: {error}",
                self.display_type(target_ty)
            )
        })?;
        let mut trait_ids = self
            .items
            .trait_defs()
            .filter_map(|(trait_id, trait_def)| {
                let expected_kind = trait_def
                    .target
                    .as_ref()
                    .map(|target| target.kind.clone())
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                (required_trait_id.is_none_or(|required| required == trait_id)
                    && expected_kind == target_kind
                    && (trait_def.methods.contains_key(method_name)
                        || trait_def.signatures.contains_key(method_name)))
                .then_some(trait_id)
            })
            .collect::<Vec<_>>();
        trait_ids.sort();

        if trait_ids.is_empty() {
            if let Some(trait_id) = required_trait_id {
                let trait_def = self
                    .trait_by_id(trait_id)
                    .ok_or_else(|| "unknown required constructor trait".to_string())?;
                let expected_kind = trait_def
                    .target
                    .as_ref()
                    .map(|target| target.kind.clone())
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                if expected_kind != target_kind {
                    return Err(format!(
                        "constructor target `{}` has kind {target_kind}, but trait '{}' requires {expected_kind}",
                        self.display_type(target_ty),
                        trait_def.name
                    ));
                }
            } else {
                let mut mismatches = self
                    .items
                    .trait_defs()
                    .filter_map(|(trait_id, trait_def)| {
                        let has_member = trait_def.methods.contains_key(method_name)
                            || trait_def.signatures.contains_key(method_name);
                        let expected_kind = trait_def.target.as_ref()?.kind.clone();
                        (has_member && expected_kind != target_kind).then_some((
                            trait_id,
                            trait_def.name.clone(),
                            expected_kind,
                        ))
                    })
                    .collect::<Vec<_>>();
                mismatches.sort_by_key(|(trait_id, _, _)| *trait_id);
                if let Some((_, trait_name, expected_kind)) = mismatches.first() {
                    return Err(format!(
                        "constructor target `{}` has kind {target_kind}, but trait '{trait_name}' requires {expected_kind}",
                        self.display_type(target_ty)
                    ));
                }
            }
            return Ok(None);
        }

        let mut candidates = Vec::new();
        for trait_id in trait_ids {
            let trait_def = self
                .trait_by_id(trait_id)
                .cloned()
                .expect("candidate trait exists");
            let member_id = trait_def
                .methods
                .get(method_name)
                .map(|method| method.id)
                .or_else(|| {
                    trait_def
                        .signatures
                        .get(method_name)
                        .map(|method| method.id)
                })
                .expect("candidate trait member exists");
            let trait_args = trait_def
                .generic_params
                .iter()
                .map(|param| {
                    self.engine
                        .fresh_type_var_at_kind(span.clone(), param.kind.clone())
                })
                .collect::<Vec<_>>();
            match self.selection_service().select_constructor_trait_member(
                target_ty,
                trait_id,
                &trait_args,
                member_id,
            ) {
                Ok(selected) => candidates.push(selected),
                Err(crate::selection::SelectionDiagnostic::NoImplementation { .. }) => {}
                Err(error) => return Err(self.display_selection_error(&error)),
            }
        }
        candidates.sort_by_key(|selected| {
            (
                selected.target.trait_id(),
                selected.target.impl_id(),
                selected.target.method_id(),
            )
        });
        match candidates.len() {
            0 => Ok(None),
            1 => Ok(candidates.pop()),
            _ => Err(format!(
                "ambiguous constructor trait member `{method_name}` for `{}`: multiple implementations match",
                self.display_type(target_ty)
            )),
        }
    }

    pub(super) fn register_static_owner_defaults(&mut self, owner_ty: &Type, signature: &Type) {
        let owner = match owner_ty {
            Type::Constructor { id, .. } | Type::Struct { id, .. } | Type::Enum { id, .. } => *id,
            _ => return,
        };
        let Some(params) =
            crate::type_lowering::TypeLoweringContext::constructor_params(self, owner)
        else {
            return;
        };
        let mut arguments = match owner_ty {
            Type::Struct { args, .. } | Type::Enum { args, .. } => Some(args.clone()),
            _ => None,
        };
        crate::type_services::visit::visit_type(signature, &mut |nested: &Type| {
            if let Type::Struct { id, args } | Type::Enum { id, args } = nested {
                if *id == owner && arguments.is_none() {
                    arguments = Some(args.clone());
                }
            }
        });
        if let Some(arguments) = arguments {
            self.engine.register_generic_defaults(&params, &arguments);
        }
    }

    pub(super) fn instantiate_selected_constructor_member(
        &mut self,
        target_ty: Type,
        mut selected: crate::selection::SelectedConstructorMember,
        display_name: String,
        span: Span,
    ) -> Result<(HirExpr, HirStaticMethodTarget), String> {
        let raw_ty = Type::function_with_safety(
            selected
                .function
                .params
                .iter()
                .map(|param| param.ty.clone())
                .collect(),
            selected.function.ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(selected.function.is_unsafe),
        );
        let ty = self.instantiate_function_type(&selected.function, span.clone());
        self.register_static_owner_defaults(&target_ty, &ty);
        let mut substitution = HashMap::new();
        Self::infer_generic_subst_from_types(&raw_ty, &ty, &mut substitution);
        let owner_params = selected
            .target
            .owner_substitution
            .iter()
            .map(|binding| binding.param)
            .collect::<HashSet<_>>();
        selected.target.method_substitution = selected
            .function
            .generic_params
            .iter()
            .filter(|param| {
                !owner_params.contains(&param.id)
                    && selected
                        .target
                        .trait_id()
                        .is_none_or(|trait_id| param.id.owner != trait_id)
            })
            .filter_map(|param| {
                substitution
                    .get(&param.id)
                    .cloned()
                    .map(|ty| HirTypeBinding {
                        param: param.id,
                        ty,
                    })
            })
            .collect();
        selected.target.method_substitution.sort_by_key(|binding| {
            (
                binding.param.owner.crate_id.0,
                binding.param.owner.local.0,
                binding.param.index,
            )
        });
        let mut bound_substitution = selected
            .target
            .owner_substitution
            .iter()
            .map(|binding| (binding.param, binding.ty.clone()))
            .collect::<HashMap<_, _>>();
        bound_substitution.extend(substitution.iter().map(|(&param, ty)| (param, ty.clone())));
        let operation_span = span.clone();
        for (subject, bound) in &selected.pending_impl_bounds {
            self.constraint_store.add_trait(
                subject.substitute_generics(&bound_substitution),
                crate::types::TraitBound {
                    trait_id: bound.trait_id,
                    type_args: bound
                        .type_args
                        .iter()
                        .map(|arg| arg.substitute_generics(&bound_substitution))
                        .collect(),
                },
                operation_span.clone(),
                "constructor trait member",
            );
        }
        let callee = HirExpr {
            ty,
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: display_name,
                target: HirVarTarget::Function(selected.function.id),
            }),
            span,
        };
        Ok((
            callee,
            HirStaticMethodTarget {
                owner_ty: target_ty,
                method: selected.target,
            },
        ))
    }
}
