//! Definition-owned conditional requirements for canonical callable bounds.
//! Retained generic HIR supplies the subject/signature; caller bodies do not.

use std::collections::{BTreeMap, BTreeSet};

use crate::hir::HirLanguageItems;
use crate::types::{CallableResultBorrowing, Type};

use super::hir_types::HirFunction;
use super::registry::{DefinitionClosureId, DefinitionGenericCallableInput};

pub(super) fn generic_inputs(
    function: &HirFunction,
    items: &HirLanguageItems,
) -> BTreeMap<usize, DefinitionGenericCallableInput> {
    let forwarded = forwarded_arrow_inputs(function);
    function
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, parameter)| {
            if parameter.is_ref {
                return None;
            }
            let Type::Generic(subject) = &parameter.ty else {
                return None;
            };
            Some((
                index,
                DefinitionGenericCallableInput {
                    subject: parameter.ty.clone(),
                    invocation: bound_requirement(function, items, subject)?,
                },
            ))
        })
        .chain(
            function
                .params
                .iter()
                .enumerate()
                .filter_map(|(index, parameter)| {
                    if parameter.is_ref {
                        return None;
                    }
                    let (invocation, changed) = nested_requirement(
                        &parameter.ty,
                        forwarded.contains(&parameter.local_id),
                        function,
                        items,
                    );
                    changed.then(|| {
                        (
                            index,
                            DefinitionGenericCallableInput {
                                subject: parameter.ty.clone(),
                                invocation,
                            },
                        )
                    })
                }),
        )
        .collect()
}

fn forwarded_arrow_inputs(function: &HirFunction) -> BTreeSet<crate::ids::HirLocalId> {
    use super::hir_types::{HirBlock, HirExpr, HirExprKind, HirStmt, HirVarTarget};
    use crate::ids::HirLocalId;

    fn local(expression: &HirExpr) -> Option<HirLocalId> {
        match &expression.kind {
            HirExprKind::ResolvedVar(variable) => match variable.target {
                HirVarTarget::Local(local) => Some(local),
                _ => None,
            },
            _ => None,
        }
    }
    #[derive(Default)]
    struct Uses {
        count: BTreeMap<HirLocalId, usize>,
        forwarded: BTreeSet<HirLocalId>,
        vetoed: BTreeSet<HirLocalId>,
    }
    impl Uses {
        fn block(&mut self, block: &HirBlock, repeated: bool) {
            for statement in &block.stmts {
                match statement {
                    HirStmt::Let { value, .. }
                    | HirStmt::Expr(value)
                    | HirStmt::Return(Some(value))
                    | HirStmt::Break(Some(value)) => self.expression(value, repeated),
                    _ => {}
                }
            }
        }
        fn expression(&mut self, expression: &HirExpr, repeated: bool) {
            if let Some(local) = local(expression) {
                *self.count.entry(local).or_default() += 1;
                if repeated {
                    self.vetoed.insert(local);
                }
            }
            match &expression.kind {
                HirExprKind::Call(_, arguments, _) => {
                    self.forwarded.extend(arguments.iter().filter_map(local))
                }
                HirExprKind::Lambda { captures, .. } => {
                    self.vetoed
                        .extend(captures.iter().map(|capture| capture.local_id));
                    // Nested parameter IDs cannot supply uses of this definition.
                    return;
                }
                _ => {}
            }
            let repeated = repeated
                || matches!(
                    expression.kind,
                    HirExprKind::While { .. } | HirExprKind::For { .. } | HirExprKind::Loop(_)
                );
            crate::hir::visit::visit_expr_children(expression, &mut |child| match child {
                crate::hir::visit::HirChild::Expr(expression) => {
                    self.expression(expression, repeated)
                }
                crate::hir::visit::HirChild::Block(block) => self.block(block, repeated),
            });
        }
    }
    let mut uses = Uses::default();
    uses.block(&function.body, false);
    // Only a single direct value forwarding use of an arrow in the retained
    // declaration adds a demand. Aliases/exposure/captures stay conservative;
    // no caller-selected binder layout or consuming protocol is introduced.
    uses.forwarded.retain(|local| {
        uses.count.get(local) == Some(&1)
            && !uses.vetoed.contains(local)
            && function.params.iter().any(|parameter| {
                parameter.local_id == *local && matches!(parameter.ty, Type::Function { .. })
            })
    });
    uses.forwarded
}

pub(super) fn closure_inputs(
    function: &HirFunction,
    items: &HirLanguageItems,
) -> BTreeMap<DefinitionClosureId, BTreeMap<usize, DefinitionGenericCallableInput>> {
    let mut inputs = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    crate::hir::visit::visit_block(&function.body, &mut |expression| {
        let super::hir_types::HirExprKind::Lambda { params, .. } = &expression.kind else {
            return;
        };
        let mut requirements = BTreeMap::new();
        for (index, parameter) in params.iter().enumerate() {
            if parameter.is_ref {
                continue;
            }
            let (invocation, changed) = match &parameter.ty {
                Type::Generic(subject) => bound_requirement(function, items, subject)
                    .map(|required| (required, true))
                    .unwrap_or_else(|| (parameter.ty.clone(), false)),
                _ => nested_requirement(&parameter.ty, false, function, items),
            };
            if changed {
                requirements.insert(
                    index,
                    DefinitionGenericCallableInput {
                        subject: parameter.ty.clone(),
                        invocation,
                    },
                );
            }
        }
        if !requirements.is_empty() {
            let ids = DefinitionClosureId::new(
                params.iter().map(|parameter| parameter.local_id).collect(),
                &expression.span,
            );
            if inputs.remove(&ids).is_some() {
                ambiguous.insert(ids);
            } else if !ambiguous.contains(&ids) {
                inputs.insert(ids, requirements);
            }
        }
    });
    inputs
}

fn bound_requirement(
    function: &HirFunction,
    items: &HirLanguageItems,
    subject: &crate::types::GenericParamId,
) -> Option<Type> {
    let mut invocation = None;
    for bound in function.generic_bounds.get(subject)? {
        let Some((kind, _, _)) = items
            .callable_protocols()
            .find(|(_, trait_id, _)| *trait_id == bound.trait_id)
        else {
            continue;
        };
        let [args, ret] = bound.type_args.as_slice() else {
            return None;
        };
        let params = match args {
            Type::Unit => vec![],
            Type::Tuple(params) => params.clone(),
            arg => vec![arg.clone()],
        };
        let mut required = Type::function(params.clone(), ret.clone());
        let Type::Function {
            callable_kind,
            result_borrowing,
            ..
        } = &mut required
        else {
            unreachable!()
        };
        *callable_kind = kind;
        *result_borrowing = crate::type_services::callable::owned_result_requirement(&params)?;
        if let Some(Type::Function {
            params: old_params,
            ret: old_ret,
            callable_kind: old_kind,
            ..
        }) = &invocation
        {
            if old_params != &params || **old_ret != *ret {
                return None;
            }
            if *old_kind < kind {
                continue;
            }
        }
        invocation = Some(required);
    }
    invocation
}

fn nested_requirement(
    ty: &Type,
    stored: bool,
    function: &HirFunction,
    items: &HirLanguageItems,
) -> (Type, bool) {
    match ty {
        // Retained definition bounds own this field demand. An unbound payload
        // or caller-specialized function layout still supplies no capability.
        Type::Generic(subject) if stored => bound_requirement(function, items, subject)
            .map(|required| (required, true))
            .unwrap_or_else(|| (ty.clone(), false)),
        Type::Function {
            params,
            result_borrowing: CallableResultBorrowing::Unknown,
            ..
        } if stored => {
            let mut required = ty.clone();
            let Some(requirement) =
                crate::type_services::callable::owned_result_requirement(params)
            else {
                return (ty.clone(), false);
            };
            let Type::Function {
                result_borrowing, ..
            } = &mut required
            else {
                unreachable!()
            };
            *result_borrowing = requirement;
            (required, true)
        }
        Type::Tuple(fields) => {
            let fields = fields
                .iter()
                .map(|field| nested_requirement(field, true, function, items))
                .collect::<Vec<_>>();
            let changed = fields.iter().any(|(_, changed)| *changed);
            (
                Type::Tuple(fields.into_iter().map(|(ty, _)| ty).collect()),
                changed,
            )
        }
        Type::Struct { id, args } | Type::Enum { id, args } => {
            let args = args
                .iter()
                .map(|arg| nested_requirement(arg, true, function, items))
                .collect::<Vec<_>>();
            let changed = args.iter().any(|(_, changed)| *changed);
            let args = args.into_iter().map(|(ty, _)| ty).collect();
            (
                if matches!(ty, Type::Struct { .. }) {
                    Type::Struct { id: *id, args }
                } else {
                    Type::Enum { id: *id, args }
                },
                changed,
            )
        }
        Type::Apply { constructor, args } => {
            let args = args
                .iter()
                .map(|arg| nested_requirement(arg, true, function, items))
                .collect::<Vec<_>>();
            let changed = args.iter().any(|(_, changed)| *changed);
            (
                Type::Apply {
                    constructor: constructor.clone(),
                    args: args.into_iter().map(|(ty, _)| ty).collect(),
                },
                changed,
            )
        }
        // References/raw storage and an unbound generic payload cannot supply a
        // stored callable protocol. In particular, do not annotate caller types.
        _ => (ty.clone(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{HirBlockFor, HirGenericBounds, HirParam};
    use crate::ids::{AssocTypeId, CrateId, DefId, HirLocalId, LocalDefId};
    use crate::language_items::FnOnceLanguageItems;
    use crate::types::{
        CallableKind, CallableResultSource as S, GenericParamDecl, GenericParamId, TraitBound,
    };

    #[test]
    fn generic_callable_demands_use_the_definition_subject_and_canonical_protocol_only() {
        let owner = DefId::new(CrateId(0), LocalDefId(80));
        let protocol = DefId::new(CrateId(1), LocalDefId(1));
        let generic = GenericParamId { owner, index: 0 };
        let result = GenericParamId { owner, index: 1 };
        let mut function = HirFunction {
            id: owner,
            name: "invoke".into(),
            generic_params: vec![
                GenericParamDecl::type_param(generic, "F"),
                GenericParamDecl::type_param(result, "T"),
            ],
            generic_bounds: HirGenericBounds::from_bounds(std::collections::HashMap::from([(
                generic,
                vec![TraitBound {
                    trait_id: protocol,
                    type_args: vec![Type::Unit, Type::Generic(result)],
                }],
            )])),
            params: vec![HirParam {
                name: "callback".into(),
                local_id: HirLocalId(0),
                ty: Type::Generic(generic),
                mutable: false,
                is_ref: false,
            }],
            ret_type: Type::Generic(result),
            body: HirBlockFor {
                stmts: vec![],
                ty: Type::Generic(result),
            },
            is_curried: false,
            is_method: false,
            self_receiver: None,
            is_unsafe: false,
        };
        let items = HirLanguageItems {
            fn_once: Some(FnOnceLanguageItems {
                trait_id: protocol,
                output_id: AssocTypeId(0),
                method_id: DefId::new(CrateId(1), LocalDefId(2)),
            }),
            ..Default::default()
        };
        let inputs = generic_inputs(&function, &items);
        assert_eq!(inputs[&0].subject, Type::Generic(generic));
        let Type::Function {
            ret,
            callable_kind,
            result_borrowing,
            ..
        } = &inputs[&0].invocation
        else {
            panic!("canonical invocation");
        };
        assert_eq!(**ret, Type::Generic(result));
        assert_eq!(*callable_kind, CallableKind::FnOnce);
        assert_eq!(
            result_borrowing,
            &CallableResultBorrowing::Sources(Box::new(vec![
                S::EnvironmentValue,
                S::EnvironmentContents,
                S::Static,
                S::Unknown
            ]))
        );
        assert!(generic_inputs(&function, &HirLanguageItems::default()).is_empty());
        function.params[0].is_ref = true;
        assert!(generic_inputs(&function, &items).is_empty());
        function.params[0].is_ref = false;
        function.params[0].ty = Type::function(vec![], Type::I64);
        assert!(
            generic_inputs(&function, &items).is_empty(),
            "a caller-specialized callback type is not a definition-owned Fn subject"
        );

        let span = crate::lexer::Span {
            file_path: "forwarding.rk".into(),
            start: 0,
            end: 1,
        };
        let argument = crate::mono::hir_types::HirExpr {
            kind: crate::mono::hir_types::HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                name: "callback".into(),
                target: crate::hir::HirVarTarget::Local(function.params[0].local_id),
            }),
            ty: function.params[0].ty.clone(),
            span: span.clone(),
        };
        let call = crate::mono::hir_types::HirStmt::Expr(crate::mono::hir_types::HirExpr {
            kind: crate::mono::hir_types::HirExprKind::Call(
                Box::new(crate::mono::hir_types::HirExpr {
                    kind: crate::mono::hir_types::HirExprKind::ResolvedVar(crate::hir::HirVarRef {
                        name: "consumer".into(),
                        target: crate::hir::HirVarTarget::Function(protocol),
                    }),
                    ty: Type::function(vec![function.params[0].ty.clone()], Type::Unit),
                    span: span.clone(),
                }),
                vec![argument.clone()],
                Some(crate::hir::HirCallTarget::Function(protocol)),
            ),
            ty: Type::Unit,
            span,
        });
        for kind in [CallableKind::Fn, CallableKind::FnMut] {
            let Type::Function { callable_kind, .. } = &mut function.params[0].ty else {
                unreachable!()
            };
            *callable_kind = kind;
            function.body.stmts = vec![call.clone()];
            let demands = generic_inputs(&function, &items);
            assert_eq!(demands[&0].subject, function.params[0].ty);
            let Type::Function {
                callable_kind,
                result_borrowing,
                ..
            } = &demands[&0].invocation
            else {
                panic!("forwarded declaration arrow")
            };
            assert_eq!(
                *callable_kind, kind,
                "forwarding cannot widen the declared protocol"
            );
            assert_ne!(*result_borrowing, CallableResultBorrowing::Unknown);
            function.body.stmts.push(call.clone());
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "multiple uses veto the conservative forwarding demand"
            );
            function.body.stmts = vec![
                call.clone(),
                crate::mono::hir_types::HirStmt::Expr(argument.clone()),
            ];
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "exposure cannot borrow another use's demand"
            );
            function.params[0].is_ref = true;
            function.body.stmts = vec![call.clone()];
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "borrowed slots do not own a forwarding demand"
            );
            function.params[0].is_ref = false;
            function.body.stmts = vec![crate::mono::hir_types::HirStmt::Expr(
                crate::mono::hir_types::HirExpr {
                    kind: crate::mono::hir_types::HirExprKind::Loop(
                        crate::mono::hir_types::HirBlock {
                            stmts: vec![call.clone()],
                            ty: Type::Unit,
                        },
                    ),
                    ty: Type::Unit,
                    span: argument.span.clone(),
                },
            )];
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "a loop does not establish once-only forwarding"
            );
            let mut nested =
                crate::mono::hir_types::HirStmt::Expr(crate::mono::hir_types::HirExpr {
                    kind: crate::mono::hir_types::HirExprKind::Lambda {
                        params: vec![function.params[0].clone()],
                        body: crate::mono::hir_types::HirBlock {
                            stmts: vec![call.clone()],
                            ty: Type::Unit,
                        },
                        captures: vec![],
                    },
                    ty: Type::function(vec![function.params[0].ty.clone()], Type::Unit),
                    span: argument.span.clone(),
                });
            function.body.stmts = vec![nested.clone()];
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "recycled nested parameter IDs cannot manufacture a demand"
            );
            function.body.stmts.push(call.clone());
            assert!(
                generic_inputs(&function, &items).contains_key(&0),
                "an unrelated nested parameter cannot veto the outer definition's forwarding use"
            );
            let crate::mono::hir_types::HirStmt::Expr(expression) = &mut nested else {
                unreachable!()
            };
            let crate::mono::hir_types::HirExprKind::Lambda { captures, .. } = &mut expression.kind
            else {
                unreachable!()
            };
            captures.push(crate::hir::HirClosureCapture {
                name: "callback".into(),
                local_id: function.params[0].local_id,
                kind: crate::hir::HirClosureCaptureKind::SharedBorrow,
                mutable: false,
                ty: function.params[0].ty.clone(),
            });
            function.body.stmts = vec![nested, call.clone()];
            assert!(
                generic_inputs(&function, &items).is_empty(),
                "capture vetoes forwarding atomically"
            );
        }
        function.body.stmts.clear();

        let record = DefId::new(CrateId(0), LocalDefId(81));
        function.params[0].ty = Type::Struct {
            id: record,
            args: vec![Type::Generic(generic)],
        };
        let nested = generic_inputs(&function, &items);
        assert_eq!(nested[&0].subject, function.params[0].ty);
        let Type::Struct { id, args } = &nested[&0].invocation else {
            panic!("stored generic protocol")
        };
        assert_eq!(*id, record);
        let Type::Function {
            callable_kind,
            result_borrowing,
            ..
        } = &args[0]
        else {
            panic!("definition-owned callable field")
        };
        assert_eq!(*callable_kind, CallableKind::FnOnce);
        assert_ne!(*result_borrowing, CallableResultBorrowing::Unknown);
        assert!(
            generic_inputs(&function, &HirLanguageItems::default()).is_empty(),
            "the field's shape cannot replace canonical protocol identity"
        );
        function.generic_bounds = HirGenericBounds::default();
        assert!(
            generic_inputs(&function, &items).is_empty(),
            "an unbound generic field has no callable demand"
        );
    }
}
