use crate::ids::FieldId;
use crate::types::{GenericParamId, Type};

use super::*;

fn fixture() -> (MirProgram, MirErasedKey) {
    let (mut program, key, owner) = super::tests::fixture(false);
    let reference = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Parameter(0)), mutable: false,
    };
    let field = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::Generic(GenericParamId { owner, index: 0 })), mutable: false,
    });
    let contract = &mut program.backend_contract;
    let MirNominalLayout::Struct { fields, .. } = contract.nominal_layouts.get_mut(&owner).unwrap() else { unreachable!() };
    fields[0].1 = field;
    let body = contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap();
    let local = MirErasedLocal(body.locals.len() as u32);
    body.locals.push(reference);
    let MirErasedRvalue::NominalAggregate { fields, .. } = &mut body.blocks[0].statements[1].value else { unreachable!() };
    assert_eq!(fields[0].0, FieldId(0));
    fields[0].1 = MirErasedOperand::Copy(MirErasedPlace::local(local));
    body.blocks[0].statements.insert(0, MirErasedAssignment {
        destination: local,
        value: MirErasedRvalue::Borrow { place: MirErasedPlace::local(MirErasedLocal(0)), mutable: false },
    });
    contract.erased.nominal_factories.clear();
    for requirement in collect_body_factories(&contract.erased.functions[&key], contract, &program.type_context).unwrap() {
        match requirement {
            MirBodyFactoryRequirement::Tuple(r) => contract.erased.tuple_factories.entry(key.clone()).or_default().push(r),
            MirBodyFactoryRequirement::Nominal(r) => contract.erased.nominal_factories.entry(key.clone()).or_default().push(r),
            MirBodyFactoryRequirement::ThinHandle(r) => contract.erased.thin_handle_factories.entry(key.clone()).or_default().push(r),
        }
    }
    contract.runtime_requirements = backend_contract::runtime_requirements_for_backend(contract, program.functions.values());
    (program, key)
}

#[test]
fn exact_handle_factory_dominates_its_nominal_field_layout() {
    let (program, key) = fixture();
    assert!(validate_body_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).is_ok());
    assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
    let handles = &program.backend_contract.erased.thin_handle_factories[&key];
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].tag(), Ok(b'r'));
    let requirements = collect_body_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).unwrap();
    assert!(matches!(requirements[0], MirBodyFactoryRequirement::ThinHandle(_)));
    assert!(matches!(requirements[1], MirBodyFactoryRequirement::Nominal(_)));
}

#[test]
fn handle_factory_rejects_forged_identity_kind_ordinal_and_abi() {
    for corruption in 0..5 {
        let (mut program, key) = fixture();
        let contract = &mut program.backend_contract;
        let handle = &mut contract.erased.thin_handle_factories.get_mut(&key).unwrap()[0];
        match corruption {
            0 => handle.result = MirErasedType::Pointer(Box::new(MirErasedType::Parameter(1))),
            1 => {
                handle.result = MirErasedType::Reference { inner: Box::new(MirErasedType::Parameter(99)), mutable: false };
                handle.pointee = MirErasedType::Parameter(99);
            }
            2 => handle.descriptor_abi += 1,
            3 => {
                handle.result = MirErasedType::Reference { inner: Box::new(MirErasedType::Scoped(0)), mutable: false };
                handle.pointee = MirErasedType::Scoped(0);
            }
            _ => contract.erased.functions.get_mut(&key).unwrap().signature.parameters[0].kind = Kind::arrow(Kind::Type, Kind::Type),
        }
        assert!(validate_body_factories(&contract.erased.functions[&key], contract, &program.type_context).is_err());
    }
}

#[test]
fn missing_handle_factory_never_becomes_shape_only_authority() {
    let (mut program, key) = fixture();
    program.backend_contract.erased.thin_handle_factories.clear();
    assert_eq!(validate_body_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::MissingLayout));
}

#[test]
fn orphaned_bodyless_and_stale_handle_runtime_roots_reject() {
    for corruption in 0..3 {
        let (mut program, key) = fixture();
        let contract = &mut program.backend_contract;
        match corruption {
            0 => { contract.erased.functions.remove(&key); },
            1 => contract.erased.functions.get_mut(&key).unwrap().body = None,
            _ => { contract.runtime_requirements.remove(&MirRuntimeHelper::HeapFree); },
        }
        if corruption == 1 { assert_eq!(validate_body_factories(&contract.erased.functions[&key], contract, &program.type_context), Err(MirErasedError::UnsupportedOperation)); }
        if corruption == 0 { assert!(!validate_erased_contract(&program).is_empty()); }
        assert!(!agreement::check_mir_runtime_agreement(&program).is_clean());
    }
}

fn scoped_fixture(nested: bool, construct: bool) -> (MirProgram, MirErasedKey) {
    let source = format!(r#"
trait Read
    @read: I64
struct Item
    < tag: I64
impl Read for Item
    @read = -> self.tag
struct Frame T
    < address: &T
consume: T -> ()
consume = value !->
    {construction}
    ()
forward: T -> ()
forward = value !-> consume value
inspect_open: &Read -> ()
inspect_open = object !->
    open object as Hidden, value
        forward {argument}
main = !->
    item = Item
        tag: 42
    object: &Read = &item
    inspect_open object
"#,
        construction = if construct { "frame = Frame\n        address: &value" } else { "()" },
        argument = if nested { "&value" } else { "value" },
    );
    let path = std::path::PathBuf::from("/virtual/scoped_handles/main.rk");
    let frontend = crate::compile_frontend(&crate::Config {
        entry_file: path.clone(), current_crate_name: Some("scoped_handles".into()),
        no_std: true, no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: source }],
        ..Default::default()
    }).unwrap();
    let identity = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        crate::products::ProductCrateIdentity::local("scoped_handles".into()), BTreeMap::new(),
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
fn scoped_handle_recipes_follow_transitive_metadata_demand_and_dependency_order() {
    for nested in [false, true] {
        let (program, key) = scoped_fixture(nested, true);
        let errors = validate_erased_contract(&program);
        assert!(errors.is_empty(), "nested={nested}: {errors:?}");
        assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
        let handles = &program.backend_contract.erased.scoped_factories[&key][&0];
        assert_eq!(handles.len(), if nested { 2 } else { 1 });
        let MirBodyFactoryRequirement::ThinHandle(first) = &handles[0] else { panic!("thin handle recipe"); };
        assert_eq!(first.pointee, MirErasedType::Scoped(0));
        if nested {
            let MirBodyFactoryRequirement::ThinHandle(second) = &handles[1] else { panic!("thin handle recipe"); };
            assert_eq!(second.pointee, first.result);
        }
    }
}

#[test]
fn allocation_free_scoped_forwarding_does_not_gain_a_metadata_factory() {
    let (program, key) = scoped_fixture(false, false);
    assert!(collect_scoped_factories(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).unwrap().is_empty());
    assert!(!program.backend_contract.erased.scoped_factories.contains_key(&key));
    assert!(!program.backend_contract.runtime_requirements.contains(&MirRuntimeHelper::HeapAlloc));
    assert!(validate_erased_contract(&program).is_empty());
}

#[test]
fn scoped_handle_recipes_reject_missing_wrong_scope_mutability_and_abi() {
    for corruption in 0..5 {
        let (mut program, key) = scoped_fixture(false, true);
        let contract = &mut program.backend_contract;
        let scopes = contract.erased.scoped_factories.get_mut(&key).unwrap();
        match corruption {
            0 => scopes.clear(),
            1 => { let handles = scopes.remove(&0).unwrap(); scopes.insert(99, handles); },
            2 => {
                let MirBodyFactoryRequirement::ThinHandle(handle) = &mut scopes.get_mut(&0).unwrap()[0] else { panic!("thin handle recipe"); };
                handle.pointee = MirErasedType::Scoped(99);
                handle.result = MirErasedType::Reference { inner: Box::new(handle.pointee.clone()), mutable: false };
            }
            3 => {
                let MirBodyFactoryRequirement::ThinHandle(handle) = &mut scopes.get_mut(&0).unwrap()[0] else { panic!("thin handle recipe"); };
                handle.result = MirErasedType::Reference { inner: Box::new(handle.pointee.clone()), mutable: true };
            }
            _ => {
                let MirBodyFactoryRequirement::ThinHandle(handle) = &mut scopes.get_mut(&0).unwrap()[0] else { panic!("thin handle recipe"); };
                handle.descriptor_abi += 1;
            },
        }
        assert!(validate_scoped_factories(&contract.erased.functions[&key], contract, &program.type_context).is_err());
    }
}

#[test]
fn scoped_handle_dependency_order_cannot_be_repaired_by_codegen() {
    let (mut program, key) = scoped_fixture(true, true);
    let contract = &mut program.backend_contract;
    contract.erased.scoped_factories.get_mut(&key).unwrap().get_mut(&0).unwrap().reverse();
    assert_eq!(validate_scoped_factories(&contract.erased.functions[&key], contract, &program.type_context), Err(MirErasedError::InvalidType));
}
