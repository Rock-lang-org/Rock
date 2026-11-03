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
        let (owner_ty, bounds) =
            if let Some(owner_param) = self.current_generic_param_id_for_name(owner_name) {
                let owner_ty = Type::Generic(owner_param);
                let Some(bounds) = self.current_impl_bounds().get(&owner_param) else {
                    return Ok(None);
                };
                let bounds = self
                    .selection_service()
                    .trait_bounds_with_supertraits(&owner_ty, bounds);
                (owner_ty, bounds)
            } else {
                let Some(nominal) = crate::lower::resolution::LowerResolutionContext::new(self)
                    .resolve_nominal_type(owner_name)
                else {
                    return Ok(None);
                };
                let owner_ty = match nominal {
                    crate::type_lowering::ResolvedNominalType::Struct(def)
                        if def.generic_params.is_empty() =>
                    {
                        Type::Struct {
                            id: def.id,
                            args: Vec::new(),
                        }
                    }
                    crate::type_lowering::ResolvedNominalType::Enum(def)
                        if def.generic_params.is_empty() =>
                    {
                        Type::Enum {
                            id: def.id,
                            args: Vec::new(),
                        }
                    }
                    _ => return Ok(None),
                };
                let matching_traits = self
                    .items
                    .impl_defs_in_order()
                    .map(|(_, imp)| imp)
                    .filter(|imp| {
                        imp.receiver_pattern == HirImplReceiverPattern::Exact(owner_ty.clone())
                    })
                    .filter(|imp| {
                        imp.methods
                            .get(method_name)
                            .is_some_and(|method| !method.is_method)
                    })
                    .map(|imp| imp.trait_id)
                    .collect::<Vec<_>>();
                let Some(Some(trait_id)) = matching_traits.first().copied() else {
                    return Ok(None);
                };
                if required_trait_id.is_some_and(|required| required != trait_id)
                    || matching_traits.len() < 2
                    || matching_traits
                        .iter()
                        .any(|candidate| *candidate != Some(trait_id))
                {
                    return Ok(None);
                }
                // Resolve the shared trait member now, but let argument inference choose
                // its implementation rather than selecting an arbitrary overload.
                let trait_def = self
                    .trait_by_id(trait_id)
                    .cloned()
                    .ok_or_else(|| "static method has unknown trait authority".to_string())?;
                let bound = crate::types::TraitBound {
                    trait_id,
                    type_args: trait_def
                        .generic_params
                        .iter()
                        .map(|param| {
                            self.engine
                                .fresh_type_var_at_kind(operation_span.clone(), param.kind.clone())
                        })
                        .collect(),
                };
                self.constraint_store.add_trait(
                    owner_ty.clone(),
                    bound.clone(),
                    operation_span.clone(),
                    "static trait member",
                );
                (owner_ty, vec![bound])
            };

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
            ast::IdentOrType::Type(ast::ParseType::Type(inner)) if inner.generics.is_empty() => Some(
                crate::type_lowering::TypeLowerer::lower_constructor(self, &ast::ParseType::Type(inner.clone())),
            ),
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

    fn register_static_owner_defaults(&mut self, owner_ty: &Type, signature: &Type) {
        let owner = match owner_ty {
            Type::Constructor { id, .. } | Type::Struct { id, .. } | Type::Enum { id, .. } => *id,
            _ => return,
        };
        let Some(params) = crate::type_lowering::TypeLoweringContext::constructor_params(self, owner) else { return; };
        let mut arguments = match owner_ty {
            Type::Struct { args, .. } | Type::Enum { args, .. } => Some(args.clone()),
            _ => None,
        };
        crate::type_services::visit::visit_type(signature, &mut |nested: &Type| {
            if let Type::Struct { id, args } | Type::Enum { id, args } = nested {
                if *id == owner && arguments.is_none() { arguments = Some(args.clone()); }
            }
        });
        if let Some(arguments) = arguments {
            self.engine.register_generic_defaults(&params, &arguments);
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

    pub(crate) fn infer_static_trait_argument_coercions(
        &mut self,
        callee: &HirExpr,
        args: &[HirExpr],
    ) {
        let HirExprKind::Lambda {
            params,
            body,
            captures,
        } = &callee.kind
        else {
            return;
        };
        let [HirStmt::Expr(HirExpr {
            kind: HirExprKind::Call(_, forwarded, Some(HirCallTarget::StaticMethod(target))),
            ..
        })] = body.stmts.as_slice()
        else {
            return;
        };
        if !captures.is_empty() || params.len() != args.len() || forwarded.len() != params.len()
            || !forwarded.iter().zip(params).all(|(arg, param)| matches!(&arg.kind,
                HirExprKind::ResolvedVar(reference) if reference.target == HirVarTarget::Local(param.local_id)))
        {
            return;
        }
        let Some(trait_id) = target.method.trait_id() else {
            return;
        };
        if target.method.impl_id().is_some() {
            return;
        }
        let owner = self.engine.resolve(&target.owner_ty);
        let mut direct = self.engine.clone_for_probe();
        if params
            .iter()
            .zip(args)
            .any(|(param, arg)| direct.unify(&param.ty, &arg.ty).is_err())
        {
            return;
        }
        let direct_args = target
            .method
            .trait_args()
            .iter()
            .map(|ty| direct.resolve(ty))
            .collect::<Vec<_>>();
        if !matches!(
            self.selection_service()
                .select_trait_impl_strict(&owner, trait_id, &direct_args),
            Err(crate::selection::SelectionDiagnostic::NoImplementation { .. })
        ) {
            return;
        }

        // Keep the expected parameter open until overload selection has considered
        // the normal array-reference-to-slice coercion. Exact impls take precedence.
        let mut coerced = self.engine.clone_for_probe();
        let mut changed = false;
        for (param, arg) in params.iter().zip(args) {
            let mut actual = self.engine.resolve(&arg.ty);
            if let Type::Reference { mutable, inner } = &actual {
                if let Type::Array(element, _) = inner.as_ref() {
                    actual = Type::Reference {
                        mutable: *mutable,
                        inner: Box::new(Type::Slice(element.clone())),
                    };
                    changed = true;
                }
            }
            if coerced.unify(&param.ty, &actual).is_err() {
                return;
            }
        }
        let trait_args = target
            .method
            .trait_args()
            .iter()
            .map(|ty| coerced.resolve(ty))
            .collect::<Vec<_>>();
        if changed
            && self
                .selection_service()
                .select_trait_impl_strict(&owner, trait_id, &trait_args)
                .is_ok()
        {
            self.engine.commit_probe(coerced.clone_for_commit());
        }
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
        self.register_static_owner_defaults(&owner_ty, &ty);
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

    fn define_capture_pattern(
        pattern: &HirPattern,
        defined: &mut std::collections::HashSet<String>,
    ) {
        match pattern {
            HirPattern::Binding { name, .. } => {
                defined.insert(name.clone());
            }
            HirPattern::Tuple(patterns)
            | HirPattern::Or(patterns)
            | HirPattern::Enum(_, _, _, patterns) => {
                for pattern in patterns {
                    Self::define_capture_pattern(pattern, defined);
                }
            }
            HirPattern::Struct(_, _, _, fields) => {
                for field in fields {
                    Self::define_capture_pattern(&field.pattern, defined);
                }
            }
            HirPattern::Wildcard | HirPattern::Literal(_) => {}
        }
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
                        Self::lambda_capture_kind_for_value(
                            &self.engine.resolve(&value.ty),
                            current_kind,
                        ),
                    );
                    defined.insert(name.clone());
                }
                HirStmt::Expr(expr) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
                ),
                HirStmt::Return(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
                ),
                HirStmt::Break(Some(expr)) => self.collect_lambda_captures_expr(
                    expr,
                    defined,
                    used,
                    capture_kinds,
                    Self::lambda_capture_kind_for_value(
                        &self.engine.resolve(&expr.ty),
                        current_kind,
                    ),
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
                let mut function_ty = self.engine.resolve(&func.ty);
                while let Type::Reference { inner, .. } = function_ty {
                    function_ty = *inner;
                }
                let callee_kind = match function_ty {
                    Type::Function {
                        callable_kind: crate::types::CallableKind::FnMut,
                        ..
                    } => HirClosureCaptureKind::MutableBorrow,
                    Type::Function {
                        callable_kind: crate::types::CallableKind::FnOnce,
                        ..
                    } => HirClosureCaptureKind::Move,
                    _ => HirClosureCaptureKind::SharedBorrow,
                };
                self.collect_lambda_captures_expr(func, defined, used, capture_kinds, callee_kind);
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
                    Some(crate::types::ReceiverMode::Shared) | None => {
                        HirClosureCaptureKind::SharedBorrow
                    }
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
                    Self::define_capture_pattern(&arm.pattern, &mut arm_defined);
                    if let Some(guard) = &arm.guard {
                        self.collect_lambda_captures_expr(
                            guard,
                            &mut arm_defined,
                            used,
                            capture_kinds,
                            current_kind,
                        );
                    }
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
            HirExprKind::For {
                var, iter, body, ..
            } => {
                self.collect_lambda_captures_expr(iter, defined, used, capture_kinds, current_kind);
                let mut nested_defined = defined.clone();
                nested_defined.insert(var.clone());
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
                                let type_args = self.engine.fresh_generic_args(&enum_info.generic_params, &span);
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

                // Check if it's a struct type name used as a value (e.g. String.from)
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
                let type_args = self.engine.fresh_generic_args(&enum_info.generic_params, &span);
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
                let fresh = self.engine.fresh_generic_args(&enum_info.generic_params, &span);
                let type_var_mapping: HashMap<GenericParamId, Type> = enum_info.generic_params.iter().zip(fresh)
                    .map(|(param, ty)| (param.id, ty)).collect();
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
            let fresh = self.engine.fresh_generic_args(&generic_params, &span);
            let type_var_mapping: HashMap<GenericParamId, Type> = generic_params.iter().zip(fresh)
                .map(|(param, ty)| (param.id, ty)).collect();

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
        let loop_stack = std::mem::take(&mut self.fold_loop_stack);
        let body = self.with_body_return_type(lambda_return_ty.clone(), |lowerer| {
            lowerer.lower_lambda_body(lambda)
        });
        self.fold_loop_stack = loop_stack;
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
        let loop_stack = std::mem::take(&mut self.fold_loop_stack);
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
        self.fold_loop_stack = loop_stack;
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
mod tests;
