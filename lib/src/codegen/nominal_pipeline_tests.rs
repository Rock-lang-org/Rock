use super::*;
use inkwell::context::Context;
use inkwell::targets::TargetData;

#[test]
fn nominal_and_nested_tuple_factories_dominate_storage_and_forward_exact_descriptors() {
    for nested in [false, true] {
        let (program, _, _) = crate::mir::nominal_factory::tests::fixture(nested);
        let context = Context::create(); let mut cg = CodeGen::new(&context, "nominal_pipeline");
        cg.compile_program_from_mir(&program).unwrap(); cg.module.verify().unwrap();
        let ir = cg.module.print_to_string().to_string();
        let body = ir.split("define ").find(|body| body.starts_with("internal void @factory_body(")).unwrap();
        assert!(body.find("nominal.factory.status").unwrap() < body.find("erased.storage =").unwrap());
        assert!(body.lines().any(|line| line.contains("call void @discard_pair(") && line.contains("nominal.factory.descriptor")));
        if nested { assert!(body.find("tuple.factory.status").unwrap() < body.find("nominal.factory.status").unwrap()); }
        let template = cg.module.get_global("rock.nominal.template").unwrap().get_initializer().unwrap().into_struct_value();
        let target = TargetData::create(cg.module.get_data_layout().as_str().to_str().unwrap());
        assert_eq!(target.get_abi_size(&template.get_type()), 24);
        assert_eq!(target.offset_of_element(&template.get_type(), 2), Some(16));
        assert!(cg.module.get_global("rock.nominal.template").unwrap().is_constant());
        assert!(body.contains("erased.field"));
    }
}

#[test]
fn unused_nominal_helper_is_not_declared_and_import_only_root_requires_it() {
    let context = Context::create(); let mut cg = CodeGen::new(&context, "no_nominal");
    cg.declare_runtime(&std::collections::BTreeSet::new());
    assert!(cg.module.get_function("rock_descriptor_nominal").is_none());
    let (mut program, _, _) = crate::mir::nominal_factory::tests::fixture(false);
    program.backend_contract.erased.nominal_factories.clear(); program.backend_contract.erased.functions.clear();
    program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::nominal_descriptors(cg.descriptor_target_key().unwrap()));
    program.backend_contract.runtime_requirements = crate::mir::backend_contract::runtime_requirements_for_backend(&program.backend_contract, program.functions.values());
    cg.compile_program_from_mir(&program).unwrap(); cg.module.verify().unwrap();
    assert!(cg.module.get_function("rock_descriptor_nominal").is_some());
    assert!(cg.module.get_function("rock_descriptor_tuple").is_none());
    assert_eq!(cg.runtime_support().unwrap().len(), 1);
}

#[test]
fn corrupt_nominal_template_and_imported_target_reject_before_emitting_a_factory_body() {
    for target in [false, true] {
        let (mut program, key, _) = crate::mir::nominal_factory::tests::fixture(false);
        let context = Context::create(); let mut cg = CodeGen::new(&context, "bad_nominal");
        if target {
            let mut bytes = cg.descriptor_target_key().unwrap(); bytes[12] ^= 1;
            program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::nominal_descriptors(bytes));
        } else { program.backend_contract.erased.nominal_factories.get_mut(&key).unwrap()[0].template.push(0); }
        assert!(cg.compile_program_from_mir(&program).is_err());
    }
}
