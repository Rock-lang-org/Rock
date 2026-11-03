use super::*;

#[test]
fn supertrait_bound_implies_parent_bound_by_trait_id() {
    let functor_id = def_id(200);
    let applicative_id = def_id(201);
    let caller_id = def_id(202);
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let functor_target = GenericParamDecl::new(
        GenericParamId {
            owner: functor_id,
            index: 0,
        },
        "F",
        unary.clone(),
    );
    let applicative_target = GenericParamDecl::new(
        GenericParamId {
            owner: applicative_id,
            index: 0,
        },
        "F",
        unary,
    );
    let traits = HashMap::from([
        (
            functor_id,
            HirTrait {
                id: functor_id,
                name: "pkg::Functor".to_string(),
                generic_params: Vec::new(),
                target: Some(functor_target),
                predicates: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        ),
        (
            applicative_id,
            HirTrait {
                id: applicative_id,
                name: "pkg::Applicative".to_string(),
                generic_params: Vec::new(),
                target: Some(applicative_target.clone()),
                predicates: vec![crate::types::Predicate::Trait {
                    subject: Type::Generic(applicative_target.id),
                    trait_id: functor_id,
                    args: Vec::new(),
                }],
                associated_types: Vec::new(),
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        ),
    ]);
    let caller_generic = GenericParamId {
        owner: caller_id,
        index: 0,
    };
    let current_bounds = HashMap::from([(
        caller_generic,
        vec![TraitBound {
            trait_id: applicative_id,
            type_args: Vec::new(),
        }],
    )])
    .into();
    let impls = HashMap::new();
    let service = SelectionService::new(&traits, &impls, None, None, &current_bounds);

    assert!(service.trait_bound_satisfied(
        &Type::Generic(caller_generic),
        &TraitBound {
            trait_id: functor_id,
            type_args: Vec::new(),
        },
    ));
}

#[test]
fn constructor_trait_selection_preserves_result_section_substitution() {
    let trait_id = def_id(700);
    let member_id = def_id(701);
    let impl_id = def_id(702);
    let method_id = def_id(703);
    let result_id = def_id(704);
    let io_error_id = def_id(705);
    let impl_error = GenericParamDecl::type_param(
        GenericParamId {
            owner: impl_id,
            index: 0,
        },
        "E",
    );
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let static_method = |id, name: &str| HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HirGenericBounds::new(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::I64,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let trait_def = HirTrait {
        id: trait_id,
        name: "Applicative".to_string(),
        generic_params: Vec::new(),
        target: Some(GenericParamDecl::new(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            unary.clone(),
        )),
        predicates: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("pure".to_string(), static_method(member_id, "pure"))]),
        signatures: HashMap::new(),
    };
    let bound = || Type::BoundVar {
        depth: 0,
        index: 0,
        kind: crate::type_services::kind::Kind::Type,
    };
    let section = |error| Type::Lambda {
        params: vec![crate::type_services::kind::Kind::Type],
        body: Box::new(Type::Enum {
            id: result_id,
            args: vec![bound(), error],
        }),
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("ResultSection".to_string()),
        type_name: "ResultSection".to_string(),
        type_generics: vec![impl_error.clone()],
        receiver_pattern: HirImplReceiverPattern::Constructor(section(Type::Generic(
            impl_error.id,
        ))),
        trait_name: Some("Applicative".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HirGenericBounds::new(),
        methods: HashMap::from([("pure".to_string(), static_method(method_id, "pure"))]),
    };
    let traits = HashMap::from([(trait_id, trait_def)]);
    let impls = HashMap::from([(impl_id, imp)]);
    let bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);
    let io_error = Type::Struct {
        id: io_error_id,
        args: Vec::new(),
    };

    let selected = service
        .select_constructor_trait_member(&section(io_error.clone()), trait_id, &[], member_id)
        .expect("Result section should select its Applicative impl");

    assert_eq!(selected.target.impl_id(), Some(impl_id));
    assert_eq!(selected.target.trait_id(), Some(trait_id));
    assert_eq!(
        selected
            .target
            .owner_substitution
            .iter()
            .find(|binding| binding.param == impl_error.id)
            .map(|binding| &binding.ty),
        Some(&io_error),
    );

    let inferred = service
        .infer_constructor_target_for_applied_type(
            &Type::Enum {
                id: result_id,
                args: vec![Type::I64, io_error.clone()],
            },
            trait_id,
            &[],
        )
        .expect("constructor inference should not be ambiguous")
        .expect("Result application should infer its constructor section");
    assert_eq!(inferred, section(io_error));
}

#[test]
fn constructor_trait_selection_reports_upstream_ambiguity() {
    let trait_id = def_id(710);
    let member_id = def_id(711);
    let option_id = def_id(712);
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let function = |id| HirFunction {
        id,
        name: "pure".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HirGenericBounds::new(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let trait_def = HirTrait {
        id: trait_id,
        name: "Applicative".to_string(),
        generic_params: Vec::new(),
        target: Some(GenericParamDecl::new(
            GenericParamId {
                owner: trait_id,
                index: 0,
            },
            "F",
            unary,
        )),
        predicates: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("pure".to_string(), function(member_id))]),
        signatures: HashMap::new(),
    };
    let target = Type::Constructor {
        id: option_id,
        flavor: crate::types::NominalTypeKind::Enum,
    };
    let impl_for = |impl_id: DefId, method_id: DefId| HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Option".to_string()),
        type_name: "Option".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Constructor(target.clone()),
        trait_name: Some("Applicative".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HirGenericBounds::new(),
        methods: HashMap::from([("pure".to_string(), function(method_id))]),
    };
    let traits = HashMap::from([(trait_id, trait_def)]);
    let impls = HashMap::from([
        (def_id(713), impl_for(def_id(713), def_id(714))),
        (def_id(715), impl_for(def_id(715), def_id(716))),
    ]);
    let bounds = HirGenericBounds::new();
    let effective = effective_trait_methods_for(&traits, &impls);
    let service = SelectionService::new(&traits, &impls, None, None, &bounds)
        .with_effective_trait_methods(&effective);

    let error = service
        .select_constructor_trait_member(&target, trait_id, &[], member_id)
        .expect_err("overlapping constructor impls must remain defensively ambiguous");

    assert!(error
        .message()
        .contains("upstream coherence invariant violation"));
}
