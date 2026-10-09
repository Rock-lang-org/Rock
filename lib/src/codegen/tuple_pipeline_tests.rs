use super::*;
use inkwell::context::Context;

#[test]
fn checked_factories_dominate_payload_allocations_and_forward_the_same_descriptor() {
    let (program, key) = crate::mir::tuple_factory::tests::fixture();
    let context = Context::create();
    let mut cg = CodeGen::new(&context, "tuple_pipeline");
    cg.set_type_context(program.type_context.clone());
    cg.compile_program_from_mir(&program).unwrap();
    cg.module.verify().unwrap();
    let ir = cg.module.print_to_string().to_string();
    let body = ir.split("define ").find(|body| body.contains("@factory_body(")).unwrap();
    assert!(body.find("tuple.factory.status").unwrap() < body.find("erased.storage =").unwrap());
    assert!(body.lines().any(|line| line.contains("call void @discard_pair(") && line.contains("tuple.factory.descriptor")));
    assert!(body.contains("llvm.trap"));
    assert_eq!(cg.runtime_support().unwrap().len(), 1);
    assert!(program.backend_contract.erased.functions[&key].body.as_ref().unwrap().blocks.iter()
        .flat_map(|block| &block.statements)
        .all(|statement| !matches!(statement.value, crate::mir::MirErasedRvalue::NativeCall { .. } | crate::mir::MirErasedRvalue::NativeApply { .. })));
}

#[test]
fn runtime_is_not_declared_or_link_required_without_a_local_or_import_root() {
    let context = Context::create();
    let mut cg = CodeGen::new(&context, "no_tuple_runtime");
    cg.declare_runtime(&std::collections::BTreeSet::new());
    assert!(cg.module.get_function("rock_descriptor_tuple").is_none());
    assert!(cg.runtime_support().unwrap().is_empty());
}

#[test]
fn mismatched_imported_runtime_target_cannot_be_repaired_by_codegen() {
    let context = Context::create();
    let mut cg = CodeGen::new(&context, "bad_import_target");
    let (mut program, _) = crate::mir::tuple_factory::tests::fixture();
    let mut target = cg.descriptor_target_key().unwrap();
    target[12] = if target[12] == b'x' { b'a' } else { b'x' };
    program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::tuple_descriptors(target));
    cg.set_type_context(program.type_context.clone());
    assert!(cg.compile_program_from_mir(&program).is_err());
}
