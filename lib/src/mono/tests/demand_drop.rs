use super::*;

fn drop_fixture() -> (Monomorphizer, Type) {
    let trait_id = DefId::new(CrateId(0), LocalDefId(2000));
    let member_id = DefId::new(CrateId(0), LocalDefId(2001));
    let impl_id = DefId::new(CrateId(0), LocalDefId(2002));
    let method_id = DefId::new(CrateId(0), LocalDefId(2003));
    let owner = DefId::new(CrateId(0), LocalDefId(2004));
    let ty = Type::Struct { id: owner, args: Vec::new() };
    let method = HirFunction {
        id: method_id,
        name: "drop".to_string(),
        generic_params: Vec::new(),
        generic_bounds: Default::default(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: HirLocalId(0),
            ty: ty.clone(),
            mutable: true,
            is_ref: true,
        }],
        ret_type: Type::Unit,
        body: HirBlock { stmts: Vec::new(), ty: Type::Unit },
        is_curried: false,
        is_method: true,
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Tracked".to_string()),
        type_name: "Tracked".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(ty.clone()),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: Default::default(),
        methods: HashMap::from([("drop".to_string(), method)]),
    };
    let mut mono = Monomorphizer::new();
    mono.drop_trait_id = Some(trait_id);
    mono.drop_method_id = Some(member_id);
    mono.trait_impls.insert(trait_id, vec![imp]);
    mono.effective_trait_methods.insert((impl_id, member_id), method_id);
    (mono, ty)
}

#[test]
fn structural_owners_demand_element_destructors() {
    for shape in 0..4 {
        let (mut mono, element) = drop_fixture();
        let ty = match shape {
            0 => Type::Array(Box::new(element.clone()), crate::types::ArrayLen::Value(2)),
            1 => Type::Tuple(vec![element.clone()]),
            2 => Type::Slice(Box::new(element.clone())),
            _ => {
                let mut closure = Type::function(Vec::new(), Type::Unit);
                if let Type::Function { captures, .. } = &mut closure {
                    captures.push(crate::types::FunctionCapture::new(
                        crate::types::CaptureKind::Move, element.clone(),
                    ));
                }
                closure
            }
        };
        mono.monomorphize_drop_for_type(&ty, None);
        assert!(mono.diagnostics.0.is_empty(), "{:?}", mono.diagnostics);
        let element_id = mono.intern_type(&element);
        assert!(mono.generated_drop_instances.contains_key(&element_id), "shape {shape}");
        assert_eq!(mono.instances.len(), 1);
    }
}

#[test]
fn empty_arrays_and_borrowed_pointees_do_not_demand_destructors() {
    for shape in 0..2 {
        let (mut mono, element) = drop_fixture();
        let ty = if shape == 0 {
            Type::Array(Box::new(element), crate::types::ArrayLen::Value(0))
        } else {
            Type::Reference { mutable: false, inner: Box::new(element) }
        };
        mono.monomorphize_drop_for_type(&ty, None);
        assert_eq!(mono.instances.len(), 0);
    }
}
