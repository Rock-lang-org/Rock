use super::*;
use crate::types::ArrayLen;

fn symbolic_lengths() -> Vec<ArrayLen> {
    vec![
        ArrayLen::Error,
        ArrayLen::Var(crate::ids::TypeVarId(123)),
        ArrayLen::Param(GenericParamId { owner: DefId::new(CrateId(0), LocalDefId(1)), index: 0 }),
        ArrayLen::Bound { depth: 0, index: 0 },
    ]
}

fn function_with_array(len: ArrayLen, elem: Type) -> HirFunction {
    let mut function = empty_function(DefId::new(CrateId(0), LocalDefId(1)), "invalid");
    function.params.push(HirParam {
        name: "arr".to_string(), local_id: crate::ids::HirLocalId(0),
        ty: Type::Array(Box::new(elem), len), mutable: false, is_ref: false,
    });
    function
}

fn build_invalid(function: HirFunction, cleanup: bool) -> Result<MirFunction, crate::diagnostic::Diagnostics> {
    let mut program = empty_program();
    program.functions.insert(function.id, function.clone());
    let types = test_type_context(&program);
    let mut builder = MirBuilder::new(&program, &types);
    if cleanup {
        builder.direct_drop_types.insert(struct_ty(0));
    }
    builder.build_function(MirFunctionId::Function(function.id), "invalid", &function, false, Some(function.id), None)
}

#[test]
fn h3_array_drop_reports_symbolic_length_instead_of_omitting_cleanup() {
    for len in symbolic_lengths() {
        let result = build_invalid(function_with_array(len, struct_ty(0)), true);
        assert!(result.is_err(), "{len:?}");
        assert!(result.unwrap_err().contains("non-concrete array length"));
    }
}

#[test]
fn h3_array_for_reports_symbolic_length_instead_of_building_empty_loop() {
    for len in symbolic_lengths() {
        let mut function = function_with_array(len, Type::I64);
        function.body.stmts.push(HirStmt::Expr(expr(HirExprKind::For {
            var: "item".to_string(), local_id: crate::ids::HirLocalId(1),
            iter: Box::new(expr(HirExprKind::Var("arr".to_string()), function.params[0].ty.clone())),
            body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
        }, Type::Unit)));
        let diagnostics = build_invalid(function, false).unwrap_err();
        assert!(diagnostics.contains("non-concrete array length"));
        assert_eq!(diagnostics.0[0].location, crate::diagnostic::DiagnosticLocation::Source(span()));
    }
}

#[test]
fn h3_repeat_and_nested_closure_errors_reach_mir_build_boundary() {
    for nested in [false, true] {
        let mut program = empty_program();
        let mut function = empty_function(DefId::new(CrateId(0), LocalDefId(1)), "invalid");
        let mut value = expr(HirExprKind::ArrayRepeat(Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)), ArrayLen::Error), Type::Array(Box::new(Type::I64), ArrayLen::Error));
        if nested {
            value = expr(HirExprKind::Lambda {
                params: Vec::new(), captures: Vec::new(),
                body: HirBlock { stmts: vec![HirStmt::Expr(value)], ty: Type::Unit },
            }, Type::function(Vec::new(), Type::Unit));
        }
        function.body.stmts.push(HirStmt::Expr(value));
        program.functions.insert(function.id, function.clone());
        program.rebuild_indexes_with_canonical_names(&std::collections::HashMap::from([(function.id, function.name)]));
        let diagnostics = MirBuilder::build(&program).unwrap_err();
        assert!(diagnostics.contains("non-concrete array length"));
    }
}

#[test]
fn h3_instance_build_errors_preserve_bodies_and_cannot_be_retried_as_success() {
    let mut function = empty_function(DefId::new(CrateId(0), LocalDefId(1)), "invalid");
    function.body.stmts.push(HirStmt::Expr(expr(HirExprKind::ArrayRepeat(
        Box::new(expr(HirExprKind::IntLiteral(1), Type::I64)), ArrayLen::Error,
    ), Type::Array(Box::new(Type::I64), ArrayLen::Error))));
    let instance = InstanceId(1);
    let mut bodies = crate::mono::PreMirInstanceBodies::new();
    bodies.insert(instance, function.clone());
    let mut mono = crate::mono::MonomorphizedProgram {
        program: empty_program(),
        instances: std::collections::BTreeMap::from([(instance, crate::mono::InstanceRecord::new(
            instance, crate::mono::InstanceOrigin::Function(function.id), Vec::new(),
            crate::mono::InstanceSymbols::new("invalid", "invalid"), None, false, false,
        ))]),
        pre_mir_instance_bodies: bodies,
        generated_drop_instances: Default::default(),
        type_context: crate::type_context::TypeContext::new(),
        source_map: Default::default(),
    };
    for _ in 0..2 {
        assert!(MirBuilder::take_mir_instance_bodies(&mut mono).is_err());
        assert!(mono.pre_mir_instance_bodies.get(instance).is_some());
    }
    assert!(MirBuilder::build_monomorphized(&mono).is_err());
}

#[test]
fn h3_slice_loop_uses_runtime_length_and_invalid_iterators_are_diagnosed() {
    for ty in [Type::Slice(Box::new(Type::I64)), Type::I64] {
        let mut function = function_with_array(ArrayLen::Value(1), Type::I64);
        function.params[0].ty = ty.clone();
        function.body.stmts.push(HirStmt::Expr(expr(HirExprKind::For {
            var: "item".to_string(), local_id: crate::ids::HirLocalId(1),
            iter: Box::new(expr(HirExprKind::Var("arr".to_string()), ty.clone())),
            body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
        }, Type::Unit)));
        let result = build_invalid(function, false);
        if matches!(ty, Type::Slice(_)) {
            let function = result.unwrap();
            let length = function.basic_blocks.iter().find_map(|block| match &block.terminator {
                Some(Terminator::Call { func: Operand::Constant(Constant::Callable(MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::ArrayLen)))), destination, .. }) => Some(destination.local),
                _ => None,
            }).unwrap();
            assert!(function.basic_blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                matches!(&statement.kind, StatementKind::Assign(_, Rvalue::BinaryOp(crate::mir::MirBinOp::Lt, _, Operand::Copy(place))) if place.local == length)
            }));
        } else {
            assert!(result.unwrap_err().contains("cannot lower for-loop iterator type"));
        }
    }
}
