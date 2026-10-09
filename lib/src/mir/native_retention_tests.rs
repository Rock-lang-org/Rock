use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::mir::borrowck::BorrowChecker;
use crate::mir::builder::MirBuilder;
use crate::mir::erased::{
    validate_erased_contract, validate_erased_environment, validate_erased_environment_with_proof,
    MirErasedCall, MirErasedError,
};
use crate::mono::CompilationIdentityContext;
use crate::products::ProductCrateIdentity;
use crate::{Config, SourceProvider};

const PROVIDER: &str = r#"
lang fn_once
< trait Consume Args, Ret
    lang output
    type Output
    lang method
    ~@consume: Args -> Ret
lang fn_mut
< trait Change Args, Ret where Self: Consume Args, Ret
    lang output
    type Output
    lang method
    ^@change: Args -> Ret
lang fn
< trait Observe Args, Ret where Self: Change Args, Ret
    lang output
    type Output
    lang method
    @observe: Args -> Ret
< trait Callbacks
    @once: F -> T -> R where F: Consume T, R
< struct Service
    < marker: I64
impl Callbacks for Service
    @once = callback, value -> callback value
as_callbacks: &Service -> &Callbacks
< as_callbacks = service ->
    object: &Callbacks = service
    object
"#;

fn source_mir(body: &str) -> MirProgram {
    let path = PathBuf::from("/virtual/retention_proof/main.rk");
    let frontend = crate::compile_frontend(&Config {
        entry_file: path.clone(),
        current_crate_name: Some("retention_proof".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path, text: format!("{PROVIDER}\n{body}") }],
        ..Config::default()
    }).expect("retention fixture must reach accepted HIR");
    let identity = CompilationIdentityContext::new(
        frontend.analysis.hir.root_crate_id,
        ProductCrateIdentity::local("retention_proof".into()), BTreeMap::new(),
    );
    let mut mono = crate::mono::monomorphize_with_crates(frontend.analysis.hir, &frontend.crate_ctx, identity)
        .expect("retention fixture must monomorphize");
    let mut bodies = MirBuilder::take_mir_instance_bodies(&mut mono);
    crate::dce::prune_unreachable_instances(&mut mono, &mut bodies);
    MirBuilder::build_monomorphized_with_instance_bodies(&mono, &bodies)
}

const UNIQUE: &str = r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    mut value = 0
    callback = reference -> reference
    result = object.once callback, &mut value
    *result = 42
    value
"#;

const SETTER: &str = r#"
main = !->
    service = Service
        marker: 0
    object = as_callbacks &service
    first = 1
    second = 2
    mut slot = &first
    callback = reference !-> slot = reference
    object.once callback, &second
"#;

fn call_site(program: &MirProgram) -> (&MirFunction, usize, &MirErasedCall) {
    program.functions.values().find_map(|function| {
        function.basic_blocks.iter().enumerate().find_map(|(index, block)| match &block.terminator {
            Some(Terminator::Call { func: Operand::Constant(crate::mir::Constant::ErasedCall(call)), .. }) => {
                Some((function, index, call))
            }
            _ => None,
        })
    }).expect("fixture contains its erased callback entry")
}

fn actual_closure(function: &MirFunction) -> (&MirClosure, TypeId) {
    function.basic_blocks.iter().flat_map(|block| &block.statements).find_map(|statement| {
        match &statement.kind {
            StatementKind::Assign(destination, Rvalue::Closure(closure)) => {
                Some((closure, function.local_decls[destination.local.0].ty))
            }
            _ => None,
        }
    }).expect("fixture contains a real native closure construction")
}

#[test]
fn checked_native_identity_environment_certifies_consuming_unique_input() {
    let program = source_mir(UNIQUE);
    let (function, block, call) = call_site(&program);
    let proof = NativeRetentionProof::for_call(&program, function, block);
    assert_eq!(validate_erased_environment(&program.backend_contract, &program.type_context, call),
        Err(MirErasedError::BorrowEscape), "signature metadata alone is still insufficient");
    validate_erased_environment_with_proof(&program.backend_contract, &program.type_context, call, Some(&proof)).unwrap();
    assert!(validate_erased_contract(&program).is_empty());
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn the_checked_environment_seal_follows_exact_native_value_forwarding() {
    let source = UNIQUE.replace("    result = object.once callback,",
        "    forwarded = callback\n    result = object.once forwarded,");
    let program = source_mir(&source);
    assert!(validate_erased_contract(&program).is_empty());
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn a_native_environment_seal_does_not_certify_a_custom_dictionary_target() {
    let program = source_mir(UNIQUE);
    let (function, block, call) = call_site(&program);
    let proof = NativeRetentionProof::for_call(&program, function, block);
    let (closure, _) = actual_closure(function);
    let mut contract = program.backend_contract.clone();
    let dictionary = contract.erased.dictionaries.get_mut(&call.dictionaries[0]).unwrap();
    dictionary.members[0].target = MirDictionaryMemberTarget::Native(MirCallableKey::Closure(
        MirFunctionId::Closure(Box::new(closure.id.clone())),
    ));
    assert_eq!(validate_erased_environment_with_proof(&contract, &program.type_context, call, Some(&proof)),
        Err(MirErasedError::BorrowEscape), "only the native-callable adapter can use this storage proof");
    let erased = contract.erased.functions.keys().next().unwrap().clone();
    contract.erased.dictionaries.get_mut(&call.dictionaries[0]).unwrap().members[0].target =
        MirDictionaryMemberTarget::Polymorphic(erased);
    assert_eq!(validate_erased_environment_with_proof(&contract, &program.type_context, call, Some(&proof)),
        Err(MirErasedError::BorrowEscape), "an erased target cannot inherit the native closure storage seal");
    let mut contract = program.backend_contract.clone();
    let subject = contract.erased.dictionaries[&call.dictionaries[0]].subject;
    contract.erased.dictionaries.get_mut(&call.dictionaries[0]).unwrap().members[0].receiver = None;
    let mut signature = call.target.signature(&contract).unwrap().clone();
    signature.dictionaries[0].members[0].receiver = None;
    assert!(!proof.covers_subject(&signature, 0, 0, &program.type_context.type_for(subject), &contract, &program.type_context),
        "receiver-free authority cannot inherit a native callable receiver-storage seal even when both rows are forged absent");
}

#[test]
fn lossy_empty_function_signature_without_an_actual_environment_stays_opaque() {
    let mut program = source_mir(UNIQUE);
    let (function, _, _) = call_site(&program);
    let id = function.id.clone();
    let (_, ty) = actual_closure(function);
    assert!(matches!(program.type_context.type_for(ty), Type::Function { captures, .. } if captures.is_empty()));
    // Model a supplied opaque native value: retain its exact function type but
    // remove the storage-construction authority. The seal analysis must not
    // invent initialization or a capture layout from that signature.
    for block in &mut program.functions.get_mut(&id).unwrap().basic_blocks {
        block.statements.retain(|statement| !matches!(&statement.kind,
            StatementKind::Assign(_, Rvalue::Closure(_))));
    }
    let (function, block, call) = call_site(&program);
    let proof = NativeRetentionProof::for_call(&program, function, block);
    assert_eq!(validate_erased_environment_with_proof(
        &program.backend_contract, &program.type_context, call, Some(&proof)), Err(MirErasedError::BorrowEscape));
}

#[test]
fn capture_slot_declarations_not_lossy_signature_metadata_control_the_seal() {
    let mut program = source_mir(SETTER);
    let (function, _, _) = call_site(&program);
    let (closure, ty) = actual_closure(function);
    let owner = function.id.clone();
    let closure = closure.clone();
    let Type::Function { params, ret, .. } = program.type_context.type_for(ty) else { panic!("native function type"); };
    let lossy = program.type_context.intern_type(&Type::function(params, *ret));
    assert!(matches!(program.type_context.type_for(lossy), Type::Function { captures, .. } if captures.is_empty()));
    assert!(check_closure(&program, &program.functions[&owner], &closure, lossy).is_none(),
        "actual borrowed capture slots cannot be erased by an empty signature capture list");
    let (function, block, call) = call_site(&program);
    let proof = NativeRetentionProof::for_call(&program, function, block);
    assert_eq!(validate_erased_environment_with_proof(
        &program.backend_contract, &program.type_context, call, Some(&proof)), Err(MirErasedError::BorrowEscape));
}

#[test]
fn unit_setter_retention_reports_a_source_borrow_error_before_agreement() {
    let program = source_mir(SETTER);
    let diagnostics = BorrowChecker::run(&program).expect_err("unit does not erase retention effects");
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Borrow)
            && diagnostic.message.contains("unproved destination environment")
            && diagnostic.primary.as_ref().is_some_and(|label| label.span.start < label.span.end)
    }), "{diagnostics:?}");
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code != Some(crate::diagnostic::DiagnosticCode::Internal)),
        "{diagnostics:?}");
    assert!(validate_erased_contract(&program).contains(&(None, MirErasedError::BorrowEscape)));
}

#[test]
fn a_checked_empty_environment_allows_borrowed_input_even_for_unit_output() {
    let program = source_mir(r#"
main = !->
    service = Service
        marker: 0
    object = as_callbacks &service
    value = 1
    callback = _ !-> ()
    object.once callback, &value
"#);
    assert!(validate_erased_contract(&program).is_empty());
    BorrowChecker::run(&program).unwrap();
}

#[test]
fn a_borrowed_scalar_capture_is_not_a_reference_free_scalar_environment() {
    let program = source_mir(r#"
main = ->
    service = Service
        marker: 0
    object = as_callbacks &service
    value = 42
    callback = _ -> value
    object.once callback, ()
"#);
    let (function, _, _) = call_site(&program);
    let (closure, ty) = actual_closure(function);
    assert!(!closure.captures.is_empty());
    assert!(closure.captures.iter().any(|capture| capture.kind == MirClosureCaptureKind::ByRef));
    assert!(check_closure(&program, function, closure, ty).is_none());
    let id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    let owner = function.id.clone();
    let closure = closure.clone();
    let mut missing = program.clone();
    missing.functions.get_mut(&id).unwrap().closure_captures.clear();
    assert!(check_closure(&missing, &missing.functions[&owner], &closure, ty).is_none());
}

#[test]
fn an_unregistered_or_incomplete_closure_layout_cannot_mint_a_seal() {
    let program = source_mir(UNIQUE);
    let (function, _, _) = call_site(&program);
    let (closure, ty) = actual_closure(function);
    let closure_id = MirFunctionId::Closure(Box::new(closure.id.clone()));
    let owner = function.id.clone();
    let closure = closure.clone();
    let mut missing_body = program.clone();
    missing_body.functions.remove(&closure_id);
    assert!(check_closure(&missing_body, &missing_body.functions[&owner], &closure, ty).is_none());
    let mut missing_declaration = program.clone();
    missing_declaration.backend_contract.function_bodies.remove(&closure_id);
    assert!(check_closure(&missing_declaration, &missing_declaration.functions[&owner], &closure, ty).is_none());
    let mut wrong_owner = closure.clone();
    wrong_owner.id.local_index = wrong_owner.id.local_index.wrapping_add(1);
    assert!(check_closure(&program, &program.functions[&owner], &wrong_owner, ty).is_none());
    let mut undeclared = program.clone();
    let scalar = undeclared.type_context.intern_type(&Type::I64);
    undeclared.functions.get_mut(&closure_id).unwrap().local_decls.push(crate::mir::LocalDecl {
        ty: scalar, mutability: crate::mir::Mutability::Not, name: None, span: None,
        source: LocalSource::ClosureCapture,
    });
    assert!(check_closure(&undeclared, &undeclared.functions[&owner], &closure, ty).is_none(),
        "an omitted actual capture declaration is not an empty environment");

    let mut retained = program.clone();
    let reference = retained.type_context.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: true });
    let source = Local(retained.functions[&owner].local_decls.len());
    let stored = Local(retained.functions[&closure_id].local_decls.len());
    let declaration = crate::mir::LocalDecl {
        ty: reference, mutability: crate::mir::Mutability::Not, name: None, span: None,
        source: LocalSource::ClosureCapture,
    };
    retained.functions.get_mut(&owner).unwrap().local_decls.push(crate::mir::LocalDecl {
        source: LocalSource::UserBinding, ..declaration.clone()
    });
    retained.functions.get_mut(&closure_id).unwrap().local_decls.push(declaration);
    let capture = crate::mir::MirClosureCapture {
        name: "reference".into(), local: stored, kind: MirClosureCaptureKind::ByValue, span: None,
    };
    retained.functions.get_mut(&closure_id).unwrap().closure_captures.push(capture.clone());
    let mut supplied = closure.clone();
    supplied.captures.push(crate::mir::MirClosureCapture { local: source, ..capture });
    assert!(check_closure(&retained, &retained.functions[&owner], &supplied, ty).is_none(),
        "even a fully declared moved unique-reference slot cannot obtain the no-reference-destination seal");
}

#[test]
fn must_evidence_cannot_survive_an_opaque_predecessor() {
    let program = source_mir(UNIQUE);
    let (function, _, _) = call_site(&program);
    let (closure, ty) = actual_closure(function);
    let seal = check_closure(&program, function, closure, ty).unwrap();
    let mut known = State(Some(HashMap::from([(Local(1), seal)])));
    assert!(known.join(&State(Some(HashMap::new()))));
    assert!(known.0.unwrap().is_empty());
}
