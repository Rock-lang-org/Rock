use super::*;

fn fixture() -> (MirProgram, MirErasedKey) {
    let source = r#"
lang sized
< trait KnownSize
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?KnownSize
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?KnownSize
lang object_owner_unique
< trait Unique State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
struct State
    < base: *U8
struct Own (T: ?KnownSize)
    < raw: *T
    < state: State
extern malloc: I64 -> *U8
impl Own T
    new: T -> Own T
    new = value ->
        base = malloc (~SizeOf value)
        pointer: *T = base as *T
        unsafe *pointer = value
        Own
            raw: pointer
            state: State
                base: base
impl Transport State for Own
    unsafe split = owner -> (owner.raw, owner.state)
    unsafe rebuild = pointer, state -> Own
        raw: pointer
        state: state
impl Unique State for Own
    unsafe release = _, _ !-> ()
trait Take
    ~@take: I64
struct Item
    < tag: I64
impl Take for Item
    ~@take = -> self.tag
consume_open: Own Take -> I64
consume_open = owner ->
    open owner as Hidden, value
        value.take!
main = !->
    owner: Own Take = Own::new (Item
        tag: 42)
    consume_open owner
"#;
    let path = std::path::PathBuf::from("/virtual/owner_open/main.rk");
    let frontend = crate::compile_frontend(&crate::Config {
        entry_file: path.clone(), current_crate_name: Some("owner_open".into()),
        no_std: true, no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: source.into() }],
        ..Default::default()
    }).unwrap();
    let identity = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        crate::products::ProductCrateIdentity::local("owner_open".into()), BTreeMap::new(),
    );
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, identity).unwrap();
    let mut bodies = builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    let program = builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let key = program.backend_contract.erased.functions.iter().find(|(_, function)| {
        function.body.as_ref().is_some_and(|body| body.scopes.iter().any(|scope| matches!(scope.source, MirErasedScopeSource::Owner { .. })))
    }).unwrap().0.clone();
    (program, key)
}

fn check(program: &MirProgram, key: &MirErasedKey) -> Result<(), MirErasedError> {
    validate_erased_function(&program.backend_contract.erased.functions[key], &program.backend_contract, &program.type_context)
}

#[test]
fn owning_open_keeps_selected_native_and_scoped_transport_authority() {
    let (program, key) = fixture();
    assert_eq!(check(&program, &key), Ok(()));
    assert!(validate_erased_contract(&program).is_empty());
    assert!(agreement::check_mir_runtime_agreement(&program).is_clean());
    let body = program.backend_contract.erased.functions[&key].body.as_ref().unwrap();
    let MirErasedScopeSource::Owner { opening } = &body.scopes[0].source else { panic!("owner scope"); };
    let MirErasedOwnerTarget::Native(native) = &opening.split.target else { panic!("closed native split"); };
    assert!(program.backend_contract.callable(native).is_some());
    assert!(matches!(opening.rebuild.target, MirErasedOwnerTarget::Erased { .. }));
    assert!(program.backend_contract.erased.scoped_factories[&key][&0].iter().any(|factory| factory.result() == &opening.opened));
}

#[test]
fn owning_open_rejects_changed_state_constructor_member_and_witness() {
    for corruption in 0..7 {
        let (mut program, key) = fixture();
        let integer = program.type_context.id_for_type(&Type::I64).unwrap();
        let body = program.backend_contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap();
        let MirErasedScopeSource::Owner { opening } = &mut body.scopes[0].source else { unreachable!() };
        match corruption {
            0 => opening.state = MirErasedType::Concrete(integer),
            1 => opening.split.member = opening.rebuild.member,
            2 => {
                opening.rebuild.constructor = integer;
                opening.split.constructor = integer;
            }
            3 => {
                let MirErasedOwnerTarget::Erased { type_args, .. } = &mut opening.rebuild.target else { unreachable!() };
                type_args[0] = MirErasedType::Concrete(integer);
            }
            4 => opening.opened = MirErasedType::Scoped(0),
            5 => {
                opening.rebuild.constructor = crate::ids::TypeId(u32::MAX);
                opening.split.constructor = crate::ids::TypeId(u32::MAX);
            }
            _ => opening.closed = crate::ids::TypeId(u32::MAX),
        }
        // Change both copies: matching the instruction to its scope is not proof.
        let changed = opening.clone();
        for statement in body.blocks.iter_mut().flat_map(|block| &mut block.statements) {
            if let MirErasedRvalue::OwnerOpen { plan, .. } = &mut statement.value { *plan = changed.clone(); }
        }
        assert!(check(&program, &key).is_err(), "corruption={corruption}");
    }
}

#[test]
fn owning_transport_never_accepts_copy_or_an_unselected_release() {
    for corruption in 0..3 {
        let (mut program, key) = fixture();
        let body = program.backend_contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap();
        for statement in body.blocks.iter_mut().flat_map(|block| &mut block.statements) {
            match &mut statement.value {
                MirErasedRvalue::OwnerOpen { source, .. } if corruption == 0 => {
                    let MirErasedOperand::Move(place) = source else { unreachable!() };
                    *source = MirErasedOperand::Copy(place.clone());
                }
                MirErasedRvalue::OwnerCall { owner, release, split, .. } if corruption != 0 => {
                    if corruption == 1 {
                        let MirErasedOperand::Move(place) = owner else { unreachable!() };
                        *owner = MirErasedOperand::Copy(place.clone());
                    } else { *release = split.clone(); }
                }
                _ => {}
            }
        }
        assert!(check(&program, &key).is_err(), "corruption={corruption}");
    }
}

#[test]
fn rebuild_raw_payload_origin_permission_cannot_be_given_to_an_unrelated_entry() {
    let (mut program, key) = fixture();
    let rebuild = program.backend_contract.erased.owner_rebuild_entries.keys().next().unwrap().clone();
    let authority = *program.backend_contract.erased.owner_rebuild_entries.values().next().unwrap();
    program.backend_contract.erased.owner_rebuild_entries.remove(&rebuild);
    assert_eq!(check(&program, &rebuild), Err(MirErasedError::BorrowEscape));
    program.backend_contract.erased.owner_rebuild_entries.insert(key, authority);
    assert!(validate_erased_contract(&program).iter().any(|(_, error)| *error == MirErasedError::InvalidType));
}

#[test]
fn consuming_ownership_requires_dominance_and_cannot_be_transferred_twice() {
    for duplicate in [false, true] {
        let (mut program, key) = fixture();
        let body = program.backend_contract.erased.functions.get_mut(&key).unwrap().body.as_mut().unwrap();
        let (block, index) = body.blocks.iter().enumerate().find_map(|(block, code)| {
            code.statements.iter().position(|statement| matches!(statement.value, MirErasedRvalue::OwnerCall { .. }))
                .map(|index| (block, index))
        }).unwrap();
        if duplicate {
            let mut second = body.blocks[block].statements[index].clone();
            body.locals.push(body.locals[second.destination.0 as usize].clone());
            second.destination = MirErasedLocal((body.locals.len() - 1) as u32);
            body.blocks[block].statements.insert(index + 1, second);
            assert_eq!(check(&program, &key), Err(MirErasedError::UninitializedOrMoved));
        } else {
            let call = body.blocks[block].statements.remove(index);
            body.blocks[0].statements.insert(0, call);
            assert!(check(&program, &key).is_err());
        }
    }
}
