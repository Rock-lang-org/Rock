use super::*;
use inkwell::context::Context;
use inkwell::targets::TargetData;

#[test]
fn polymorphic_dictionary_adapter_passes_local_evidence_to_one_genuine_erased_member() {
    let (program, _, _) = crate::mir::polymorphic_dictionary::tests::fixture();
    let context = Context::create(); let mut cg = CodeGen::new(&context, "polymorphic_dictionary");
    cg.compile_program_from_mir(&program).unwrap(); cg.module.verify().unwrap();
    let ir = cg.module.print_to_string().to_string();
    let helper = ir.split("define ").find(|body| body.starts_with("internal void @selected_helper(")).unwrap().split_once("\n}").unwrap().0;
    let call = helper.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
    assert_eq!(call.split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 5,
        "out/env/receiver/actual U descriptor/value; no U-specific slot");
    let adapter = ir.split("define ").find(|body| body.starts_with("internal void @rock.dictionary.selected.")).unwrap().split_once("\n}").unwrap().0;
    assert_eq!(adapter.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count(), 5);
    assert!(adapter.lines().any(|line| line.contains("call void @selected_relay_member(") && line.contains("ptr %3,")), "per-call U reaches the actual body: {adapter}");
    let body = ir.split("define ").find(|body| body.starts_with("internal void @selected_relay_member(")).unwrap().split_once("\n}").unwrap().0;
    assert!(body.contains("%dictionary.entry = load ptr") && body.lines().any(|line| line.trim_start().starts_with("call void %dictionary.entry(")), "actual captured Read runs inside the generic member");
    let target = TargetData::create(cg.module.get_data_layout().as_str().to_str().unwrap());
    assert_eq!(target.get_abi_size(&cg.erased_dictionary_type()), 128);
    assert_eq!(target.offset_of_element(&cg.erased_dictionary_type(), 10), Some(80));
    assert_eq!(target.get_abi_size(&cg.erased_descriptor_type()), 160);
    assert_eq!(target.offset_of_element(&cg.erased_descriptor_type(), 13), Some(104));
}

#[test]
fn polymorphic_dictionary_wrong_kind_and_authority_reject_before_emission() {
    for corrupt_kind in [false, true] {
        let (mut program, _, implementation) = crate::mir::polymorphic_dictionary::tests::fixture();
        if corrupt_kind {
            program.backend_contract.erased.dictionary_templates.get_mut(&implementation).unwrap().result.members[0].binder.parameters[0].kind = crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type, crate::type_services::kind::Kind::Type);
        } else { program.backend_contract.dictionary_member_authorities.clear(); }
        let context = Context::create(); let mut cg = CodeGen::new(&context, "polymorphic_dictionary_bad");
        assert!(cg.compile_program_from_mir(&program).is_err());
        assert!(!cg.get_ir().contains("dictionary.polymorphic.invoke"));
    }
}
