use super::*;
use crate::ids::LocalDefId;
use crate::mir::{MirProgram, MirTypeDescriptor};
use crate::types::{GenericParamId, Type};

fn nominal(crate_id: u32, origin: ProductCrateIdentity, padding: bool) -> (MirBackendContract, TypeContext, TypeId) {
    let mut contract = MirBackendContract::default();
    contract.descriptor_origins.insert(CrateId(crate_id), origin);
    let mut tc = TypeContext::new();
    if padding { tc.intern_type(&Type::F32); }
    let ty = tc.intern_type(&Type::Struct { id: DefId::new(CrateId(crate_id), LocalDefId(7)), args: vec![Type::I64] });
    (contract, tc, ty)
}

#[test]
fn importer_and_interner_ids_do_not_change_semantic_identity() {
    let origin = ProductCrateIdentity::local("actual-defining-crate".into());
    let (first, first_tc, first_ty) = nominal(3, origin.clone(), false);
    let (second, second_tc, second_ty) = nominal(99, origin, true);
    assert_ne!(first_ty, second_ty);
    assert_eq!(semantic_identity(&first, &first_tc, first_ty), semantic_identity(&second, &second_tc, second_ty));
}

#[test]
fn defining_product_identity_and_ordered_arguments_are_not_layout_or_name_keys() {
    let (first, mut tc, ty) = nominal(3, ProductCrateIdentity::local("provider".into()), false);
    let mut second = first.clone();
    second.descriptor_origins.get_mut(&CrateId(3)).unwrap().version = "0.2.0".into();
    assert_ne!(semantic_identity(&first, &tc, ty), semantic_identity(&second, &tc, ty));
    second = first.clone();
    second.descriptor_origins.get_mut(&CrateId(3)).unwrap().target_triple = Some(String::new());
    assert_ne!(semantic_identity(&first, &tc, ty), semantic_identity(&second, &tc, ty), "optional origin fields retain presence, not just display text");
    let a = tc.intern_type(&Type::Tuple(vec![Type::I64, Type::U64]));
    let b = tc.intern_type(&Type::Tuple(vec![Type::U64, Type::I64]));
    assert_ne!(semantic_identity(&first, &tc, a), semantic_identity(&first, &tc, b));
    assert!(semantic_identity(&first, &tc, a).unwrap().starts_with(b"RK01t"));
}

#[test]
fn native_tuple_uses_the_same_framed_identity_and_drop_template_as_composition() {
    let contract = MirBackendContract::default();
    let mut tc = TypeContext::new();
    let a = tc.intern_type(&Type::I64);
    let b = tc.intern_type(&Type::Reference { inner: Box::new(Type::U64), mutable: true });
    let tuple = tc.intern_type(&Type::Tuple(vec![Type::I64, Type::Reference { inner: Box::new(Type::U64), mutable: true }]));
    let first = MirPayloadDropPlan { ty: a, user_drop: None, children: MirPayloadDropChildren::None };
    let second = MirPayloadDropPlan { ty: b, user_drop: None, children: MirPayloadDropChildren::None };
    let a = descriptor_identity(&contract, &tc, &first).unwrap();
    let b = descriptor_identity(&contract, &tc, &second).unwrap();
    let native = descriptor_identity(&contract, &tc, &MirPayloadDropPlan { ty: tuple, user_drop: None, children: MirPayloadDropChildren::Fields(vec![first, second]) }).unwrap();
    assert_eq!(native.semantic, tuple_identity(&[a.semantic, b.semantic]));
    assert_eq!(native.drop_template, tuple_drop_template(&[a.drop_template, b.drop_template]));
}

#[test]
fn malformed_native_identity_or_drop_template_is_rejected_by_strict_mir() {
    let mut tc = TypeContext::new();
    let ty = tc.intern_type(&Type::I64);
    let mut contract = MirBackendContract::default();
    let plan = MirPayloadDropPlan { ty, user_drop: None, children: MirPayloadDropChildren::None };
    let identity = descriptor_identity(&contract, &tc, &plan).unwrap();
    contract.erased.descriptors.insert(ty, MirTypeDescriptor { ty, drop: plan, fields: Vec::new() });
    contract.descriptor_identities.insert(ty, identity.clone());
    assert!(validate_descriptor_identities(&contract, &tc));
    for drop_key in [false, true] {
        let mut corrupt = contract.clone();
        let supplied = corrupt.descriptor_identities.get_mut(&ty).unwrap();
        if drop_key { supplied.drop_template.push(0); } else { supplied.semantic[3] = b'9'; }
        assert!(!validate_descriptor_identities(&corrupt, &tc));
        let errors = crate::mir::validate_erased_contract(&MirProgram { functions: BTreeMap::new(), type_context: tc.clone(), backend_contract: corrupt });
        assert!(errors.contains(&(None, crate::mir::MirErasedError::InvalidType)));
    }
}

#[test]
fn missing_origin_or_abstract_binder_cannot_supply_exact_semantic_identity() {
    let (mut contract, mut tc, ty) = nominal(3, ProductCrateIdentity::local("provider".into()), false);
    contract.descriptor_origins.clear();
    assert!(semantic_identity(&contract, &tc, ty).is_none());
    let generic = tc.intern_type(&Type::Generic(GenericParamId { owner: DefId::new(CrateId(3), LocalDefId(8)), index: 0 }));
    assert!(semantic_identity(&contract, &tc, generic).is_none());
}

#[test]
fn mismatched_actual_tuple_drop_fields_cannot_mint_a_template() {
    let contract = MirBackendContract::default();
    let mut tc = TypeContext::new();
    let ty = tc.intern_type(&Type::Tuple(vec![Type::I64]));
    assert!(descriptor_identity(&contract, &tc, &MirPayloadDropPlan { ty, user_drop: None, children: MirPayloadDropChildren::Fields(Vec::new()) }).is_none());
}
