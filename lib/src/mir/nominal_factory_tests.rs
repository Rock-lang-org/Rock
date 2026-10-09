use std::collections::BTreeSet;

use crate::ids::{CrateId, DefId, FieldId, LocalDefId};
use crate::mir::backend_contract::runtime_requirements_for_backend;
use crate::products::ProductCrateIdentity;
use crate::types::{GenericParamId, NominalTypeKind, Type};

use super::*;

pub(crate) fn fixture(nested: bool) -> (MirProgram, MirErasedKey, DefId) {
    let (mut program, key) = crate::mir::tuple_factory::tests::fixture();
    let owner = DefId::new(CrateId(0), LocalDefId(100));
    let generic = GenericParamId { owner, index: 0 };
    let tc = &mut program.type_context;
    let tag = tc.intern_type(&Type::I64);
    let generic_type = tc.intern_type(&Type::Generic(generic));
    let pair_type = tc.intern_type(&Type::Tuple(vec![Type::Generic(generic), Type::Generic(generic)]));
    let contract = &mut program.backend_contract;
    contract.descriptor_origins.insert(CrateId(0), ProductCrateIdentity::local("nominal_fixture".into()));
    contract.nominal_field_ids.insert(owner, if nested { vec![FieldId(0), FieldId(2)] } else { vec![FieldId(0), FieldId(1), FieldId(2)] });
    contract.nominal_drop_authority.insert(owner, MirNominalDropAuthority::FieldsOnly);
    contract.nominal_layouts.insert(owner, MirNominalLayout::Struct {
        id: owner, generic_params: vec![generic],
        fields: if nested { vec![("values".into(), pair_type), ("tag".into(), tag)] }
            else { vec![("first".into(), generic_type), ("second".into(), generic_type), ("tag".into(), tag)] },
    });
    contract.erased.constructor_kinds.insert(owner, Kind::arrow(Kind::Type, Kind::Type));
    let plan = payload_drop_plan(contract, tc, tag).unwrap();
    let identity = descriptor_identity::descriptor_identity(contract, tc, &plan).unwrap();
    contract.descriptor_identities.insert(tag, identity);
    contract.erased.descriptors.insert(tag, MirTypeDescriptor { ty: tag, drop: plan, fields: vec![] });
    let result = MirErasedType::Nominal { id: owner, flavor: NominalTypeKind::Struct, args: vec![MirErasedType::Parameter(0)] };
    let function = contract.erased.functions.get_mut(&key).unwrap();
    let body = function.body.as_mut().unwrap();
    body.locals[2] = result.clone();
    body.locals.extend([MirErasedType::Concrete(tag), MirErasedType::Concrete(tag)]);
    let moving = |local| MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(local)));
    let copying = |local| MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(local)));
    let mut statements = vec![MirErasedAssignment { destination: MirErasedLocal(4), value: MirErasedRvalue::Literal(MirErasedLiteral::Int(42)) }];
    let fields = if nested {
        body.locals.push(MirErasedType::Tuple(vec![MirErasedType::Parameter(0); 2]));
        statements.push(MirErasedAssignment { destination: MirErasedLocal(6), value: MirErasedRvalue::Aggregate(vec![moving(0), moving(1)]) });
        vec![(FieldId(0), moving(6)), (FieldId(2), copying(4))]
    } else { vec![(FieldId(0), moving(0)), (FieldId(1), moving(1)), (FieldId(2), copying(4))] };
    statements.push(MirErasedAssignment { destination: MirErasedLocal(2), value: MirErasedRvalue::NominalAggregate { ty: result.clone(), fields } });
    statements.push(MirErasedAssignment { destination: MirErasedLocal(5), value: MirErasedRvalue::Use(MirErasedOperand::Copy(MirErasedPlace {
        local: MirErasedLocal(2), projection: vec![MirErasedProjection::NominalField { owner, field: FieldId(2) }],
    })) });
    let mut call = body.blocks[0].statements[1].clone();
    let MirErasedRvalue::Call { type_args, .. } = &mut call.value else { panic!("fixture helper call"); };
    *type_args = vec![result]; statements.push(call); body.blocks[0].statements = statements;
    let requirements = collect_body_factories(&contract.erased.functions[&key], contract, tc).unwrap();
    contract.erased.tuple_factories.clear();
    for requirement in requirements {
        match requirement {
            MirBodyFactoryRequirement::Tuple(r) => contract.erased.tuple_factories.entry(key.clone()).or_default().push(r),
            MirBodyFactoryRequirement::Nominal(r) => contract.erased.nominal_factories.entry(key.clone()).or_default().push(r),
        }
    }
    contract.runtime_requirements = runtime_requirements_for_backend(contract, program.functions.values());
    (program, key, owner)
}

#[test]
fn whole_product_construction_copy_tag_and_generic_helper_are_checked() {
    for nested in [false, true] {
        let (program, key, _) = fixture(nested);
        assert!(validate_erased_contract(&program).is_empty());
        assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
        assert!(!program.backend_contract.erased.functions[&key].signature.layouts.iter().any(|ty| matches!(ty, MirErasedType::Nominal { .. })));
        let order = collect_body_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).unwrap();
        assert!(matches!(order.last(), Some(MirBodyFactoryRequirement::Nominal(_))));
        if nested { assert!(matches!(order.first(), Some(MirBodyFactoryRequirement::Tuple(_)))); }
    }
}

#[test]
fn raw_nominal_body_without_factory_still_rejects() {
    let (mut program, key, _) = fixture(false);
    program.backend_contract.erased.nominal_factories.clear();
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn canonical_field_owner_identity_and_order_cannot_be_replaced_by_positions() {
    for control in 0..3 {
        let (mut program, key, owner) = fixture(false);
        let function = program.backend_contract.erased.functions.get_mut(&key).unwrap();
        let body = function.body.as_mut().unwrap();
        if control < 2 {
            let MirErasedRvalue::Use(MirErasedOperand::Copy(place)) = &mut body.blocks[0].statements[2].value else { panic!("tag projection"); };
            place.projection = vec![MirErasedProjection::NominalField { owner: if control == 0 { DefId::new(CrateId(0), LocalDefId(101)) } else { owner }, field: if control == 1 { FieldId(99) } else { FieldId(2) } }];
        } else {
            let MirErasedRvalue::NominalAggregate { fields, .. } = &mut body.blocks[0].statements[1].value else { panic!("nominal aggregate"); };
            fields.swap(0, 1);
        }
        assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::InvalidType));
    }
}

#[test]
fn unsupported_user_drop_never_turns_into_a_fields_only_factory() {
    let (mut program, key, owner) = fixture(false);
    program.backend_contract.nominal_drop_authority.insert(owner, MirNominalDropAuthority::UnsupportedUserDrop);
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::UnsupportedOperation));
}

#[test]
fn corrupt_template_argument_kind_and_unbound_children_fail_closed() {
    for control in 0..3 {
        let (mut program, key, _) = fixture(false);
        match control {
            0 => program.backend_contract.erased.nominal_factories.get_mut(&key).unwrap()[0].template.push(0),
            1 => program.backend_contract.erased.functions.get_mut(&key).unwrap().signature.parameters[0].kind = Kind::arrow(Kind::Type, Kind::Type),
            _ => { program.backend_contract.erased.nominal_factories.get_mut(&key).unwrap()[0].arguments[0] = MirErasedType::Parameter(99); },
        }
        assert!(!validate_erased_contract(&program).is_empty());
    }
}

#[test]
fn projected_noncopy_move_and_copy_are_not_granted_by_a_layout() {
    for moving in [true, false] {
        let (mut program, key, owner) = fixture(false);
        let function = program.backend_contract.erased.functions.get_mut(&key).unwrap();
        let body = function.body.as_mut().unwrap();
        body.locals[5] = MirErasedType::Parameter(0);
        let place = MirErasedPlace { local: MirErasedLocal(2), projection: vec![MirErasedProjection::NominalField { owner, field: FieldId(0) }] };
        body.blocks[0].statements[2].value = MirErasedRvalue::Use(if moving { MirErasedOperand::Move(place) } else { MirErasedOperand::Copy(place) });
        assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(if moving { MirErasedError::ProjectedMoveRequiresDropProof } else { MirErasedError::CopyWithoutProof }));
    }
}

#[test]
fn native_and_runtime_product_drop_keys_use_the_same_selected_template() {
    let (mut program, _, owner) = fixture(false);
    let tc = &mut program.type_context;
    let concrete = tc.intern_type(&Type::Struct { id: owner, args: vec![Type::I64] });
    let drop = payload_drop_plan(&program.backend_contract, tc, concrete).unwrap();
    let identity = descriptor_identity::descriptor_identity(&program.backend_contract, tc, &drop).unwrap();
    assert_eq!(identity.drop_template[4], b'g');
    let (template, _) = nominal_template(&program.backend_contract, tc, owner).unwrap();
    let mut payload = Vec::new(); frame(&mut payload, &identity.semantic); payload.push(0); frame(&mut payload, &template);
    let MirPayloadDropChildren::Fields(fields) = &drop.children else { panic!("actual struct drop fields"); };
    let children = fields.iter().map(|field| descriptor_identity::descriptor_identity(&program.backend_contract, tc, field).unwrap().drop_template).collect::<Vec<_>>();
    assert_eq!(identity.drop_template, descriptor_identity::key(b"RD01", b'g', &payload, &children));
}

#[test]
fn cyclic_by_value_templates_reject_without_recursive_factory_callbacks() {
    let (mut program, key, owner) = fixture(false);
    let param = GenericParamId { owner, index: 0 };
    let recursive = program.type_context.intern_type(&Type::Struct { id: owner, args: vec![Type::Generic(param)] });
    let MirNominalLayout::Struct { fields, .. } = program.backend_contract.nominal_layouts.get_mut(&owner).unwrap() else { panic!("struct"); };
    fields[0].1 = recursive;
    assert!(collect_body_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).is_err());
}

#[test]
fn missing_and_stale_nominal_runtime_roots_remain_agreement_errors() {
    let (mut program, _, _) = fixture(false);
    program.backend_contract.runtime_requirements.remove(&MirRuntimeHelper::NominalDescriptorFactory);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
    program.backend_contract.erased.nominal_factories.clear(); program.backend_contract.erased.functions.clear();
    program.backend_contract.runtime_requirements = BTreeSet::from([MirRuntimeHelper::NominalDescriptorFactory]);
    assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
}

#[test]
fn missing_canonical_declaration_field_and_drop_authorities_reject() {
    for control in 0..3 {
        let (mut program, key, owner) = fixture(false);
        match control {
            0 => { program.backend_contract.nominal_layouts.remove(&owner); },
            1 => { program.backend_contract.nominal_field_ids.remove(&owner); },
            _ => { program.backend_contract.nominal_drop_authority.remove(&owner); },
        }
        assert!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).is_err());
    }
}

#[test]
fn nominal_lists_preserve_dependency_order_rather_than_sorting_bad_evidence() {
    let (mut program, key, owner) = fixture(true);
    let outer = DefId::new(CrateId(0), LocalDefId(102));
    let generic = GenericParamId { owner: outer, index: 0 };
    let field = program.type_context.intern_type(&Type::Struct { id: owner, args: vec![Type::Generic(generic)] });
    let contract = &mut program.backend_contract;
    contract.nominal_layouts.insert(outer, MirNominalLayout::Struct {
        id: outer, generic_params: vec![generic], fields: vec![("frame".into(), field)],
    });
    contract.nominal_field_ids.insert(outer, vec![FieldId(9)]);
    contract.nominal_drop_authority.insert(outer, MirNominalDropAuthority::FieldsOnly);
    contract.erased.constructor_kinds.insert(outer, Kind::arrow(Kind::Type, Kind::Type));
    let ty = MirErasedType::Nominal { id: outer, flavor: NominalTypeKind::Struct, args: vec![MirErasedType::Parameter(0)] };
    contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap().locals.push(ty.clone());
    let requirement = nominal_requirement(contract, &program.type_context, &ty).unwrap();
    contract.erased.nominal_factories.get_mut(&key).unwrap().push(requirement);
    assert!(validate_erased_function(&contract.erased.functions[&key], contract, &program.type_context).is_ok());
    contract.erased.nominal_factories.get_mut(&key).unwrap().reverse();
    assert_eq!(validate_erased_function(&contract.erased.functions[&key], contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn orphaned_and_bodyless_nominal_factories_cannot_supply_runtime_authority() {
    let (mut program, key, _) = fixture(false);
    program.backend_contract.erased.functions.get_mut(&key).unwrap().body = None;
    assert!(!runtime_requirements_for_backend(&program.backend_contract, program.functions.values()).contains(&MirRuntimeHelper::NominalDescriptorFactory));
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::UnsupportedOperation));
    program.backend_contract.erased.functions.remove(&key);
    assert!(!validate_erased_contract(&program).is_empty());
}
