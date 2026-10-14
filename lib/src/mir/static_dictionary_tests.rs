use crate::hir::{HirCallTarget, HirExprKindFor, HirSelectedMethodTarget, HirStmtFor};
use crate::mir::*;
use crate::types::{ReceiverMode, Type};

fn inspect_static_block(
    hir: &crate::hir::HirProgramFor<crate::hir::AcceptedHir>, owner: crate::ids::DefId,
    body: &crate::hir::HirBlockFor<crate::hir::AcceptedHir>, calls: &mut Vec<(crate::ids::DefId, crate::ids::DefId, crate::ids::DefId)>,
) {
    for statement in &body.stmts {
        match statement {
            HirStmtFor::Expr(expr) | HirStmtFor::Return(Some(expr)) | HirStmtFor::Break(Some(expr))
            | HirStmtFor::Let { value: expr, .. } => inspect_static_expression(hir, owner, expr, calls),
            HirStmtFor::Return(None) | HirStmtFor::Break(None) | HirStmtFor::Continue => {},
        }
    }
}

fn inspect_static_expression(
    hir: &crate::hir::HirProgramFor<crate::hir::AcceptedHir>, owner: crate::ids::DefId,
    expr: &crate::hir::HirExprFor<crate::hir::AcceptedHir>, calls: &mut Vec<(crate::ids::DefId, crate::ids::DefId, crate::ids::DefId)>,
) {
    match &expr.kind {
        HirExprKindFor::Block(body) | HirExprKindFor::UnsafeBlock(body) | HirExprKindFor::Loop(body)
        | HirExprKindFor::Lambda { body, .. } =>
            inspect_static_block(hir, owner, body, calls),
        HirExprKindFor::Open { source, body, .. } | HirExprKindFor::PackageOpen { source, body, .. } => {
            inspect_static_expression(hir, owner, source, calls);
            inspect_static_block(hir, owner, body, calls);
        }
        HirExprKindFor::PackagePack { payload: inner, .. } | HirExprKindFor::ObjectCoercion(inner, _)
        | HirExprKindFor::Ref(_, inner) | HirExprKindFor::Deref(inner) | HirExprKindFor::Cast(inner, _)
        | HirExprKindFor::UnaryOp(_, inner) | HirExprKindFor::FieldAccess(inner, _, _)
        | HirExprKindFor::TupleIndex(inner, _) | HirExprKindFor::ArrayRepeat(inner, _)
        | HirExprKindFor::Try { expr: inner, .. } => inspect_static_expression(hir, owner, inner, calls),
        HirExprKindFor::BinOp(_, left, right) | HirExprKindFor::Assign(left, right) => {
            inspect_static_expression(hir, owner, left, calls);
            inspect_static_expression(hir, owner, right, calls);
        }
        HirExprKindFor::If { condition, then_branch, else_branch } => {
            inspect_static_expression(hir, owner, condition, calls);
            inspect_static_block(hir, owner, then_branch, calls);
            if let Some(body) = else_branch { inspect_static_block(hir, owner, body, calls); }
        }
        HirExprKindFor::Match { scrutinee, arms } => {
            inspect_static_expression(hir, owner, scrutinee, calls);
            for arm in arms {
                if let Some(guard) = &arm.guard { inspect_static_expression(hir, owner, guard, calls); }
                inspect_static_block(hir, owner, &arm.body, calls);
            }
        }
        HirExprKindFor::While { condition, body } => {
            inspect_static_expression(hir, owner, condition, calls);
            inspect_static_block(hir, owner, body, calls);
        }
        HirExprKindFor::For { iter, body, .. } => {
            inspect_static_expression(hir, owner, iter, calls);
            inspect_static_block(hir, owner, body, calls);
        }
        HirExprKindFor::MethodCall(callee, _, args, _, _) | HirExprKindFor::OwnedObjectCall { owner: callee, args, .. } => {
            eprintln!("accepted non-path call boundary: source_owner={owner:?} expression={expr:?}");
            inspect_static_expression(hir, owner, callee, calls);
            for argument in args { inspect_static_expression(hir, owner, argument, calls); }
        }
        HirExprKindFor::ArrayLiteral(values) | HirExprKindFor::TupleLiteral(values)
        | HirExprKindFor::EnumVariant(_, _, values, _) | HirExprKindFor::Intrinsic { args: values, .. } => {
            for value in values { inspect_static_expression(hir, owner, value, calls); }
        }
        HirExprKindFor::StructLiteral(_, _, fields) => {
            for field in fields { inspect_static_expression(hir, owner, &field.value, calls); }
        }
        HirExprKindFor::Call(callee, args, authority) => {
            if let HirExprKindFor::Lambda { params, body, captures } = &callee.kind {
                eprintln!("accepted static path application: owner={owner:?} outer_authority={authority:?} result={:?} args={:?} lambda_signature={:?} lambda_params={params:?} captures={captures:?} body={body:?}",
                    expr.ty, args.iter().map(|arg| &arg.ty).collect::<Vec<_>>(), callee.ty);
            }
            let inner = if authority.is_none() {
                crate::mono::object_schema::applied_static_forwarder(callee, args, &expr.ty).unwrap()
            } else { None };
            let canonical = inner.unwrap_or(expr);
            if let HirExprKindFor::Call(source, _, Some(HirCallTarget::StaticMethod(target))) = &canonical.kind {
                let trait_id = target.method.trait_id().unwrap();
                let member_id = match &target.method.target {
                    HirSelectedMethodTarget::TraitMethod { member_id, .. } => *member_id,
                    HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait.as_ref().unwrap().member_id,
                };
                let declaration = &hir.traits[&trait_id];
                let member = declaration.signatures.values().find(|member| member.id == member_id).unwrap();
                eprintln!("accepted static producer: owner={owner:?} trait={trait_id:?} target_decl={:?} member={member_id:?} header={member:?} result={:?} selected_owner={:?} authority={:?} applied_forwarder={} source={:?}",
                    declaration.target, expr.ty, target.owner_ty, target.method, inner.is_some(), source.kind);
                let self_id = crate::mono::object_schema::static_member_self(declaration, member).unwrap();
                assert_eq!(member.self_receiver, None); assert_eq!(member.params, vec![Type::I64]);
                assert_eq!(member.ret, Type::Generic(self_id));
                let mut forged_header = member.clone();
                forged_header.ret = target.owner_ty.clone();
                assert!(crate::mono::object_schema::static_member_self(declaration, &forged_header).is_err(),
                    "a caller-owned generic cannot replace the declared Self ID");
                let mut forged_header = member.clone();
                forged_header.generic_params.push(crate::types::GenericParamDecl::new(
                    self_id, String::new(), crate::type_services::kind::Kind::arrow(
                        crate::type_services::kind::Kind::Type,
                        crate::type_services::kind::Kind::Type,
                    ),
                ));
                assert!(crate::mono::object_schema::static_member_self(declaration, &forged_header).is_err(),
                    "declared Self must retain Type-kind authority");
                assert!(target.method.method_substitution.is_empty());
                assert_eq!(args.len(), 1); assert_eq!(args[0].ty, Type::I64);
                assert_eq!(expr.ty, target.owner_ty);
                if inner.is_some() {
                    let mut forged = callee.as_ref().clone();
                    let HirExprKindFor::Lambda { params, .. } = &mut forged.kind else { unreachable!() };
                    params[0].ty = Type::Bool;
                    assert!(crate::mono::object_schema::applied_static_forwarder(&forged, args, &expr.ty).is_err(), "typed forwarding params cannot be guessed");
                    let mut forged = callee.as_ref().clone();
                    let HirExprKindFor::Lambda { body, .. } = &mut forged.kind else { unreachable!() };
                    let HirStmtFor::Expr(call) = &mut body.stmts[0] else { unreachable!() };
                    let HirExprKindFor::Call(source, _, _) = &mut call.kind else { unreachable!() };
                    source.kind = HirExprKindFor::Unit;
                    assert!(crate::mono::object_schema::applied_static_forwarder(&forged, args, &expr.ty).is_err(), "an untyped source cannot supply static entry authority");
                    let mut forged = callee.as_ref().clone();
                    let HirExprKindFor::Lambda { body, .. } = &mut forged.kind else { unreachable!() };
                    let HirStmtFor::Expr(call) = &mut body.stmts[0] else { unreachable!() };
                    let HirExprKindFor::Call(_, forwarded, _) = &mut call.kind else { unreachable!() };
                    forwarded[0].kind = HirExprKindFor::Unit;
                    assert!(crate::mono::object_schema::applied_static_forwarder(&forged, args, &expr.ty).is_err(), "argument types alone cannot replace actual forwarding local IDs");
                    let mut forged = callee.as_ref().clone();
                    let Type::Function { captures, .. } = &mut forged.ty else { unreachable!() };
                    let mut changed_captures = captures.to_vec();
                    changed_captures.push(crate::types::FunctionCapture {
                        kind: crate::types::CaptureKind::SharedBorrow,
                        ty: Type::Reference { inner: Box::new(Type::I64), mutable: false },
                    });
                    *captures = changed_captures.into_boxed_slice();
                    assert!(crate::mono::object_schema::applied_static_forwarder(&forged, args, &expr.ty).is_err(), "captured callables cannot be opened as receiver-free static forwarders");
                }
                calls.push((owner, trait_id, member_id));
            } else { inspect_static_expression(hir, owner, callee, calls); }
            for argument in args { inspect_static_expression(hir, owner, argument, calls); }
        }
        HirExprKindFor::IntLiteral(_) | HirExprKindFor::FloatLiteral(_) | HirExprKindFor::BoolLiteral(_)
        | HirExprKindFor::StringLiteral(_) | HirExprKindFor::CharLiteral(_) | HirExprKindFor::Unit
        | HirExprKindFor::Var(_) | HirExprKindFor::ResolvedVar(_) => {},
    }
}

pub(crate) const PRODUCER: &str = r#"
< trait Factory
    make: I64 -> Self
< trait Dispatch
    @build: I64 -> T where T: Factory
< struct Service
    < marker: I64
impl Dispatch for Service
    @build: I64 -> T where T: Factory
    @build = seed -> T::make seed
as_dispatch: &Service -> &Dispatch
< as_dispatch = service ->
    object: &Dispatch = service
    object
"#;

#[test]
fn native_static_forwarder_keeps_exact_safe_capture_free_source_authority() {
    let path = std::path::PathBuf::from("/virtual/native_static_forwarder/main.rk");
    let analysis = crate::analyze(&crate::Config {
        entry_file: path.clone(), no_std: true, no_prelude: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: r#"
struct Service
impl Service
    finish: I64 -> ()
    finish = value !-> ()
main = !-> Service::finish 2
"#.into() }], ..Default::default()
    }).unwrap();
    let main = analysis.hir.program.functions.values().find(|function| function.name == "main").unwrap();
    let HirStmtFor::Expr(expression) = &main.body.stmts[0] else { panic!("source application"); };
    let HirExprKindFor::Call(callee, args, None) = &expression.kind else { panic!("static path wrapper: {expression:?}"); };
    assert!(crate::mono::object_schema::applied_static_forwarder(callee, args, &expression.ty).is_err());
    assert!(crate::mono::object_schema::applied_native_static_forwarder(callee, args, &expression.ty).unwrap().is_some());
    for control in 0..5 {
        let mut forged = callee.as_ref().clone();
        let HirExprKindFor::Lambda { params, body, .. } = &mut forged.kind else { unreachable!(); };
        match control {
            0 => params[0].mutable = true,
            1 => params[0].ty = Type::Bool,
            2 => {
                let HirStmtFor::Expr(inner) = &mut body.stmts[0] else { unreachable!(); };
                let HirExprKindFor::Call(source, _, _) = &mut inner.kind else { unreachable!(); };
                source.kind = HirExprKindFor::Unit;
            }
            3 => {
                let HirStmtFor::Expr(inner) = &mut body.stmts[0] else { unreachable!(); };
                let HirExprKindFor::Call(_, forwarded, _) = &mut inner.kind else { unreachable!(); };
                forwarded[0].kind = HirExprKindFor::Unit;
            }
            _ => {
                let Type::Function { safety, .. } = &mut forged.ty else { unreachable!(); };
                *safety = crate::types::FunctionSafety::Unsafe;
            }
        }
        assert!(crate::mono::object_schema::applied_native_static_forwarder(&forged, args, &expression.ty).is_err(), "control={control}");
    }
}

pub(crate) fn selected_source() -> String {
    PRODUCER.replace("< trait Factory", "< trait StaticMake\n    create: I64 -> Self\n< trait Factory")
        .replace("where T: Factory", "where T: StaticMake")
        .replace("< trait Dispatch", "impl Factory for T where T: StaticMake\n    make = seed -> T::create seed\n< trait Dispatch")
        .replace("< trait Dispatch", "static_helper: I64 -> U where U: Factory\nstatic_helper = seed -> U::make seed\n< trait Dispatch")
        .replace("@build = seed -> T::make seed", "@build = seed -> static_helper seed")
}

pub(crate) fn accepted_program(source: &str) -> MirProgram {
    let path = std::path::PathBuf::from("/virtual/static_dictionary/producer.rk");
    let config = crate::Config { entry_file: path.clone(), no_std: true, no_prelude: true, no_link: true,
        source_providers: vec![crate::SourceProvider::Virtual { path, text: source.into() }], ..Default::default() };
    let frontend = crate::compile_frontend(&config).unwrap();
    let fingerprint = crate::product_source_fingerprint(&config, &frontend.source_db, &frontend.fingerprint_source_files);
    let identity = crate::mono::CompilationIdentityContext::new(frontend.analysis.hir.root_crate_id,
        crate::products::ProductCrateIdentity::local(frontend.effective_current_crate_name.clone()).with_source_fingerprint(fingerprint),
        frontend.crate_ctx.product_crate_ids.iter().map(|(identity, id)| (*id, identity.clone())).collect());
    let hir = &frontend.analysis.hir.program;
    for declaration in hir.traits.values().filter(|declaration| declaration.signatures.values().any(|member| member.self_receiver.is_none())) {
        eprintln!("accepted static trait: id={:?} target={:?} generics={:?} signatures={:?}",
            declaration.id, declaration.target, declaration.generic_params, declaration.signatures);
    }
    let mut static_calls = Vec::new();
    for method in hir.functions.values().chain(hir.impls.values().flat_map(|implementation| implementation.methods.values())) {
        eprintln!("accepted static source owner={:?} generics={:?} params={:?} ret={:?}", method.id, method.generic_params, method.params, method.ret_type);
        eprintln!("accepted declared source bounds: owner={:?} bounds={:?}", method.id, method.generic_bounds);
        eprintln!("accepted complete source body: owner={:?} body={:?}", method.id, method.body);
        inspect_static_block(hir, method.id, &method.body, &mut static_calls);
    }
    assert!(!static_calls.is_empty(), "proof requires an actual accepted receiver-free static call");
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, identity).unwrap();
    let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut mono);
    let program = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies);
    for (method, trait_id, member_id) in static_calls {
        assert!(program.backend_contract.erased.functions.iter().any(|(key, function)| {
            if key.definition != method { return false; }
            let dictionaries = function.signature.dictionaries.iter().chain(program.backend_contract.erased.dictionary_factories
                .get(key).into_iter().flatten().map(|requirement| &requirement.result)).collect::<Vec<_>>();
            function.body.as_ref().is_some_and(|body| body.blocks.iter().flat_map(|block| &block.statements).any(|statement| {
                let MirErasedRvalue::DictionaryCall { dictionary, member, receiver, args, .. } = &statement.value else { return false; };
                dictionaries.get(*dictionary as usize).is_some_and(|schema|
                    schema.trait_id == trait_id && schema.members.get(*member as usize).is_some_and(|entry|
                        entry.member_id == member_id && entry.receiver.is_none() && entry.ret == schema.subject))
                    && receiver.is_none() && args.len() == 1
            }))
        }), "accepted method {method:?}/trait {trait_id:?}/member {member_id:?} must reach the genuine receiver-free MIR dictionary call");
    }
    program
}

pub(crate) fn fixture(selected: bool) -> MirProgram {
    if selected { accepted_program(&selected_source()) } else { accepted_program(PRODUCER) }
}

#[test]
fn static_dictionary_accepted_producer_has_no_receiver_and_exact_self_result() {
    let program = fixture(false);
    let report = agreement::check_mir_runtime_agreement(&program);
    assert!(report.is_clean(), "{report:?}");
    assert!(validate_erased_contract(&program).is_empty());
    let slot = program.backend_contract.object_schemas.values().flat_map(|schema| &schema.erased_slots).next().unwrap();
    assert_eq!(slot.signature.parameters.len(), 2);
    assert_eq!(slot.signature.params.len(), 2, "virtual Dispatch retains its real Service receiver and seed");
    assert_eq!(slot.signature.ret, MirErasedType::Parameter(1));
    assert!(slot.signature.require_borrow_free.is_empty());
    assert_eq!(slot.signature.dictionaries.len(), 1);
    let dictionary = &slot.signature.dictionaries[0];
    let member = &dictionary.members[0];
    assert_eq!(member.receiver, None);
    assert_eq!(member.params, vec![MirErasedType::Concrete(program.type_context.id_for_type(&Type::I64).unwrap())]);
    assert_eq!(member.ret, dictionary.subject);
    assert!(member.binder.parameters.is_empty());
    assert_eq!(member.borrowing.returns.len(), 1);
    assert_eq!(member.borrowing.returns[0].sources[0].argument, 0);
    assert_eq!(member.borrowing.returns[0].sources[0].kind, crate::hir::borrowing::BorrowSourceKind::Value);
    assert!(member.borrowing.retentions.is_empty());
    assert!(program.backend_contract.erased.dictionaries.is_empty(), "producer knows no later Self");
    let calls = program.backend_contract.erased.functions.values().filter_map(|function| function.body.as_ref())
        .flat_map(|body| &body.blocks).flat_map(|block| &block.statements).filter_map(|statement| match &statement.value {
            MirErasedRvalue::DictionaryCall { receiver, args, type_args, dictionaries, .. } => Some((receiver, args, type_args, dictionaries)), _ => None,
        }).collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].0.is_none()); assert_eq!(calls[0].1.len(), 1);
    assert!(calls[0].2.is_empty() && calls[0].3.is_empty());
}

#[test]
fn static_dictionary_selected_template_captures_only_parent_evidence() {
    let program = fixture(true);
    assert!(validate_erased_contract(&program).is_empty(), "{:?}", validate_erased_contract(&program));
    assert_eq!(program.backend_contract.erased.dictionary_templates.len(), 1);
    let template = program.backend_contract.erased.dictionary_templates.values().next().unwrap();
    assert_eq!(template.parameters.len(), 1); assert_eq!(template.bounds.len(), 1);
    let member = &template.result.members[0];
    assert!(member.receiver.is_none()); assert_eq!(member.ret, MirErasedType::Parameter(0));
    let entry = &program.backend_contract.erased.functions[&template.members[0].entry];
    assert_eq!(entry.signature.params.len(), 1);
    assert_eq!(entry.signature.dictionaries.len(), 1);
    assert!(entry.signature.require_borrow_free.is_empty());
    assert!(entry.body.as_ref().unwrap().blocks.iter().flat_map(|block| &block.statements).all(|statement|
        !matches!(statement.value, MirErasedRvalue::NativeCall { .. } | MirErasedRvalue::NativeApply { .. })));
    assert!(program.backend_contract.runtime_requirements.contains(&MirRuntimeHelper::SelectedDictionaryFactory));
}

#[test]
fn static_dictionary_rejects_fake_receiver_call_arity_result_and_origin() {
    for control in 0..7 {
        let mut program = fixture(false);
        let key = program.backend_contract.erased.functions.iter().find(|(_, function)| function.body.as_ref().is_some_and(|body|
            body.blocks.iter().flat_map(|block| &block.statements).any(|statement| matches!(statement.value, MirErasedRvalue::DictionaryCall { .. }))))
            .map(|(key, _)| key.clone()).unwrap();
        let function = program.backend_contract.erased.functions.get_mut(&key).unwrap();
        let member = &mut function.signature.dictionaries[0].members[0];
        match control {
            0 => member.receiver = Some(ReceiverMode::Shared),
            1 => member.ret = MirErasedType::Parameter(0),
            2 => member.borrowing.returns.clear(),
            3 => member.borrowing.returns[0].sources[0].kind = crate::hir::borrowing::BorrowSourceKind::Contents,
            6 => member.borrowing.retentions.push(crate::hir::borrowing::BorrowRetention {
                destination: crate::hir::borrowing::BorrowSource { argument: 0, path: vec![], kind: crate::hir::borrowing::BorrowSourceKind::Contents },
                sources: member.borrowing.returns[0].sources.clone(),
            }),
            _ => {
                let value = function.body.as_mut().unwrap().blocks.iter_mut().flat_map(|block| &mut block.statements)
                    .find_map(|statement| if let MirErasedRvalue::DictionaryCall { receiver, args, .. } = &mut statement.value { Some((receiver, args)) } else { None }).unwrap();
                if control == 4 { *value.0 = Some(MirErasedPlace::local(MirErasedLocal(0))); } else { value.1.clear(); }
            }
        }
        assert!(!validate_erased_contract(&program).is_empty(), "control {control}");
    }
}

#[test]
fn static_dictionary_schema_keys_frame_no_receiver_scalar_and_full_borrowing() {
    let program = fixture(false);
    let contract = &program.backend_contract; let tc = &program.type_context;
    let mut schema = contract.object_schemas.values().flat_map(|schema| &schema.erased_slots).next().unwrap().signature.dictionaries[0].clone();
    let key = dictionary_schema_key(contract, tc, &schema, false).unwrap();
    assert!(key.windows(4).any(|word| word == b"BQ01"));
    schema.members[0].receiver = Some(ReceiverMode::Move);
    assert!(dictionary_schema_key(contract, tc, &schema, false).is_err());
    schema.members[0].receiver = None;
    schema.members[0].borrowing.returns[0].sources[0].argument = 1;
    assert!(dictionary_schema_key(contract, tc, &schema, false).is_err());
}
