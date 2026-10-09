//! Constructor-owned split/convert/rebuild, retaining ordinary call authority.
use crate::hir::*;
use crate::ids::{DefId, HirLocalId};
use crate::selection::{type_pattern_matches, SelectionService};
use crate::types::{FunctionSafety, TraitBound, Type};

pub(crate) fn is_owner_destination(ty: &Type) -> bool {
    // This only defers a candidate coercion; the constructor impl determines
    // which slot is the payload and must prove split/rebuild before admission.
    matches!(ty, Type::Struct { args, .. } if args.iter().any(|arg| {
        crate::type_services::visit::type_any(arg, |nested| matches!(nested, Type::Object(_)))
    }))
}

pub(crate) fn select_operation(
    service: &SelectionService<'_>,
    constructor: &Type,
    trait_id: DefId,
    state: &[Type],
    member: DefId,
    instantiated: &Type,
) -> Result<(HirFunction, HirStaticMethodTarget), String> {
    let mut selected = service
        .select_constructor_trait_member(constructor, trait_id, state, member)
        .map_err(|error| error.message())?;
    if !selected.function.is_unsafe || selected.function.self_receiver.is_some() {
        return Err("owner operation must be an unsafe static method".into());
    }
    let template = Type::function_with_safety(
        selected
            .function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        selected.function.ret_type.clone(),
        FunctionSafety::Unsafe,
    );
    let mut substitution = selected.owner_substitution.clone();
    if !type_pattern_matches(&template, instantiated, &mut substitution) {
        return Err(
            "owner implementation signature does not match its canonical transport contract".into(),
        );
    }
    if selected.pending_impl_bounds.iter().any(|(subject, bound)| {
        let bound = crate::types::TraitBound {
            trait_id: bound.trait_id,
            type_args: bound
                .type_args
                .iter()
                .map(|ty| ty.substitute_generics(&substitution))
                .collect(),
        };
        !service.trait_bound_satisfied(&subject.substitute_generics(&substitution), &bound)
    }) {
        return Err("owner implementation has unproven obligations".into());
    }
    for parameter in &selected.function.generic_params {
        if selected
            .target
            .owner_substitution
            .iter()
            .any(|binding| binding.param == parameter.id)
        {
            continue;
        }
        if parameter.kind != crate::type_services::kind::Kind::Type
            || (!parameter.maybe_unsized
                && !selected
                    .function
                    .generic_bounds
                    .relaxed_sized
                    .contains(&parameter.id))
            || selected
                .function
                .generic_bounds
                .get(&parameter.id)
                .is_some_and(|bounds| !bounds.is_empty())
        {
            return Err("owner operation implementation must preserve the unrestricted ?Sized payload contract".into());
        }
        let ty = substitution
            .get(&parameter.id)
            .ok_or("unresolved owner method parameter")?
            .clone();
        selected.target.method_substitution.push(HirTypeBinding {
            param: parameter.id,
            ty,
        });
    }
    selected
        .target
        .method_substitution
        .sort_by_key(|binding| binding.param);
    Ok((
        selected.function,
        HirStaticMethodTarget {
            owner_ty: constructor.clone(),
            method: selected.target,
        },
    ))
}

pub(crate) fn lower(
    value: HirExpr,
    target: &Type,
    service: &SelectionService<'_>,
    language_items: &HirLanguageItems,
    local_id: HirLocalId,
) -> Result<HirExpr, String> {
    let Type::Struct { id, .. } = target else {
        return Err("owner destination must be a nominal owner".into());
    };
    let Type::Struct { id: source_id, .. } = &value.ty else {
        return Err("owner coercion requires an owned source".into());
    };
    if source_id != id {
        return Err("owner coercion must preserve the exact owner constructor".into());
    }
    let protocol = language_items
        .object_owner
        .as_ref()
        .ok_or("owner coercion requires an explicitly provided object_owner protocol")?;
    let (constructor, source, state) = service
        .infer_unary_constructor_application(&value.ty, protocol.trait_id)
        .map_err(|error| error.message())?
        .ok_or(
            "owner coercion requires one proven constructor implementation with independent State",
        )?;
    let (destination_constructor, destination, destination_state) = service
        .infer_unary_constructor_application(target, protocol.trait_id)
        .map_err(|error| error.message())?
        .ok_or("owner destination requires a proven constructor application")?;
    if constructor != destination_constructor || state != destination_state {
        return Err("owner coercion must preserve the exact owner constructor and State".into());
    }
    if !matches!(&destination, Type::Object(_)) {
        return Err("owner coercion must convert the constructor's payload slot".into());
    }
    let [state_ty] = state.as_slice() else {
        return Err("invalid object_owner State arity".into());
    };
    if crate::type_services::visit::type_any(state_ty, |ty| {
        matches!(
            ty,
            Type::TypeVar(_) | Type::Error | Type::ObjectSelf { .. } | Type::Witness(_)
        )
    }) || !language_items.sized.as_ref().map_or_else(
        || {
            crate::type_services::layout::TypeLayout::sizedness(state_ty, &|_| {
                crate::type_services::layout::Sizedness::Unknown
            }) == crate::type_services::layout::Sizedness::Sized
        },
        |sized| {
            service.trait_bound_satisfied(
                state_ty,
                &TraitBound {
                    trait_id: sized.trait_id,
                    type_args: Vec::new(),
                },
            )
        },
    ) {
        return Err(
            "owner State must be a uniquely determined sized type independent of its payload"
                .into(),
        );
    }
    let span = value.span.clone();
    let parts_ty = Type::Tuple(vec![
        Type::Pointer(Box::new(source.clone())),
        state_ty.clone(),
    ]);
    let pointer_ty = Type::Pointer(Box::new(destination));
    let operation =
        |member, params: Vec<Type>, ret: Type| -> Result<(HirExpr, HirCallTarget), String> {
            let instantiated = Type::function_with_safety(params, ret, FunctionSafety::Unsafe);
            let (function, authority) = select_operation(
                service,
                &constructor,
                protocol.trait_id,
                &state,
                member,
                &instantiated,
            )?;
            let callee = HirExpr {
                ty: instantiated,
                span: span.clone(),
                kind: HirExprKind::ResolvedVar(HirVarRef {
                    name: function.name,
                    target: HirVarTarget::Function(function.id),
                }),
            };
            Ok((callee, HirCallTarget::StaticMethod(authority)))
        };
    let (into, into_target) = operation(
        protocol.into_parts_id,
        vec![value.ty.clone()],
        parts_ty.clone(),
    )?;
    let (from, from_target) = operation(
        protocol.from_parts_id,
        vec![pointer_ty.clone(), state_ty.clone()],
        target.clone(),
    )?;
    let parts = HirExpr {
        ty: parts_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::ResolvedVar(HirVarRef {
            name: "<owner-parts>".into(),
            target: HirVarTarget::Local(local_id),
        }),
    };
    let pointer = HirExpr {
        ty: Type::Pointer(Box::new(source)),
        span: span.clone(),
        kind: HirExprKind::TupleIndex(Box::new(parts.clone()), 0),
    };
    let converted = HirExpr {
        ty: pointer_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::ObjectCoercion(
            Box::new(pointer),
            HirObjectCoercion {
                target: pointer_ty,
                evidence: None,
            },
        ),
    };
    let state_value = HirExpr {
        ty: state_ty.clone(),
        span: span.clone(),
        kind: HirExprKind::TupleIndex(Box::new(parts), 1),
    };
    let body = HirBlock {
        ty: target.clone(),
        stmts: vec![
            HirStmt::Let {
                name: "<owner-parts>".into(),
                local_id,
                ty: parts_ty.clone(),
                mutable: false,
                value: HirExpr {
                    ty: parts_ty,
                    span: span.clone(),
                    kind: HirExprKind::Call(Box::new(into), vec![value], Some(into_target)),
                },
            },
            HirStmt::Expr(HirExpr {
                ty: target.clone(),
                span: span.clone(),
                kind: HirExprKind::Call(
                    Box::new(from),
                    vec![converted, state_value],
                    Some(from_target),
                ),
            }),
        ],
    };
    Ok(HirExpr {
        ty: target.clone(),
        span,
        kind: HirExprKind::UnsafeBlock(body),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::type_services::kind::Kind;

    fn source(payload_position: usize, caller_first: bool) -> String {
        let args = |payload: &str, allocator: &str| {
            let mut args = vec![allocator, "I64"];
            args.insert(payload_position, payload);
            args.join(", ")
        };
        let declarations = format!(r#"
lang sized
< trait LayoutKnown
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?LayoutKnown
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?LayoutKnown
trait Read
    @read: I64
trait Rich where Self: Read
    @marker: I64
struct Payload
    < value: I64
impl Read for Payload
    @read = -> self.value
impl Rich for Payload
    @marker = -> 7
trait Memory
    @key: I64
struct Small
    < tag: I64
impl Memory for Small
    @key = -> self.tag
struct Large
    < padding: (I64, I64)
    < tag: I64
impl Memory for Large
    @key = -> self.tag
struct State A
    < allocator: A
struct Shell {header}
    < pointer: *T
    < state: State A
    < marker: B
impl Transport (State A) for Shell {section} where A: Memory
    unsafe split = owner -> (owner.pointer, owner.state)
    unsafe rebuild = pointer, state -> Shell
        pointer: pointer
        state: state
        marker: 0
erase: Shell {erase_source} -> Shell {erase_target} where A: Memory
erase = owner -> owner
small = -> Shell
    pointer: 0 as *Payload
    state: State
        allocator: Small
            tag: 1
    marker: 0
large = -> Shell
    pointer: 0 as *Payload
    state: State
        allocator: Large
            padding: (2, 3)
            tag: 4
    marker: 0
"#,
            header = {
                let mut header = vec!["A", "B"];
                header.insert(payload_position, "(T: ?LayoutKnown)");
                header.join(", ")
            },
            section = args("_", "A"),
            erase_source = args("Payload", "A"),
            erase_target = args("Rich", "A"),
        );
        let upcast = format!("upcast: Shell {} -> Shell {} where A: Memory\nupcast = owner -> owner\n",
            args("Rich", "A"), args("Read", "A"));
        let callers = "run_small = ->\n    owner = small!\n    rich = erase owner\n    upcast rich\nrun_large = ->\n    owner = large!\n    rich = erase owner\n    upcast rich\nmain = !-> ()\n";
        if caller_first { format!("{declarations}{callers}{upcast}") }
        else { format!("{declarations}{upcast}{callers}") }
    }

    fn config(source: &str) -> crate::Config {
        let path = PathBuf::from("/virtual/allocator_upcast_authority.rk");
        crate::Config {
            entry_file: path.clone(), no_std: true, no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual { path, text: source.into() }],
            ..crate::Config::default()
        }
    }

    fn partial(source: &str) -> crate::infer::PartialHir {
        let config = config(source);
        let module = crate::parser::parse_source(config.entry_file.clone(), source, &config).unwrap();
        crate::lower::Lowerer::with_options(false).lower_program(&crate::ast::Program { module }).unwrap()
    }

    #[test]
    fn owner_upcast_retains_actual_allocator_and_nonterminal_payload_authority() {
        for position in 0..3 {
            for caller_first in [false, true] {
                let source = source(position, caller_first);
                let analysis = crate::analyze(&config(&source)).unwrap_or_else(|errors| panic!("{errors:?}"));
                let program = &analysis.hir.program;
                let shell = program.structs.values().find(|definition| definition.name == "Shell").unwrap();
                let state = program.structs.values().find(|definition| definition.name == "State").unwrap();
                let read = program.traits.values().find(|definition| definition.name == "Read").unwrap();
                let protocol = program.language_items.object_owner.as_ref().unwrap();
                for (caller, allocator) in [("run_small", "Small"), ("run_large", "Large")] {
                    let function = program.functions.values().find(|function| function.name == caller).unwrap();
                    let allocator = program.structs.values().find(|definition| definition.name == allocator).unwrap();
                    let Type::Struct { id, args } = &function.ret_type else { panic!("{function:?}") };
                    assert_eq!(*id, shell.id);
                    assert_eq!(args.len(), 3);
                    let Type::Object(payload) = &args[position] else { panic!("{args:?}") };
                    assert_eq!(payload.principal.trait_id, read.id);
                    let mut fixed = args.clone();
                    fixed.remove(position);
                    assert_eq!(fixed, vec![Type::Struct { id: allocator.id, args: Vec::new() }, Type::I64]);
                }
                let upcast = program.functions.values().find(|function| function.name == "upcast").unwrap();
                let memory = program.traits.values().find(|definition| definition.name == "Memory").unwrap();
                let allocator = upcast.generic_params[0].id;
                assert!(upcast.generic_bounds[&allocator].iter().any(|bound| bound.trait_id == memory.id));
                let crate::hir::HirStmtFor::Expr(converted) = &upcast.body.stmts[0] else { unreachable!() };
                let crate::hir::HirExprKindFor::Block(body) = &converted.kind else { panic!("{converted:?}") };
                let crate::hir::HirStmtFor::Let { value: split, .. } = &body.stmts[0] else { unreachable!() };
                let crate::hir::HirStmtFor::Expr(rebuild) = &body.stmts[1] else { unreachable!() };
                let mut constructor = None;
                let expected_state = Type::Struct { id: state.id, args: vec![Type::Generic(allocator)] };
                for (call, member) in [(split, protocol.into_parts_id), (rebuild, protocol.from_parts_id)] {
                    let crate::hir::HirExprKindFor::Call(_, _, Some(HirCallTarget::StaticMethod(target))) = &call.kind else { panic!("{call:?}") };
                    assert_eq!(target.method.trait_id(), Some(protocol.trait_id));
                    assert!(target.method.impl_id().is_some());
                    let HirSelectedMethodTarget::ImplMethod { selected_trait: Some(selected), .. } = &target.method.target else { unreachable!() };
                    assert_eq!(selected.member_id, member);
                    assert_eq!(selected.trait_args, vec![expected_state.clone()]);
                    let Type::Lambda { params, body } = &target.owner_ty else { panic!("{target:?}") };
                    assert_eq!(params, &vec![Kind::Type]);
                    let Type::Struct { id, args } = body.as_ref() else { unreachable!() };
                    assert_eq!(*id, shell.id);
                    assert_eq!(args[position], Type::BoundVar { depth: 0, index: 0, kind: Kind::Type });
                    let mut fixed = args.clone();
                    fixed.remove(position);
                    assert_eq!(fixed, vec![Type::Generic(allocator), Type::I64]);
                    if let Some(previous) = &constructor { assert_eq!(&target.owner_ty, previous); }
                    constructor = Some(target.owner_ty.clone());
                }
            }
        }
    }

    #[test]
    fn owner_relation_separate_fixed_slots_wait_for_real_state_evidence() {
        for position in 0..3 {
            for known_end in 0..3 {
                let mut hir = partial(&source(position, true));
                let upcast = hir.functions.values().find(|function| function.name == "upcast").unwrap();
                let parameter = upcast.generic_params[0].id;
                let source_template = upcast.params[0].ty.clone();
                let target_template = upcast.ret_type.clone();
                let small = hir.structs.values().find(|definition| definition.name == "Small").unwrap().id;
                let actual = Type::Struct { id: small, args: Vec::new() };
                let source_slot = hir.engine.fresh_type_var();
                let target_slot = hir.engine.fresh_type_var();
                if known_end == 0 { hir.engine.unify(&source_slot, &actual).unwrap(); }
                if known_end == 1 { hir.engine.unify(&target_slot, &actual).unwrap(); }
                let source = source_template.substitute_generics(&HashMap::from([(parameter, source_slot.clone())]));
                let target = target_template.substitute_generics(&HashMap::from([(parameter, target_slot.clone())]));
                let protocol = hir.language_items.object_owner.as_ref().unwrap();
                let bounds = HirGenericBounds::new();
                let service = SelectionService::new(&hir.traits, &hir.impls,
                    hir.language_items.sized.as_ref().map(|items| items.trait_id), None, &bounds);
                if known_end == 0 {
                    assert!(service.infer_unary_constructor_application(&hir.engine.resolve(&source), protocol.trait_id).unwrap().is_some());
                    assert!(service.infer_unary_constructor_application(&hir.engine.resolve(&target), protocol.trait_id).unwrap().is_none());
                }
                let (source_constructor, target_constructor) = service.unary_constructor_relation(
                    &hir.engine.resolve(&source), &hir.engine.resolve(&target), protocol.trait_id,
                ).unwrap().unwrap();
                let implementation = hir.impls.values().find(|implementation| implementation.trait_id == Some(protocol.trait_id)).unwrap();
                let (_, source_bindings) = crate::selection::constructor_target_from_applied_type(
                    &implementation.receiver_pattern, &hir.engine.resolve(&source),
                ).unwrap();
                let (_, target_bindings) = crate::selection::constructor_target_from_applied_type(
                    &implementation.receiver_pattern, &hir.engine.resolve(&target),
                ).unwrap();
                let source_obligations = implementation.bounds.predicates.iter().map(|predicate| predicate.substitute_generics(&source_bindings)).collect::<Vec<_>>();
                let target_obligations = implementation.bounds.predicates.iter().map(|predicate| predicate.substitute_generics(&target_bindings)).collect::<Vec<_>>();
                let source_state = implementation.trait_arg_types.iter().map(|ty| ty.substitute_generics(&source_bindings)).collect::<Vec<_>>();
                let target_state = implementation.trait_arg_types.iter().map(|ty| ty.substitute_generics(&target_bindings)).collect::<Vec<_>>();
                eprintln!("owner fixed-slot relation: impl={:?}, pattern={:?}, source={:?}, target={:?}, source_constructor={source_constructor:?}, target_constructor={target_constructor:?}, source_bindings={source_bindings:?}, target_bindings={target_bindings:?}, source_obligations={source_obligations:?}, target_obligations={target_obligations:?}, source_state={source_state:?}, target_state={target_state:?}, context={bounds:?}", implementation.id, implementation.receiver_pattern, hir.engine.resolve(&source), hir.engine.resolve(&target));
                hir.engine.unify(&source_constructor, &target_constructor).unwrap();
                if known_end == 2 {
                    assert!(service.infer_unary_constructor_application(&hir.engine.resolve(&source), protocol.trait_id).unwrap().is_none());
                    assert!(service.infer_unary_constructor_application(&hir.engine.resolve(&target), protocol.trait_id).unwrap().is_none());
                    hir.engine.unify(&source_slot, &actual).unwrap();
                }
                let (left, source_payload, state) = service.infer_unary_constructor_application(
                    &hir.engine.resolve(&source), protocol.trait_id,
                ).unwrap().unwrap();
                let (right, target_payload, target_state) = service.infer_unary_constructor_application(
                    &hir.engine.resolve(&target), protocol.trait_id,
                ).unwrap().unwrap();
                assert_eq!(left, right);
                assert_eq!(state, target_state);
                assert_ne!(source_payload, target_payload);
                assert_eq!(hir.engine.resolve(&source_slot), actual);
                assert_eq!(hir.engine.resolve(&target_slot), actual);
                assert!(!crate::type_services::visit::type_any(&state[0], |ty| matches!(ty, Type::TypeVar(_) | Type::Error | Type::Witness(_) | Type::ObjectSelf { .. })));
                let applied = Type::Apply { constructor: Box::new(left), args: vec![source_payload] };
                assert_eq!(hir.engine.normalize_resolved_type(&applied).unwrap(), hir.engine.resolve(&source));
            }
        }
    }

    #[test]
    fn owner_relation_never_supplies_missing_or_foreign_contextual_bounds() {
        let hir = partial(&source(1, false));
        let upcast = hir.functions.values().find(|function| function.name == "upcast").unwrap();
        let erase = hir.functions.values().find(|function| function.name == "erase").unwrap();
        let parameter = upcast.generic_params[0].id;
        let foreign = erase.generic_params[0].id;
        let source = &upcast.params[0].ty;
        let target = &upcast.ret_type;
        let protocol = hir.language_items.object_owner.as_ref().unwrap();
        let sized = hir.language_items.sized.as_ref().unwrap().trait_id;
        let scoped = |parameter| {
            let mut bounds = HirGenericBounds::new();
            bounds.insert(parameter, upcast.generic_bounds[&upcast.generic_params[0].id].clone());
            bounds.entry(parameter).or_default().push(TraitBound { trait_id: sized, type_args: Vec::new() });
            bounds
        };
        for bounds in [HirGenericBounds::new(), scoped(foreign)] {
            let service = SelectionService::new(&hir.traits, &hir.impls, Some(sized), None, &bounds);
            let (left, right) = service.unary_constructor_relation(source, target, protocol.trait_id).unwrap().unwrap();
            assert_eq!(left, right);
            assert!(service.infer_unary_constructor_application(source, protocol.trait_id).unwrap().is_none());
            assert!(service.infer_unary_constructor_application(target, protocol.trait_id).unwrap().is_none());
        }
        let bounds = scoped(parameter);
        let service = SelectionService::new(&hir.traits, &hir.impls, Some(sized), None, &bounds);
        let (left, _, state) = service.infer_unary_constructor_application(source, protocol.trait_id).unwrap().unwrap();
        let (right, _, destination_state) = service.infer_unary_constructor_application(target, protocol.trait_id).unwrap().unwrap();
        assert_eq!(left, right);
        assert_eq!(state, destination_state);

        let mut wrong_state = hir.impls.clone();
        let implementation = wrong_state.values_mut().find(|implementation| implementation.trait_id == Some(protocol.trait_id)).unwrap();
        let Type::Struct { args, .. } = &mut implementation.trait_arg_types[0] else { unreachable!() };
        args[0] = Type::Generic(foreign);
        let service = SelectionService::new(&hir.traits, &wrong_state, Some(sized), None, &bounds);
        assert!(service.unary_constructor_relation(source, target, protocol.trait_id).unwrap().is_some());
        assert!(service.infer_unary_constructor_application(source, protocol.trait_id).unwrap().is_none());
        assert!(service.infer_unary_constructor_application(target, protocol.trait_id).unwrap().is_none());
    }

    #[test]
    fn owner_relation_rejects_ambiguous_impls_and_different_fixed_allocators() {
        let mut hir = partial(&source(1, false));
        let upcast = hir.functions.values().find(|function| function.name == "upcast").unwrap();
        let parameter = upcast.generic_params[0].id;
        let actual = |name| Type::Struct {
            id: hir.structs.values().find(|definition| definition.name == name).unwrap().id,
            args: Vec::new(),
        };
        let source = upcast.params[0].ty.substitute_generics(&HashMap::from([(parameter, actual("Small"))]));
        let target = upcast.ret_type.substitute_generics(&HashMap::from([(parameter, actual("Large"))]));
        let protocol = hir.language_items.object_owner.as_ref().unwrap();
        let bounds = HirGenericBounds::new();
        let service = SelectionService::new(&hir.traits, &hir.impls, None, None, &bounds);
        let (left, right) = service.unary_constructor_relation(&source, &target, protocol.trait_id).unwrap().unwrap();
        assert_ne!(left, right);
        assert!(hir.engine.unify(&left, &right).is_err());
        let mut overlapping = hir.impls.clone();
        let mut duplicate = overlapping.values().find(|implementation| implementation.trait_id == Some(protocol.trait_id)).unwrap().clone();
        duplicate.id = DefId::new(duplicate.id.crate_id, crate::ids::LocalDefId(duplicate.id.local.0 + 1000));
        overlapping.insert(duplicate.id, duplicate);
        let service = SelectionService::new(&hir.traits, &overlapping, None, None, &bounds);
        assert!(matches!(service.unary_constructor_relation(&source, &target, protocol.trait_id),
            Err(crate::selection::SelectionDiagnostic::AmbiguousCandidates { candidates, .. }) if candidates.len() == 2));
    }
}
