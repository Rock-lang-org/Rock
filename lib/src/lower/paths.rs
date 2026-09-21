//! Path, instance, lambda, and tuple lowering

use std::collections::{HashMap, HashSet};

use crate::ast;
use crate::hir::*;
use crate::lexer::Span;
use crate::types::{
    CallableKind, CaptureKind, FunctionCapture, FunctionSafety, GenericParamId, Type,
};

use crate::lower::{path_names, seg_name, Lowerer};

fn display_type_without_ids(ty: &Type) -> String {
    let context = crate::type_services::display::TypeDisplayContext::default();
    crate::type_services::display::display_type_with_context(ty, &context).to_string()
}

impl Lowerer {
    fn full_path_span(path: &[ast::IdentOrType]) -> Span {
        let first = path.first().expect("path must not be empty");
        let last = path.last().expect("path must not be empty");
        let start = match first {
            ast::IdentOrType::Ident(ident) => ident.span.start,
            ast::IdentOrType::Type(ty) => ty.span().start,
        };
        let end = match last {
            ast::IdentOrType::Ident(ident) => ident.span.end,
            ast::IdentOrType::Type(ty) => ty.span().end,
        };
        let file_path = match first {
            ast::IdentOrType::Ident(ident) => ident.span.file_path.clone(),
            ast::IdentOrType::Type(ty) => ty.span().file_path,
        };
        Span::new(file_path, start, end)
    }

    fn record_var_reference(&mut self, span: Span, target: &HirVarTarget) {
        let symbol = match target {
            HirVarTarget::Local(local) => {
                self.current_body_def_id()
                    .map(|owner| crate::source_map::SourceSymbol::Local {
                        owner,
                        local: *local,
                    })
            }
            HirVarTarget::Function(id) | HirVarTarget::Extern(id) => {
                Some(crate::source_map::SourceSymbol::Definition(*id))
            }
            HirVarTarget::Instance(_) => None,
        };
        if let Some(symbol) = symbol {
            self.record_source_reference(span, symbol);
        }
    }

    pub(crate) fn lambda_function_type(
        params: Vec<Type>,
        ret: Type,
        safety: FunctionSafety,
        captures: &[HirClosureCapture],
    ) -> Type {
        let captures = captures
            .iter()
            .map(|capture| FunctionCapture {
                kind: match capture.kind {
                    HirClosureCaptureKind::SharedBorrow => CaptureKind::SharedBorrow,
                    HirClosureCaptureKind::MutableBorrow => CaptureKind::MutableBorrow,
                    HirClosureCaptureKind::Move => CaptureKind::Move,
                },
                ty: capture.ty.clone(),
            })
            .collect::<Vec<_>>();
        let callable_kind = CallableKind::from_captures(&captures);
        Type::function_with_metadata(params, ret, safety, callable_kind, captures)
    }

    /// Replace semantic generic parameters in `ty` with fresh TypeVars in the current engine.
    /// This is called at every function reference site so generic functions can be
    /// independently instantiated per call (like HM polymorphism).
    pub(crate) fn instantiate_generics(&mut self, ty: Type, span: Span) -> Type {
        let mut generic_params = std::collections::HashSet::new();
        ty.collect_generic_params(&mut generic_params);
        if generic_params.is_empty() {
            return ty;
        }
        let subst: HashMap<crate::types::GenericParamId, Type> = generic_params
            .into_iter()
            .map(|param| {
                let kind = self
                    .engine
                    .kind_of(&Type::Generic(param))
                    .unwrap_or(crate::type_services::kind::Kind::Type);
                (
                    param,
                    self.engine.fresh_type_var_at_kind(span.clone(), kind),
                )
            })
            .collect();
        ty.substitute_generics(&subst)
    }

    fn current_generic_param_id_for_name(&self, name: &str) -> Option<GenericParamId> {
        let owner = self.current_generic_owner()?;
        let index = self
            .current_generic_params()
            .iter()
            .position(|param| param == name)?;
        Some(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    pub(crate) fn resolve_static_bound_method_path(
        &mut self,
        owner_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        self.resolve_static_bound_member_path(owner_name, None, method_name, span)
    }

    pub(crate) fn resolve_static_qualified_bound_method_path(
        &mut self,
        owner_name: &str,
        trait_name: &str,
        method_name: &str,
        span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        let trait_id = self
            .trait_by_name(trait_name)
            .map(|trait_def| trait_def.id)
            .ok_or_else(|| format!("unknown constructor trait `{trait_name}`"))?;
        self.resolve_static_bound_member_path(owner_name, Some(trait_id), method_name, span)
    }

    fn resolve_static_bound_member_path(
        &mut self,
        owner_name: &str,
        required_trait_id: Option<crate::ids::DefId>,
        method_name: &str,
        operation_span: Span,
    ) -> Result<Option<(Type, HirStaticMethodTarget)>, String> {
        let Some(owner_param) = self.current_generic_param_id_for_name(owner_name) else {
            return Ok(None);
        };
        let owner_ty = Type::Generic(owner_param);
        let Some(bounds) = self.current_impl_bounds().get(&owner_param) else {
            return Ok(None);
        };
        let bounds = self
            .selection_service()
            .trait_bounds_with_supertraits(&owner_ty, bounds);

        let mut candidates = Vec::new();
        for bound in &bounds {
            if required_trait_id.is_some_and(|required| required != bound.trait_id) {
                continue;
            }
            let Some(trait_def) = self.trait_by_id(bound.trait_id).cloned() else {
                continue;
            };
            let owner_kind = self.engine.kind_of(&owner_ty).map_err(|error| {
                format!("cannot determine kind of constructor `{owner_name}`: {error}")
            })?;
            let target_kind = trait_def
                .target
                .as_ref()
                .map(|target| target.kind.clone())
                .unwrap_or(crate::type_services::kind::Kind::Type);
            if owner_kind != target_kind {
                if required_trait_id == Some(trait_def.id) {
                    return Err(format!(
                        "constructor target `{owner_name}` has kind {owner_kind}, but trait '{}' requires {target_kind}",
                        trait_def.name
                    ));
                }
                continue;
            }

            let mut subst = HashMap::new();
            for (index, arg) in bound.type_args.iter().enumerate() {
                subst.insert(
                    GenericParamId {
                        owner: trait_def.id,
                        index: index as u32,
                    },
                    arg.clone(),
                );
            }
            let target_param =
                trait_def
                    .target
                    .as_ref()
                    .map(|target| target.id)
                    .unwrap_or(GenericParamId {
                        owner: trait_def.id,
                        index: trait_def.generic_params.len() as u32,
                    });
            subst.insert(target_param, owner_ty.clone());

            let (method_id, method_generic_params, method_generic_bounds, params, ret, safety) =
                if let Some(method) = trait_def
                    .methods
                    .get(method_name)
                    .filter(|method| !method.is_method)
                {
                    (
                        method.id,
                        method.generic_params.clone(),
                        method.generic_bounds.clone(),
                        method.params.iter().map(|param| param.ty.clone()).collect(),
                        method.ret_type.clone(),
                        crate::types::FunctionSafety::from_is_unsafe(method.is_unsafe),
                    )
                } else if let Some(sig) = trait_def
                    .signatures
                    .get(method_name)
                    .filter(|sig| sig.self_receiver.is_none())
                {
                    (
                        sig.id,
                        sig.generic_params.clone(),
                        sig.generic_bounds.clone(),
                        sig.params.clone(),
                        sig.ret.clone(),
                        crate::types::FunctionSafety::from_is_unsafe(sig.is_unsafe),
                    )
                } else {
                    continue;
                };

            let mut inherited_generic_params = trait_def
                .generic_params
                .iter()
                .map(|param| param.id)
                .collect::<HashSet<_>>();
            if let Some(target) = trait_def.target.as_ref() {
                inherited_generic_params.insert(target.id);
            }
            let is_constructor_trait = trait_def.target.as_ref().is_some_and(|target| {
                !matches!(target.kind, crate::type_services::kind::Kind::Type)
            });
            let method_generic_params = method_generic_params
                .into_iter()
                .filter(|param| {
                    if is_constructor_trait {
                        !inherited_generic_params.contains(&param.id)
                    } else {
                        param.id.owner == method_id
                    }
                })
                .collect::<Vec<_>>();

            if let Some(target_decl) = trait_def.target.as_ref() {
                let declared_generics = trait_def
                    .methods
                    .get(method_name)
                    .map(|method| method.generic_params.as_slice())
                    .or_else(|| {
                        trait_def
                            .signatures
                            .get(method_name)
                            .map(|signature| signature.generic_params.as_slice())
                    })
                    .unwrap_or_default();
                for generic in declared_generics {
                    if generic.name == target_decl.name && generic.kind == target_decl.kind {
                        subst.insert(generic.id, owner_ty.clone());
                    }
                }
            }

            let params: Vec<Type> = params
                .into_iter()
                .map(|param| param.substitute_generics(&subst))
                .collect();
            let ret = ret.substitute_generics(&subst);
            let method_generic_bounds = method_generic_bounds
                .iter()
                .map(|(&param, bounds)| {
                    (
                        param,
                        bounds
                            .iter()
                            .map(|bound| crate::types::TraitBound {
                                trait_id: bound.trait_id,
                                type_args: bound
                                    .type_args
                                    .iter()
                                    .map(|arg| arg.substitute_generics(&subst))
                                    .collect(),
                            })
                            .collect(),
                    )
                })
                .collect::<HirGenericBounds>();
            let target = HirMethodCallTarget::trait_method(
                trait_def.id,
                method_id,
                bound
                    .type_args
                    .iter()
                    .map(|arg| arg.substitute_generics(&subst))
                    .collect(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            );
            let mut target = target;
            if !matches!(target_kind, crate::type_services::kind::Kind::Type) {
                target.owner_substitution = subst
                    .iter()
                    .map(|(&param, ty)| HirTypeBinding {
                        param,
                        ty: ty.clone(),
                    })
                    .collect();
                target.owner_substitution.sort_by_key(|binding| {
                    (
                        binding.param.owner.crate_id.0,
                        binding.param.owner.local.0,
                        binding.param.index,
                    )
                });
            }
            candidates.push((
                params,
                ret,
                safety,
                target,
                method_generic_params,
                method_generic_bounds,
            ));
        }
        candidates.sort_by_key(|(_, _, _, target, _, _)| {
            (
                target.trait_id(),
                target.method_id(),
                format!("{:?}", target.trait_args()),
            )
        });
        candidates.dedup_by(|left, right| left.3 == right.3);
        let mut candidate_descriptions = candidates
            .iter()
            .map(|(_, _, _, target, _, _)| {
                let trait_name = target
                    .trait_id()
                    .and_then(|trait_id| self.trait_by_id(trait_id))
                    .map(|trait_def| trait_def.name.as_str())
                    .unwrap_or("<unknown trait>");
                let args = target
                    .trait_args()
                    .iter()
                    .map(|arg| self.display_type(arg))
                    .collect::<Vec<_>>();
                if args.is_empty() {
                    trait_name.to_string()
                } else {
                    format!("{} {}", trait_name, args.join(", "))
                }
            })
            .collect::<Vec<_>>();
        candidate_descriptions.sort();
        candidate_descriptions.dedup();
        match candidates.len() {
            0 => Ok(None),
            1 => {
                let (
                    params,
                    ret,
                    safety,
                    mut target,
                    method_generic_params,
                    method_generic_bounds,
                ) =
                    candidates.pop().expect("single static bound candidate");
                let mut method_subst = HashMap::new();
                target.method_substitution = method_generic_params
                    .into_iter()
                    .map(|param| {
                        let ty = self
                            .engine
                            .fresh_type_var_at_kind(operation_span.clone(), param.kind);
                        method_subst.insert(param.id, ty.clone());
                        HirTypeBinding {
                            param: param.id,
                            ty,
                        }
                    })
                    .collect();
                for (generic_param, bounds) in &method_generic_bounds {
                    let Some(subject) = method_subst.get(generic_param).cloned() else {
                        continue;
                    };
                    for bound in bounds {
                        self.constraint_store.add_trait(
                            subject.clone(),
                            crate::types::TraitBound {
                                trait_id: bound.trait_id,
                                type_args: bound
                                    .type_args
                                    .iter()
                                    .map(|arg| arg.substitute_generics(&method_subst))
                                    .collect(),
                            },
                            operation_span.clone(),
                            "static constructor trait member",
                        );
                    }
                }
                let params = params
                    .into_iter()
                    .map(|param| param.substitute_generics(&method_subst))
                    .collect();
                let ret = ret.substitute_generics(&method_subst);
                Ok(Some((
                    Type::function_with_safety(params, ret, safety),
                    HirStaticMethodTarget {
                        owner_ty,
                        method: target,
                    },
                )))
            }
            _ => Err(format!(
                "ambiguous static bound method `{owner_name}::{method_name}`: matching trait bounds {}",
                candidate_descriptions.join(", ")
            )),
        }
    }

    fn constructor_target_for_path_segment(&mut self, segment: &ast::IdentOrType) -> Option<Type> {
        match segment {
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

    fn select_constructor_static_member(
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

    fn instantiate_selected_constructor_member(
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

    fn static_method_value_lambda(
        &mut self,
        callee: HirExpr,
        target: HirStaticMethodTarget,
    ) -> HirExpr {
        let span = callee.span.clone();
        let Type::Function {
            params,
            ret,
            safety,
            ..
        } = callee.ty.clone()
        else {
            self.diagnostics.push_with_span(
                "selected static method value is not callable".to_string(),
                span.clone(),
            );
            return self.error_expression_at(span);
        };
        let lambda_params = params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("__static_arg_{index}"),
                local_id: self.fresh_local_id(),
                ty: ty.clone(),
                mutable: false,
                is_ref: false,
            })
            .collect::<Vec<_>>();
        let args = lambda_params
            .iter()
            .map(|param| HirExpr {
                ty: param.ty.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: param.name.clone(),
                    target: HirVarTarget::Local(param.local_id),
                }),
                span: span.clone(),
            })
            .collect::<Vec<_>>();
        let method_info = target
            .method
            .impl_id()
            .and_then(|impl_id| self.items.impl_def(impl_id))
            .and_then(|imp| {
                let method_id = target.method.method_id()?;
                imp.methods.iter().find_map(|(name, method)| {
                    (method.id == method_id).then_some((name.clone(), method.clone()))
                })
            });
        let method_info = method_info.or_else(|| {
            target.method.method_id().and_then(|method_id| {
                self.items
                    .impl_defs_in_order()
                    .flat_map(|(_, imp)| imp.methods.iter())
                    .find_map(|(name, method)| {
                        (method.id == method_id).then_some((name.clone(), method.clone()))
                    })
            })
        });
        let call = if let Some((method_name, method)) =
            method_info.filter(|(_, method)| method.self_receiver.is_some())
        {
            let Some(receiver) = args.first().cloned() else {
                self.diagnostics.push_with_span(
                    "selected receiver method value has no receiver parameter".to_string(),
                    span.clone(),
                );
                return self.error_expression_at(span.clone());
            };
            HirExpr {
                ty: ret.as_ref().clone(),
                kind: HirExprKind::MethodCall(
                    Box::new(receiver),
                    method_name.clone(),
                    args.into_iter().skip(1).collect(),
                    method.self_receiver,
                    Some(target.method.clone()),
                ),
                span: span.clone(),
            }
        } else {
            HirExpr {
                ty: ret.as_ref().clone(),
                kind: HirExprKind::Call(
                    Box::new(callee),
                    args,
                    Some(HirCallTarget::StaticMethod(target)),
                ),
                span: span.clone(),
            }
        };
        let body = HirBlock {
            ty: ret.as_ref().clone(),
            stmts: vec![HirStmt::Expr(call)],
        };
        let captures = self.collect_lambda_captures(&body, &lambda_params);

        HirExpr {
            ty: Self::lambda_function_type(params, ret.as_ref().clone(), safety, &captures),
            kind: HirExprKind::Lambda {
                params: lambda_params,
                body,
                captures,
            },
            span,
        }
    }

    fn instantiate_static_method_value(
        &mut self,
        resolved: crate::lower::resolution::LowerResolvedStaticMethod,
        span: Span,
    ) -> Result<(HirExpr, HirStaticMethodTarget), String> {
        let resolved_ty = self.instantiate_resolved_value_type(&resolved.value, span.clone());
        let (ty, fresh_substitution) = self.freshen_type_vars(resolved_ty, span.clone());
        let mut substitution = HashMap::new();
        Self::infer_generic_subst_from_types(&resolved.value.ty, &ty, &mut substitution);
        let mut method_generic_params = if !resolved.method_generic_params.is_empty() {
            resolved.method_generic_params.clone()
        } else if resolved
            .target
            .impl_id()
            .is_some_and(|impl_id| impl_id.crate_id == self.root_crate_id)
            && resolved.target.method_id().is_some()
        {
            let method_id = resolved.target.method_id().expect("checked above");
            self.items
                .impl_defs_in_order()
                .find_map(|(_, imp)| {
                    imp.methods
                        .values()
                        .find(|method| method.id == method_id)
                        .map(|method| method.generic_params.iter().map(|param| param.id).collect())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        method_generic_params.retain(|param| !resolved.owner_generic_params.contains(param));
        let mut raw_type_vars = Vec::new();
        crate::type_services::visit::visit_type(&resolved.value.ty, &mut |nested: &Type| {
            if let Type::TypeVar(id) = nested {
                if !raw_type_vars.contains(id) {
                    raw_type_vars.push(*id);
                }
            }
        });
        if method_generic_params.is_empty()
            && resolved
                .target
                .impl_id()
                .is_some_and(|impl_id| impl_id.crate_id == self.root_crate_id)
        {
            if let Some(method_id) = resolved.target.method_id() {
                method_generic_params = raw_type_vars
                    .iter()
                    .enumerate()
                    .map(|(index, _)| GenericParamId {
                        owner: method_id,
                        index: index as u32,
                    })
                    .collect();
            }
        }
        for (param, id) in method_generic_params.iter().zip(raw_type_vars) {
            if let Some(ty) = fresh_substitution.get(&id) {
                substitution.entry(*param).or_insert_with(|| ty.clone());
            }
        }
        let receiver_pattern = match &resolved.receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                ty.clone()
            }
            HirImplReceiverPattern::SliceFamily { element } => {
                Type::Slice(Box::new(element.clone()))
            }
        };
        let owner_relation = Self::infer_static_owner_signature_relation(
            &receiver_pattern,
            &resolved.value.ty,
            &resolved.owner_generic_params,
            &resolved.value.name,
        )?;

        let mut target = resolved.target;
        target.owner_substitution = resolved
            .owner_generic_params
            .iter()
            .map(|&param| {
                substitution
                    .get(&param)
                    .cloned()
                    .or_else(|| {
                        owner_relation
                            .get(&param)
                            .map(|relation_ty| relation_ty.substitute_generics(&substitution))
                    })
                    .map(|ty| HirTypeBinding { param, ty })
                    .ok_or_else(|| {
                        format!(
                            "static method `{}` could not infer an owner generic parameter",
                            resolved.value.name
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        for binding in &mut target.owner_substitution {
            binding.ty = binding.ty.substitute(&fresh_substitution);
        }
        target.method_substitution = method_generic_params
            .iter()
            .map(|&param| {
                substitution
                    .get(&param)
                    .cloned()
                    .map(|ty| HirTypeBinding { param, ty })
                    .ok_or_else(|| {
                        format!(
                            "static method `{}` could not infer a method generic parameter",
                            resolved.value.name
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        let mut target_substitution = HashMap::new();
        for binding in target
            .owner_substitution
            .iter()
            .chain(&target.method_substitution)
        {
            match target_substitution.get(&binding.param) {
                Some(previous) if previous != &binding.ty => {
                    return Err(format!(
                        "static method `{}` has conflicting substitutions: {} vs {}",
                        resolved.value.name,
                        self.display_type(previous),
                        self.display_type(&binding.ty)
                    ));
                }
                Some(_) => {}
                None => {
                    target_substitution.insert(binding.param, binding.ty.clone());
                }
            }
        }
        if let Some(trait_args) = target.trait_args_mut() {
            for trait_arg in trait_args {
                *trait_arg = trait_arg.substitute_generics(&target_substitution);
            }
        }

        let owner_substitution = target
            .owner_substitution
            .iter()
            .map(|binding| (binding.param, binding.ty.clone()))
            .collect::<HashMap<_, _>>();
        let owner_ty = match resolved.receiver_pattern {
            HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
                ty.substitute_generics(&owner_substitution)
            }
            HirImplReceiverPattern::SliceFamily { element } => {
                Type::Slice(Box::new(element.substitute_generics(&owner_substitution)))
            }
        };
        let function_id = match resolved.value.target {
            Some(HirVarTarget::Function(function_id)) => function_id,
            _ => {
                return Err(format!(
                    "static method `{}` has no function identity",
                    resolved.value.name
                ));
            }
        };
        let callee = HirExpr {
            ty,
            kind: HirExprKind::ResolvedVar(HirVarRef {
                name: resolved.value.name,
                target: HirVarTarget::Function(function_id),
            }),
            span,
        };

        Ok((
            callee,
            HirStaticMethodTarget {
                owner_ty,
                method: target,
            },
        ))
    }

    fn freshen_type_vars(
        &mut self,
        ty: Type,
        span: Span,
    ) -> (Type, HashMap<crate::ids::TypeVarId, Type>) {
        let mut ids = HashSet::new();
        crate::type_services::visit::visit_type(&ty, &mut |nested: &Type| {
            if let Type::TypeVar(id) = nested {
                ids.insert(*id);
            }
        });
        let substitution = ids
            .into_iter()
            .map(|id| {
                let kind = self.engine.kind_of_type_var(id);
                (id, self.engine.fresh_type_var_at_kind(span.clone(), kind))
            })
            .collect::<HashMap<_, _>>();
        (ty.substitute(&substitution), substitution)
    }

    fn infer_static_owner_signature_relation(
        receiver_pattern: &Type,
        signature: &Type,
        owner_params: &[GenericParamId],
        method_name: &str,
    ) -> Result<HashMap<GenericParamId, Type>, String> {
        let owner_params = owner_params.iter().copied().collect::<HashSet<_>>();
        let mut relation = HashMap::new();
        Self::collect_static_owner_signature_relations(
            receiver_pattern,
            signature,
            &owner_params,
            &mut relation,
            method_name,
        )?;
        Ok(relation)
    }

    fn collect_static_owner_signature_relations(
        receiver_pattern: &Type,
        signature: &Type,
        owner_params: &HashSet<GenericParamId>,
        relation: &mut HashMap<GenericParamId, Type>,
        method_name: &str,
    ) -> Result<(), String> {
        if let Some(candidate) =
            Self::static_owner_relation_for_matching_type(receiver_pattern, signature, owner_params)
        {
            for (owner_param, method_ty) in candidate {
                if let Some(previous) = relation.insert(owner_param, method_ty.clone()) {
                    if previous != method_ty {
                        return Err(format!(
                            "static method `{method_name}` has conflicting owner inference: {} vs {}",
                            display_type_without_ids(&previous),
                            display_type_without_ids(&method_ty)
                        ));
                    }
                }
            }
        }

        match signature {
            Type::Slice(inner) | Type::Pointer(inner) => {
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    inner,
                    owner_params,
                    relation,
                    method_name,
                )
            }
            Type::Array(inner, _) => Self::collect_static_owner_signature_relations(
                receiver_pattern,
                inner,
                owner_params,
                relation,
                method_name,
            ),
            Type::Tuple(elements) => {
                for element in elements {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        element,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                for param in params {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        param,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    ret,
                    owner_params,
                    relation,
                    method_name,
                )?;
                for capture in captures {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        &capture.ty,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                for arg in args {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        arg,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Reference { inner, .. } => Self::collect_static_owner_signature_relations(
                receiver_pattern,
                inner,
                owner_params,
                relation,
                method_name,
            ),
            Type::Projection { ty, trait_args, .. } => {
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    ty,
                    owner_params,
                    relation,
                    method_name,
                )?;
                for trait_arg in trait_args {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        trait_arg,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn static_owner_relation_for_matching_type(
        receiver_pattern: &Type,
        signature_ty: &Type,
        owner_params: &HashSet<GenericParamId>,
    ) -> Option<HashMap<GenericParamId, Type>> {
        let mut relation = HashMap::new();
        Self::match_static_owner_pattern(
            receiver_pattern,
            signature_ty,
            owner_params,
            &mut relation,
        )
        .then_some(relation)
    }

    fn match_static_owner_pattern(
        receiver_pattern: &Type,
        signature_ty: &Type,
        owner_params: &HashSet<GenericParamId>,
        relation: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        match (receiver_pattern, signature_ty) {
            (Type::Generic(owner_param), signature_ty) if owner_params.contains(owner_param) => {
                relation
                    .insert(*owner_param, signature_ty.clone())
                    .is_none_or(|previous| previous == *signature_ty)
            }
            (Type::Slice(receiver), Type::Slice(signature))
            | (Type::Pointer(receiver), Type::Pointer(signature)) => {
                Self::match_static_owner_pattern(receiver, signature, owner_params, relation)
            }
            (Type::Array(receiver, receiver_len), Type::Array(signature, signature_len)) => {
                receiver_len == signature_len
                    && Self::match_static_owner_pattern(receiver, signature, owner_params, relation)
            }
            (Type::Tuple(receivers), Type::Tuple(signatures)) => {
                receivers.len() == signatures.len()
                    && receivers
                        .iter()
                        .zip(signatures)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            (
                Type::Function {
                    params: receiver_params,
                    ret: receiver_ret,
                    safety: receiver_safety,
                    ..
                },
                Type::Function {
                    params: signature_params,
                    ret: signature_ret,
                    safety: signature_safety,
                    ..
                },
            ) => {
                receiver_safety == signature_safety
                    && receiver_params.len() == signature_params.len()
                    && receiver_params
                        .iter()
                        .zip(signature_params)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
                    && Self::match_static_owner_pattern(
                        receiver_ret,
                        signature_ret,
                        owner_params,
                        relation,
                    )
            }
            (
                Type::Struct {
                    id: receiver_id,
                    args: receiver_args,
                },
                Type::Struct {
                    id: signature_id,
                    args: signature_args,
                },
            )
            | (
                Type::Enum {
                    id: receiver_id,
                    args: receiver_args,
                },
                Type::Enum {
                    id: signature_id,
                    args: signature_args,
                },
            ) => {
                receiver_id == signature_id
                    && receiver_args.len() == signature_args.len()
                    && receiver_args
                        .iter()
                        .zip(signature_args)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            (
                Type::Reference {
                    mutable: receiver_mutable,
                    inner: receiver_inner,
                },
                Type::Reference {
                    mutable: signature_mutable,
                    inner: signature_inner,
                },
            ) => {
                receiver_mutable == signature_mutable
                    && Self::match_static_owner_pattern(
                        receiver_inner,
                        signature_inner,
                        owner_params,
                        relation,
                    )
            }
            (
                Type::Projection {
                    ty: receiver_ty,
                    trait_id: receiver_trait,
                    assoc_type: receiver_assoc,
                    trait_args: receiver_args,
                },
                Type::Projection {
                    ty: signature_ty,
                    trait_id: signature_trait,
                    assoc_type: signature_assoc,
                    trait_args: signature_args,
                },
            ) => {
                receiver_trait == signature_trait
                    && receiver_assoc == signature_assoc
                    && receiver_args.len() == signature_args.len()
                    && Self::match_static_owner_pattern(
                        receiver_ty,
                        signature_ty,
                        owner_params,
                        relation,
                    )
                    && receiver_args
                        .iter()
                        .zip(signature_args)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            _ => receiver_pattern == signature_ty,
        }
    }

    pub(crate) fn collect_lambda_captures(
        &self,
        body: &HirBlock,
        params: &[HirParam],
    ) -> Vec<HirClosureCapture> {
        let mut defined = std::collections::HashSet::new();
        for param in params {
            defined.insert(param.name.clone());
        }

        let mut used = Vec::new();
        let mut capture_kinds = std::collections::HashMap::new();
        self.collect_lambda_captures_block(
            body,
            &mut defined,
            &mut used,
            &mut capture_kinds,
            HirClosureCaptureKind::SharedBorrow,
        );

        let mut captures = Vec::new();
        let mut seen = std::collections::HashSet::new();

        for name in used {
            if !seen.insert(name.clone()) {
                continue;
            }

            if let Some(binding) = self.scope.lookup(&name) {
                if binding.is_top_level && matches!(&binding.ty, Type::Function { .. }) {
                    continue;
                }

                let Some(local_id) = binding.local_id else {
                    continue;
                };
                let kind = capture_kinds.remove(&name).unwrap_or_else(|| {
                    if binding.mutable {
                        HirClosureCaptureKind::MutableBorrow
                    } else {
                        HirClosureCaptureKind::SharedBorrow
                    }
                });
                captures.push(HirClosureCapture {
                    name,
                    local_id,
                    kind,
                    mutable: binding.mutable,
                    ty: binding.ty.clone(),
                });
            }
        }

        captures
    }

    fn collect_lambda_captures_block(
        &self,
        block: &HirBlock,
        defined: &mut std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
        current_kind: HirClosureCaptureKind,
    ) {
        for stmt in &block.stmts {
            match stmt {
                HirStmt::Let { name, value, .. } => {
                    self.collect_lambda_captures_expr(
                        value,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                    defined.insert(name.clone());
                }
                HirStmt::Expr(expr) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                ),
                HirStmt::Return(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                ),
                HirStmt::Break(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                ),
                _ => {}
            }
        }
    }

    fn collect_lambda_captures_expr(
        &self,
        expr: &HirExpr,
        defined: &mut std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
        current_kind: HirClosureCaptureKind,
    ) {
        match &expr.kind {
            HirExprKind::Var(name) => {
                self.record_lambda_capture(name, current_kind, defined, used, capture_kinds);
            }
            HirExprKind::ResolvedVar(reference)
                if matches!(reference.target, HirVarTarget::Local(_)) =>
            {
                self.record_lambda_capture(
                    &reference.name,
                    current_kind,
                    defined,
                    used,
                    capture_kinds,
                );
            }
            HirExprKind::ResolvedVar(_) => {}
            HirExprKind::FieldAccess(base, _, _) => {
                self.collect_lambda_captures_expr(base, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::TupleIndex(base, _) => {
                self.collect_lambda_captures_expr(base, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::BinOp(_, lhs, rhs) => {
                self.collect_lambda_captures_expr(lhs, defined, used, capture_kinds, current_kind);
                self.collect_lambda_captures_expr(rhs, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::UnaryOp(_, inner)
            | HirExprKind::Deref(inner)
            | HirExprKind::Cast(inner, _) => {
                self.collect_lambda_captures_expr(
                    inner,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::Ref(mutable, inner) => {
                let ref_kind = if *mutable {
                    HirClosureCaptureKind::MutableBorrow
                } else {
                    HirClosureCaptureKind::SharedBorrow
                };
                self.collect_lambda_captures_expr(inner, defined, used, capture_kinds, ref_kind);
            }
            HirExprKind::Call(func, args, _) => {
                self.collect_lambda_captures_expr(func, defined, used, capture_kinds, current_kind);
                let expected_params = match &func.ty {
                    Type::Function { params, .. } => Some(params.as_slice()),
                    _ => None,
                };
                for (index, arg) in args.iter().enumerate() {
                    let arg_kind = expected_params
                        .and_then(|params| params.get(index))
                        .map(|expected| Self::lambda_capture_kind_for_value(expected, current_kind))
                        .unwrap_or_else(|| {
                            Self::lambda_capture_kind_for_value(&arg.ty, current_kind)
                        });
                    self.collect_lambda_captures_expr(arg, defined, used, capture_kinds, arg_kind);
                }
            }
            HirExprKind::MethodCall(recv, _, args, self_receiver, _) => {
                let recv_kind = match self_receiver {
                    Some(crate::types::ReceiverMode::Move) => HirClosureCaptureKind::Move,
                    Some(crate::types::ReceiverMode::Mut) => HirClosureCaptureKind::MutableBorrow,
                    Some(crate::types::ReceiverMode::Shared) | None => current_kind,
                };
                self.collect_lambda_captures_expr(recv, defined, used, capture_kinds, recv_kind);
                for arg in args {
                    self.collect_lambda_captures_expr(
                        arg,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Try { expr, .. } => {
                self.collect_lambda_captures_expr(expr, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                for field in fields {
                    let field_kind =
                        Self::lambda_capture_kind_for_value(&field.value.ty, current_kind);
                    self.collect_lambda_captures_expr(
                        &field.value,
                        defined,
                        used,
                        capture_kinds,
                        field_kind,
                    );
                }
            }
            HirExprKind::EnumVariant(_, _, args, _) => {
                for arg in args {
                    let arg_kind = Self::lambda_capture_kind_for_value(&arg.ty, current_kind);
                    self.collect_lambda_captures_expr(arg, defined, used, capture_kinds, arg_kind);
                }
            }
            HirExprKind::TupleLiteral(elems) | HirExprKind::ArrayLiteral(elems) => {
                for elem in elems {
                    self.collect_lambda_captures_expr(
                        elem,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::ArrayRepeat(value, _) => {
                self.collect_lambda_captures_expr(value, defined, used, capture_kinds, current_kind)
            }
            HirExprKind::Block(block)
            | HirExprKind::Loop(block)
            | HirExprKind::UnsafeBlock(block) => {
                let mut nested_defined = defined.clone();
                self.collect_lambda_captures_block(
                    block,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_lambda_captures_expr(
                    condition,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                let mut then_defined = defined.clone();
                self.collect_lambda_captures_block(
                    then_branch,
                    &mut then_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                if let Some(else_branch) = else_branch {
                    let mut else_defined = defined.clone();
                    self.collect_lambda_captures_block(
                        else_branch,
                        &mut else_defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Match { scrutinee, arms } => {
                self.collect_lambda_captures_expr(
                    scrutinee,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                for arm in arms {
                    let mut arm_defined = defined.clone();
                    self.collect_lambda_captures_block(
                        &arm.body,
                        &mut arm_defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::While { condition, body } => {
                self.collect_lambda_captures_expr(
                    condition,
                    defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
                let mut nested_defined = defined.clone();
                self.collect_lambda_captures_block(
                    body,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::For { iter, body, .. } => {
                self.collect_lambda_captures_expr(iter, defined, used, capture_kinds, current_kind);
                let mut nested_defined = defined.clone();
                self.collect_lambda_captures_block(
                    body,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::Lambda { params, body, .. } => {
                let mut nested_defined = defined.clone();
                for param in params {
                    nested_defined.insert(param.name.clone());
                }
                self.collect_lambda_captures_block(
                    body,
                    &mut nested_defined,
                    used,
                    capture_kinds,
                    current_kind,
                );
            }
            HirExprKind::Intrinsic { args, .. } => {
                for arg in args {
                    self.collect_lambda_captures_expr(
                        arg,
                        defined,
                        used,
                        capture_kinds,
                        current_kind,
                    );
                }
            }
            HirExprKind::Assign(lhs, rhs) => {
                self.collect_lambda_captures_expr(
                    lhs,
                    defined,
                    used,
                    capture_kinds,
                    HirClosureCaptureKind::MutableBorrow,
                );
                self.collect_lambda_captures_expr(rhs, defined, used, capture_kinds, current_kind);
            }
            HirExprKind::Unit
            | HirExprKind::IntLiteral(_)
            | HirExprKind::FloatLiteral(_)
            | HirExprKind::BoolLiteral(_)
            | HirExprKind::StringLiteral(_)
            | HirExprKind::CharLiteral(_) => {}
        }
    }

    fn lambda_capture_kind_for_value(
        ty: &Type,
        default: HirClosureCaptureKind,
    ) -> HirClosureCaptureKind {
        match ty {
            Type::Reference { mutable: true, .. } => HirClosureCaptureKind::MutableBorrow,
            Type::Reference { mutable: false, .. } => HirClosureCaptureKind::SharedBorrow,
            _ if !crate::type_services::facts::TypeFacts::is_copy(ty) => {
                HirClosureCaptureKind::Move
            }
            _ => default,
        }
    }

    fn record_lambda_capture(
        &self,
        name: &str,
        kind: HirClosureCaptureKind,
        defined: &std::collections::HashSet<String>,
        used: &mut Vec<String>,
        capture_kinds: &mut std::collections::HashMap<String, HirClosureCaptureKind>,
    ) {
        if defined.contains(name) {
            return;
        }

        match capture_kinds.get_mut(name) {
            Some(existing) => {
                if Self::capture_kind_rank(kind) > Self::capture_kind_rank(*existing) {
                    *existing = kind;
                }
            }
            None => {
                used.push(name.to_string());
                capture_kinds.insert(name.to_string(), kind);
            }
        }
    }

    fn capture_kind_rank(kind: HirClosureCaptureKind) -> u8 {
        match kind {
            HirClosureCaptureKind::SharedBorrow => 0,
            HirClosureCaptureKind::MutableBorrow => 1,
            HirClosureCaptureKind::Move => 2,
        }
    }

    pub(crate) fn lower_identifier_path(&mut self, path: &ast::IdentifierPath) -> HirExpr {
        let span = Self::full_path_span(&path.path);
        // Simple case: single identifier
        if path.path.len() == 1 {
            if let Some(name) = seg_name(&path.path[0]) {
                let name = &name;

                if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_identifier_value(name)
                {
                    let target = resolved.target.clone();
                    let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                    if let Some(target) = target.as_ref() {
                        self.record_var_reference(span.clone(), target);
                    }
                    let kind = if let Some(target) = target {
                        HirExprKind::ResolvedVar(HirVarRef {
                            name: resolved.name,
                            target,
                        })
                    } else {
                        HirExprKind::Var(resolved.name)
                    };
                    return HirExpr { ty, kind, span };
                }

                // Check if it's an enum variant (no args)
                let enum_entries: Vec<_> = self
                    .items
                    .enumerations()
                    .map(|(id, enum_info)| {
                        (
                            self.canonical_name_for_def_id(id)
                                .map(str::to_string)
                                .unwrap_or_else(|| enum_info.name.clone()),
                            enum_info.clone(),
                        )
                    })
                    .collect();
                for (enum_name, enum_info) in &enum_entries {
                    for variant in &enum_info.variants {
                        if variant.name == *name {
                            if matches!(variant.fields, HirVariantFields::Unit) {
                                self.record_source_reference(
                                    span.clone(),
                                    crate::source_map::SourceSymbol::Variant {
                                        owner: enum_info.id,
                                        variant: variant.id,
                                    },
                                );
                                // For generic enums, create fresh type vars
                                let type_args = if !enum_info.generic_params.is_empty() {
                                    enum_info
                                        .generic_params
                                        .iter()
                                        .map(|param| {
                                            self.engine.fresh_type_var_at_kind(
                                                span.clone(),
                                                param.kind.clone(),
                                            )
                                        })
                                        .collect()
                                } else {
                                    vec![]
                                };
                                return HirExpr {
                                    ty: Type::Enum {
                                        id: enum_info.id,
                                        args: type_args,
                                    },
                                    kind: HirExprKind::EnumVariant(
                                        enum_name.to_string(),
                                        variant.name.clone(),
                                        vec![],
                                        Some(HirVariantLocation {
                                            owner: enum_info.id,
                                            variant_id: variant.id,
                                            name: variant.name.clone(),
                                        }),
                                    ),
                                    span,
                                };
                            }
                        }
                    }
                }

                // Check if it's a struct type name used as a value (e.g. String.from_str)
                // Struct names are not valid standalone values — use `Type::method` syntax
                if crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_struct_type(name)
                    .is_some()
                {
                    self.diagnostics.push_with_span(
                        format!("Type '{}' cannot be used as a value. Use '{}::method' syntax to call associated functions", name, name),
                        span.clone(),
                    );
                    return HirExpr {
                        ty: Type::Error,
                        kind: HirExprKind::Var(name.clone()),
                        span: span.clone(),
                    };
                }

                // Unknown variable - emit an error and use Error type to stop cascade errors
                self.diagnostics
                    .push_with_span(format!("Unknown variable: {}", name), span.clone());
                return HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Var(name.clone()),
                    span,
                };
            }
        }

        // Multi-segment path (e.g., Crate::func, Module::func, or Enum::Variant)
        let names = path_names(&path.path);

        if names.len() == 3 {
            match self.resolve_static_qualified_bound_method_path(
                &names[0],
                &names[1],
                &names[2],
                span.clone(),
            ) {
                Ok(Some((ty, target))) => {
                    let Some(method_id) = target.method.method_id() else {
                        self.diagnostics.push_with_span(
                            "selected qualified constructor bound has no member identity"
                                .to_string(),
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    };
                    let callee = HirExpr {
                        ty,
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: names.join("::"),
                            target: HirVarTarget::Function(method_id),
                        }),
                        span: span.clone(),
                    };
                    self.record_source_reference(
                        span.clone(),
                        crate::source_map::SourceSymbol::Definition(method_id),
                    );
                    return self.static_method_value_lambda(callee, target);
                }
                Ok(None) => {}
                Err(message) if self.current_generic_param_id_for_name(&names[0]).is_some() => {
                    self.diagnostics.push_with_span(message, span.clone());
                    return self.error_expression_at(span.clone());
                }
                Err(_) => {}
            }

            if let Some(target_ty) = self.constructor_target_for_path_segment(&path.path[0]) {
                if let Some(trait_id) = self.trait_by_name(&names[1]).map(|trait_def| trait_def.id)
                {
                    match self.select_constructor_static_member(
                        &target_ty,
                        Some(trait_id),
                        &names[2],
                        span.clone(),
                    ) {
                        Ok(Some(selected)) => {
                            match self.instantiate_selected_constructor_member(
                                target_ty,
                                selected,
                                names.join("::"),
                                span.clone(),
                            ) {
                                Ok((callee, target)) => {
                                    return self.static_method_value_lambda(callee, target)
                                }
                                Err(message) => {
                                    self.diagnostics.push_with_span(message, span.clone());
                                    return self.error_expression_at(span.clone());
                                }
                            }
                        }
                        Ok(None) => {}
                        Err(message) => {
                            self.diagnostics.push_with_span(message, span.clone());
                            return self.error_expression_at(span.clone());
                        }
                    }
                }
            }
        }

        if names.len() == 2 {
            // Check for Enum::Variant first
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_enum_variant_path(&names)
            {
                let enum_info = resolved.owner;
                let variant = resolved.variant;
                self.record_source_reference(
                    span.clone(),
                    crate::source_map::SourceSymbol::Variant {
                        owner: enum_info.id,
                        variant: variant.id,
                    },
                );
                // For generic enums, create fresh type vars for type params
                let type_args = if !enum_info.generic_params.is_empty() {
                    enum_info
                        .generic_params
                        .iter()
                        .map(|param| {
                            self.engine
                                .fresh_type_var_at_kind(span.clone(), param.kind.clone())
                        })
                        .collect()
                } else {
                    vec![]
                };
                let ty = Type::Enum {
                    id: enum_info.id,
                    args: type_args,
                };
                return HirExpr {
                    ty,
                    kind: HirExprKind::EnumVariant(
                        resolved.owner_name,
                        variant.name.clone(),
                        vec![],
                        Some(HirVariantLocation {
                            owner: enum_info.id,
                            variant_id: variant.id,
                            name: variant.name.clone(),
                        }),
                    ),
                    span,
                };
            }

            match self.resolve_static_bound_method_path(&names[0], &names[1], span.clone()) {
                Ok(Some((ty, target))) => {
                    let Some(method_id) = target.method.method_id() else {
                        self.diagnostics.push_with_span(
                            "selected static bound method has no member identity".to_string(),
                            span.clone(),
                        );
                        return self.error_expression_at(span.clone());
                    };
                    let callee = HirExpr {
                        ty,
                        kind: HirExprKind::ResolvedVar(HirVarRef {
                            name: names.join("::"),
                            target: HirVarTarget::Function(method_id),
                        }),
                        span: span.clone(),
                    };
                    self.record_source_reference(
                        span.clone(),
                        crate::source_map::SourceSymbol::Definition(method_id),
                    );
                    return self.static_method_value_lambda(callee, target);
                }
                Ok(None) => {}
                Err(message) => {
                    self.diagnostics
                        .push_selection_with_span(message, span.clone());
                    return self.error_expression_at(span.clone());
                }
            }

            // Inherent associated functions retain precedence over trait members.
            let mut static_method_error = None;
            match crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_static_method_path(&names)
            {
                Ok(Some(resolved)) => match self
                    .instantiate_static_method_value(resolved, span.clone())
                {
                    Ok((callee, target)) => return self.static_method_value_lambda(callee, target),
                    Err(message) => {
                        self.diagnostics
                            .push_selection_with_span(message, span.clone());
                        return self.error_expression_at(span.clone());
                    }
                },
                Ok(None) => {}
                Err(message) => {
                    static_method_error = Some(message);
                }
            }

            if let Some(target_ty) = self.constructor_target_for_path_segment(&path.path[0]) {
                match self.select_constructor_static_member(
                    &target_ty,
                    None,
                    &names[1],
                    span.clone(),
                ) {
                    Ok(Some(selected)) => {
                        match self.instantiate_selected_constructor_member(
                            target_ty,
                            selected,
                            names.join("::"),
                            span.clone(),
                        ) {
                            Ok((callee, target)) => {
                                return self.static_method_value_lambda(callee, target)
                            }
                            Err(message) => {
                                self.diagnostics
                                    .push_selection_with_span(message, span.clone());
                                return self.error_expression_at(span.clone());
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(message) => {
                        self.diagnostics
                            .push_selection_with_span(message, span.clone());
                        return self.error_expression_at(span.clone());
                    }
                }
            }

            if let Some(message) = static_method_error {
                self.diagnostics.push_with_span(message, span.clone());
                return self.error_expression_at(span.clone());
            }

            // Check for Crate::Function or Module::Function
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_qualified_value_path(&names)
            {
                let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                if let Some(target) = resolved.target.as_ref() {
                    self.record_var_reference(span.clone(), target);
                }
                let kind = if let Some(target) = resolved.target {
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: resolved.name,
                        target,
                    })
                } else {
                    HirExprKind::Var(resolved.name)
                };
                return HirExpr { ty, kind, span };
            }
        }

        // Handle longer paths (e.g., crate::module::function)
        if names.len() > 2 {
            if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
                .resolve_qualified_value_path(&names)
            {
                let ty = self.instantiate_resolved_value_type(&resolved, span.clone());
                if let Some(target) = resolved.target.as_ref() {
                    self.record_var_reference(span.clone(), target);
                }
                let kind = if let Some(target) = resolved.target {
                    HirExprKind::ResolvedVar(HirVarRef {
                        name: resolved.name,
                        target,
                    })
                } else {
                    HirExprKind::Var(resolved.name)
                };
                return HirExpr { ty, kind, span };
            }
        }

        // Check if the first segment looks like an unexported struct/type
        // (exists under a qualified name but not under the short name)
        if !names.is_empty() {
            let type_name = &names[0];
            if crate::lower::resolution::LowerResolutionContext::new(self)
                .struct_segment_looks_unexported(type_name)
            {
                self.diagnostics.push_with_span(
                    format!("Type '{}' is not publicly exported", type_name),
                    span.clone(),
                );
                return HirExpr {
                    ty: Type::Error,
                    kind: HirExprKind::Var(names.join("::")),
                    span,
                };
            }
        }

        // Fallback: use the full path as a name
        let full_name = names.join("::");
        let ty = self.engine.fresh_type_var_at(span.clone());
        HirExpr {
            ty,
            kind: HirExprKind::Var(full_name),
            span,
        }
    }

    pub(crate) fn lower_instance(&mut self, inst: &ast::Instance) -> HirExpr {
        let span = Self::full_path_span(&inst.name.path);
        if matches!(
            inst.name.path.as_slice(),
            [ast::IdentOrType::Type(ast::ParseType::Unit(_))]
        ) && inst.fields.is_empty()
        {
            return HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Unit,
                span,
            };
        }
        let segments = path_names(&inst.name.path);

        let struct_resolution = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_struct_literal_path(&segments);
        let type_name = struct_resolution.name;

        // Check for Enum::Variant pattern (2-segment path where first is an enum)
        if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_enum_variant_path(&segments)
        {
            let enum_info = resolved.owner;
            let variant = resolved.variant;
            self.record_source_reference(
                span.clone(),
                crate::source_map::SourceSymbol::Variant {
                    owner: enum_info.id,
                    variant: variant.id,
                },
            );
            let args: Vec<HirExpr> = match &variant.fields {
                HirVariantFields::Named(fields) => fields
                    .iter()
                    .filter_map(|field| {
                        inst.fields
                            .iter()
                            .find(|(ident, _)| ident.name == field.name)
                            .map(|(_, expr)| self.lower_expression(expr))
                    })
                    .collect(),
                _ => inst
                    .fields
                    .values()
                    .map(|expr| self.lower_expression(expr))
                    .collect(),
            };

            // Handle generic enum type params
            let type_args = if !enum_info.generic_params.is_empty() {
                // Create fresh type vars for each generic param
                let mut type_var_mapping: HashMap<GenericParamId, Type> = HashMap::new();
                for param in &enum_info.generic_params {
                    let tv = self.engine.fresh_type_var_at(span.clone());
                    type_var_mapping.insert(param.id, tv);
                }
                // Unify type vars with arg types based on variant field types
                match &variant.fields {
                    HirVariantFields::Positional(field_types) => {
                        for (arg, field_ty) in args.iter().zip(field_types.iter()) {
                            let expected = field_ty.substitute_generics(&type_var_mapping);
                            let _ = self.engine.unify(&arg.ty, &expected);
                        }
                    }
                    HirVariantFields::Named(fields) => {
                        for (arg, field) in args.iter().zip(fields.iter()) {
                            let expected = field.ty.substitute_generics(&type_var_mapping);
                            let _ = self.engine.unify(&arg.ty, &expected);
                        }
                    }
                    HirVariantFields::Unit => {}
                }
                // Resolve type vars to concrete types
                enum_info
                    .generic_params
                    .iter()
                    .map(|param| {
                        if let Some(tv) = type_var_mapping.get(&param.id) {
                            self.engine.resolve(tv)
                        } else {
                            Type::Error
                        }
                    })
                    .collect()
            } else {
                vec![]
            };

            return HirExpr {
                ty: Type::Enum {
                    id: enum_info.id,
                    args: type_args,
                },
                kind: HirExprKind::EnumVariant(
                    resolved.owner_name,
                    variant.name.clone(),
                    args,
                    Some(HirVariantLocation {
                        owner: enum_info.id,
                        variant_id: variant.id,
                        name: variant.name.clone(),
                    }),
                ),
                span,
            };
        }

        // Check if this is a known struct
        if let Some(hir_struct) = struct_resolution.structure {
            let current_impl_matches = self.current_struct_impl.as_deref()
                == Some(type_name.as_str())
                || self.current_struct_impl.as_ref().is_some_and(|name| {
                    crate::lower::resolution::LowerResolutionContext::new(self)
                        .resolve_item_id(name)
                        .is_some_and(|id| id == hir_struct.id)
                });

            if !current_impl_matches && hir_struct.fields.iter().any(|field| !field.public) {
                self.diagnostics.push_with_span(
                    format!(
                        "Cannot construct struct '{}' because it has private fields",
                        type_name
                    ),
                    span.clone(),
                );
            }

            // For generic structs, create type variables for each generic parameter
            // and infer them from field values
            let generic_params = hir_struct.generic_params.clone();

            // Create a mapping from generic param names to fresh type variables
            let mut type_var_mapping: HashMap<GenericParamId, Type> = HashMap::new();
            for param in &generic_params {
                let tv = self.engine.fresh_type_var_at(span.clone());
                type_var_mapping.insert(param.id, tv);
            }

            let mut fields = Vec::new();
            for (ident, expr) in &inst.fields {
                let mut hir_expr = self.lower_expression(expr);

                // Find the expected field type and substitute generic params
                if let Some(field_info) = hir_struct.fields.iter().find(|f| f.name == ident.name) {
                    let expected_ty = field_info.ty.substitute_generics(&type_var_mapping);
                    hir_expr = self.coerce_argument_to_expected(hir_expr, &expected_ty);
                    // Unify expected type with actual type
                    let _ = self.engine.unify(&expected_ty, &hir_expr.ty);
                }

                let field = hir_struct
                    .fields
                    .iter()
                    .find(|f| f.name == ident.name)
                    .map(|f| HirFieldLocation {
                        owner: hir_struct.id,
                        field_id: f.id,
                        name: ident.name.clone(),
                    });
                if let Some(field) = field.as_ref() {
                    self.record_source_reference(
                        ident.span.clone(),
                        crate::source_map::SourceSymbol::Field {
                            owner: field.owner,
                            field: field.field_id,
                        },
                    );
                }
                fields.push(HirStructLiteralField {
                    name: ident.name.clone(),
                    value: hir_expr,
                    field,
                });
            }

            // Resolve the type variables to get actual type arguments
            let type_args: Vec<Type> = generic_params
                .iter()
                .map(|param| {
                    if let Some(tv) = type_var_mapping.get(&param.id) {
                        self.engine.resolve(tv)
                    } else {
                        Type::Error
                    }
                })
                .collect();
            HirExpr {
                ty: Type::Struct {
                    id: hir_struct.id,
                    args: type_args,
                },
                kind: HirExprKind::StructLiteral(type_name, Some(hir_struct.id), fields),
                span,
            }
        } else {
            // Unknown struct - lower fields and use a type variable
            let mut fields = Vec::new();
            for (ident, expr) in &inst.fields {
                let hir_expr = self.lower_expression(expr);
                fields.push(HirStructLiteralField {
                    name: ident.name.clone(),
                    value: hir_expr,
                    field: None,
                });
            }

            // Might be an enum variant with named fields
            let ty = self.engine.fresh_type_var_at(span.clone());
            HirExpr {
                ty,
                kind: HirExprKind::StructLiteral(type_name, None, fields),
                span,
            }
        }
    }

    pub(crate) fn lower_lambda(&mut self, lambda: &ast::LambdaDecl) -> HirExpr {
        self.lower_lambda_with_expected_params(lambda, None)
    }

    pub(crate) fn lower_lambda_with_expected_params(
        &mut self,
        lambda: &ast::LambdaDecl,
        expected_params: Option<&[Type]>,
    ) -> HirExpr {
        if matches!(lambda.arrow_kind, ast::LambdaArrowKind::Curried) && lambda.parameters.len() > 1
        {
            return self.lower_curried_lambda(lambda, &lambda.span);
        }

        let mut params = Vec::new();
        let mut param_types = Vec::new();

        self.push_scope();

        for (index, param_pat) in lambda.parameters.iter().enumerate() {
            let (name, inner_ty, mutable, is_ref) =
                self.lower_param_pattern_at(param_pat, lambda.span.clone());
            let inferred_ty = if is_ref {
                Type::Reference {
                    mutable,
                    inner: Box::new(inner_ty),
                }
            } else {
                inner_ty
            };
            let ty = expected_params
                .and_then(|params| params.get(index))
                .cloned()
                .unwrap_or(inferred_ty);
            let local_id = self.fresh_local_id();
            self.scope
                .define_local(name.clone(), ty.clone(), mutable, local_id);
            if let Some(owner) = self.current_body_def_id() {
                let span =
                    Self::pattern_binding_span(param_pat).unwrap_or_else(|| lambda.span.clone());
                self.source_map.insert_local_in_scope(
                    owner,
                    local_id,
                    span,
                    self.source_scope_stack.last().copied(),
                );
            }
            param_types.push(ty.clone());
            params.push(HirParam {
                name,
                local_id,
                ty,
                mutable,
                is_ref,
            });
        }

        let lambda_return_ty = self.engine.fresh_type_var_at(lambda.span.clone());
        let body = self.with_body_return_type(lambda_return_ty.clone(), |lowerer| {
            lowerer.lower_lambda_body(lambda)
        });
        let span = lambda.span.clone();
        let _ = self.engine.unify(&body.ty, &lambda_return_ty);
        let ret_type = self.engine.resolve(&lambda_return_ty);
        let captures = self.collect_lambda_captures(&body, &params);

        self.pop_scope();

        let func_type =
            Self::lambda_function_type(param_types, ret_type, FunctionSafety::Safe, &captures);

        HirExpr {
            ty: func_type,
            kind: HirExprKind::Lambda {
                params,
                body,
                captures,
            },
            span,
        }
    }

    pub(crate) fn lower_curried_lambda(
        &mut self,
        lambda: &ast::LambdaDecl,
        span: &crate::lexer::Span,
    ) -> HirExpr {
        self.lower_curried_lambda_stage(&lambda.parameters, &lambda.body, span)
    }

    pub(crate) fn lower_curried_lambda_stage(
        &mut self,
        params: &[ast::Pattern],
        body: &ast::Block,
        span: &crate::lexer::Span,
    ) -> HirExpr {
        self.lower_curried_lambda_stage_with_expected_params(params, body, span, None)
    }

    pub(crate) fn lower_curried_lambda_stage_with_expected_params(
        &mut self,
        params: &[ast::Pattern],
        body: &ast::Block,
        span: &crate::lexer::Span,
        expected_params: Option<&[Type]>,
    ) -> HirExpr {
        self.push_scope();

        let (name, inferred_ty, mutable, is_ref) =
            self.lower_param_pattern_at(&params[0], span.clone());
        let ty = expected_params
            .and_then(|params| params.first())
            .cloned()
            .unwrap_or(inferred_ty);
        let local_id = self.fresh_local_id();
        self.scope
            .define_local(name.clone(), ty.clone(), mutable, local_id);
        if let Some(owner) = self.current_body_def_id() {
            let param_span = Self::pattern_binding_span(&params[0]).unwrap_or_else(|| span.clone());
            self.source_map.insert_local_in_scope(
                owner,
                local_id,
                param_span,
                self.source_scope_stack.last().copied(),
            );
        }
        let param = HirParam {
            name,
            local_id,
            ty: ty.clone(),
            mutable,
            is_ref,
        };

        let lambda_return_ty = self.engine.fresh_type_var_at(span.clone());
        let body = if params.len() == 1 {
            self.with_body_return_type(lambda_return_ty.clone(), |lowerer| {
                lowerer.lower_block(body)
            })
        } else {
            let inner = self.lower_curried_lambda_stage_with_expected_params(
                &params[1..],
                body,
                span,
                expected_params.and_then(|params| params.get(1..)),
            );
            HirBlock {
                ty: inner.ty.clone(),
                stmts: vec![HirStmt::Expr(inner)],
            }
        };
        let _ = self.engine.unify(&body.ty, &lambda_return_ty);
        let body_ty = self.engine.resolve(&lambda_return_ty);
        let mut captures = self.collect_lambda_captures(&body, std::slice::from_ref(&param));
        for capture in &mut captures {
            capture.kind = HirClosureCaptureKind::Move;
        }

        self.pop_scope();

        HirExpr {
            ty: Self::lambda_function_type(vec![ty], body_ty, FunctionSafety::Safe, &captures),
            kind: HirExprKind::Lambda {
                params: vec![param],
                body,
                captures,
            },
            span: span.clone(),
        }
    }

    pub(crate) fn lower_tuple(&mut self, tuple: &ast::Tuple) -> HirExpr {
        let elements: Vec<HirExpr> = tuple
            .elements
            .iter()
            .map(|e| self.lower_expression(e))
            .collect();
        let span = elements
            .first()
            .map(|element| element.span.clone())
            .expect("tuple requires a source element span");
        let types: Vec<Type> = elements.iter().map(|e| e.ty.clone()).collect();
        HirExpr {
            ty: Type::Tuple(types),
            kind: HirExprKind::TupleLiteral(elements),
            span,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    use crate::ast::{
        Block, Expression, Ident, IdentOrType, IdentPattern, IdentifierPath, LambdaArrowKind,
        LambdaDecl, Operand, Pattern, PatternKind, PrimaryExpr, Statement, UnaryExpr,
    };
    use crate::crate_system::CrateContext;
    use crate::ids::{CrateId, DefId, FieldId, HirLocalId, LocalDefId, VariantId};
    use crate::infer::ResolvedHirProgram;
    use crate::lower::body_context::{BodyLoweringContext, BodyOwner};
    use crate::types::{
        CallableKind, CaptureKind, GenericParamDecl, GenericParamId, Predicate, TraitBound, Type,
    };
    use crate::{collect, infer, lower, macro_expansion, parser, Config};

    static LOWER_PATH_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn compile_source_to_resolved_hir_for_test(source: &str) -> ResolvedHirProgram {
        let id = LOWER_PATH_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rock_lower_path_hir_ref_test_{}_{}",
            std::process::id(),
            id
        ));
        let entry_file = dir.join("main.rk");

        let config = Config {
            entry_file: entry_file.clone(),
            output_dir: dir,
            debug_print: Vec::new(),
            meta_files: Vec::new(),
            extern_artifacts: Vec::new(),
            source_providers: Vec::new(),
            current_crate_name: Some("test".to_string()),
            opt_level: 0,
            emit_llvm: false,
            no_link: true,
            emit_object: None,
            no_prelude: true,
            no_std: true,
            sysroot: None,
        };

        let ast = parser::parse_source(entry_file, source, &config)
            .map(|module| crate::ast::Program { module })
            .expect("test source parses");
        let macro_context = macro_expansion::MacroExpansionContext::new(&config);
        let ast = macro_expansion::expand_macros_with_context(ast, &macro_context)
            .expect("test macros expand");
        let crate_ctx = CrateContext::new();
        let decls = collect::collect(&ast, &crate_ctx, false, Some("test"))
            .expect("test declarations collect");
        let partial =
            lower::program::lower_from_declarations(&ast, decls, &crate_ctx, Some("test"))
                .expect("test source lowers");
        infer::finalize(partial).expect("test HIR finalizes")
    }

    fn find_first_struct_literal(
        block: &crate::hir::AcceptedHirBlock,
    ) -> Option<(
        &String,
        &Option<crate::ids::DefId>,
        &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
    )> {
        block.stmts.iter().find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
                find_first_struct_literal_expr(expr)
            }
            crate::hir::HirStmtFor::Let { value, .. } => find_first_struct_literal_expr(value),
            _ => None,
        })
    }

    fn find_first_struct_literal_expr(
        expr: &crate::hir::AcceptedHirExpr,
    ) -> Option<(
        &String,
        &Option<crate::ids::DefId>,
        &Vec<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>>,
    )> {
        match &expr.kind {
            crate::hir::HirExprKindFor::StructLiteral(name, id, fields) => Some((name, id, fields)),
            crate::hir::HirExprKindFor::Call(callee, args, _) => {
                find_first_struct_literal_expr(callee)
                    .or_else(|| args.iter().find_map(find_first_struct_literal_expr))
            }
            crate::hir::HirExprKindFor::Block(block)
            | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_struct_literal(block),
            _ => None,
        }
    }

    fn find_static_method_target(
        expr: &crate::hir::AcceptedHirExpr,
    ) -> Option<&HirStaticMethodTarget> {
        match &expr.kind {
            crate::hir::HirExprKindFor::Call(
                _,
                _,
                Some(crate::hir::HirCallTarget::StaticMethod(target)),
            ) => Some(target),
            crate::hir::HirExprKindFor::Call(callee, args, _) => find_static_method_target(callee)
                .or_else(|| args.iter().find_map(find_static_method_target)),
            crate::hir::HirExprKindFor::Lambda { body, .. }
            | crate::hir::HirExprKindFor::Block(body)
            | crate::hir::HirExprKindFor::UnsafeBlock(body) => {
                body.stmts.iter().find_map(|stmt| match stmt {
                    crate::hir::HirStmtFor::Let { value, .. }
                    | crate::hir::HirStmtFor::Expr(value)
                    | crate::hir::HirStmtFor::Return(Some(value))
                    | crate::hir::HirStmtFor::Break(Some(value)) => {
                        find_static_method_target(value)
                    }
                    _ => None,
                })
            }
            _ => None,
        }
    }

    fn find_first_enum_variant(
        block: &crate::hir::AcceptedHirBlock,
    ) -> Option<(
        &String,
        &String,
        &Vec<crate::hir::AcceptedHirExpr>,
        &Option<crate::hir::HirVariantLocation>,
    )> {
        block.stmts.iter().find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
                find_first_enum_variant_expr(expr)
            }
            crate::hir::HirStmtFor::Let { value, .. } => find_first_enum_variant_expr(value),
            _ => None,
        })
    }

    fn find_first_enum_variant_expr(
        expr: &crate::hir::AcceptedHirExpr,
    ) -> Option<(
        &String,
        &String,
        &Vec<crate::hir::AcceptedHirExpr>,
        &Option<crate::hir::HirVariantLocation>,
    )> {
        match &expr.kind {
            crate::hir::HirExprKindFor::EnumVariant(enum_name, variant_name, args, location) => {
                Some((enum_name, variant_name, args, location))
            }
            crate::hir::HirExprKindFor::Call(callee, args, _) => {
                find_first_enum_variant_expr(callee)
                    .or_else(|| args.iter().find_map(find_first_enum_variant_expr))
            }
            crate::hir::HirExprKindFor::Block(block)
            | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_enum_variant(block),
            _ => None,
        }
    }

    fn find_first_resolved_var(block: &crate::hir::AcceptedHirBlock) -> Option<&HirVarRef> {
        block.stmts.iter().find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
                find_first_resolved_var_expr(expr)
            }
            _ => None,
        })
    }

    fn find_first_resolved_var_expr(expr: &crate::hir::AcceptedHirExpr) -> Option<&HirVarRef> {
        match &expr.kind {
            crate::hir::HirExprKindFor::ResolvedVar(reference) => Some(reference),
            crate::hir::HirExprKindFor::Call(callee, args, _) => {
                find_first_resolved_var_expr(callee)
                    .or_else(|| args.iter().find_map(find_first_resolved_var_expr))
            }
            crate::hir::HirExprKindFor::Block(block)
            | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_resolved_var(block),
            _ => None,
        }
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn static_method_value_lambda_carries_exact_target() {
        let resolved = compile_source_to_resolved_hir_for_test(
            "struct Math\n\nimpl Math\n    double = value -> value\n\nmain = ->\n    operation = Math::double\n    operation 6\n",
        );
        let program = resolved.program.program();
        let imp = program
            .impls
            .values()
            .find(|imp| imp.methods.contains_key("double"))
            .expect("static impl");
        let method_id = imp.methods["double"].id;
        let (_, main) = program.test_function_by_name("main").expect("main");
        let value = main
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                HirStmtFor::Let { value, .. } => Some(value),
                _ => None,
            })
            .expect("static method value binding");
        let HirExprKindFor::Lambda { body, .. } = &value.kind else {
            panic!("expected static method value lambda, got {:?}", value.kind);
        };
        let [HirStmtFor::Expr(HirExprFor {
            kind: HirExprKindFor::Call(_, _, Some(HirCallTarget::StaticMethod(target))),
            ..
        })] = body.stmts.as_slice()
        else {
            panic!("expected exact static call in generated lambda")
        };

        assert_eq!(target.method.impl_id(), Some(imp.id));
        assert_eq!(target.method.method_id(), Some(method_id));
    }

    #[test]
    fn generic_box_static_constructor_value_lowers() {
        let resolved = compile_source_to_resolved_hir_for_test(
            r#"
struct Box T
    value: T

impl Box T
    new = value ->
        Box
            value: value

main = ->
    Box::new 21
"#,
        );

        assert!(resolved.program.test_function_by_name("main").is_some());
    }

    fn resolved_generic_static_method(
        _lowerer: &mut Lowerer,
        impl_id: DefId,
        method_id: DefId,
        struct_id: DefId,
        owner_param: GenericParamId,
        method_params: Vec<GenericParamId>,
        params: Vec<Type>,
        ret: Type,
    ) -> crate::lower::resolution::LowerResolvedStaticMethod {
        let mut method = test_function(method_id, "new");
        method.generic_params = method_params
            .iter()
            .enumerate()
            .map(|(index, param)| GenericParamDecl::type_param(*param, format!("Method{index}")))
            .collect();
        method.params = params
            .iter()
            .enumerate()
            .map(|(index, ty)| HirParam {
                name: format!("arg{index}"),
                local_id: HirLocalId(index as u32),
                ty: ty.clone(),
                mutable: false,
                is_ref: false,
            })
            .collect();
        method.ret_type = ret.clone();
        crate::lower::resolution::LowerResolvedStaticMethod {
            value: crate::lower::resolution::LowerResolvedValue {
                name: "Box::new".to_string(),
                ty: Type::function(params, ret),
                target: Some(HirVarTarget::Function(method_id)),
                is_alias: false,
                scope_index: None,
                should_instantiate: true,
            },
            target: HirMethodCallTarget::impl_method(impl_id, method_id, None),
            receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(owner_param)],
            }),
            owner_generic_params: vec![owner_param],
            method_generic_params: method_params,
        }
    }

    #[test]
    fn trait_backed_structural_static_owner_substitutes_trait_args_for_artifact_validation() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(160);
        let trait_id = def_id(159);
        let trait_member_id = def_id(1600);
        let impl_id = def_id(161);
        let method_id = def_id(162);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let mut resolved = resolved_generic_static_method(
            &mut lowerer,
            impl_id,
            method_id,
            struct_id,
            owner_param,
            vec![method_param],
            vec![Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(method_param)],
            }],
            Type::Unit,
        );
        resolved.target = HirMethodCallTarget::impl_method(
            impl_id,
            method_id,
            Some(HirSelectedTraitMember {
                trait_id,
                member_id: trait_member_id,
                trait_args: vec![Type::Generic(owner_param)],
            }),
        );

        let (_, target) = lowerer
            .with_test_body_context(|lowerer| {
                lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
            })
            .expect("structural owner inference");

        assert_eq!(target.method.owner_substitution[0].param, owner_param);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert_eq!(
            target.method.owner_substitution[0].ty,
            target.method.method_substitution[0].ty
        );
        assert!(matches!(
            target.method.owner_substitution[0].ty,
            Type::TypeVar(_)
        ));
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected_trait),
            ..
        } = target.method.target
        else {
            panic!("expected trait-backed static method target")
        };
        assert_eq!(
            selected_trait.trait_args,
            vec![target.method.owner_substitution[0].ty.clone()]
        );
    }

    #[test]
    fn structural_static_owner_fallback_preserves_nested_method_generic_shape() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(163);
        let impl_id = def_id(164);
        let method_id = def_id(165);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let resolved = resolved_generic_static_method(
            &mut lowerer,
            impl_id,
            method_id,
            struct_id,
            owner_param,
            vec![method_param],
            vec![Type::Struct {
                id: struct_id,
                args: vec![Type::Tuple(vec![Type::Generic(method_param)])],
            }],
            Type::Unit,
        );

        let (_, target) = lowerer
            .with_test_body_context(|lowerer| {
                lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
            })
            .expect("nested structural owner inference");

        assert_eq!(target.method.owner_substitution[0].param, owner_param);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert_eq!(
            target.method.owner_substitution[0].ty,
            Type::Tuple(vec![target.method.method_substitution[0].ty.clone()])
        );
    }

    #[test]
    fn structural_static_owner_fallback_visits_projection_nominal_types() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(169);
        let impl_id = def_id(170);
        let method_id = def_id(171);
        let trait_id = def_id(172);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let resolved = resolved_generic_static_method(
            &mut lowerer,
            impl_id,
            method_id,
            struct_id,
            owner_param,
            vec![method_param],
            vec![Type::Projection {
                ty: Box::new(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(method_param)],
                }),
                trait_id,
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: trait_id,
                    assoc_type_id: crate::ids::AssocTypeId(0),
                },
                trait_args: Vec::new(),
            }],
            Type::Unit,
        );

        let (_, target) = lowerer
            .with_test_body_context(|lowerer| {
                lowerer.instantiate_static_method_value(resolved, crate::lexer::Span::test())
            })
            .expect("projection structural owner inference");

        assert_eq!(
            target.method.owner_substitution[0].ty,
            target.method.method_substitution[0].ty
        );
        assert!(matches!(
            target.method.owner_substitution[0].ty,
            Type::TypeVar(_)
        ));
    }

    #[test]
    fn conflicting_structural_static_owner_occurrences_are_an_error() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(166);
        let impl_id = def_id(167);
        let method_id = def_id(168);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let left_method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let right_method_param = GenericParamId {
            owner: method_id,
            index: 1,
        };
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "Box"));
        register_item_path(&mut lowerer, "Box", struct_id);
        let mut method = test_function(method_id, "new");
        method.generic_params = vec![
            GenericParamDecl::type_param(left_method_param, "Left"),
            GenericParamDecl::type_param(right_method_param, "Right"),
        ];
        method.params = vec![
            HirParam {
                name: "left".to_string(),
                local_id: HirLocalId(0),
                ty: Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(left_method_param)],
                },
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "right".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(right_method_param)],
                },
                mutable: false,
                is_ref: false,
            },
        ];
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Box".to_string()),
                type_name: "Box".to_string(),
                type_generics: vec![GenericParamDecl::type_param(owner_param, "Owner")],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(owner_param)],
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("new".to_string(), method)]),
            })
            .unwrap();

        let expr = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_identifier_path(&identifier_path(&["Box", "new"]))
        });

        assert_eq!(expr.ty, Type::Error);
        assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
        let diagnostic = lowerer
            .diagnostics
            .errors()
            .iter()
            .find(|error| error.message.contains("conflicting owner inference"))
            .expect("conflicting owner inference diagnostic");
        assert!(diagnostic.message.contains("_ vs _"));
        assert!(!diagnostic.message.contains("GenericParamId"));
    }

    fn register_item_path(lowerer: &mut Lowerer, path: &str, id: DefId) {
        lowerer.resolver.item_paths.insert(path.to_string(), id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(id, path.to_string());
    }

    fn static_method_value_parts(expr: &HirExpr) -> (&HirVarRef, &HirStaticMethodTarget) {
        let HirExprKind::Lambda { body, .. } = &expr.kind else {
            panic!("expected static method value lambda, got {:?}", expr.kind);
        };
        let [HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(callee, _, Some(HirCallTarget::StaticMethod(static_target))),
            ..
        })] = body.stmts.as_slice()
        else {
            panic!("expected exact static call in generated lambda")
        };
        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            panic!("expected resolved static callee")
        };
        (reference, static_target)
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: crate::lexer::Span::test(),
        }
    }

    fn ident_pattern(name: &str) -> Pattern {
        Pattern {
            binding: Some(ident(name)),
            kind: PatternKind::Ident(IdentPattern {
                name: ident(name),
                mut_: false,
            }),
        }
    }

    fn var_expr(name: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(ident(name))],
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn int_expr(value: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: Operand::Literal(crate::ast::Literal {
                kind: crate::ast::LiteralKind::Number(value.parse().unwrap()),
                span: crate::lexer::Span::test(),
            }),
            secondaries: None,
            type_annotation: None,
        }))
    }

    fn identifier_path(names: &[&str]) -> IdentifierPath {
        IdentifierPath {
            path: names
                .iter()
                .map(|name| IdentOrType::Ident(ident(name)))
                .collect(),
        }
    }

    fn test_function(id: DefId, name: &str) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
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

    fn register_static_impl(
        lowerer: &mut Lowerer,
        impl_id: DefId,
        owner_id: DefId,
        owner_name: &str,
        method_name: &str,
        method: HirFunction,
    ) {
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named(owner_name.to_string()),
                type_name: owner_name.to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: owner_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([(method_name.to_string(), method)]),
            })
            .unwrap();
    }

    #[test]
    fn local_reads_resolve_to_shadowing_local_ids() {
        let mut lowerer = Lowerer::new_for_test();
        let (inner, expr) = lowerer.with_test_body_context(|lowerer| {
            let outer = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("x".to_string(), Type::I64, false, outer);
            lowerer.scope.push();
            let inner = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("x".to_string(), Type::Bool, false, inner);
            (
                inner,
                lowerer.lower_identifier_path(&identifier_path(&["x"])),
            )
        });

        match expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Local(id),
                name,
            }) => {
                assert_eq!(name, "x");
                assert_eq!(id, inner);
            }
            other => panic!("expected resolved local var, got {other:?}"),
        }
    }

    #[test]
    fn parameter_reads_resolve_to_parameter_local_ids() {
        let resolved = compile_source_to_resolved_hir_for_test(
            r#"
identity = x -> x
"#,
        );
        let (_, function) = resolved.program.test_function_by_name("identity").unwrap();
        let param_id = function.params[0].local_id;

        let var = find_first_resolved_var(&function.body).unwrap();
        assert_eq!(var.name, "x");
        assert_eq!(var.target, HirVarTarget::Local(param_id));
    }

    fn point_struct() -> HirStruct {
        HirStruct {
            id: def_id(10),
            name: "Point".to_string(),
            generic_params: Vec::new(),
            fields: vec![
                HirField {
                    id: FieldId(0),
                    name: "x".to_string(),
                    ty: Type::I64,
                    public: true,
                },
                HirField {
                    id: FieldId(1),
                    name: "y".to_string(),
                    ty: Type::I64,
                    public: true,
                },
            ],
        }
    }

    fn option_enum() -> HirEnum {
        HirEnum {
            id: def_id(20),
            name: "Maybe".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Some".to_string(),
                fields: HirVariantFields::Positional(vec![Type::I64]),
            }],
        }
    }

    fn record_enum() -> HirEnum {
        HirEnum {
            id: def_id(21),
            name: "Record".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Pair".to_string(),
                fields: HirVariantFields::Named(vec![
                    HirField {
                        id: FieldId(0),
                        name: "first".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                    HirField {
                        id: FieldId(1),
                        name: "second".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                    HirField {
                        id: FieldId(2),
                        name: "third".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                    HirField {
                        id: FieldId(3),
                        name: "fourth".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                ]),
            }],
        }
    }

    fn single_unit_variant_enum(id: DefId, name: &str, variant_name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: variant_name.to_string(),
                fields: HirVariantFields::Unit,
            }],
        }
    }

    fn empty_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn private_field_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: vec![HirField {
                id: FieldId(0),
                name: "ptr".to_string(),
                ty: Type::I64,
                public: false,
            }],
        }
    }

    #[test]
    fn lower_struct_literal_records_struct_id() {
        let source = r#"
struct Box
    < value: I64

main = ->
    box = Box
        value: 7
    box.value
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let (_, main) = resolved.program.test_function_by_name("main").unwrap();
        let struct_id = resolved.program.names.structs_by_name["Box"];

        let literal = find_first_struct_literal(&main.body).unwrap();
        assert_eq!(literal.1, &Some(struct_id));
    }

    #[test]
    fn lower_enum_variant_records_enum_and_variant_ids() {
        let source = r#"
enum Maybe
    Some I64
    None

main = ->
    Maybe::Some 7
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let (_, main) = resolved.program.test_function_by_name("main").unwrap();
        let (enum_id, maybe_enum) = resolved.program.test_enum_by_name("Maybe").unwrap();
        let expected_variant_id = maybe_enum
            .variants
            .iter()
            .find(|variant| variant.name == "Some")
            .expect("Maybe::Some exists")
            .id;

        let variant = find_first_enum_variant(&main.body).unwrap();
        let location = variant.3.as_ref().expect("enum variant must be resolved");
        assert_eq!(location.owner, enum_id);
        assert_eq!(location.variant_id, expected_variant_id);
        assert_eq!(location.name, "Some");
    }

    #[test]
    fn lower_direct_function_reference_records_function_id() {
        let source = r#"
answer = -> 7

main = ->
    answer!
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let (answer_id, _) = resolved.program.test_function_by_name("answer").unwrap();
        let (_, main) = resolved.program.test_function_by_name("main").unwrap();

        let var = find_first_resolved_var(&main.body).unwrap();
        assert_eq!(var.name, "answer");
        assert_eq!(var.target, HirVarTarget::Function(answer_id));
    }

    #[test]
    fn lower_direct_extern_reference_records_extern_id() {
        let source = r#"
extern puts: I32 -> I32

main = ->
    puts 0
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let extern_id = resolved.program.names.externs_by_name["puts"];
        let (_, main) = resolved.program.test_function_by_name("main").unwrap();

        let var = find_first_resolved_var(&main.body).unwrap();
        assert_eq!(var.name, "puts");
        assert_eq!(var.target, HirVarTarget::Extern(extern_id));
    }

    #[test]
    fn lower_qualified_function_reference_records_function_id() {
        let mut lowerer = Lowerer::new_for_test();
        let answer_id = def_id(30);
        lowerer
            .items
            .insert_function(test_function(answer_id, "answer"));
        register_item_path(&mut lowerer, "pkg::answer", answer_id);

        let expr = lowerer.lower_identifier_path(&identifier_path(&["pkg", "answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "pkg::answer");
                assert_eq!(reference.target, HirVarTarget::Function(answer_id));
            }
            other => panic!("expected resolved function reference, got {other:?}"),
        }
    }

    #[test]
    fn lower_static_impl_reference_records_method_id() {
        let mut lowerer = Lowerer::new_for_test();
        let box_id = def_id(40);
        lowerer.items.insert_structure(HirStruct {
            id: box_id,
            name: "Box".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        register_item_path(&mut lowerer, "Box", box_id);
        let method_id = def_id(41);
        register_static_impl(
            &mut lowerer,
            def_id(42),
            box_id,
            "Box",
            "make",
            test_function(method_id, "make"),
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["Box", "make"]));

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.name, "Box::make");
        assert_eq!(reference.target, HirVarTarget::Function(method_id));
        assert_eq!(target.method.impl_id(), Some(def_id(42)));
        assert_eq!(target.method.method_id(), Some(method_id));
    }

    #[test]
    fn constructor_trait_selection_concrete_call_records_exact_impl_authority() {
        let hir = compile_source_to_resolved_hir_for_test(
            r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
        );
        let main = hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .expect("main function");
        let value = main
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Let { value, .. } => Some(value),
                _ => None,
            })
            .expect("lowered constructor call");
        let crate::hir::HirExprKindFor::Call(callee, _, _) = &value.kind else {
            panic!("expected call of static method value, got {:?}", value.kind);
        };
        let crate::hir::HirExprKindFor::Lambda { body, .. } = &callee.kind else {
            panic!("expected static method lambda, got {:?}", callee.kind);
        };
        let target = body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Expr(crate::hir::HirExprFor {
                    kind:
                        crate::hir::HirExprKindFor::Call(
                            _,
                            _,
                            Some(crate::hir::HirCallTarget::StaticMethod(target)),
                        ),
                    ..
                }) => Some(target),
                _ => None,
            })
            .expect("static method authority");

        assert!(target.method.impl_id().is_some());
        assert!(target.method.trait_id().is_some());
        assert!(target.method.method_id().is_some());
        assert!(matches!(target.owner_ty, Type::Constructor { .. }));
    }

    #[test]
    fn constructor_trait_selection_generic_calls_record_deferred_bound_authority() {
        let hir = compile_source_to_resolved_hir_for_test(
            r#"
trait Applicative for F _
    pure: A -> F A

make: A -> F A where F _: Applicative
make = value -> F::pure value

make_qualified: A -> F A where F _: Applicative
make_qualified = value -> F::Applicative::pure value
"#,
        );

        for function_name in ["make", "make_qualified"] {
            let function = hir
                .program
                .functions
                .values()
                .find(|function| function.name == function_name)
                .unwrap_or_else(|| panic!("{function_name} function"));
            let target = function
                .body
                .stmts
                .iter()
                .find_map(|stmt| match stmt {
                    crate::hir::HirStmtFor::Expr(expr)
                    | crate::hir::HirStmtFor::Return(Some(expr))
                    | crate::hir::HirStmtFor::Let { value: expr, .. } => {
                        find_static_method_target(expr)
                    }
                    _ => None,
                })
                .expect("deferred constructor authority");

            assert!(target.method.impl_id().is_none());
            assert!(matches!(
                target.method.target,
                HirSelectedMethodTarget::TraitMethod {
                    dispatch: HirTraitDispatchKind::TraitBound,
                    ..
                }
            ));
            assert!(matches!(target.owner_ty, Type::Generic(_)));
            assert!(target.method.trait_id().is_some());
            assert!(target.method.method_id().is_some());
            assert!(!target.method.owner_substitution.is_empty());
        }
    }

    #[test]
    fn constructor_trait_selection_result_section_preserves_fixed_error() {
        let hir = compile_source_to_resolved_hir_for_test(
            r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = (Outcome _, IoError)::pure 42
    0
"#,
        );
        let io_error_id = hir
            .program
            .structs
            .values()
            .find(|structure| structure.name == "IoError")
            .expect("IoError structure")
            .id;
        let main = hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .expect("main function");
        let target = main
            .body
            .stmts
            .iter()
            .find_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
                _ => None,
            })
            .expect("selected Outcome section authority");

        assert!(target.method.impl_id().is_some());
        assert!(target.method.owner_substitution.iter().any(|binding| {
            binding.ty
                == Type::Struct {
                    id: io_error_id,
                    args: Vec::new(),
                }
        }));
    }

    #[test]
    fn constructor_trait_selection_preserves_inherent_precedence_and_qualification() {
        let hir = compile_source_to_resolved_hir_for_test(
            r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Maybe T
    pure = value -> Maybe::None

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    inherent: Maybe I64 = Maybe::pure 1
    qualified: Maybe I64 = Maybe::Applicative::pure 2
    0
"#,
        );
        let main = hir
            .program
            .functions
            .values()
            .find(|function| function.name == "main")
            .expect("main function");
        let targets = main
            .body
            .stmts
            .iter()
            .filter_map(|stmt| match stmt {
                crate::hir::HirStmtFor::Let { value, .. } => find_static_method_target(value),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(targets.len(), 2);
        assert!(matches!(
            targets[0].method.target,
            HirSelectedMethodTarget::ImplMethod {
                selected_trait: None,
                ..
            }
        ));
        assert!(matches!(
            targets[1].method.target,
            HirSelectedMethodTarget::ImplMethod {
                selected_trait: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn generic_static_method_value_preserves_owner_generic_param_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(43);
        let impl_id = def_id(44);
        let method_id = def_id(45);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Widget".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "StructDisplay",
            )],
            fields: Vec::new(),
        });
        register_item_path(&mut lowerer, "Widget", struct_id);

        let mut method = test_function(method_id, "create");
        method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
        method.params = vec![
            HirParam {
                name: "owner".to_string(),
                local_id: HirLocalId(0),
                ty: Type::Generic(owner_param),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "value".to_string(),
                local_id: HirLocalId(1),
                ty: Type::Generic(method_param),
                mutable: false,
                is_ref: false,
            },
        ];
        method.ret_type = Type::Tuple(vec![
            Type::Generic(owner_param),
            Type::Generic(method_param),
        ]);
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Widget".to_string()),
                type_name: "Widget".to_string(),
                type_generics: vec![GenericParamDecl::type_param(owner_param, "ImplDisplay")],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(owner_param)],
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("create".to_string(), method)]),
            })
            .unwrap();

        let expr = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
        });

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.target, HirVarTarget::Function(method_id));
        assert_eq!(target.method.impl_id(), Some(impl_id));
        assert_eq!(target.method.method_id(), Some(method_id));
        assert_eq!(target.method.owner_substitution.len(), 1);
        assert_eq!(target.method.owner_substitution[0].param, owner_param);
        assert_eq!(target.method.method_substitution.len(), 1);
        assert_eq!(target.method.method_substitution[0].param, method_param);
    }

    #[test]
    fn enum_static_method_value_preserves_exact_impl_and_method_authority() {
        let mut lowerer = Lowerer::new_for_test();
        let enum_id = def_id(50);
        let impl_id = def_id(51);
        let method_id = def_id(52);
        lowerer.items.insert_enumeration(HirEnum {
            id: enum_id,
            name: "Choice".to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        });
        register_item_path(&mut lowerer, "Choice", enum_id);
        let method = test_function(method_id, "default");
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Choice".to_string()),
                type_name: "Choice".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Enum {
                    id: enum_id,
                    args: Vec::new(),
                }),
                trait_name: None,
                trait_id: None,
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("default".to_string(), method)]),
            })
            .unwrap();

        let expr = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_identifier_path(&identifier_path(&["Choice", "default"]))
        });

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.target, HirVarTarget::Function(method_id));
        assert_eq!(target.method.impl_id(), Some(impl_id));
        assert_eq!(target.method.method_id(), Some(method_id));
    }

    #[test]
    fn canonical_static_owner_without_method_is_an_error_not_a_raw_var() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(53);
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "Widget"));
        register_item_path(&mut lowerer, "Widget", struct_id);

        let expr = lowerer.lower_identifier_path(&identifier_path(&["Widget", "missing"]));

        assert_eq!(expr.ty, Type::Error);
        assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
        assert!(lowerer
            .diagnostics
            .errors()
            .iter()
            .any(|error| { error.message.contains("Widget::missing") && error.span().is_some() }));
    }

    #[test]
    fn ambiguous_static_bound_method_is_an_error_without_internal_candidate_ids() {
        let mut lowerer = Lowerer::new_for_test();
        let generic_owner = def_id(53);
        let owner_param = GenericParamId {
            owner: generic_owner,
            index: 0,
        };
        let left_trait = def_id(54);
        let left_member = def_id(55);
        let right_trait = def_id(56);
        let right_member = def_id(57);
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: left_trait,
            name: "Left".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("build".to_string(), test_function(left_member, "build"))]),
            signatures: HashMap::new(),
        });
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: right_trait,
            name: "Right".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("build".to_string(), test_function(right_member, "build"))]),
            signatures: HashMap::new(),
        });
        let bounds = HashMap::from([(
            owner_param,
            vec![
                TraitBound {
                    trait_id: right_trait,
                    type_args: Vec::new(),
                },
                TraitBound {
                    trait_id: left_trait,
                    type_args: Vec::new(),
                },
            ],
        )]);

        let expr = lowerer.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(def_id(58)),
                Some(generic_owner),
                vec!["T".to_string()],
                bounds.into(),
                false,
            ),
            |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
        );

        assert_eq!(expr.ty, Type::Error);
        assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
        let diagnostic = lowerer
            .diagnostics
            .errors()
            .iter()
            .find(|error| error.message.contains("ambiguous static bound method"))
            .expect("ambiguity diagnostic");
        assert!(diagnostic.message.contains("matching trait bounds"));
        assert!(!diagnostic.message.contains("DefId"));
        assert!(diagnostic.span().is_some());
    }

    #[test]
    fn static_bound_method_lookup_expands_supertraits() {
        let mut lowerer = Lowerer::new_for_test();
        let generic_owner = def_id(59);
        let owner_param = GenericParamId {
            owner: generic_owner,
            index: 0,
        };
        let parent_trait = def_id(60);
        let parent_member = def_id(61);
        let child_trait = def_id(62);
        let child_target = GenericParamId {
            owner: child_trait,
            index: 0,
        };
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: parent_trait,
            name: "Parent".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("build".to_string(), test_function(parent_member, "build"))]),
            signatures: HashMap::new(),
        });
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: vec![Predicate::Trait {
                subject: Type::Generic(child_target),
                trait_id: parent_trait,
                args: Vec::new(),
            }],
            id: child_trait,
            name: "Child".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        let bounds = HashMap::from([(
            owner_param,
            vec![TraitBound {
                trait_id: child_trait,
                type_args: Vec::new(),
            }],
        )]);

        let expr = lowerer.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(def_id(63)),
                Some(generic_owner),
                vec!["T".to_string()],
                bounds.into(),
                false,
            ),
            |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
        );

        let (_, target) = static_method_value_parts(&expr);
        assert_eq!(target.method.trait_id(), Some(parent_trait));
        assert_eq!(target.method.method_id(), Some(parent_member));
    }

    #[test]
    fn distinct_static_bound_trait_args_remain_ambiguous() {
        let mut lowerer = Lowerer::new_for_test();
        let generic_owner = def_id(64);
        let owner_param = GenericParamId {
            owner: generic_owner,
            index: 0,
        };
        let trait_id = def_id(65);
        let member_id = def_id(66);
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "Arg",
            )],
            associated_types: Vec::new(),
            methods: HashMap::from([("build".to_string(), test_function(member_id, "build"))]),
            signatures: HashMap::new(),
        });
        let bounds = HashMap::from([(
            owner_param,
            vec![
                TraitBound {
                    trait_id,
                    type_args: vec![Type::I64],
                },
                TraitBound {
                    trait_id,
                    type_args: vec![Type::Bool],
                },
            ],
        )]);

        let expr = lowerer.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(def_id(67)),
                Some(generic_owner),
                vec!["T".to_string()],
                bounds.into(),
                false,
            ),
            |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "build"])),
        );

        assert_eq!(expr.ty, Type::Error);
        assert!(matches!(expr.kind, HirExprKind::Var(ref name) if name == "<error>"));
        let diagnostic = lowerer
            .diagnostics
            .errors()
            .iter()
            .find(|error| error.message.contains("ambiguous static bound method"))
            .expect("ambiguity diagnostic");
        assert!(diagnostic.message.contains("Bool"));
        assert!(diagnostic.message.contains("I64"));
        assert!(diagnostic.message.find("Bool") < diagnostic.message.find("I64"));
    }

    #[test]
    fn generic_static_bound_method_instantiates_method_generics_without_replacing_owner() {
        let mut lowerer = Lowerer::new_for_test();
        let generic_owner = def_id(68);
        let owner_param = GenericParamId {
            owner: generic_owner,
            index: 0,
        };
        let trait_id = def_id(69);
        let method_id = def_id(70);
        let trait_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let mut method = test_function(method_id, "make");
        method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
        method.params = vec![HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Generic(method_param),
            mutable: false,
            is_ref: false,
        }];
        method.ret_type = Type::Tuple(vec![
            Type::Generic(trait_param),
            Type::Generic(method_param),
        ]);
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "TraitDisplay",
            )],
            associated_types: Vec::new(),
            methods: HashMap::from([("make".to_string(), method)]),
            signatures: HashMap::new(),
        });
        let bounds = HashMap::from([(
            owner_param,
            vec![TraitBound {
                trait_id,
                type_args: vec![Type::Generic(owner_param)],
            }],
        )]);

        let expr = lowerer.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(def_id(71)),
                Some(generic_owner),
                vec!["T".to_string()],
                bounds.into(),
                false,
            ),
            |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
        );

        let Type::Function { params, ret, .. } = &expr.ty else {
            panic!("expected generic static method value function type")
        };
        assert!(matches!(params.as_slice(), [Type::TypeVar(_)]));
        assert!(matches!(
            ret.as_ref(),
            Type::Tuple(elements)
                if matches!(elements.as_slice(), [Type::Generic(param), Type::TypeVar(_)] if *param == owner_param)
        ));
        let (_, target) = static_method_value_parts(&expr);
        assert_eq!(target.method.method_id(), Some(method_id));
        assert_eq!(target.method.method_substitution.len(), 1);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert!(matches!(
            target.method.method_substitution[0].ty,
            Type::TypeVar(_)
        ));
        assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
    }

    #[test]
    fn identical_generic_static_bounds_select_one_fresh_method_target() {
        let mut lowerer = Lowerer::new_for_test();
        let generic_owner = def_id(72);
        let owner_param = GenericParamId {
            owner: generic_owner,
            index: 0,
        };
        let trait_id = def_id(73);
        let method_id = def_id(74);
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        let mut method = test_function(method_id, "make");
        method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
        method.params = vec![HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Generic(method_param),
            mutable: false,
            is_ref: false,
        }];
        method.ret_type = Type::Generic(method_param);
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "TraitDisplay",
            )],
            associated_types: Vec::new(),
            methods: HashMap::from([("make".to_string(), method)]),
            signatures: HashMap::new(),
        });
        let bound = TraitBound {
            trait_id,
            type_args: vec![Type::Generic(owner_param)],
        };
        let bounds = HashMap::from([(owner_param, vec![bound.clone(), bound])]);

        let expr = lowerer.with_body_context(
            BodyLoweringContext::new(
                "test".to_string(),
                BodyOwner::Function(def_id(75)),
                Some(generic_owner),
                vec!["T".to_string()],
                bounds.into(),
                false,
            ),
            |lowerer| lowerer.lower_identifier_path(&identifier_path(&["T", "make"])),
        );

        assert!(matches!(expr.ty, Type::Function { .. }));
        assert!(lowerer.diagnostics.errors().is_empty());
        let (_, target) = static_method_value_parts(&expr);
        assert_eq!(target.method.method_id(), Some(method_id));
        assert_eq!(target.method.trait_args(), &[Type::Generic(owner_param)]);
        assert_eq!(target.method.method_substitution.len(), 1);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert!(matches!(
            target.method.method_substitution[0].ty,
            Type::TypeVar(_)
        ));
    }

    #[test]
    fn trait_backed_generic_static_method_preserves_member_and_trait_args() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(59);
        let trait_id = def_id(60);
        let trait_member_id = def_id(61);
        let impl_id = def_id(62);
        let method_id = def_id(63);
        let owner_param = GenericParamId {
            owner: impl_id,
            index: 0,
        };
        let method_param = GenericParamId {
            owner: method_id,
            index: 0,
        };
        lowerer.items.insert_structure(HirStruct {
            id: struct_id,
            name: "Tagged".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "NominalDisplay",
            )],
            fields: Vec::new(),
        });
        register_item_path(&mut lowerer, "Tagged", struct_id);
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Factory".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "TraitArgumentDisplay",
            )],
            associated_types: Vec::new(),
            methods: HashMap::from([(
                "create".to_string(),
                test_function(trait_member_id, "create"),
            )]),
            signatures: HashMap::new(),
        });
        let mut method = test_function(method_id, "create");
        method.generic_params = vec![GenericParamDecl::type_param(method_param, "MethodDisplay")];
        method.params = vec![HirParam {
            name: "value".to_string(),
            local_id: HirLocalId(0),
            ty: Type::Struct {
                id: struct_id,
                args: vec![Type::Generic(method_param)],
            },
            mutable: false,
            is_ref: false,
        }];
        method.ret_type = Type::Struct {
            id: struct_id,
            args: vec![Type::Generic(method_param)],
        };
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Tagged".to_string()),
                type_name: "Tagged".to_string(),
                type_generics: vec![GenericParamDecl::type_param(
                    owner_param,
                    "ImplOwnerDisplay",
                )],
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: vec![Type::Generic(owner_param)],
                }),
                trait_name: Some("Factory".to_string()),
                trait_id: Some(trait_id),
                trait_generics: vec![GenericParamDecl::type_param(owner_param, "TraitDisplay")],
                trait_arg_types: vec![Type::Generic(owner_param)],
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("create".to_string(), method)]),
            })
            .unwrap();

        let expr = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_identifier_path(&identifier_path(&["Tagged", "create"]))
        });

        let (_, target) = static_method_value_parts(&expr);
        assert_eq!(target.method.impl_id(), Some(impl_id));
        assert_eq!(target.method.method_id(), Some(method_id));
        assert_eq!(target.method.owner_substitution.len(), 1);
        assert_eq!(target.method.owner_substitution[0].param, owner_param);
        assert_eq!(target.method.method_substitution.len(), 1);
        assert_eq!(target.method.method_substitution[0].param, method_param);
        assert!(matches!(
            target.method.owner_substitution[0].ty,
            Type::TypeVar(_)
        ));
        assert_eq!(
            target.method.owner_substitution[0].ty,
            target.method.method_substitution[0].ty
        );
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected_trait),
            ..
        } = &target.method.target
        else {
            panic!("expected trait-backed impl method authority")
        };
        assert_eq!(selected_trait.trait_id, trait_id);
        assert_eq!(selected_trait.member_id, trait_member_id);
        assert_eq!(selected_trait.trait_args.len(), 1);
        assert_eq!(
            selected_trait.trait_args[0],
            target.method.owner_substitution[0].ty
        );
    }

    #[test]
    fn trait_backed_static_method_with_unknown_trait_authority_is_an_error() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(46);
        let impl_id = def_id(47);
        let method_id = def_id(48);
        let missing_trait_id = def_id(49);
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "Widget"));
        register_item_path(&mut lowerer, "Widget", struct_id);
        let method = test_function(method_id, "create");
        lowerer
            .items
            .insert_impl(HirImpl {
                id: impl_id,
                owner: HirImplOwner::Named("Widget".to_string()),
                type_name: "Widget".to_string(),
                type_generics: Vec::new(),
                receiver_pattern: HirImplReceiverPattern::Exact(Type::Struct {
                    id: struct_id,
                    args: Vec::new(),
                }),
                trait_name: Some("MissingTrait".to_string()),
                trait_id: Some(missing_trait_id),
                trait_generics: Vec::new(),
                trait_arg_types: Vec::new(),
                associated_types: Vec::new(),
                bounds: HashMap::new().into(),
                methods: HashMap::from([("create".to_string(), method)]),
            })
            .unwrap();

        let expr = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_identifier_path(&identifier_path(&["Widget", "create"]))
        });

        assert_eq!(expr.ty, Type::Error);
        assert!(!matches!(
            expr.kind,
            HirExprKind::ResolvedVar(HirVarRef {
                target: HirVarTarget::Function(id),
                ..
            }) if id == method_id
        ));
        assert!(lowerer
            .diagnostics
            .errors()
            .iter()
            .any(|error| { error.message.contains("unknown trait") && error.span().is_some() }));
    }

    #[test]
    fn module_local_enum_alias_shadows_root_enum_for_qualified_variant() {
        let mut lowerer = Lowerer::new_for_test();
        let root_id = def_id(42);
        let module_id = def_id(43);
        lowerer
            .items
            .insert_enumeration(single_unit_variant_enum(root_id, "Thing", "Variant"));
        lowerer.items.insert_enumeration(single_unit_variant_enum(
            module_id,
            "demo::helper::Thing",
            "Variant",
        ));
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

        let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "Variant"]));

        match expr.kind {
            HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
                assert_eq!(enum_name, "demo::helper::Thing");
                assert_eq!(variant_name, "Variant");
                assert_eq!(location.owner, module_id);
            }
            other => panic!("expected module alias enum variant, got {other:?}"),
        }
    }

    #[test]
    fn module_local_struct_alias_shadows_root_struct_for_static_method() {
        let mut lowerer = Lowerer::new_for_test();
        let root_struct_id = def_id(44);
        let module_struct_id = def_id(45);
        let root_method_id = def_id(46);
        let module_method_id = def_id(47);
        lowerer
            .items
            .insert_structure(empty_struct(root_struct_id, "Thing"));
        lowerer
            .items
            .insert_structure(empty_struct(module_struct_id, "demo::helper::Thing"));
        register_static_impl(
            &mut lowerer,
            def_id(146),
            root_struct_id,
            "Thing",
            "make",
            test_function(root_method_id, "make"),
        );
        register_static_impl(
            &mut lowerer,
            def_id(147),
            module_struct_id,
            "demo::helper::Thing",
            "make",
            test_function(module_method_id, "make"),
        );
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), root_struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_struct_id, "Thing".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Thing".to_string(),
            "demo::helper::Thing".to_string(),
            module_struct_id,
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["Thing", "make"]));

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.name, "demo::helper::Thing::make");
        assert_eq!(reference.target, HirVarTarget::Function(module_method_id));
        assert_eq!(target.method.impl_id(), Some(def_id(147)));
        assert_eq!(target.method.method_id(), Some(module_method_id));
    }

    #[test]
    fn import_alias_struct_resolves_static_impl_method_by_canonical_owner() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(48);
        let method_id = def_id(49);
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "stdlib::string::String"));
        lowerer.resolver.insert_import_alias_with_name(
            "String".to_string(),
            "stdlib::string::String".to_string(),
            struct_id,
        );
        lowerer
            .items
            .insert_impl(HirImpl {
                id: def_id(50),
                owner: HirImplOwner::Named("stdlib::string::String".to_string()),
                type_name: "String".to_string(),
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
                bounds: std::collections::HashMap::new().into(),
                methods: HashMap::from([(
                    "from_str".to_string(),
                    test_function(method_id, "from_str"),
                )]),
            })
            .unwrap();
        let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from_str"]));

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.name, "stdlib::string::String::from_str");
        assert_eq!(reference.target, HirVarTarget::Function(method_id));
        assert_eq!(target.method.impl_id(), Some(def_id(50)));
        assert_eq!(target.method.method_id(), Some(method_id));
    }

    #[test]
    fn import_alias_struct_without_canonical_name_does_not_resolve_static_impl_method_by_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(51);
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "String"));
        lowerer
            .resolver
            .import_aliases
            .insert("String".to_string(), struct_id);
        let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from_str"]));

        match expr.kind {
            HirExprKind::Var(name) => assert_eq!(name, "String::from_str"),
            other => panic!("expected unresolved static method path, got {other:?}"),
        }
    }

    #[test]
    fn import_alias_struct_uses_canonical_static_impl_owner() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(53);
        let canonical_method_id = def_id(55);
        lowerer
            .items
            .insert_structure(empty_struct(struct_id, "String"));
        lowerer.resolver.insert_import_alias_with_name(
            "String".to_string(),
            "stdlib::string::String".to_string(),
            struct_id,
        );
        register_static_impl(
            &mut lowerer,
            def_id(155),
            struct_id,
            "stdlib::string::String",
            "from_str",
            test_function(canonical_method_id, "from_str"),
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["String", "from_str"]));

        let (reference, target) = static_method_value_parts(&expr);
        assert_eq!(reference.name, "stdlib::string::String::from_str");
        assert_eq!(
            reference.target,
            HirVarTarget::Function(canonical_method_id)
        );
        assert_eq!(target.method.impl_id(), Some(def_id(155)));
        assert_eq!(target.method.method_id(), Some(canonical_method_id));
    }

    #[test]
    fn lower_import_alias_function_reference_records_function_id() {
        let mut lowerer = Lowerer::new_for_test();
        let answer_id = def_id(31);
        lowerer
            .items
            .insert_function(test_function(answer_id, "answer"));
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer.resolver.insert_import_alias_with_name(
            "answer".to_string(),
            "pkg::answer".to_string(),
            answer_id,
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "pkg::answer");
                assert_eq!(reference.target, HirVarTarget::Function(answer_id));
            }
            other => panic!("expected resolved function alias, got {other:?}"),
        }
    }

    #[test]
    fn module_local_alias_resolves_through_resolver_without_lowerer_string_map() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(32);
        lowerer
            .items
            .insert_function(test_function(function_id, "answer"));
        lowerer.resolver.insert_module_alias_with_name(
            "answer".to_string(),
            "demo::math::answer".to_string(),
            function_id,
        );
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                name,
                target: HirVarTarget::Function(id),
            }) => {
                assert_eq!(name, "demo::math::answer");
                assert_eq!(id, function_id);
            }
            other => panic!("expected resolver-backed module alias, got {other:?}"),
        }
    }

    #[test]
    fn lower_identifier_path_requires_resolver_id_for_top_level_function() {
        let mut lowerer = Lowerer::new_for_test();
        let function_id = def_id(38);
        lowerer
            .items
            .insert_function(test_function(function_id, "answer"));

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::Var(name) => assert_eq!(name, "answer"),
            other => panic!("expected unresolved var without resolver id, got {other:?}"),
        }
    }

    #[test]
    fn module_local_alias_shadows_root_item_with_same_short_name() {
        let mut lowerer = Lowerer::new_for_test();
        let root_id = def_id(36);
        let module_id = def_id(37);
        lowerer
            .items
            .insert_function(test_function(root_id, "answer"));
        lowerer
            .items
            .insert_function(test_function(module_id, "answer"));
        lowerer
            .resolver
            .item_paths
            .insert("answer".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "answer".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "answer".to_string(),
            "demo::helper::answer".to_string(),
            module_id,
        );
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(HirVarRef {
                name,
                target: HirVarTarget::Function(id),
            }) => {
                assert_eq!(name, "demo::helper::answer");
                assert_eq!(id, module_id);
            }
            other => panic!("expected module alias to shadow root item, got {other:?}"),
        }
    }

    #[test]
    fn lower_nested_scope_module_local_alias_function_reference_records_function_id() {
        let mut lowerer = Lowerer::new_for_test();
        let answer_id = def_id(34);
        lowerer
            .items
            .insert_function(test_function(answer_id, "answer"));
        lowerer.scope.push();
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer.resolver.insert_module_alias_with_name(
            "answer".to_string(),
            "pkg::answer".to_string(),
            answer_id,
        );
        lowerer.scope.push();

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "pkg::answer");
                assert_eq!(reference.target, HirVarTarget::Function(answer_id));
            }
            other => panic!("expected resolved nested module-local function alias, got {other:?}"),
        }
    }

    #[test]
    fn lower_local_shadowing_module_local_alias_stays_var() {
        let mut lowerer = Lowerer::new_for_test();
        let answer_id = def_id(35);
        lowerer
            .items
            .insert_function(test_function(answer_id, "answer"));
        lowerer.scope.push();
        lowerer.scope.define_alias(
            "answer".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );
        lowerer.resolver.insert_module_alias_with_name(
            "answer".to_string(),
            "pkg::answer".to_string(),
            answer_id,
        );
        lowerer.scope.push();
        lowerer.scope.define("answer".to_string(), Type::I64, false);

        let expr = lowerer.lower_identifier_path(&identifier_path(&["answer"]));

        match expr.kind {
            HirExprKind::Var(name) => assert_eq!(name, "answer"),
            other => panic!("expected local shadow to stay a var, got {other:?}"),
        }
    }

    #[test]
    fn lower_import_alias_extern_reference_records_extern_id() {
        let mut lowerer = Lowerer::new_for_test();
        let extern_id = def_id(33);
        lowerer.items.insert_extern(HirExtern {
            id: extern_id,
            name: "pkg::puts".to_string(),
            params: vec![Type::I32],
            ret: Type::I32,
            variadic: false,
            is_unsafe: false,
        });
        lowerer.scope.define_alias(
            "puts".to_string(),
            Type::function(vec![Type::I32], Type::I32),
            false,
        );
        lowerer.resolver.insert_import_alias_with_name(
            "puts".to_string(),
            "pkg::puts".to_string(),
            extern_id,
        );

        let expr = lowerer.lower_identifier_path(&identifier_path(&["puts"]));

        match expr.kind {
            HirExprKind::ResolvedVar(reference) => {
                assert_eq!(reference.name, "pkg::puts");
                assert_eq!(reference.target, HirVarTarget::Extern(extern_id));
            }
            other => panic!("expected resolved extern alias, got {other:?}"),
        }
    }

    #[test]
    fn test_lower_lambda_carries_capture_metadata() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![ident_pattern("x")],
            body: Block {
                statements: vec![Statement::Expression(var_expr("base"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };

        let hir = lowerer.with_test_body_context(|lowerer| {
            let base_id = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("base".to_string(), Type::I64, false, base_id);
            lowerer.lower_lambda(&lambda)
        });
        let Type::Function {
            callable_kind,
            captures: type_captures,
            ..
        } = &hir.ty
        else {
            panic!("expected lambda function type");
        };
        assert_eq!(*callable_kind, CallableKind::Fn);
        assert_eq!(type_captures[0].kind, CaptureKind::SharedBorrow);
        assert_eq!(type_captures[0].ty, Type::I64);
        match hir.kind {
            HirExprKind::Lambda { captures, .. } => {
                assert_eq!(captures.len(), 1);
                assert_eq!(captures[0].name, "base");
            }
            _ => panic!("expected lambda"),
        }
    }

    #[test]
    fn lower_lambda_reference_pattern_wraps_parameter_type() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![Pattern {
                binding: None,
                kind: PatternKind::Reference {
                    pattern: Box::new(ident_pattern("value")),
                    mutable: false,
                },
            }],
            body: Block {
                statements: vec![Statement::Expression(var_expr("value"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };

        let hir = lowerer.with_test_body_context(|lowerer| lowerer.lower_lambda(&lambda));
        let HirExprKind::Lambda { params, .. } = &hir.kind else {
            panic!("expected lambda");
        };
        let Type::Function {
            params: function_params,
            ..
        } = &hir.ty
        else {
            panic!("expected function type");
        };

        assert_eq!(params[0].ty, function_params[0]);
        assert!(matches!(
            &params[0].ty,
            Type::Reference {
                mutable: false,
                inner
            } if matches!(inner.as_ref(), Type::TypeVar(_))
        ));
    }

    #[test]
    fn outer_lambda_does_not_capture_local_used_by_nested_lambda() {
        let mut lowerer = Lowerer::new_for_test();
        let captures = lowerer.with_test_body_context(|lowerer| {
            let next_id = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("next".to_string(), Type::I64, false, next_id);
            let nested_body = HirBlock {
                stmts: vec![HirStmt::Expr(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "next".to_string(),
                        target: HirVarTarget::Local(next_id),
                    }),
                    ty: Type::I64,
                    span: crate::lexer::Span::test(),
                })],
                ty: Type::I64,
            };
            let body = HirBlock {
                stmts: vec![
                    HirStmt::Let {
                        name: "next".to_string(),
                        local_id: next_id,
                        ty: Type::I64,
                        value: HirExpr {
                            kind: HirExprKind::IntLiteral(1),
                            ty: Type::I64,
                            span: crate::lexer::Span::test(),
                        },
                        mutable: false,
                    },
                    HirStmt::Expr(HirExpr {
                        kind: HirExprKind::Lambda {
                            params: Vec::new(),
                            body: nested_body,
                            captures: Vec::new(),
                        },
                        ty: Type::function(Vec::new(), Type::I64),
                        span: crate::lexer::Span::test(),
                    }),
                ],
                ty: Type::function(Vec::new(), Type::I64),
            };

            lowerer.collect_lambda_captures(&body, &[])
        });

        assert!(captures.is_empty());
    }

    #[test]
    fn lower_lambda_captures_resolved_local_reference() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![ident_pattern("x")],
            body: Block {
                statements: vec![Statement::Expression(var_expr("base"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };

        let hir = lowerer.with_test_body_context(|lowerer| {
            let base_id = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("base".to_string(), Type::I64, false, base_id);
            lowerer.lower_lambda(&lambda)
        });
        match hir.kind {
            HirExprKind::Lambda { captures, .. } => {
                assert_eq!(captures.len(), 1);
                assert_eq!(captures[0].name, "base");
            }
            _ => panic!("expected lambda"),
        }
    }

    #[test]
    fn lambda_capture_records_captured_local_id() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![ident_pattern("x")],
            body: Block {
                statements: vec![Statement::Expression(var_expr("base"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };

        let (captured_id, hir) = lowerer.with_test_body_context(|lowerer| {
            let captured_id = lowerer.fresh_local_id();
            lowerer
                .scope
                .define_local("base".to_string(), Type::I64, false, captured_id);
            (captured_id, lowerer.lower_lambda(&lambda))
        });
        match hir.kind {
            HirExprKind::Lambda { captures, .. } => {
                assert_eq!(captures.len(), 1);
                assert_eq!(captures[0].name, "base");
                assert_eq!(captures[0].local_id, captured_id);
            }
            other => panic!("expected lambda, got {other:?}"),
        }
    }

    #[test]
    fn lambda_captures_root_scope_lexical_function_value() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![],
            body: Block {
                statements: vec![Statement::Expression(var_expr("callback"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };
        let callback_id = HirLocalId(91);
        lowerer.scope.define_local(
            "callback".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
            callback_id,
        );

        let hir = lowerer.lower_lambda(&lambda);

        let HirExprKind::Lambda { captures, .. } = hir.kind else {
            panic!("expected lambda");
        };
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].name, "callback");
        assert_eq!(captures[0].local_id, callback_id);
    }

    #[test]
    fn lambda_does_not_capture_explicit_top_level_function() {
        let mut lowerer = Lowerer::new_for_test();
        let lambda = LambdaDecl {
            parameters: vec![],
            body: Block {
                statements: vec![Statement::Expression(var_expr("callback"))],
            },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        };
        lowerer.scope.define_top_level(
            "callback".to_string(),
            Type::function(Vec::new(), Type::I64),
            false,
        );

        let hir = lowerer.lower_lambda(&lambda);

        let HirExprKind::Lambda { captures, .. } = hir.kind else {
            panic!("expected lambda");
        };
        assert!(captures.is_empty());
    }

    #[test]
    fn lowers_known_field_access_with_resolved_field_location() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(point_struct());
        register_item_path(&mut lowerer, "Point", def_id(10));

        let base = HirExpr {
            kind: HirExprKind::Var("point".to_string()),
            ty: Type::Struct {
                id: def_id(10),
                args: Vec::new(),
            },
            span: crate::lexer::Span::test(),
        };

        let field = lowerer.apply_secondary(
            base,
            &ast::SecondaryExpr::Dot(ast::IdentOrNumber::Ident(ident("y"))),
            crate::lower::expression::ExprUse::Value,
            false,
        );

        match field.kind {
            HirExprKind::FieldAccess(_, name, Some(location)) => {
                assert_eq!(name, "y");
                assert_eq!(location.owner, def_id(10));
                assert_eq!(location.field_id, FieldId(1));
                assert_eq!(location.name, "y");
            }
            other => panic!("expected resolved field access, got {other:?}"),
        }
    }

    #[test]
    fn lowers_known_struct_literal_fields_with_resolved_field_locations() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(point_struct());
        register_item_path(&mut lowerer, "Point", def_id(10));

        let literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("Point"))],
            },
            fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
        });

        match literal.kind {
            HirExprKind::StructLiteral(_, _, fields) => {
                let y = fields.iter().find(|field| field.name == "y").unwrap();
                let location = y.field.as_ref().expect("known field should be resolved");
                assert_eq!(location.owner, def_id(10));
                assert_eq!(location.field_id, FieldId(1));
                assert_eq!(location.name, "y");
            }
            other => panic!("expected struct literal, got {other:?}"),
        }
    }

    #[test]
    fn struct_literal_resolved_fields_all_have_locations() {
        let point_id = def_id(10);
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(point_struct());
        register_item_path(&mut lowerer, "Point", point_id);

        let literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("Point"))],
            },
            fields: HashMap::from([(ident("x"), int_expr("1")), (ident("y"), int_expr("2"))]),
        });

        match literal.kind {
            HirExprKind::StructLiteral(struct_name, Some(id), fields) => {
                assert_eq!(struct_name, "Point");
                assert_eq!(id, point_id);
                let x = fields.iter().find(|field| field.name == "x").unwrap();
                let location = x
                    .field
                    .as_ref()
                    .expect("resolved field should have location");
                assert_eq!(location.owner, point_id);
                assert_eq!(location.field_id, FieldId(0));
                assert_eq!(location.name, "x");
                assert!(
                    fields.iter().all(|field| field.field.is_some()),
                    "all resolved fields should have locations"
                );
            }
            other => panic!("expected resolved Point struct literal, got {other:?}"),
        }
    }

    #[test]
    fn module_local_struct_alias_shadows_root_struct_for_instance_construction() {
        let mut lowerer = Lowerer::new_for_test();
        let root_id = def_id(48);
        let module_id = def_id(49);
        lowerer
            .items
            .insert_structure(empty_struct(root_id, "Thing"));
        lowerer
            .items
            .insert_structure(empty_struct(module_id, "demo::helper::Thing"));
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

        let literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("Thing"))],
            },
            fields: HashMap::new(),
        });

        match literal.kind {
            HirExprKind::StructLiteral(name, Some(id), _) => {
                assert_eq!(name, "demo::helper::Thing");
                assert_eq!(id, module_id);
            }
            other => panic!("expected module alias struct literal, got {other:?}"),
        }
    }

    #[test]
    fn struct_literal_private_field_check_matches_current_impl_by_struct_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(56);
        lowerer
            .items
            .insert_structure(private_field_struct(struct_id, "String"));
        lowerer.items.insert_structure(private_field_struct(
            struct_id,
            "stdlib::string_type::String",
        ));
        lowerer
            .resolver
            .item_paths
            .insert("String".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "stdlib::string_type::String".to_string());
        lowerer.current_struct_impl = Some("String".to_string());

        let _literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("String"))],
            },
            fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
        });

        assert!(
            lowerer
                .errors()
                .iter()
                .all(|error| !error.message.contains("private fields")),
            "same-struct construction should not report private fields: {:?}",
            lowerer.errors()
        );
    }

    #[test]
    fn struct_literal_private_field_check_matches_current_impl_by_pre_resolved_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(58);
        lowerer.items.insert_structure(private_field_struct(
            struct_id,
            "stdlib::string_type::String",
        ));
        lowerer
            .resolver
            .item_paths
            .insert("String".to_string(), struct_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(struct_id, "stdlib::string_type::String".to_string());
        lowerer.current_struct_impl = Some("stdlib::string_type::String".to_string());

        let _literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("String"))],
            },
            fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
        });

        assert!(
            lowerer
                .errors()
                .iter()
                .all(|error| !error.message.contains("private fields")),
            "same-struct construction should not report private fields: {:?}",
            lowerer.errors()
        );
    }

    #[test]
    fn struct_literal_private_field_check_requires_resolved_current_impl_id() {
        let mut lowerer = Lowerer::new_for_test();
        let struct_id = def_id(57);
        lowerer
            .items
            .insert_structure(private_field_struct(struct_id, "String"));
        lowerer.items.insert_structure(private_field_struct(
            struct_id,
            "stdlib::string_type::String",
        ));
        register_item_path(&mut lowerer, "stdlib::string_type::String", struct_id);
        lowerer.current_struct_impl = Some("String".to_string());

        let _literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![
                    IdentOrType::Ident(ident("stdlib")),
                    IdentOrType::Ident(ident("string_type")),
                    IdentOrType::Ident(ident("String")),
                ],
            },
            fields: HashMap::from([(ident("ptr"), int_expr("1"))]),
        });

        assert!(
            lowerer
                .errors()
                .iter()
                .any(|error| error.message.contains("private fields")),
            "unresolved current impl name should not grant private construction: {:?}",
            lowerer.errors()
        );
    }

    #[test]
    fn lowers_known_enum_variant_constructor_with_resolved_variant_location() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_enumeration(option_enum());
        register_item_path(&mut lowerer, "Maybe", def_id(20));

        let variant = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![
                    IdentOrType::Ident(ident("Maybe")),
                    IdentOrType::Ident(ident("Some")),
                ],
            },
            fields: HashMap::from([(ident("value"), int_expr("1"))]),
        });

        match variant.kind {
            HirExprKind::EnumVariant(enum_name, variant_name, _, Some(location)) => {
                assert_eq!(enum_name, "Maybe");
                assert_eq!(variant_name, "Some");
                assert_eq!(location.owner, def_id(20));
                assert_eq!(location.variant_id, VariantId(0));
                assert_eq!(location.name, "Some");
            }
            other => panic!("expected resolved enum variant, got {other:?}"),
        }
    }

    #[test]
    fn lowers_named_enum_variant_fields_in_variant_definition_order() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_enumeration(record_enum());
        register_item_path(&mut lowerer, "Record", def_id(21));

        let variant = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![
                    IdentOrType::Ident(ident("Record")),
                    IdentOrType::Ident(ident("Pair")),
                ],
            },
            fields: HashMap::from([
                (ident("fourth"), int_expr("4")),
                (ident("third"), int_expr("3")),
                (ident("second"), int_expr("2")),
                (ident("first"), int_expr("1")),
            ]),
        });

        match variant.kind {
            HirExprKind::EnumVariant(_, _, args, Some(location)) => {
                assert_eq!(location.owner, def_id(21));
                assert_eq!(location.variant_id, VariantId(0));
                assert!(matches!(args[0].kind, HirExprKind::IntLiteral(1)));
                assert!(matches!(args[1].kind, HirExprKind::IntLiteral(2)));
                assert!(matches!(args[2].kind, HirExprKind::IntLiteral(3)));
                assert!(matches!(args[3].kind, HirExprKind::IntLiteral(4)));
            }
            other => panic!("expected resolved enum variant, got {other:?}"),
        }
    }

    #[test]
    fn leaves_unknown_struct_literal_fields_unresolved() {
        let mut lowerer = Lowerer::new_for_test();

        let literal = lowerer.lower_instance(&ast::Instance {
            name: ast::TypePath {
                path: vec![IdentOrType::Ident(ident("Unknown"))],
            },
            fields: HashMap::from([(ident("value"), int_expr("1"))]),
        });

        match literal.kind {
            HirExprKind::StructLiteral(_, _, fields) => {
                assert_eq!(fields[0].name, "value");
                assert!(fields[0].field.is_none());
            }
            other => panic!("expected fallback struct literal, got {other:?}"),
        }
    }
}
