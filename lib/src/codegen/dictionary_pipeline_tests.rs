use super::*;
use inkwell::context::Context;
use inkwell::targets::TargetData;

#[test]
fn selected_environment_precedes_storage_and_calls_the_actual_erased_member() {
    let (program, _, _) = crate::mir::dictionary_factory::tests::fixture();
    let context = Context::create(); let mut cg = CodeGen::new(&context, "dictionary_pipeline");
    cg.compile_program_from_mir(&program).unwrap(); cg.module.verify().unwrap();
    let ir = cg.module.print_to_string().to_string();
    let body = ir.split("define ").find(|body| body.starts_with("internal void @selected_producer(")).unwrap();
    assert_eq!(body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 4);
    assert!(body.find("dictionary.factory.status").unwrap() < body.find("erased.storage =").unwrap());
    assert!(body.lines().any(|line| line.contains("call void @selected_helper(") && line.contains("dictionary.factory.dictionary")));
    let adapter = ir.split("define ").find(|body| body.starts_with("internal void @rock.dictionary.selected.")).unwrap();
    assert!(adapter.contains("dictionary.environment") && adapter.contains("call void @selected_relay_member("));
    assert!(adapter.contains("dictionary.receiver.reference"));
    let member = ir.split("define ").find(|body| body.starts_with("internal void @selected_relay_member(")).unwrap();
    assert!(member.contains("%dictionary.entry = load ptr")
        && member.lines().any(|line| line.trim_start().starts_with("call void %dictionary.entry(") && line.contains("ptr %3,"))
        && !member.contains("call void @selected_helper"), "captured Read dictionary dispatch: {member}");
    let template = cg.module.get_global("rock.dictionary.template.0.30").unwrap();
    let target = TargetData::create(cg.module.get_data_layout().as_str().to_str().unwrap());
    assert_eq!(target.get_abi_size(&cg.erased_dictionary_type()), 128);
    assert_eq!(target.offset_of_element(&cg.erased_dictionary_type(), 8), Some(64));
    assert_eq!(target.offset_of_element(&cg.erased_dictionary_type(), 10), Some(80));
    assert_eq!(target.offset_of_element(&cg.erased_dictionary_type(), 12), Some(96));
    assert_eq!(target.get_abi_size(&template.get_initializer().unwrap().into_struct_value().get_type()), 40);
    assert!(template.is_constant());
}

#[test]
fn dictionary_runtime_declaration_is_conditional_and_import_only_root_survives() {
    let context = Context::create(); let mut cg = CodeGen::new(&context, "dictionary_root");
    cg.declare_runtime(&std::collections::BTreeSet::new());
    assert!(cg.module.get_function("rock_dictionary_selected").is_none());
    let (mut program, _, _) = crate::mir::dictionary_factory::tests::fixture();
    program.backend_contract.erased.functions.clear(); program.backend_contract.erased.dictionary_templates.clear();
    program.backend_contract.erased.dictionary_factories.clear();
    program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::selected_dictionaries(cg.descriptor_target_key().unwrap()));
    program.backend_contract.runtime_requirements = crate::mir::backend_contract::runtime_requirements_for_backend(&program.backend_contract, program.functions.values());
    cg.compile_program_from_mir(&program).unwrap(); cg.module.verify().unwrap();
    assert!(cg.module.get_function("rock_dictionary_selected").is_some());
    assert!(cg.module.get_function("rock_descriptor_tuple").is_none());
    assert_eq!(cg.runtime_support().unwrap().len(), 1);
}

#[test]
fn forged_template_and_wrong_import_target_reject_before_factory_emission() {
    for wrong_target in [false, true] {
        let (mut program, _, implementation) = crate::mir::dictionary_factory::tests::fixture();
        let context = Context::create(); let mut cg = CodeGen::new(&context, "dictionary_bad");
        if wrong_target {
            let mut target = cg.descriptor_target_key().unwrap(); target[12] ^= 1;
            program.backend_contract.erased.imported_runtime_support.insert(crate::products::ProductRuntimeSupport::selected_dictionaries(target));
            program.backend_contract.runtime_requirements = crate::mir::backend_contract::runtime_requirements_for_backend(&program.backend_contract, program.functions.values());
        } else { program.backend_contract.erased.dictionary_templates.get_mut(&implementation).unwrap().members[0].method_id.local.0 += 1; }
        assert!(cg.compile_program_from_mir(&program).is_err());
    }
}
