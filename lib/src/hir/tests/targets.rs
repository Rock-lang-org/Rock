use super::*;

#[test]
fn hir_var_target_can_identify_local_bindings() {
    let local = crate::ids::HirLocalId(7);
    let target = HirVarTarget::Local(local);

    assert_eq!(target, HirVarTarget::Local(crate::ids::HirLocalId(7)));
}

#[test]
fn hir_call_target_can_identify_direct_function_calls() {
    let function_id = def_id(44);
    let target = HirCallTarget::Function(function_id);

    assert_eq!(target, HirCallTarget::Function(function_id));
}

#[test]
fn hir_var_target_can_reference_monomorphized_instance() {
    let target = HirVarTarget::Instance(crate::ids::InstanceId(9));

    assert_eq!(target, HirVarTarget::Instance(crate::ids::InstanceId(9)));
}

#[test]
fn method_selection_target_has_explicit_validated_variants() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(90));
    let method_id = DefId::new(CrateId(0), LocalDefId(91));
    let trait_id = DefId::new(CrateId(0), LocalDefId(92));
    let member_id = DefId::new(CrateId(0), LocalDefId(93));

    let inherent = HirMethodCallTarget {
        target: HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait: None,
        },
        owner_substitution: Vec::new(),
        method_substitution: Vec::new(),
    };
    assert_eq!(inherent.impl_id(), Some(impl_id));
    assert_eq!(inherent.method_id(), Some(method_id));

    let trait_method = HirMethodCallTarget {
        target: HirSelectedMethodTarget::TraitMethod {
            trait_id,
            member_id,
            trait_args: vec![Type::I64],
            dispatch: HirTraitDispatchKind::TraitBound,
        },
        owner_substitution: vec![HirTypeBinding {
            param: GenericParamId {
                owner: impl_id,
                index: 0,
            },
            ty: Type::I64,
        }],
        method_substitution: Vec::new(),
    };
    assert_eq!(trait_method.trait_id(), Some(trait_id));
    assert_eq!(trait_method.method_id(), Some(member_id));
    assert_eq!(trait_method.trait_args(), &[Type::I64]);
}

#[test]
fn hir_reference_shapes_carry_resolved_targets() {
    let owner = def_id(10);
    let field = field_id(0);
    let variant = variant_id(0);

    let var = HirExprKind::ResolvedVar(HirVarRef {
        name: "make_box".to_string(),
        target: HirVarTarget::Function(def_id(11)),
    });
    let struct_literal = HirExprKind::StructLiteral("Box".to_string(), Some(owner), Vec::new());
    let struct_pattern = HirPattern::Struct(
        "Box".to_string(),
        Some(owner),
        Vec::new(),
        vec![HirStructPatternField {
            name: "value".to_string(),
            field: Some(HirFieldLocation {
                owner,
                field_id: field,
                name: "value".to_string(),
            }),
            pattern: HirPattern::Wildcard,
        }],
    );
    let enum_pattern = HirPattern::Enum(
        "Option".to_string(),
        "Some".to_string(),
        Some(HirVariantLocation {
            owner,
            variant_id: variant,
            name: "Some".to_string(),
        }),
        Vec::new(),
    );

    match var {
        HirExprKind::ResolvedVar(reference) => assert_eq!(reference.name, "make_box"),
        _ => panic!("expected resolved var"),
    }
    match struct_literal {
        HirExprKind::StructLiteral(_, Some(id), _) => assert_eq!(id, owner),
        _ => panic!("expected resolved struct literal"),
    }
    match struct_pattern {
        HirPattern::Struct(_, Some(id), _, fields) => {
            assert_eq!(id, owner);
            assert_eq!(fields[0].field.as_ref().unwrap().field_id, field);
        }
        _ => panic!("expected resolved struct pattern"),
    }
    match enum_pattern {
        HirPattern::Enum(_, _, Some(location), _) => assert_eq!(location.variant_id, variant),
        _ => panic!("expected resolved enum pattern"),
    }
}
