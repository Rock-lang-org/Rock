use std::collections::BTreeSet;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::*;
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, Type};

fn accepted_forward_producer() -> MirProgram {
    // The original producer bodies, with no later U layout or caller instance.
    let path = std::path::PathBuf::from("/virtual/polymorphic_dictionary/producer.rk");
    let source = r#"
< trait Forward
    @forward: U -> U
< trait Dispatch
    @apply: M -> U -> U where M: Forward
< struct Service
    < marker: I64
impl Dispatch for Service
    @apply = maker, value -> maker.forward value
as_dispatch: &Service -> &Dispatch
< as_dispatch = service ->
    object: &Dispatch = service
    object
< struct Maker
    < reads: *I64
note: &Maker -> ()
note = maker !->
    unsafe *maker.reads = ~I64Add (*maker.reads), 1
impl Forward for Maker
    @forward = value ->
        note self
        value
"#;
    let config = crate::Config {
        entry_file: path.clone(), no_std: true, no_prelude: true, no_link: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: source.into() }],
        ..Default::default()
    };
    let frontend = crate::compile_frontend(&config).unwrap();
    let fingerprint = crate::product_source_fingerprint(&config, &frontend.source_db, &frontend.fingerprint_source_files);
    let identity = crate::mono::CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone())
            .with_source_fingerprint(fingerprint),
        frontend.crate_ctx.product_crate_ids.iter().map(|(identity, id)| (*id, identity.clone())).collect(),
    );
    let analysis = frontend.analysis;
    let hir = &analysis.hir.program;
    let dispatch = hir.traits.values().find(|declaration| declaration.name == "Dispatch").unwrap();
    let forward = hir.traits.values().find(|declaration| declaration.name == "Forward").unwrap();
    let apply = dispatch.signatures.values().next().unwrap();
    let apply_id = apply.id;
    let forward_member = forward.signatures.values().next().unwrap();
    let Type::Reference { inner: declared_self, mutable: false } = &apply.params[0] else { panic!("declared shared receiver missing") };
    let Type::Generic(apply_self) = declared_self.as_ref() else { panic!("declared Self authority missing") };
    let apply_self = *apply_self;
    let Type::Generic(apply_m) = apply.params[1] else { panic!("declared M missing") };
    let Type::Generic(apply_u) = apply.ret else { panic!("declared U missing") };
    let Type::Generic(forward_u) = forward_member.ret else { panic!("declared member-local U missing") };
    assert_ne!(apply_u, forward_u);
    assert_ne!(apply_m, forward_u);
    let implementation = hir.impls.values().find(|implementation| implementation.trait_id == Some(dispatch.id)).unwrap();
    let method = implementation.methods.values().next().unwrap();
    let Type::Generic(actual_u) = method.ret_type else { panic!("accepted impl U missing") };
    let call = method.body.stmts.iter().find_map(|statement| match statement {
        crate::hir::HirStmtFor::Expr(expression) | crate::hir::HirStmtFor::Return(Some(expression)) => Some(expression),
        _ => None,
    }).unwrap();
    let crate::hir::HirExprKindFor::MethodCall(_, _, arguments, _, target) = &call.kind else { panic!("genuine Forward call missing") };
    assert_eq!(target.trait_id(), Some(forward.id));
    assert_eq!(target.method_substitution, vec![crate::hir::HirTypeBinding { param: forward_u, ty: Type::Generic(actual_u) }]);
    assert_eq!(arguments[0].ty, Type::Generic(actual_u));
    eprintln!("accepted producer binders: Dispatch {:?} Self={apply_self:?} M={apply_m:?} U={apply_u:?}; impl {:?} generics={:?} receiver={:?} ret={:?}; Forward {:?} U={forward_u:?}; call={target:?}",
        apply_id, method.id, method.generic_params, method.params[0].ty, method.ret_type, forward_member.id);
    let mut mono = crate::mono::monomorphize_with_crates(analysis.hir, &frontend.crate_ctx, identity).unwrap();
    let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    let report = crate::mir::agreement::check_mir_runtime_agreement(&program);
    assert!(report.is_clean(), "actual producer authority: {report:?}");
    let schema = program.backend_contract.object_schemas.values().find(|schema|
        schema.erased_slots.iter().any(|slot| slot.member_id == apply_id)).unwrap();
    let signature = &schema.erased_slots.iter().find(|slot| slot.member_id == apply_id).unwrap().signature;
    assert_eq!(signature.parameters.iter().map(|p| p.source).collect::<Vec<_>>(), vec![apply_self, apply_m, apply_u]);
    assert_eq!(signature.params[0], MirErasedType::Reference { inner: Box::new(MirErasedType::Parameter(0)), mutable: false });
    assert_eq!(signature.layouts, vec![MirErasedType::Parameter(0), MirErasedType::Parameter(1), MirErasedType::Parameter(2)]);
    assert_eq!(signature.dictionaries.len(), 1);
    assert!(signature.require_borrow_free.is_empty());
    let member = &signature.dictionaries[0].members[0];
    assert_eq!(member.binder.parameters[0].source, forward_u);
    assert_eq!(member.params, vec![MirErasedType::Parameter(1), MirErasedType::MemberParameter(0)]);
    assert_eq!(member.ret, MirErasedType::MemberParameter(0));
    program
}

pub(crate) fn fixture() -> (MirProgram, MirErasedKey, DefId) {
    let (mut program, producer, implementation) = crate::mir::dictionary_factory::tests::fixture();
    let contract = &mut program.backend_contract;
    let template = contract.erased.dictionary_templates.get_mut(&implementation).unwrap();
    let member = &mut template.result.members[0];
    member.binder.parameters = vec![MirErasedParameter { source: GenericParamId { owner: member.member_id, index: 0 }, kind: Kind::Type }];
    member.params.push(MirErasedType::MemberParameter(0));
    member.ret = MirErasedType::MemberParameter(0);
    member.borrowing = parametric_member_borrow_contract(&member.binder, &member.params, &member.ret, member.receiver).unwrap();
    let member = member.clone();
    let template = template.clone();
    contract.dictionary_impl_members.get_mut(&implementation).unwrap().template = Some(template.clone());
    contract.erased.dictionary_factories.get_mut(&producer).unwrap()[0].result = template.result.clone();
    let entry_key = template.members[0].entry.clone();
    let mut signature = member.entry_signature(&template.parameters).unwrap();
    signature.dictionaries = template.bounds.clone();
    let scalar = contract.erased.functions[&entry_key].signature.ret.clone();
    let authority = MirErasedMemberEntry { implementation, trait_id: template.result.trait_id, method_id: entry_key.definition,
        captures: template.parameters.clone(), member };
    contract.dictionary_member_authorities.insert(entry_key.clone(), authority.clone());
    contract.erased.member_entries.insert(entry_key.clone(), authority);
    let entry = contract.erased.functions.get_mut(&entry_key).unwrap();
    entry.signature = signature.clone();
    entry.body = Some(MirErasedBody { locals: signature.params.iter().cloned().chain([scalar]).collect(), scopes: vec![], mutable_locals: BTreeSet::new(),
        blocks: vec![MirErasedBasicBlock { statements: vec![MirErasedAssignment { destination: MirErasedLocal(2), value: MirErasedRvalue::DictionaryCall {
            dictionary: 0, member: 0, type_args: vec![], dictionaries: vec![],
            receiver: Some(MirErasedPlace { local: MirErasedLocal(0), projection: vec![MirErasedProjection::Deref] }), args: vec![],
        } }], terminator: MirErasedTerminator::Return(MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1)))) }] });
    let helper = contract.erased.functions.keys().find(|key| key.definition.local.0 == 41).unwrap().clone();
    for key in [&producer, &helper] {
        let function = contract.erased.functions.get_mut(key).unwrap();
        function.signature.parameters.push(MirErasedParameter { source: GenericParamId { owner: key.definition, index: 1 }, kind: Kind::Type });
        function.signature.params.push(MirErasedType::Parameter(1));
        function.signature.ret = MirErasedType::Parameter(1);
        function.signature.layouts.push(MirErasedType::Parameter(1));
        function.signature.require_borrow_free.clear();
        if key == &helper { function.signature.dictionaries = vec![template.result.clone()]; }
        let value = if key == &producer { MirErasedRvalue::Call { target: helper.clone(), type_args: vec![MirErasedType::Parameter(0), MirErasedType::Parameter(1)],
            dictionaries: vec![1], args: vec![MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0))), MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1)))] }
        } else { MirErasedRvalue::DictionaryCall { dictionary: 0, member: 0, type_args: vec![MirErasedType::Parameter(1)], dictionaries: vec![],
            receiver: Some(MirErasedPlace::local(MirErasedLocal(0))), args: vec![MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1)))] } };
        function.body = Some(MirErasedBody { locals: vec![MirErasedType::Parameter(0), MirErasedType::Parameter(1), MirErasedType::Parameter(1)],
            scopes: vec![], mutable_locals: BTreeSet::new(), blocks: vec![MirErasedBasicBlock { statements: vec![MirErasedAssignment { destination: MirErasedLocal(2), value }],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(2)))) }] });
    }
    contract.runtime_requirements = crate::mir::backend_contract::runtime_requirements_for_backend(contract, std::iter::empty());
    (program, producer, implementation)
}

#[test]
fn polymorphic_dictionary_member_has_separate_local_and_capture_binders() {
    let source_program = accepted_forward_producer();
    assert_eq!(source_program.backend_contract.erased.member_entries.len(), 1);
    let (program, producer, implementation) = fixture();
    assert!(validate_erased_contract(&program).is_empty(), "{:?}", validate_erased_contract(&program));
    let contract = &program.backend_contract;
    let template = &contract.erased.dictionary_templates[&implementation];
    let member = &template.result.members[0];
    assert_eq!(member.ret, MirErasedType::MemberParameter(0));
    let substituted = template.result.substitute(&[MirErasedType::Concrete(program.type_context.id_for_type(&Type::I64).unwrap())], &program.type_context).unwrap();
    assert_eq!(substituted.members[0].ret, MirErasedType::MemberParameter(0));
    assert_eq!(member.borrowing.returns[0].sources[0].argument, 1);
    assert_eq!(contract.erased.functions[&producer].signature.dictionaries.len(), 1);
    assert!(contract.erased.functions[&template.members[0].entry].signature.require_borrow_free.is_empty());
    let mut parents = Vec::new();
    member.collect_parent_layouts(&mut parents, false);
    assert_eq!(parents, vec![MirErasedType::Parameter(0)]);
}

#[test]
fn polymorphic_dictionary_keys_alpha_normalize_but_never_drop_borrowing() {
    let (program, _, implementation) = fixture();
    let contract = &program.backend_contract;
    let mut schema = contract.erased.dictionary_templates[&implementation].result.clone();
    let original = dictionary_schema_key(contract, &program.type_context, &schema, false).unwrap();
    assert!(original.windows(4).any(|w| w == b"PM01"));
    assert!(original.windows(4).any(|w| w == b"BQ01"));
    schema.members[0].binder.parameters[0].source = GenericParamId { owner: DefId::new(CrateId(0), LocalDefId(999)), index: 9 };
    assert_eq!(dictionary_schema_key(contract, &program.type_context, &schema, false).unwrap(), original);
    schema.members[0].borrowing.returns.clear();
    assert_eq!(dictionary_schema_key(contract, &program.type_context, &schema, false), Err(MirErasedError::InvalidDictionary));
}

#[test]
fn polymorphic_dictionary_rejects_kind_ordinal_capture_result_and_call_arity_forgery() {
    for control in 0..11 {
        let (mut program, _, implementation) = fixture();
        let contract = &mut program.backend_contract;
        let entry = contract.erased.dictionary_templates[&implementation].members[0].entry.clone();
        let helper = contract.erased.functions.keys().find(|key| key.definition.local.0 == 41).unwrap().clone();
        match control {
            0 => contract.erased.dictionary_templates.get_mut(&implementation).unwrap().result.members[0].binder.parameters[0].kind = Kind::arrow(Kind::Type, Kind::Type),
            1 => contract.erased.member_entries.get_mut(&entry).unwrap().member.ret = MirErasedType::MemberParameter(1),
            2 => contract.erased.member_entries.get_mut(&entry).unwrap().member.binder.parameters[0].source = contract.erased.member_entries[&entry].captures[0].source,
            3 => contract.erased.functions.get_mut(&entry).unwrap().signature.ret = MirErasedType::Parameter(0),
            4 => { let MirErasedRvalue::DictionaryCall { type_args, .. } = &mut contract.erased.functions.get_mut(&helper).unwrap().body.as_mut().unwrap().blocks[0].statements[0].value else { unreachable!() }; type_args.clear(); }
            5 => { let MirErasedRvalue::DictionaryCall { type_args, .. } = &mut contract.erased.functions.get_mut(&helper).unwrap().body.as_mut().unwrap().blocks[0].statements[0].value else { unreachable!() }; type_args[0] = MirErasedType::Parameter(0); }
            6 => contract.dictionary_member_authorities.clear(),
            7 => { contract.erased.functions.get_mut(&entry).unwrap().signature.require_borrow_free.insert(MirErasedType::Parameter(1)); }
            8 => { let function = contract.erased.functions.get_mut(&entry).unwrap(); function.body = None; function.linkage = MirLinkage::External; }
            9 => { let function = contract.erased.functions.get_mut(&entry).unwrap(); function.body = None; function.linkage = MirLinkage::External;
                contract.erased.imported_member_links.insert(entry.clone(), ("wrong-authority".into(), contract.erased.member_entries[&entry].clone())); }
            _ => contract.erased.functions.get_mut(&helper).unwrap().signature.layouts.push(MirErasedType::MemberParameter(0)),
        }
        assert!(!validate_erased_contract(&program).is_empty(), "control {control}");
    }
}

#[test]
fn polymorphic_dictionary_opening_preserves_actual_reference_input_origins() {
    let (mut program, _, implementation) = fixture();
    let member = &program.backend_contract.erased.dictionary_templates[&implementation].result.members[0];
    let reference = MirErasedType::Concrete(program.type_context.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: false }));
    let (params, ret) = member.opened_types(&[reference.clone()]).unwrap();
    assert_eq!(params[1], reference); assert_eq!(ret, reference);
    assert!(member.borrowing.retentions.is_empty());
    assert!(member.borrowing.returns.iter().flat_map(|effect| &effect.sources).all(|source| source.argument == 1));
    assert_eq!(member.ret.kind(&program.backend_contract.erased.functions.values().next().unwrap().signature, &program.backend_contract, &program.type_context), Err(MirErasedError::UnboundParameter));
    let scalar = MirErasedType::Concrete(program.type_context.id_for_type(&Type::I64).unwrap());
    let shared = MirErasedType::Reference { inner: Box::new(scalar.clone()), mutable: false };
    assert!(erased_types_agree(&shared, &reference, &program.type_context));
    assert!(!erased_types_agree(&MirErasedType::Reference { inner: Box::new(scalar), mutable: true }, &reference, &program.type_context));
    assert!(!erased_types_agree(&MirErasedType::Parameter(0), &reference, &program.type_context));
}

#[test]
fn polymorphic_dictionary_body_cannot_return_receiver_loans_or_use_a_native_closed_bridge() {
    let (mut program, _, implementation) = fixture();
    let entry_key = program.backend_contract.erased.dictionary_templates[&implementation].members[0].entry.clone();
    program.backend_contract.erased.functions.get_mut(&entry_key).unwrap().body.as_mut().unwrap().blocks[0].terminator =
        MirErasedTerminator::Return(MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0))));
    assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&entry_key], &program.backend_contract, &program.type_context), Err(MirErasedError::BorrowEscape));
    let (mut program, _, implementation) = fixture();
    let entry_key = program.backend_contract.erased.dictionary_templates[&implementation].members[0].entry.clone();
    program.backend_contract.erased.functions.get_mut(&entry_key).unwrap().body.as_mut().unwrap().blocks[0].statements[0].value =
        MirErasedRvalue::NativeCall { target: MirCallableKey::Extern(entry_key.definition), args: vec![] };
    assert_eq!(validate_member_entry(&entry_key, &program.backend_contract, &program.type_context), Err(MirErasedError::InvalidDictionary));
}
