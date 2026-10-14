use super::*;
use crate::mir::descriptor_identity::descriptor_identity;
use crate::type_context::TypeContext;
use crate::types::Type;
use inkwell::context::Context;
use inkwell::targets::TargetData;

#[test]
fn native_and_uniform_descriptor_suffixes_match_the_native_c_abi() {
    let context = Context::create();
    let mut cg = CodeGen::new(&context, "descriptor_abi");
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let tuple = tc.intern_type(&Type::Tuple(vec![Type::I64, Type::I64]));
    let mut contract = MirBackendContract::default();
    let child = payload_drop_plan(&contract, &mut tc, scalar).unwrap();
    let parent = payload_drop_plan(&contract, &mut tc, tuple).unwrap();
    for (ty, plan, fields) in [(scalar, child, vec![]), (tuple, parent, vec![scalar, scalar])] {
        let identity = descriptor_identity(&contract, &tc, &plan).unwrap();
        contract.descriptor_identities.insert(ty, identity);
        contract.erased.descriptors.insert(ty, MirTypeDescriptor { ty, drop: plan, fields });
    }
    cg.set_type_context(tc);
    cg.emit_erased_program(&contract).unwrap();
    let descriptor_ty = cg.erased_descriptor_type();
    assert_eq!(descriptor_ty.count_fields(), 20);
    let data = TargetData::create(cg.module.get_data_layout().as_str().to_str().unwrap());
    assert_eq!(data.get_abi_size(&descriptor_ty), 160);
    assert_eq!(data.offset_of_element(&descriptor_ty, 9), Some(72));
    assert_eq!(data.offset_of_element(&descriptor_ty, 13), Some(104));
    cg.emit_uniform_descriptor(&MirUniformLayout::Tuple(Vec::new()), "shape_only").unwrap();
    let shape = cg.module.get_global("shape_only").unwrap().get_initializer().unwrap().into_struct_value();
    assert_eq!(shape.get_field_at_index(6).unwrap().into_int_value().get_zero_extended_constant(), Some(0));
    assert!(shape.get_field_at_index(9).unwrap().into_pointer_value().is_null());
    let native = cg.erased_descriptors[&tuple].get_initializer().unwrap().into_struct_value();
    assert_eq!(native.get_field_at_index(6).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
    assert_eq!(native.get_field_at_index(12).unwrap().into_int_value().get_zero_extended_constant(), Some(2));
    assert!(!native.get_field_at_index(9).unwrap().into_pointer_value().is_null());
    assert!(!native.get_field_at_index(13).unwrap().into_pointer_value().is_null());
    cg.module.verify().unwrap();
}

#[test]
fn codegen_rejects_a_corrupt_native_descriptor_identity_without_repair() {
    let context = Context::create();
    let mut cg = CodeGen::new(&context, "corrupt_descriptor");
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let mut contract = MirBackendContract::default();
    let plan = payload_drop_plan(&contract, &mut tc, scalar).unwrap();
    let mut identity = descriptor_identity(&contract, &tc, &plan).unwrap();
    identity.semantic.push(0);
    contract.descriptor_identities.insert(scalar, identity);
    contract.erased.descriptors.insert(scalar, MirTypeDescriptor { ty: scalar, drop: plan, fields: vec![] });
    cg.set_type_context(tc);
    assert!(cg.emit_erased_program(&contract).is_err());
    assert!(cg.module.get_global(&format!("rock.descriptor.{}", scalar.0)).is_none());
}
