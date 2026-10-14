use super::*;

fn fixture() -> (MirProgram, MirErasedKey) {
    fixture_with_construction("        frame = Frame\n            address: &value\n            tag: 42\n        nested = (frame, (42, 17))\n        consume nested")
}

fn fixture_with_construction(construction: &str) -> (MirProgram, MirErasedKey) {
    let source = r#"
lang drop
< trait Dispose
    lang method
    ~@dispose: ()
trait Read
    @read: I64
struct Item
    < tag: I64
impl Read for Item
    @read = -> self.tag
struct Frame T
    < address: T
    < tag: I64
impl Dispose for Frame T
    ~@dispose = !-> ()
consume: T -> ()
consume = value !-> ()
inspect: &Read -> ()
inspect = object !->
    open object as Hidden, value
        frame = Frame
            address: &value
            tag: 42
        nested = (frame, (42, 17))
        consume nested
main = !->
    item = Item
        tag: 42
    object: &Read = &item
    inspect object
"#.replace("        frame = Frame\n            address: &value\n            tag: 42\n        nested = (frame, (42, 17))\n        consume nested", construction);
    let path = std::path::PathBuf::from("/virtual/scoped_factories/main.rk");
    let frontend = crate::compile_frontend(&crate::Config {
        entry_file: path.clone(), current_crate_name: Some("scoped_factories".into()),
        no_std: true, no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: source.into() }],
        ..Default::default()
    }).unwrap();
    let identity = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        crate::products::ProductCrateIdentity::local("scoped_factories".into()), BTreeMap::new(),
    );
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, identity).unwrap();
    let mut bodies = builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let program = builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let key = program.backend_contract.erased.functions.iter()
        .find(|(_, function)| function.body.as_ref().is_some_and(|body| !body.scopes.is_empty()))
        .unwrap().0.clone();
    (program, key)
}

#[test]
fn scoped_products_retain_dependency_order_and_selected_destructor_environment() {
    let (program, key) = fixture();
    let contract = &program.backend_contract;
    assert!(validate_erased_contract(&program).is_empty());
    assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
    let requirements = &contract.erased.scoped_factories[&key][&0];
    assert_eq!(requirements.len(), 4);
    let MirBodyFactoryRequirement::ThinHandle(first) = &requirements[0] else { panic!("pointee handle first"); };
    let MirBodyFactoryRequirement::ThinHandle(handle) = &requirements[1] else { panic!("nested handle second"); };
    let MirBodyFactoryRequirement::Nominal(nominal) = &requirements[2] else { panic!("nominal third"); };
    let MirBodyFactoryRequirement::Tuple(tuple) = &requirements[3] else { panic!("tuple last"); };
    assert_eq!(first.pointee, MirErasedType::Scoped(0));
    assert_eq!(handle.pointee, first.result);
    assert_eq!(nominal.arguments, vec![handle.result.clone()]);
    assert_eq!(nominal.user_drop.as_ref().unwrap().arguments, nominal.arguments);
    assert_eq!(tuple.children[0], nominal.result);
    let entry = collect_body_factories(&contract.erased.functions[&key], contract, &program.type_context).unwrap();
    assert!(entry.iter().all(|requirement| {
        let mut scopes = BTreeSet::new();
        requirement.result().scoped_binders(&mut scopes);
        scopes.is_empty()
    }));
    let MirErasedType::Concrete(fixed) = tuple.children[1] else { panic!("fixed tuple evidence"); };
    assert!(descriptor_identity::concrete_descriptor_is_sealed(contract, fixed));
}

#[test]
fn scoped_product_corruption_never_becomes_shape_only_authority() {
    for corruption in 0..10 {
        let (mut program, key) = fixture();
        let contract = &mut program.backend_contract;
        let requirements = contract.erased.scoped_factories.get_mut(&key).unwrap().get_mut(&0).unwrap();
        match corruption {
            0 => { requirements.remove(0); },
            1 => requirements.reverse(),
            2 => { requirements.push(requirements[2].clone()); },
            3..=7 => {
                let MirBodyFactoryRequirement::Nominal(nominal) = &mut requirements[2] else { unreachable!() };
                match corruption {
                    3 => nominal.template.push(0),
                    4 => nominal.fields.reverse(),
                    5 => nominal.arguments[0] = MirErasedType::Scoped(0),
                    6 => nominal.user_drop = None,
                    _ => nominal.descriptor_abi += 1,
                }
            },
            8 => {
                let MirBodyFactoryRequirement::Tuple(tuple) = &mut requirements[3] else { unreachable!() };
                tuple.children.reverse();
            },
            _ => {
                let requirements = contract.erased.scoped_factories.get_mut(&key).unwrap().remove(&0).unwrap();
                contract.erased.scoped_factories.get_mut(&key).unwrap().insert(1, requirements);
            },
        }
        assert!(validate_scoped_factories(&contract.erased.functions[&key], contract, &program.type_context).is_err(), "corruption={corruption}");
        assert!(!validate_erased_contract(&program).is_empty(), "corruption={corruption}");
    }
}

#[test]
fn scoped_products_require_exact_runtime_roots_and_live_body_authority() {
    for corruption in 0..4 {
        let (mut program, key) = fixture();
        let contract = &mut program.backend_contract;
        match corruption {
            0 => { contract.runtime_requirements.remove(&MirRuntimeHelper::NominalDescriptorFactory); },
            1 => { contract.runtime_requirements.remove(&MirRuntimeHelper::TupleDescriptorFactory); },
            2 => contract.erased.functions.get_mut(&key).unwrap().body = None,
            _ => { contract.erased.functions.remove(&key); },
        }
        assert!(!agreement::check_mir_runtime_agreement(&program).is_clean(), "corruption={corruption}");
        if corruption >= 2 { assert!(!validate_erased_contract(&program).is_empty()); }
    }
}

#[test]
fn scoped_products_cannot_allocate_before_their_open() {
    let (mut program, key) = fixture();
    let contract = &mut program.backend_contract;
    let body = contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap();
    let (block, statement) = body.blocks.iter().enumerate().find_map(|(index, block)| {
        block.statements.iter().position(|statement| matches!(statement.value, MirErasedRvalue::NominalAggregate { .. }))
            .map(|statement| (index, statement))
    }).unwrap();
    let construction = body.blocks[block].statements.remove(statement);
    let opening = body.blocks.iter_mut().find(|block| {
        block.statements.iter().any(|statement| matches!(statement.value, MirErasedRvalue::Open { .. }))
    }).unwrap();
    let index = opening.statements.iter().position(|statement| matches!(statement.value, MirErasedRvalue::Open { .. })).unwrap();
    opening.statements.insert(index, construction);
    assert_eq!(validate_erased_function(&contract.erased.functions[&key], contract, &program.type_context), Err(MirErasedError::UnboundParameter));
}

#[test]
fn uniform_drop_free_scoped_products_do_not_acquire_unrequested_metadata() {
    let (program, key) = fixture_with_construction("        nested = (value, (42, 17))\n        ()");
    let errors = validate_erased_contract(&program);
    assert!(errors.is_empty(), "{errors:?}");
    assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
    assert!(!program.backend_contract.erased.scoped_factories.contains_key(&key));
    assert!(!program.backend_contract.runtime_requirements.contains(&MirRuntimeHelper::TupleDescriptorFactory));
    assert!(!program.backend_contract.runtime_requirements.contains(&MirRuntimeHelper::NominalDescriptorFactory));

    // Uniform local layout alone does not certify an additional callee layout.
    let (forwarded, key) = fixture_with_construction("        nested = (value, (42, 17))\n        consume nested");
    assert_eq!(validate_erased_function(&forwarded.backend_contract.erased.functions[&key], &forwarded.backend_contract, &forwarded.type_context), Err(MirErasedError::MissingLayout));
}
