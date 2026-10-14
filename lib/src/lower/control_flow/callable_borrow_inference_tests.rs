use std::collections::HashMap;
use std::path::PathBuf;

use super::Lowerer;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::language_items::{FnLanguageItems, FnOnceLanguageItems};
use crate::types::{CallableKind, CaptureKind, FunctionCapture, FunctionSafety, TraitBound, Type};
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
    @shared: &F -> T -> R where F: Observe T, R
    @once: F -> T -> R where F: Consume T, R
    @pair: &F -> T -> U -> R where F: Observe (T, U), R
relay_callback: &F -> T -> R where F: Observe T, R
relay_callback = callback, value -> callback.observe value
< struct Service
    < marker: I64
impl Callbacks for Service
    @shared = callback, value -> relay_callback callback, value
    @once = callback, value -> callback value
    @pair = callback, first, second -> callback.observe (first, second)
as_callbacks: &Service -> &Callbacks
< as_callbacks = service ->
    object: &Callbacks = service
    object
"#;

fn accept_application(body: &str) {
    let path = PathBuf::from("/virtual/callable_borrow_inference/main.rk");
    let source = format!("{PROVIDER}\nmain = ->\n    service = Service\n        marker: 0\n    object = as_callbacks &service\n{body}\n");
    let frontend = crate::compile_frontend(&Config {
        entry_file: path.clone(),
        current_crate_name: Some("callable_borrow_inference".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path, text: source }],
        ..Config::default()
    }).expect("unannotated callback Args/Ret must resolve before dereference lowering");
    let (_, main) = frontend.analysis.hir.program.test_function_by_name("main").unwrap();
    assert_eq!(main.ret_type, Type::I64);
}

#[test]
fn shared_callback_result_is_inferred_before_dereference() {
    accept_application("    value = 42\n    callback = reference -> reference\n    result = object.shared (&callback), &value\n    *result");
}

#[test]
fn packed_callback_result_is_inferred_before_field_dereference() {
    accept_application("    first = 40\n    second = 2\n    callback = left, right -> (left, right)\n    result = object.pair (&callback), &first, &second\n    ~I64Add (*result.0), (*result.1)");
}

#[test]
fn consuming_callback_unique_result_is_inferred_before_assignment() {
    accept_application("    mut value = 0\n    callback = reference -> reference\n    result = object.once callback, &mut value\n    *result = 42\n    value");
}

#[test]
fn consuming_callback_capture_result_is_inferred_before_dereference() {
    accept_application("    value = 42\n    reference = &value\n    callback = _ -> reference\n    result = object.once callback, ()\n    *result");
}

fn canonical_field_source(body: &str) -> (String, Config) {
    let path = PathBuf::from("/virtual/canonical_callable_field.rk");
    let source = format!("{PROVIDER}\n{body}\n");
    let config = Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path, text: source.clone() }],
        ..Config::default()
    };
    (source, config)
}

#[test]
fn canonical_field_unknown_type_uses_callable_protocol() {
    use crate::hir::{HirExprKind, HirSelectedMethodTarget, HirStmt};

    let (source, config) = canonical_field_source("struct Holder F\n    < callback: F\nrun = holder -> holder.callback!\npending = owner -> owner.missing!\nmain = !-> ()");
    let module = crate::parser::parse_source(config.entry_file.clone(), &source, &config).unwrap();
    let partial = Lowerer::with_options(false).lower_program(&crate::ast::Program { module })
        .expect("structural field and unresolved member must remain distinct");
    let run = partial.functions.values().find(|function| function.name == "run").unwrap();
    let Type::Struct { id: holder_id, args } = partial.engine.resolve(&run.params[0].ty) else {
        panic!("expected actual Holder schema");
    };
    let [Type::TypeVar(subject)] = args.as_slice() else {
        panic!("calling an unknown canonical field must not force a native function: {args:?}");
    };
    assert_eq!(partial.engine.kind_of_type_var(*subject), crate::type_services::kind::Kind::Type);
    let protocol = partial.language_items.fn_once.as_ref().unwrap();
    let bound = partial.engine.get_bounds(*subject).into_iter()
        .find(|bound| bound.trait_id == protocol.trait_id).expect("callability belongs to the field argument");
    assert_eq!(bound.type_args.len(), 2);
    assert_eq!(partial.engine.resolve(&bound.type_args[0]), Type::Unit);
    assert_eq!(partial.engine.resolve(&bound.type_args[1]), partial.engine.resolve(&run.ret_type));
    let HirStmt::Expr(call) = run.body.stmts.last().unwrap() else { panic!("expected call tail"); };
    let HirExprKind::MethodCall(receiver, _, _, mode, Some(target)) = &call.kind else {
        panic!("expected canonical callable protocol call: {call:?}");
    };
    assert_eq!(*mode, Some(crate::types::ReceiverMode::Move));
    assert!(matches!(target.target, HirSelectedMethodTarget::TraitMethod { trait_id, member_id, .. }
        if trait_id == protocol.trait_id && member_id == protocol.method_id));
    let HirExprKind::FieldAccess(owner, _, Some(location)) = &receiver.kind else {
        panic!("callable receiver must be the actual structural field");
    };
    assert_eq!(location.owner, holder_id);
    assert_eq!(location.field_id, partial.structs[&holder_id].fields[0].id);
    assert_eq!(partial.engine.resolve(&receiver.ty), args[0]);
    assert_eq!(partial.engine.resolve(&owner.ty), partial.engine.resolve(&run.params[0].ty));

    let pending = partial.functions.values().find(|function| function.name == "pending").unwrap();
    let HirStmt::Expr(call) = pending.body.stmts.last().unwrap() else { panic!("expected member call tail"); };
    let HirExprKind::Call(callee, _, None) = &call.kind else { panic!("unknown member must stay deferred"); };
    assert!(matches!(callee.kind, HirExprKind::FieldAccess(_, _, None)));
    let Type::TypeVar(subject) = partial.engine.resolve(&callee.ty) else {
        panic!("deferred member must not acquire a native signature");
    };
    assert!(partial.engine.get_bounds(subject).is_empty(), "a method slot is not a callable field subject");
    assert!(matches!(partial.engine.resolve(&pending.params[0].ty), Type::TypeVar(_)));
}

#[test]
fn canonical_field_task_selects_callable_protocol() {
    use crate::hir::{HirExprKindFor as HirExprKind, HirImplReceiverPattern, HirSelectedMethodTarget, HirStmtFor as HirStmt};

    let (_, config) = canonical_field_source(r#"
struct Task
impl Consume (), I64 for Task
    type Output = I64
    ~@consume = _ -> 19
struct Holder F
    < callback: F
run = holder -> holder.callback!
main = ->
    holder = Holder
        callback: Task
    answer = run holder
    next = Holder
        callback: Task
    ~I64Add answer, (next.callback!)
"#);
    let analysis = crate::analyze(&config).expect("annotation-free Holder Task must infer ordinary protocol dispatch");
    let program = &analysis.hir.program;
    let (_, main) = program.test_function_by_name("main").unwrap();
    assert_eq!(main.ret_type, Type::I64);
    let (_, run) = program.test_function_by_name("run").unwrap();
    let Type::Struct { args: run_args, .. } = &run.params[0].ty else { panic!("expected inferred Holder parameter"); };
    assert_eq!(run_args.len(), 1);
    assert!(!matches!(run_args[0], Type::Function { .. }), "the inferred scheme must not require a native function");
    let protocol = program.language_items.fn_once.as_ref().unwrap();
    let task = program.structs.values().find(|structure| structure.name == "Task").unwrap();
    let task_ty = Type::Struct { id: task.id, args: Vec::new() };
    let implementation = program.impls.values().find(|implementation| {
        implementation.trait_id == Some(protocol.trait_id)
            && implementation.receiver_pattern == HirImplReceiverPattern::Exact(task_ty.clone())
    }).unwrap();
    let HirStmt::Expr(tail) = main.body.stmts.last().unwrap() else { panic!("expected intrinsic tail"); };
    let HirExprKind::Intrinsic { args, .. } = &tail.kind else { panic!("expected sum"); };
    let HirExprKind::MethodCall(receiver, _, _, mode, target) = &args[1].kind else {
        panic!("Task field must use the selected callable implementation: {:?}", args[1]);
    };
    assert_eq!(receiver.ty, task_ty);
    assert_eq!(*mode, Some(crate::types::ReceiverMode::Move));
    let HirSelectedMethodTarget::ImplMethod { impl_id, method_id, selected_trait: Some(selected) } = &target.target else {
        panic!("expected actual Task impl identity");
    };
    assert_eq!(*impl_id, implementation.id);
    assert_eq!(*method_id, implementation.methods["consume"].id);
    assert_eq!(selected.trait_id, protocol.trait_id);
    assert_eq!(selected.member_id, protocol.method_id);
    assert_eq!(selected.trait_args, vec![Type::Unit, Type::I64]);
    let HirExprKind::FieldAccess(_, _, Some(location)) = &receiver.kind else {
        panic!("Task receiver must retain canonical field authority");
    };
    let holder = &program.structs[&location.owner];
    assert_eq!(holder.name, "Holder");
    assert_eq!(location.field_id, holder.fields[0].id);
}

#[test]
fn canonical_field_native_function_keeps_signature_validation() {
    for (definition, call) in [("-> (19 as I64)", "holder.callback!"), ("value -> (~I64Add value, 1)", "holder.callback (18 as I64)")] {
        let (_, config) = canonical_field_source(&format!("struct Holder F\n    < callback: F\nmain = ->\n    holder = Holder\n        callback: {definition}\n    {call}"));
        let analysis = crate::analyze(&config).expect("known native field signature remains callable");
        assert_eq!(analysis.hir.program.test_function_by_name("main").unwrap().1.ret_type, Type::I64);
    }
    for (call, code, message, excerpt) in [
        ("holder.callback!", crate::diagnostic::DiagnosticCode::Selection,
            "Type mismatch: function expected 1 args, got 0", "holder.callback!"),
        ("holder.callback true", crate::diagnostic::DiagnosticCode::Type,
            "Type mismatch: Bool vs I64", "true"),
    ] {
        let (source, config) = canonical_field_source(&format!("struct Holder F\n    < callback: F\nmain = ->\n    holder = Holder\n        callback: value -> (~I64Add value, 1)\n    {call}"));
        let errors = crate::analyze(&config).err().expect("native field arity and argument types remain checked");
        assert!(errors.0.iter().all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
        let error = errors.0.iter().find(|error| error.code == Some(code) && error.message == message)
            .unwrap_or_else(|| panic!("{errors:?}"));
        let span = &error.primary.as_ref().unwrap().span;
        assert_eq!(span.file_path, config.entry_file);
        assert_eq!(&source[span.start..span.end], excerpt);
    }
}

#[test]
fn canonical_field_non_callable_rejects_actual_call() {
    let (source, config) = canonical_field_source("struct Holder F\n    < callback: F\nmain = ->\n    holder = Holder\n        callback: (19 as I64)\n    holder.callback!");
    let errors = crate::analyze(&config).err().expect("a canonical numeric field is not callable");
    assert!(errors.0.iter().all(|error| error.code != Some(crate::diagnostic::DiagnosticCode::Internal)), "{errors:?}");
    let error = errors.0.iter().find(|error| {
        error.code == Some(crate::diagnostic::DiagnosticCode::Type)
            && error.message == "value of type I64 is not callable with these arguments"
    }).unwrap_or_else(|| panic!("{errors:?}"));
    let span = &error.primary.as_ref().unwrap().span;
    assert_eq!(span.file_path, config.entry_file);
    assert_eq!(&source[span.start..span.end], "holder.callback!");
}

fn deferred_member_array_tail(function: &crate::hir::AcceptedHirFunction) -> &crate::hir::AcceptedHirExpr {
    assert!(function.body.stmts.len() <= 4, "bounded source body, not a whole-HIR observer");
    let Some(crate::hir::HirStmtFor::Expr(tail)) = function.body.stmts.last() else {
        panic!("expected source call tail");
    };
    tail
}

fn deferred_member_array_receiver(mut expression: &crate::hir::AcceptedHirExpr) -> &crate::hir::AcceptedHirExpr {
    for _ in 0..=2 {
        match &expression.kind {
            crate::hir::HirExprKindFor::Ref(_, inner) | crate::hir::HirExprKindFor::Deref(inner) => expression = inner,
            _ => return expression,
        }
    }
    panic!("receiver wrapper cap2 exceeded");
}

fn assert_deferred_member_array_conversion(
    function: &crate::hir::AcceptedHirFunction,
    argument: &crate::hir::AcceptedHirExpr,
    expected: &Type,
) {
    use crate::hir::{HirExprKindFor as Expr, HirStmtFor as Stmt, HirVarTarget};

    let slice = Type::Reference { mutable: true, inner: Box::new(Type::Slice(Box::new(Type::U8))) };
    let array = Type::Array(Box::new(Type::U8), 4);
    assert_eq!(expected, &slice);
    assert_eq!(&argument.ty, expected);
    let Expr::Intrinsic { name, args } = &argument.kind else { panic!("actual checked slice conversion required"); };
    assert_eq!(name, "ArrayRefToSlice");
    assert_eq!(args.len(), 1);
    assert_eq!(args[0].ty, Type::Reference { mutable: true, inner: Box::new(array.clone()) });
    let Expr::Ref(true, local) = &args[0].kind else { panic!("conversion must retain the actual mutable array borrow"); };
    assert_eq!(local.ty, array);
    let Expr::ResolvedVar(reference) = &local.kind else { panic!("actual source-local array required"); };
    let mut bindings = function.body.stmts.iter().filter_map(|statement| match statement {
        Stmt::Let { name, local_id, ty, mutable, .. } if name == "bytes" => Some((local_id, ty, mutable)),
        _ => None,
    });
    let (local_id, ty, mutable) = bindings.next().expect("actual source bytes binding");
    assert!(bindings.next().is_none());
    assert!(*mutable);
    assert_eq!(ty, &array);
    assert_eq!(reference.target, HirVarTarget::Local(*local_id));
    assert_eq!(argument.span, args[0].span);
    eprintln!("DEFERRED_MEMBER_ARRAY owner={:?} sourceLocal={local_id:?} span={:?} checkedArgument={expected:?} sourceArray={array:?} conversion=ArrayRefToSlice execution=NONE", function.id, argument.span);
}

#[test]
fn deferred_member_array_argument_exact_sink_wrapper_and_unknown_receiver() {
    use crate::hir::{HirExprKindFor as Expr, HirImplReceiverPattern, HirSelectedMethodTarget, HirVarTarget};

    // Read the original literal without altering any source bytes or signatures.
    let fixture = include_str!("../../../tests/integration.rs");
    let fixture = fixture.split_once("fn test_signatureless_deferred_method_constrains_argument_from_concrete_parameter() {").unwrap().1;
    let source = fixture.split_once("r#\"").unwrap().1.split_once("\"#,").unwrap().0;
    assert!(source.contains("forward_unknown = sink ->\n    mut bytes = [0; 4]\n    sink.write_bytes &mut bytes"));
    let (_, mut config) = canonical_field_source(source);
    config.no_std = false;
    config.no_prelude = false;
    config.source_providers = vec![SourceProvider::Virtual { path: config.entry_file.clone(), text: source.to_string() }];
    let artifact = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/artifacts/stdlib60/contracts-2/stdlib.rkca");
    let artifact_bytes = std::fs::read(&artifact).expect("existing explicit stdlib artifact, no source build");
    assert_eq!(artifact_bytes.len(), 967266);
    config.extern_artifacts = vec![("stdlib".into(), artifact.clone())];
    let analysis = crate::analyze(&config).expect("exact original signatureless Sink source must discharge argument constraints");
    let program = &analysis.hir.program;
    let sink = program.structs.values().find(|structure| structure.name == "Sink").unwrap();
    let wrapper = program.structs.values().find(|structure| structure.name == "Wrapper").unwrap();
    let sink_ty = Type::Struct { id: sink.id, args: Vec::new() };
    let sink_impl = program.impls.values().find(|imp| imp.trait_id.is_none()
        && imp.receiver_pattern == HirImplReceiverPattern::Exact(sink_ty.clone())).unwrap();
    let wrapper_impl = program.impls.values().find(|imp| imp.trait_id.is_none()
        && imp.receiver_pattern == HirImplReceiverPattern::Exact(Type::Struct { id: wrapper.id, args: Vec::new() })).unwrap();
    let method = &sink_impl.methods["write_bytes"];
    assert!(method.generic_params.is_empty() && method.generic_bounds.is_empty());
    let expected = &method.params.last().unwrap().ty;
    let unknown = program.test_function_by_name("forward_unknown").unwrap().1;
    assert_eq!(unknown.params.len(), 1);
    assert_eq!(unknown.params[0].ty, sink_ty);
    for (function, known) in [(&wrapper_impl.methods["forward_bytes"], true), (unknown, false)] {
        let call = deferred_member_array_tail(function);
        let Expr::MethodCall(receiver, _, args, _, target) = &call.kind else { panic!("actual canonical Sink method required"); };
        assert_eq!(args.len(), 1);
        assert_eq!(target.target, HirSelectedMethodTarget::ImplMethod {
            impl_id: sink_impl.id, method_id: method.id, selected_trait: None,
        });
        assert!(target.owner_substitution.is_empty() && target.method_substitution.is_empty());
        assert_eq!(call.ty, method.ret_type);
        assert_eq!(function.ret_type, Type::I64);
        let receiver = deferred_member_array_receiver(receiver);
        assert_eq!(receiver.ty, sink_ty);
        if known {
            let Expr::FieldAccess(_, _, Some(location)) = &receiver.kind else { panic!("known path must retain Wrapper.sink field authority"); };
            assert_eq!(location.owner, wrapper.id);
            assert_eq!(location.field_id, wrapper.fields.iter().find(|field| field.name == "sink").unwrap().id);
        } else {
            let Expr::ResolvedVar(reference) = &receiver.kind else { panic!("unknown path must retain its actual parameter"); };
            assert_eq!(reference.target, HirVarTarget::Local(unknown.params[0].local_id));
        }
        assert_deferred_member_array_conversion(function, &args[0], expected);
        eprintln!("DEFERRED_MEMBER_ARRAY knownReceiver={known} canonicalTarget={:?} declaredParameter={expected:?}", target.target);
    }
    assert_eq!(std::fs::read(&artifact).unwrap(), artifact_bytes);
}

#[test]
fn deferred_member_array_argument_caller_selected_native_field_keeps_conversion() {
    use crate::hir::{HirExprKind, HirExprKindFor as Expr, HirStmt, HirVarTarget};
    use crate::infer::constraints::{Constraint, ConstraintOwner};

    let (source, config) = canonical_field_source(r#"
struct Alpha
    < callback: &mut [U8] -> I64
struct Beta
    < callback: &mut [U8] -> I64
run = holder ->
    mut bytes = [0 as U8; 4]
    holder.callback &mut bytes
main = ->
    holder = Alpha
        callback: _ -> 42
    run holder
"#);
    let module = crate::parser::parse_source(config.entry_file.clone(), &source, &config).unwrap();
    let partial = Lowerer::with_options(false).lower_program(&crate::ast::Program { module }).unwrap();
    let run = partial.functions.values().find(|function| function.name == "run").unwrap();
    let HirStmt::Expr(call) = run.body.stmts.last().unwrap() else { panic!("unresolved source call tail"); };
    let HirExprKind::Call(callee, args, None) = &call.kind else { panic!("must await member authority"); };
    let HirExprKind::FieldAccess(receiver, _, None) = &callee.kind else { panic!("no guessed FieldId before caller inference"); };
    assert!(matches!(partial.engine.resolve(&receiver.ty), Type::TypeVar(_)));
    let Type::TypeVar(member_slot) = partial.engine.resolve(&callee.ty) else { panic!("no eager native function signature"); };
    assert!(partial.engine.get_bounds(member_slot).is_empty());
    assert_eq!(args.len(), 1);
    assert_eq!(partial.engine.resolve(&args[0].ty), Type::Reference {
        mutable: true, inner: Box::new(Type::Array(Box::new(Type::U8), 4)),
    });
    assert!(!partial.constraint_store.iter().any(|(id, constraint)| {
        partial.constraint_store.owner(id) == Some(ConstraintOwner::Body(run.id))
            && matches!(constraint, Constraint::Coercion { context, .. } if context == "deferred callable argument coercion")
    }), "no orphan expected-argument slot while member authority is absent");

    let analysis = crate::analyze(&config).expect("actual Alpha caller must select its field and discharge array coercion");
    let program = &analysis.hir.program;
    let alpha = program.structs.values().find(|structure| structure.name == "Alpha").unwrap();
    let beta = program.structs.values().find(|structure| structure.name == "Beta").unwrap();
    assert_ne!(alpha.id, beta.id);
    assert_eq!(alpha.fields[0].name, beta.fields[0].name);
    let run = program.test_function_by_name("run").unwrap().1;
    assert_eq!(run.params[0].ty, Type::Struct { id: alpha.id, args: Vec::new() });
    let call = deferred_member_array_tail(run);
    let Expr::Call(callee, args, _) = &call.kind else { panic!("native field must retain native invocation"); };
    let Expr::FieldAccess(owner, _, Some(location)) = &callee.kind else { panic!("actual canonical Alpha field required"); };
    assert_eq!(location.owner, alpha.id);
    assert_eq!(location.field_id, alpha.fields[0].id);
    assert_eq!(callee.ty, alpha.fields[0].ty);
    assert_eq!(owner.ty, run.params[0].ty);
    let Expr::ResolvedVar(reference) = &owner.kind else { panic!("actual source receiver parameter required"); };
    assert_eq!(reference.target, HirVarTarget::Local(run.params[0].local_id));
    let Type::Function { params, ret, .. } = &callee.ty else { panic!("canonical native field declaration"); };
    assert_eq!(params.len(), 1);
    assert_eq!(args.len(), 1);
    assert_eq!(ret.as_ref(), &Type::I64);
    assert_eq!(call.ty, Type::I64);
    assert_deferred_member_array_conversion(run, &args[0], &params[0]);
    eprintln!("DEFERRED_MEMBER_ARRAY actualOwner={:?} actualField={:?} otherDeclaredOwner={:?} execution=NONE", location.owner, location.field_id, beta.id);
}

#[test]
fn deferred_member_array_argument_known_method_rejects_immutable_array_borrow() {
    let (source, config) = canonical_field_source(r#"
struct Sink
impl Sink
    @write_bytes: &mut [U8] -> I64
    @write_bytes = _ -> 42
main = ->
    sink = Sink
    bytes = [0 as U8; 4]
    sink.write_bytes &bytes
"#);
    let diagnostics = crate::analyze(&config).expect_err("immutable source borrow cannot become a mutable slice argument");
    assert_eq!(diagnostics.0.len(), 1, "{diagnostics:?}");
    let error = &diagnostics.0[0];
    assert_eq!(error.code, Some(crate::diagnostic::DiagnosticCode::Type));
    assert_eq!(error.message, "Mutability mismatch on references");
    let start = source.rfind("&bytes").unwrap();
    assert_eq!(error.primary.as_ref().unwrap().span, crate::lexer::Span::new(config.entry_file, start, start + 1));
}

fn assert_sourcefree_factory_authority(payload_first: bool) {
    use crate::hir::{
        AcceptedHirExpr, HirCallTarget, HirExprKindFor as HirExprKind, HirImplReceiverPattern,
        HirSelectedMethodTarget, HirStmtFor as HirStmt, HirTraitDispatchKind, HirVarTarget,
    };
    use crate::types::ReceiverMode;

    // Producer only: no allocator implementations, concrete payloads, caller
    // types or signatures on either factory. This is never executed.
    let declarations = r#"
lang sized
< trait LayoutKnown
< struct Audit
    < value: I64
< trait Memory
    @audit: *Audit
< struct State A
    < allocator: A
    < audit: *Audit
    < alignment: I64
< struct Shell $PARAMS
    < pointer: *T
    < state: State A
impl Shell $ARGS where A: Memory
    new = allocator, value -> Shell::new_aligned allocator, value, (~AlignOf value)
    new_aligned = allocator, value, alignment ->
        audit = allocator.audit!
        pointer = 0 as *T
        unsafe *pointer = value
        state = State
            allocator: allocator
            audit: audit
            alignment: alignment
        Shell
            pointer: pointer
            state: state
main = !-> ()
"#;
    let (params, args, payload_slot, allocator_slot) = if payload_first {
        ("(T: ?LayoutKnown), A", "T, A", 0, 1)
    } else {
        ("A, (T: ?LayoutKnown)", "A, T", 1, 0)
    };
    let (_, config) = canonical_field_source(
        &declarations.replace("$PARAMS", params).replace("$ARGS", args),
    );
    let analysis = crate::analyze(&config)
        .unwrap_or_else(|errors| panic!("producer must establish bound method authority without callers: {errors:?}"));
    let program = &analysis.hir.program;
    let shell = program.structs.values().find(|definition| definition.name == "Shell").unwrap();
    let state = program.structs.values().find(|definition| definition.name == "State").unwrap();
    let audit = program.structs.values().find(|definition| definition.name == "Audit").unwrap();
    let memory = program.traits.values().find(|definition| definition.name == "Memory").unwrap();
    let member = &memory.signatures["audit"];
    assert_eq!(member.self_receiver, Some(ReceiverMode::Shared));
    let implementation = program.impls.values().find(|implementation| {
        implementation.trait_id.is_none()
            && matches!(&implementation.receiver_pattern, HirImplReceiverPattern::Exact(Type::Struct { id, .. }) if *id == shell.id)
    }).unwrap();
    let allocator = implementation.type_generics[allocator_slot].id;
    let payload = implementation.type_generics[payload_slot].id;
    assert_eq!(allocator.owner, implementation.id);
    assert_eq!(payload.owner, implementation.id);
    let memory_bound = TraitBound { trait_id: memory.id, type_args: Vec::new() };
    let sized_bound = TraitBound { trait_id: program.language_items.sized.as_ref().unwrap().trait_id, type_args: Vec::new() };
    assert!(implementation.bounds[&allocator].contains(&memory_bound));
    assert!(implementation.bounds[&allocator].contains(&sized_bound));
    assert!(implementation.bounds[&payload].contains(&sized_bound));
    let aligned = &implementation.methods["new_aligned"];
    let wrapper = &implementation.methods["new"];
    assert_ne!(aligned.id, wrapper.id);

    fn store(expression: &AcceptedHirExpr, depth: usize) -> &AcceptedHirExpr {
        assert!(depth <= 2, "FACTORY_PAYLOAD_OVERFLOW store block depth cap=2");
        match &expression.kind {
            HirExprKind::Assign(_, _) => expression,
            HirExprKind::Block(block) => {
                assert!(block.stmts.len() <= 4, "FACTORY_PAYLOAD_OVERFLOW store statements cap=4");
                let mut operations = block.stmts.iter().filter_map(|statement| match statement {
                    HirStmt::Expr(expression) if matches!(expression.kind, HirExprKind::Assign(_, _) | HirExprKind::Block(_)) => Some(expression),
                    _ => None,
                });
                let operation = operations.next().expect("actual accepted store block operation");
                assert!(operations.next().is_none(), "expected exactly one store operation");
                store(operation, depth + 1)
            }
            _ => panic!("expected actual accepted Block/Assign, kind={:?}", std::mem::discriminant(&expression.kind)),
        }
    }

    fn closed_type(ty: &Type, declared: &std::collections::HashSet<crate::types::GenericParamId>) {
        let mut used = std::collections::HashSet::new();
        ty.collect_generic_params(&mut used);
        assert!(used.is_subset(declared), "free factory scheme generic: {ty:?}");
        assert!(!crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::TypeVar(_) | Type::Error)),
            "unresolved factory scheme type: {ty:?}");
    }

    fn closed_body(
        body: &crate::hir::AcceptedHirBlock,
        declared: &std::collections::HashSet<crate::types::GenericParamId>,
        depth: usize,
    ) {
        assert!(depth <= 8 && body.stmts.len() <= 5, "bounded factory body");
        closed_type(&body.ty, declared);
        for statement in &body.stmts {
            match statement {
                HirStmt::Let { ty, value, .. } => {
                    closed_type(ty, declared);
                    closed_expr(value, declared, depth + 1);
                }
                HirStmt::Expr(expression) => closed_expr(expression, declared, depth + 1),
                _ => panic!("unexpected statement in fixed factory fixture"),
            }
        }
    }

    fn closed_expr(
        expression: &AcceptedHirExpr,
        declared: &std::collections::HashSet<crate::types::GenericParamId>,
        depth: usize,
    ) {
        assert!(depth <= 8, "bounded factory expression depth");
        closed_type(&expression.ty, declared);
        match &expression.kind {
            HirExprKind::ResolvedVar(_) | HirExprKind::IntLiteral(_) | HirExprKind::Unit => {}
            HirExprKind::Cast(inner, ty) => {
                closed_type(ty, declared);
                closed_expr(inner, declared, depth + 1);
            }
            HirExprKind::Deref(inner) => closed_expr(inner, declared, depth + 1),
            HirExprKind::Assign(left, right) => {
                closed_expr(left, declared, depth + 1);
                closed_expr(right, declared, depth + 1);
            }
            HirExprKind::StructLiteral(_, _, fields) => {
                assert!(fields.len() <= 3);
                for field in fields { closed_expr(&field.value, declared, depth + 1); }
            }
            HirExprKind::Block(body) => closed_body(body, declared, depth + 1),
            HirExprKind::Lambda { params, body, captures } => {
                assert!(params.len() <= 3 && captures.len() <= 3);
                for parameter in params { closed_type(&parameter.ty, declared); }
                for capture in captures { closed_type(&capture.ty, declared); }
                closed_body(body, declared, depth + 1);
            }
            HirExprKind::Call(callee, args, target) => {
                closed_expr(callee, declared, depth + 1);
                assert!(args.len() <= 3);
                for argument in args { closed_expr(argument, declared, depth + 1); }
                if let Some(HirCallTarget::StaticMethod(target)) = target {
                    closed_type(&target.owner_ty, declared);
                    for ty in target.method.trait_args() { closed_type(ty, declared); }
                    for binding in target.method.owner_substitution.iter().chain(&target.method.method_substitution) {
                        closed_type(&binding.ty, declared);
                    }
                }
            }
            HirExprKind::MethodCall(receiver, _, args, _, target) => {
                closed_expr(receiver, declared, depth + 1);
                assert!(args.len() <= 3);
                for argument in args { closed_expr(argument, declared, depth + 1); }
                for ty in target.trait_args() { closed_type(ty, declared); }
                for binding in target.owner_substitution.iter().chain(&target.method_substitution) {
                    closed_type(&binding.ty, declared);
                }
            }
            HirExprKind::Intrinsic { args, .. } => {
                assert!(args.len() <= 3);
                for argument in args { closed_expr(argument, declared, depth + 1); }
            }
            _ => panic!("unexpected expression in fixed factory fixture: {:?}", std::mem::discriminant(&expression.kind)),
        }
    }

    if std::env::var("ROCK_FACTORY_PAYLOAD_AUTHORITY").as_deref() == Ok("1") {
        use crate::hir::{AcceptedHirExpr, HirGenericBounds};

        struct Receipt {
            types: std::collections::HashSet<Type>,
        }
        impl Receipt {
            fn ty(&mut self, role: &str, ty: &Type) {
                self.types.insert(ty.clone());
                eprintln!("FACTORY_PAYLOAD_TYPE role={role} type={ty:?} distinct={} cap=12", self.types.len());
                assert!(self.types.len() <= 12,
                    "FACTORY_PAYLOAD_OVERFLOW role={role} distinctTopLevelTypes={} cap=12", self.types.len());
            }

            fn bounds(&mut self, role: &str, bounds: &HirGenericBounds) {
                assert!(bounds.len() <= 4 && bounds.predicates.len() <= 4 && bounds.relaxed_sized.len() <= 2,
                    "FACTORY_PAYLOAD_OVERFLOW context={role}");
                for rows in bounds.values() {
                    assert!(rows.len() <= 4, "FACTORY_PAYLOAD_OVERFLOW bounds={role}");
                    for bound in rows {
                        assert!(bound.type_args.len() <= 4, "FACTORY_PAYLOAD_OVERFLOW bound arguments={role}");
                        for ty in &bound.type_args { self.ty(role, ty); }
                    }
                }
                for predicate in &bounds.predicates {
                    let crate::types::Predicate::Trait { subject, args, .. } = predicate;
                    assert!(args.len() <= 4, "FACTORY_PAYLOAD_OVERFLOW predicate arguments={role}");
                    self.ty(role, subject);
                    for ty in args { self.ty(role, ty); }
                }
                eprintln!("FACTORY_PAYLOAD_BOUNDS role={role} actual={bounds:?}");
            }
        }

        fn local(expression: &AcceptedHirExpr) -> Option<&HirVarTarget> {
            let mut leaf = expression;
            for _ in 0..=2 {
                match &leaf.kind {
                    HirExprKind::ResolvedVar(reference) => return Some(&reference.target),
                    HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) => leaf = inner,
                    _ => return None,
                }
            }
            panic!("FACTORY_PAYLOAD_OVERFLOW local wrapper depth cap=2");
        }

        fn relay(
            receipt: &mut Receipt,
            expression: &AcceptedHirExpr,
            layers: &mut usize,
            depth: usize,
            supplied: Option<&[AcceptedHirExpr]>,
        ) {
            assert!(depth <= 6, "FACTORY_PAYLOAD_OVERFLOW relay node depth cap=6");
            eprintln!("FACTORY_PAYLOAD_RELAY_NODE depth={depth} layers={layers} span={:?} kind={:?}",
                expression.span, std::mem::discriminant(&expression.kind));
            match &expression.kind {
                HirExprKind::Call(callee, arguments, target) => {
                    assert!(arguments.len() <= 3, "FACTORY_PAYLOAD_OVERFLOW relay arguments cap=3");
                    if let Some(HirCallTarget::StaticMethod(target)) = target {
                        assert!(target.method.owner_substitution.len() <= 2 && target.method.method_substitution.len() <= 2,
                            "FACTORY_PAYLOAD_OVERFLOW relay substitutions cap=2");
                    }
                    eprintln!("FACTORY_PAYLOAD_RELAY_CALL depth={depth} actualTarget={target:?} calleeSpan={:?} calleeKind={:?} canonicalCallee={:?} argumentLocals={:?}",
                        callee.span, std::mem::discriminant(&callee.kind), local(callee), arguments.iter().map(local).collect::<Vec<_>>());
                    for argument in arguments { receipt.ty("relay-argument", &argument.ty); }
                    if !matches!(expression.ty, Type::Function { .. }) {
                        receipt.ty("relay-result", &expression.ty);
                    }
                    if let Some(HirCallTarget::StaticMethod(target)) = target {
                        receipt.ty("relay-owner", &target.owner_ty);
                        for binding in &target.method.owner_substitution {
                            eprintln!("FACTORY_PAYLOAD_RELAY_BINDING namespace=owner parameter={:?}", binding.param);
                            receipt.ty("relay-owner-substitution", &binding.ty);
                        }
                        for binding in &target.method.method_substitution {
                            eprintln!("FACTORY_PAYLOAD_RELAY_BINDING namespace=method parameter={:?}", binding.param);
                            receipt.ty("relay-method-substitution", &binding.ty);
                        }
                    }
                    if matches!(callee.kind, HirExprKind::Lambda { .. } | HirExprKind::Call(..) | HirExprKind::Block(_)) {
                        relay(receipt, callee, layers, depth + 1, Some(arguments));
                    } else {
                        eprintln!("FACTORY_PAYLOAD_RELAY_TERMINAL actualTarget={target:?} actualCallee={:?} span={:?}", local(callee), callee.span);
                        if let Type::Function { params, ret, safety, callable_kind, .. } = &callee.ty {
                            assert!(params.len() <= 3, "FACTORY_PAYLOAD_OVERFLOW terminal signature params cap=3");
                            eprintln!("FACTORY_PAYLOAD_RELAY_SIGNATURE safety={safety:?} callableKind={callable_kind:?}");
                            for parameter in params { receipt.ty("relay-terminal-parameter", parameter); }
                            if !matches!(ret.as_ref(), Type::Function { .. }) {
                                receipt.ty("relay-terminal-return", ret);
                            }
                        }
                    }
                }
                HirExprKind::Lambda { params, body, captures } => {
                    *layers += 1;
                    assert!(*layers <= 3 && params.len() <= 3 && captures.len() <= 3 && body.stmts.len() <= 4,
                        "FACTORY_PAYLOAD_OVERFLOW relay layers cap=3 params/captures cap=3 statements cap=4");
                    eprintln!("FACTORY_PAYLOAD_RELAY_LAMBDA layer={layers} span={:?} parameterLocals={:?} captureLocals={:?}",
                        expression.span, params.iter().map(|parameter| parameter.local_id).collect::<Vec<_>>(),
                        captures.iter().map(|capture| (capture.local_id, capture.kind, capture.mutable)).collect::<Vec<_>>());
                    if let Type::Function { safety, callable_kind, .. } = &expression.ty {
                        eprintln!("FACTORY_PAYLOAD_RELAY_NATIVE safety={safety:?} callableKind={callable_kind:?} wrapperTypeNotCounted=true");
                    }
                    for parameter in params { receipt.ty("relay-lambda-parameter", &parameter.ty); }
                    for capture in captures { receipt.ty("relay-lambda-capture", &capture.ty); }
                    if let Some(arguments) = supplied {
                        assert!(arguments.len() <= 3, "FACTORY_PAYLOAD_OVERFLOW relay supplied arguments cap=3");
                        eprintln!("FACTORY_PAYLOAD_RELAY_APPLICATION parameters={} supplied={}", params.len(), arguments.len());
                        for (slot, (parameter, argument)) in params.iter().zip(arguments).enumerate() {
                            eprintln!("FACTORY_PAYLOAD_RELAY_LOCAL_MAP layer={layers} slot={slot} parameterLocal={:?} suppliedLocal={:?} suppliedSpan={:?}",
                                parameter.local_id, local(argument), argument.span);
                            receipt.ty("relay-mapped-parameter", &parameter.ty);
                            receipt.ty("relay-mapped-argument", &argument.ty);
                        }
                    }
                    if !matches!(body.ty, Type::Function { .. }) { receipt.ty("relay-lambda-body", &body.ty); }
                    let Some(HirStmt::Expr(tail)) = body.stmts.last() else { panic!("actual relay Lambda expression tail"); };
                    relay(receipt, tail, layers, depth + 1, None);
                }
                HirExprKind::Block(body) => {
                    assert!(body.stmts.len() <= 4, "FACTORY_PAYLOAD_OVERFLOW relay block statements cap=4");
                    let Some(HirStmt::Expr(tail)) = body.stmts.last() else { panic!("actual relay Block expression tail"); };
                    relay(receipt, tail, layers, depth + 1, supplied);
                }
                _ => panic!("expected actual relay Call/Lambda/Block, kind={:?}", std::mem::discriminant(&expression.kind)),
            }
        }

        let mut receipt = Receipt { types: Default::default() };
        assert!(implementation.type_generics.len() <= 2 && shell.generic_params.len() <= 2
            && state.generic_params.len() <= 1 && shell.fields.len() <= 2 && state.fields.len() <= 3,
            "FACTORY_PAYLOAD_OVERFLOW declaration context");
        eprintln!("FACTORY_PAYLOAD_BEGIN payloadFirst={payload_first} methods=2 cap=2 topLevelTypesCap=12 implId={:?} implGenerics={:?} shellId={:?} shellGenerics={:?} stateId={:?} stateGenerics={:?} sourceAllocator={allocator:?} sourcePayload={payload:?}",
            implementation.id, implementation.type_generics, shell.id, shell.generic_params, state.id, state.generic_params);
        receipt.bounds("impl", &implementation.bounds);
        for function in [aligned, wrapper] {
            assert!(function.params.len() <= 3 && function.generic_params.len() <= 2 && function.body.stmts.len() <= 5,
                "FACTORY_PAYLOAD_OVERFLOW method header/body");
            eprintln!("FACTORY_PAYLOAD_HEADER id={:?} generics={:?} receiver={:?} unsafe={}",
                function.id, function.generic_params, function.self_receiver, function.is_unsafe);
            for (slot, parameter) in function.params.iter().enumerate() {
                eprintln!("FACTORY_PAYLOAD_PARAMETER method={:?} slot={slot} localId={:?}", function.id, parameter.local_id);
                receipt.ty("parameter", &parameter.ty);
            }
            receipt.ty("return", &function.ret_type);
            receipt.ty("body", &function.body.ty);
            receipt.bounds("method", &function.generic_bounds);
        }
        let HirStmt::Let { value: audit_call, .. } = &aligned.body.stmts[0] else { panic!("actual audit binding"); };
        let HirExprKind::MethodCall(receiver, _, arguments, mode, target) = &audit_call.kind else { panic!("actual accepted audit method"); };
        assert!(arguments.len() <= 1 && target.owner_substitution.len() <= 2 && target.method_substitution.len() <= 2);
        eprintln!("FACTORY_PAYLOAD_MEMORY declaredTraitId={:?} declaredMemberId={:?} declaredReceiver={:?} actualTarget={target:?} actualMode={mode:?} receiverLocal={:?} arguments={}",
            memory.id, member.id, member.self_receiver, local(receiver), arguments.len());
        receipt.ty("memory-receiver", &receiver.ty);
        for binding in target.owner_substitution.iter().chain(&target.method_substitution) {
            receipt.ty("memory-substitution", &binding.ty);
        }

        let HirStmt::Let { local_id, ty, value: pointer, .. } = &aligned.body.stmts[1] else { panic!("actual pointer binding"); };
        eprintln!("FACTORY_PAYLOAD_POINTER localId={local_id:?} span={:?}", pointer.span);
        receipt.ty("pointer-binding", ty);
        receipt.ty("pointer-expression", &pointer.ty);
        let HirExprKind::Cast(input, cast_target) = &pointer.kind else { panic!("actual pointer cast"); };
        receipt.ty("cast-target", cast_target);
        receipt.ty("cast-source", &input.ty);
        let HirStmt::Expr(expression) = &aligned.body.stmts[2] else { panic!("actual store statement"); };
        let assignment = store(expression, 0);
        let HirExprKind::Assign(place, value) = &assignment.kind else { unreachable!() };
        let HirExprKind::Deref(pointer) = &place.kind else { panic!("actual dereference destination"); };
        eprintln!("FACTORY_PAYLOAD_STORE span={:?} pointerLocal={:?} valueLocal={:?}",
            assignment.span, local(pointer), local(value));
        receipt.ty("store-pointer", &pointer.ty);
        receipt.ty("store-pointee", &place.ty);
        receipt.ty("store-value", &value.ty);

        let HirStmt::Let { local_id, ty, value: state_literal, .. } = &aligned.body.stmts[3] else { panic!("actual State binding"); };
        let HirExprKind::StructLiteral(_, owner, state_fields) = &state_literal.kind else { panic!("actual State literal"); };
        assert!(state_fields.len() <= 3);
        eprintln!("FACTORY_PAYLOAD_STATE localId={local_id:?} literalOwner={owner:?} fields={:?}",
            state_fields.iter().map(|field| (&field.field, local(&field.value))).collect::<Vec<_>>());
        receipt.ty("state-binding", ty);
        receipt.ty("state-literal", &state_literal.ty);
        let allocator_field_id = state.fields.iter().find(|field| field.name == "allocator")
            .expect("fixture's declared State allocator field").id;
        let allocator_field = state_fields.iter().find(|field| field.field.as_ref()
            .is_some_and(|location| location.owner == state.id && location.field_id == allocator_field_id))
            .expect("actual State initializer selected by canonical allocator FieldId");
        eprintln!("FACTORY_PAYLOAD_STATE_ALLOCATOR owner={:?} declaredFieldId={allocator_field_id:?} actualLocation={:?} actualLocal={:?}",
            state.id, allocator_field.field, local(&allocator_field.value));
        receipt.ty("state-allocator-value", &allocator_field.value.ty);
        let HirStmt::Expr(shell_literal) = aligned.body.stmts.last().unwrap() else { panic!("actual Shell tail"); };
        let HirExprKind::StructLiteral(_, owner, fields) = &shell_literal.kind else { panic!("actual Shell literal"); };
        assert!(fields.len() <= 2);
        eprintln!("FACTORY_PAYLOAD_SHELL literalOwner={owner:?} fields={:?}",
            fields.iter().map(|field| (&field.field, local(&field.value))).collect::<Vec<_>>());
        receipt.ty("shell-literal", &shell_literal.ty);
        for field in fields { receipt.ty("shell-field-value", &field.value.ty); }

        let HirStmt::Expr(call) = wrapper.body.stmts.last().unwrap() else { panic!("actual relay tail"); };
        let mut layers = 0;
        eprintln!("FACTORY_PAYLOAD_RELAY_EXPECTED implId={:?} methodId={:?} callerMethodId={:?}", implementation.id, aligned.id, wrapper.id);
        relay(&mut receipt, call, &mut layers, 0, None);
        eprintln!("FACTORY_PAYLOAD_TOTAL payloadFirst={payload_first} methods=2 cap=2 layers={layers} cap=3 distinctTopLevelTypes={} cap=12 authority=ACCEPTED_HIR_OBSERVATION execution=NONE", receipt.types.len());
    }
    for (function, parameter_count) in [(aligned, 3), (wrapper, 2)] {
        assert_eq!(function.self_receiver, None);
        assert_eq!(function.params.len(), parameter_count);
        assert_eq!(function.generic_params.len(), 2);
        let declared = function.generic_params.iter().map(|parameter| {
            assert_eq!(parameter.kind, crate::type_services::kind::Kind::Type);
            // Implicit Sized is recorded on the declaration; do not invent a
            // bound-map row when inference only stored the explicit Memory row.
            assert!(!parameter.maybe_unsized);
            parameter.id
        }).collect::<std::collections::HashSet<_>>();
        assert_eq!(declared.len(), 2);
        let Type::Generic(actual_allocator) = function.params[0].ty else { panic!("generic allocator slot"); };
        let Type::Generic(actual_payload) = function.params[1].ty else { panic!("generic payload slot"); };
        assert_ne!(actual_allocator, actual_payload);
        assert!(declared.contains(&actual_allocator) && declared.contains(&actual_payload));
        assert!(function.generic_bounds[&actual_allocator].contains(&memory_bound));
        for parameter in &function.params { closed_type(&parameter.ty, &declared); }
        closed_type(&function.ret_type, &declared);
        for (subject, bounds) in &function.generic_bounds {
            assert!(declared.contains(subject));
            for bound in bounds {
                for argument in &bound.type_args { closed_type(argument, &declared); }
            }
        }
        for subject in &function.generic_bounds.relaxed_sized { assert!(declared.contains(subject)); }
        for predicate in &function.generic_bounds.predicates {
            let crate::types::Predicate::Trait { subject, args, .. } = predicate;
            closed_type(subject, &declared);
            for argument in args { closed_type(argument, &declared); }
        }
        closed_body(&function.body, &declared, 0);
        let expected_args = if payload_first {
            vec![Type::Generic(actual_payload), Type::Generic(actual_allocator)]
        } else {
            vec![Type::Generic(actual_allocator), Type::Generic(actual_payload)]
        };
        assert_eq!(function.ret_type, Type::Struct { id: shell.id, args: expected_args });
        assert_eq!(function.body.ty, function.ret_type);
    }
    assert_eq!(aligned.params[1].ty, Type::Generic(payload));
    assert_eq!(aligned.params[2].ty, Type::I64);
    let Type::Generic(aligned_allocator) = aligned.params[0].ty else { unreachable!() };
    let Type::Generic(wrapper_allocator) = wrapper.params[0].ty else { unreachable!() };
    let Type::Generic(wrapper_payload) = wrapper.params[1].ty else { unreachable!() };
    assert_eq!(aligned_allocator.owner, aligned.id);
    assert_eq!(wrapper_allocator.owner, wrapper.id);
    assert_eq!(wrapper_payload.owner, wrapper.id);
    let HirStmt::Let { value: call, .. } = &aligned.body.stmts[0] else { panic!("expected actual audit binding"); };
    let HirExprKind::MethodCall(receiver, _, arguments, mode, target) = &call.kind else {
        panic!("accepted producer must install a method target, not a deferred call/State field: {call:?}");
    };
    assert!(arguments.is_empty());
    assert_eq!(*mode, Some(ReceiverMode::Shared));
    assert_eq!(call.ty, Type::Pointer(Box::new(Type::Struct { id: audit.id, args: Vec::new() })));
    assert_eq!(target.target, HirSelectedMethodTarget::TraitMethod {
        trait_id: memory.id, member_id: member.id, trait_args: Vec::new(), dispatch: HirTraitDispatchKind::TraitBound,
    });
    // Ordinary Shared calls retain the value place plus receiver mode; unlike
    // object/scoped handles they do not materialize an explicit Ref wrapper.
    assert_eq!(receiver.ty, aligned.params[0].ty);
    assert!(matches!(&receiver.kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(aligned.params[0].local_id)));
    let HirStmt::Let { local_id: pointer_local, value: pointer, ty: pointer_ty, .. } = &aligned.body.stmts[1] else { panic!("expected pointer binding"); };
    let expected_pointer = Type::Pointer(Box::new(aligned.params[1].ty.clone()));
    assert_eq!(*pointer_ty, expected_pointer);
    assert_eq!(pointer.ty, expected_pointer);
    let HirExprKind::Cast(_, cast_target) = &pointer.kind else { panic!("expected pointer cast"); };
    assert_eq!(*cast_target, expected_pointer);
    let HirStmt::Expr(expression) = &aligned.body.stmts[2] else { panic!("expected actual store"); };
    let HirExprKind::Assign(place, value) = &store(expression, 0).kind else { unreachable!() };
    let HirExprKind::Deref(destination) = &place.kind else { panic!("expected pointer dereference"); };
    assert_eq!(place.ty, aligned.params[1].ty);
    assert_eq!(value.ty, aligned.params[1].ty);
    assert_eq!(destination.ty, expected_pointer);
    assert!(matches!(&destination.kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(*pointer_local)));
    assert!(matches!(&value.kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(aligned.params[1].local_id)));

    let HirStmt::Let { local_id: state_local, value: state_value, ty: state_ty, .. } = &aligned.body.stmts[3] else { panic!("expected State constructor"); };
    assert_eq!(*state_ty, Type::Struct { id: state.id, args: vec![aligned.params[0].ty.clone()] });
    assert_eq!(state_value.ty, *state_ty);
    let HirExprKind::StructLiteral(_, Some(owner), fields) = &state_value.kind else { panic!("expected actual State literal"); };
    assert_eq!(*owner, state.id);
    let allocator_field_id = state.fields.iter().find(|field| field.name == "allocator").unwrap().id;
    let allocator_field = fields.iter().find(|field| field.field.as_ref()
        .is_some_and(|location| location.owner == state.id && location.field_id == allocator_field_id)).unwrap();
    assert_eq!(allocator_field.value.ty, aligned.params[0].ty);
    assert!(matches!(&allocator_field.value.kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(aligned.params[0].local_id)));
    let HirStmt::Expr(shell_value) = aligned.body.stmts.last().unwrap() else { panic!("expected Shell tail"); };
    assert_eq!(shell_value.ty, aligned.ret_type);
    let HirExprKind::StructLiteral(_, Some(owner), fields) = &shell_value.kind else { panic!("expected actual Shell literal"); };
    assert_eq!(*owner, shell.id);
    for (field_name, expected_ty, local_id) in [
        ("pointer", &expected_pointer, *pointer_local),
        ("state", state_ty, *state_local),
    ] {
        let field_id = shell.fields.iter().find(|field| field.name == field_name).unwrap().id;
        let field = fields.iter().find(|field| field.field.as_ref()
            .is_some_and(|location| location.owner == shell.id && location.field_id == field_id)).unwrap();
        assert_eq!(&field.value.ty, expected_ty);
        assert!(matches!(&field.value.kind, HirExprKind::ResolvedVar(reference)
            if reference.target == HirVarTarget::Local(local_id)));
    }

    let HirStmt::Expr(wrapper_call) = wrapper.body.stmts.last().unwrap() else { panic!("expected factory relay"); };
    let HirExprKind::Call(lambda, outer_args, None) = &wrapper_call.kind else {
        panic!("expected the measured native Lambda application");
    };
    let HirExprKind::Lambda { params, body, captures } = &lambda.kind else { panic!("expected exactly one relay Lambda layer (cap=3)"); };
    assert_eq!(params.len(), 3);
    assert!(captures.is_empty());
    assert_eq!(outer_args.len(), 3);
    assert_eq!(wrapper_call.ty, wrapper.ret_type);
    assert_eq!(body.ty, wrapper.ret_type);
    let expected_params = vec![wrapper.params[0].ty.clone(), wrapper.params[1].ty.clone(), Type::I64];
    assert_eq!(params.iter().map(|parameter| parameter.ty.clone()).collect::<Vec<_>>(), expected_params);
    assert_eq!(outer_args.iter().map(|argument| argument.ty.clone()).collect::<Vec<_>>(), expected_params);
    for slot in 0..2 {
        assert!(matches!(&outer_args[slot].kind, HirExprKind::ResolvedVar(reference)
            if reference.target == HirVarTarget::Local(wrapper.params[slot].local_id)));
    }
    let HirExprKind::Intrinsic { args: alignment_args, .. } = &outer_args[2].kind else { panic!("expected actual alignment intrinsic"); };
    assert_eq!(alignment_args.len(), 1);
    assert_eq!(alignment_args[0].ty, wrapper.params[1].ty);
    assert!(matches!(&alignment_args[0].kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(wrapper.params[1].local_id)));
    let [HirStmt::Expr(inner_call)] = body.stmts.as_slice() else { panic!("expected exactly one targeted relay call"); };
    let HirExprKind::Call(callee, inner_args, Some(HirCallTarget::StaticMethod(target))) = &inner_call.kind else {
        panic!("relay Lambda must contain its explicit canonical static target");
    };
    assert_eq!(target.method.target, HirSelectedMethodTarget::ImplMethod {
        impl_id: implementation.id, method_id: aligned.id, selected_trait: None,
    });
    assert!(matches!(&callee.kind, HirExprKind::ResolvedVar(reference)
        if reference.target == HirVarTarget::Function(aligned.id)));
    assert_eq!(inner_args.len(), 3);
    for (argument, parameter) in inner_args.iter().zip(params) {
        assert_eq!(argument.ty, parameter.ty);
        assert!(matches!(&argument.kind, HirExprKind::ResolvedVar(reference)
            if reference.target == HirVarTarget::Local(parameter.local_id)));
    }
    assert_eq!(target.owner_ty, wrapper.ret_type);
    assert_eq!(target.method.owner_substitution.len(), 2);
    let owner_map = target.method.owner_substitution.iter().map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(owner_map, HashMap::from([
        (payload, Type::Generic(wrapper_payload)), (allocator, Type::Generic(wrapper_allocator)),
    ]));
    assert_eq!(target.method.method_substitution.len(), 1);
    let method_map = target.method.method_substitution.iter().map(|binding| (binding.param, binding.ty.clone()))
        .collect::<HashMap<_, _>>();
    assert_eq!(method_map, HashMap::from([(aligned_allocator, Type::Generic(wrapper_allocator))]));
    let substitution = owner_map.into_iter().chain(method_map).collect::<HashMap<_, _>>();
    let substituted_params = aligned.params.iter().map(|parameter| parameter.ty.substitute_generics(&substitution)).collect::<Vec<_>>();
    let substituted_return = aligned.ret_type.substitute_generics(&substitution);
    assert_eq!(substituted_params, expected_params);
    assert_eq!(substituted_params, inner_args.iter().map(|argument| argument.ty.clone()).collect::<Vec<_>>());
    assert_eq!(substituted_return, inner_call.ty);
    assert_eq!(substituted_return, wrapper.ret_type);
    for callable in [lambda.as_ref(), callee.as_ref()] {
        let Type::Function { params, ret, safety, callable_kind, captures, result_borrowing } = &callable.ty else { panic!("actual native relay function type"); };
        assert_eq!(*params, substituted_params);
        assert_eq!(ret.as_ref(), &substituted_return);
        assert_eq!(*safety, FunctionSafety::Safe);
        assert_eq!(*callable_kind, CallableKind::Fn);
        assert!(captures.is_empty());
        assert_eq!(*result_borrowing, crate::types::CallableResultBorrowing::Unknown);
    }
}

#[test]
fn sourcefree_field_authority_factory_payload_first() {
    assert_sourcefree_factory_authority(true);
}

#[test]
fn sourcefree_field_authority_factory_payload_last() {
    assert_sourcefree_factory_authority(false);
}

#[test]
fn sourcefree_field_authority_known_pointer_rejects_actual_call() {
    let (source, config) = canonical_field_source("struct Audit\nstruct Holder\n    < audit: *Audit\nmain = ->\n    holder = Holder\n        audit: 0 as *Audit\n    holder.audit!");
    let errors = crate::analyze(&config).expect_err("known pointer field is still not callable");
    assert_eq!(errors.0.len(), 1, "{errors:?}");
    let error = &errors.0[0];
    assert_eq!(error.code, Some(crate::diagnostic::DiagnosticCode::Type));
    assert_eq!(error.message, "value of type *Audit is not callable with these arguments");
    let span = &error.primary.as_ref().unwrap().span;
    let start = source.find("holder.audit!").unwrap();
    assert_eq!(span, &crate::lexer::Span::new(config.entry_file, start, start + "holder.audit!".len()));
}

#[test]
fn sourcefree_field_authority_native_and_reference_fields_keep_unknown_receiver_inference() {
    use crate::hir::{
        HirExprKindFor as HirExprKind, HirSelectedMethodTarget, HirStmtFor as HirStmt,
        HirTraitDispatchKind,
    };

    for (field_type, borrowed) in [("() -> I64", false), ("&(() -> I64)", true)] {
        let (_, config) = canonical_field_source(&format!(
            "struct Holder\n    < callback: {field_type}\nrun = holder -> holder.callback!\nmain = !-> ()",
        ));
        let analysis = crate::analyze(&config).expect("closed native/reference callable field must still infer its owner");
        let program = &analysis.hir.program;
        let holder = program.structs.values().find(|definition| definition.name == "Holder").unwrap();
        let run = program.test_function_by_name("run").unwrap().1;
        assert_eq!(run.params[0].ty, Type::Struct { id: holder.id, args: Vec::new() });
        assert_eq!(run.ret_type, Type::I64);
        let HirStmt::Expr(call) = run.body.stmts.last().unwrap() else { panic!("expected field invocation"); };
        let field = if borrowed {
            let HirExprKind::MethodCall(receiver, _, arguments, mode, target) = &call.kind else {
                panic!("borrowed native field must use canonical shared protocol dispatch: {call:?}");
            };
            let protocol = program.language_items.fn_trait.as_ref().unwrap();
            assert_eq!(*mode, Some(crate::types::ReceiverMode::Shared));
            assert_eq!(target.target, HirSelectedMethodTarget::TraitMethod {
                trait_id: protocol.trait_id,
                member_id: protocol.method_id,
                trait_args: vec![Type::Unit, Type::I64],
                dispatch: HirTraitDispatchKind::TraitBound,
            });
            assert_eq!(arguments.len(), 1);
            assert_eq!(arguments[0].ty, Type::Unit);
            assert!(matches!(arguments[0].kind, HirExprKind::Unit));
            receiver.as_ref()
        } else {
            let HirExprKind::Call(callee, arguments, _) = &call.kind else {
                panic!("native value field must retain native invocation: {call:?}");
            };
            assert!(arguments.is_empty());
            callee.as_ref()
        };
        let HirExprKind::FieldAccess(owner, _, Some(location)) = &field.kind else { panic!("expected actual callable field: {field:?}"); };
        assert_eq!(field.ty, holder.fields[0].ty);
        assert_eq!(location.owner, holder.id);
        assert_eq!(location.field_id, holder.fields[0].id);
        assert_eq!(owner.ty, run.params[0].ty);
    }
}

#[test]
fn sourcefree_field_authority_closed_callable_fields_keep_unknown_receiver_inference() {
    use crate::hir::{HirExprKind, HirSelectedMethodTarget, HirStmt};

    let task = "struct Task\nimpl Consume I64, I64 for Task\n    type Output = I64\n    ~@consume = value -> value\n";
    let object = "type Callback = Observe I64, I64 { (Consume I64, I64)::Output = I64, (Change I64, I64)::Output = I64, (Observe I64, I64)::Output = I64 }\n";
    for (declaration, field_type, object_dispatch) in [(task, "Task", false), (object, "&Callback", true)] {
        let (source, config) = canonical_field_source(&format!(
            "{declaration}struct Holder\n    < callback: {field_type}\nrun = holder -> holder.callback (7 as I64)\nmain = !-> ()",
        ));
        let module = crate::parser::parse_source(config.entry_file.clone(), &source, &config).unwrap();
        let partial = Lowerer::with_options(false).lower_program(&crate::ast::Program { module })
            .expect("closed protocol capability must preserve unknown receiver field inference");
        let holder = partial.structs.values().find(|definition| definition.name == "Holder").unwrap();
        let run = partial.functions.values().find(|function| function.name == "run").unwrap();
        assert_eq!(partial.engine.resolve(&run.params[0].ty), Type::Struct { id: holder.id, args: Vec::new() });
        assert_eq!(partial.engine.resolve(&run.ret_type), Type::I64);
        let HirStmt::Expr(call) = run.body.stmts.last().unwrap() else { panic!("expected protocol call"); };
        let HirExprKind::MethodCall(receiver, _, arguments, mode, Some(target)) = &call.kind else { panic!("expected real protocol selection: {call:?}"); };
        assert_eq!(arguments.len(), 1);
        assert_eq!(partial.engine.resolve(&arguments[0].ty), Type::I64);
        let mut field = receiver.as_ref();
        while let HirExprKind::Ref(_, inner) | HirExprKind::Deref(inner) = &field.kind { field = inner; }
        let HirExprKind::FieldAccess(owner, _, Some(location)) = &field.kind else { panic!("protocol receiver must retain canonical field: {field:?}"); };
        assert_eq!(location.owner, holder.id);
        assert_eq!(location.field_id, holder.fields[0].id);
        assert_eq!(partial.engine.resolve(&owner.ty), partial.engine.resolve(&run.params[0].ty));
        if object_dispatch {
            let protocol = partial.language_items.fn_trait.as_ref().unwrap();
            assert_eq!(*mode, Some(crate::types::ReceiverMode::Shared));
            assert_eq!(target.target, HirSelectedMethodTarget::TraitMethod {
                trait_id: protocol.trait_id,
                member_id: protocol.method_id,
                trait_args: vec![Type::I64, Type::I64],
                dispatch: crate::hir::HirTraitDispatchKind::Object,
            });
            assert_eq!(partial.engine.resolve(&field.ty), holder.fields[0].ty);
        } else {
            let protocol = partial.language_items.fn_once.as_ref().unwrap();
            assert_eq!(*mode, Some(crate::types::ReceiverMode::Move));
            assert!(matches!(&target.target, HirSelectedMethodTarget::ImplMethod { selected_trait: Some(selected), .. }
                if selected.trait_id == protocol.trait_id && selected.member_id == protocol.method_id));
        }
    }
}

#[test]
fn sourcefree_field_authority_plain_field_read_infers_unique_owner() {
    use crate::hir::{HirExprKindFor as HirExprKind, HirStmtFor as HirStmt};

    let (_, config) = canonical_field_source("struct Record\n    < value: I64\nread = record -> record.value\nmain = !-> ()");
    let analysis = crate::analyze(&config).expect("unfollowed numeric field still infers its unique owner");
    let program = &analysis.hir.program;
    let record = program.structs.values().find(|definition| definition.name == "Record").unwrap();
    let read = program.test_function_by_name("read").unwrap().1;
    assert_eq!(read.params[0].ty, Type::Struct { id: record.id, args: Vec::new() });
    assert_eq!(read.ret_type, Type::I64);
    let HirStmt::Expr(value) = read.body.stmts.last().unwrap() else { panic!("expected numeric read"); };
    let HirExprKind::FieldAccess(_, _, Some(location)) = &value.kind else { panic!("expected canonical field read"); };
    assert_eq!(location.owner, record.id);
    assert_eq!(location.field_id, record.fields[0].id);
}

#[test]
fn sourcefree_field_authority_assignment_place_infers_unique_owner() {
    use crate::hir::{HirExprKindFor as HirExprKind, HirStmtFor as HirStmt};

    let (_, config) = canonical_field_source("struct Record\n    < value: I64\nwrite = mut record ->\n    record.value = (42 as I64)\n    record\nmain = !-> ()");
    let analysis = crate::analyze(&config).expect("assignment-place Dot is not a direct call");
    let program = &analysis.hir.program;
    let record = program.structs.values().find(|definition| definition.name == "Record").unwrap();
    let write = program.test_function_by_name("write").unwrap().1;
    let expected = Type::Struct { id: record.id, args: Vec::new() };
    assert_eq!(write.params[0].ty, expected);
    assert_eq!(write.ret_type, expected);
    let HirStmt::Expr(assignment) = &write.body.stmts[0] else { panic!("expected assignment"); };
    let HirExprKind::Assign(place, _) = &assignment.kind else { panic!("expected actual assignment"); };
    let HirExprKind::FieldAccess(_, _, Some(location)) = &place.kind else { panic!("expected canonical assignment field"); };
    assert_eq!(location.owner, record.id);
    assert_eq!(location.field_id, record.fields[0].id);
}

fn id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn callable_lowerer() -> Lowerer {
    let mut lowerer = Lowerer::new_for_test();
    lowerer.language_items.fn_trait = Some(FnLanguageItems {
        trait_id: id(1), output_id: AssocTypeId(0), method_id: id(2),
    });
    lowerer.language_items.fn_once = Some(FnOnceLanguageItems {
        trait_id: id(3), output_id: AssocTypeId(1), method_id: id(4),
    });
    lowerer
}

#[test]
fn reference_argument_transports_callable_args_and_ret_without_retagging() {
    let mut lowerer = callable_lowerer();
    let subject = lowerer.engine.fresh_type_var();
    let result = lowerer.engine.fresh_type_var();
    let input = Type::Reference { inner: Box::new(Type::I64), mutable: false };
    let Type::TypeVar(subject_id) = subject else { unreachable!() };
    lowerer.engine.add_bound(subject_id, TraitBound {
        trait_id: id(1), type_args: vec![input.clone(), result.clone()],
    });
    let parameter = lowerer.engine.fresh_type_var();
    let callable = Type::function(vec![parameter.clone()], parameter);
    let expected = Type::Reference { inner: Box::new(subject), mutable: false };
    let actual = Type::Reference { inner: Box::new(callable.clone()), mutable: false };
    lowerer.propagate_call_argument_bounds(&expected, &actual);
    assert_eq!(lowerer.engine.resolve(&result), input.clone());
    assert_eq!(lowerer.engine.resolve(&callable), Type::function(vec![input.clone()], input));
    // Signature evidence is not a unification of the borrowed handle destination.
    assert_eq!(lowerer.engine.resolve(&expected), expected);
}

#[test]
fn callable_signature_propagation_is_transactional_and_marker_scoped() {
    let mut lowerer = callable_lowerer();
    let parameter = lowerer.engine.fresh_type_var();
    let result = lowerer.engine.fresh_type_var();
    let callable = Type::function(vec![parameter.clone()], Type::I64);
    let bad_result = TraitBound { trait_id: id(1), type_args: vec![Type::Bool, Type::Bool] };
    assert!(!crate::infer::solve::propagate_native_callable_bound(
        &mut lowerer.engine, &callable, &bad_result, &lowerer.language_items,
    ));
    assert_eq!(lowerer.engine.resolve(&parameter), parameter);
    let unmarked = TraitBound { trait_id: id(99), type_args: vec![Type::Bool, result.clone()] };
    assert!(!crate::infer::solve::propagate_native_callable_bound(
        &mut lowerer.engine, &callable, &unmarked, &lowerer.language_items,
    ));
    assert_eq!(lowerer.engine.resolve(&result), result);
}

#[test]
fn callable_signature_propagation_preserves_capability_and_safety() {
    let mut lowerer = callable_lowerer();
    for (kind, safety) in [(CallableKind::FnOnce, FunctionSafety::Safe), (CallableKind::Fn, FunctionSafety::Unsafe)] {
        let result = lowerer.engine.fresh_type_var();
        let captures = if kind == CallableKind::FnOnce {
            // Capability is derived from resolved captures, not an arbitrary tag.
            vec![FunctionCapture::new(CaptureKind::Move, Type::Reference {
                inner: Box::new(Type::I64), mutable: true,
            })]
        } else { Vec::new() };
        let callable = Type::function_with_metadata(vec![Type::I64], Type::I64, safety, kind, captures);
        assert!(matches!(lowerer.engine.resolve(&callable), Type::Function {
            callable_kind, safety: resolved_safety, ..
        } if callable_kind == kind && resolved_safety == safety));
        let bound = TraitBound { trait_id: id(1), type_args: vec![Type::I64, result.clone()] };
        assert!(!crate::infer::solve::propagate_native_callable_bound(
            &mut lowerer.engine, &callable, &bound, &lowerer.language_items,
        ));
        assert_eq!(lowerer.engine.resolve(&result), result);
        let mut store = crate::infer::ConstraintStore::new();
        store.add_trait(callable, bound, crate::lexer::Span::test(), "callable capability regression");
        let mut probe = lowerer.engine.clone_for_probe();
        let solved = crate::infer::solve::solve_constraints(
            &mut probe, &store, &HashMap::new(), &HashMap::new(), &HashMap::new(),
            &HashMap::new(), &lowerer.language_items,
        );
        assert!(!solved.errors.is_empty(), "canonical validation must reject unsupported callability");
    }
}

#[test]
fn wildcard_unit_argument_keeps_native_capture_result_and_exact_arity() {
    let mut lowerer = callable_lowerer();
    let argument = lowerer.engine.fresh_type_var();
    let result = lowerer.engine.fresh_type_var();
    let reference = Type::Reference { inner: Box::new(Type::I64), mutable: false };
    let captures = vec![FunctionCapture::new(CaptureKind::SharedBorrow, reference.clone())];
    let callable = Type::function_with_metadata(
        vec![argument.clone()], reference.clone(), FunctionSafety::Safe, CallableKind::Fn, captures,
    );
    let bound = TraitBound { trait_id: id(3), type_args: vec![Type::Unit, result.clone()] };
    assert!(crate::infer::solve::propagate_native_callable_bound(
        &mut lowerer.engine, &callable, &bound, &lowerer.language_items,
    ));
    assert_eq!(lowerer.engine.resolve(&argument), Type::Unit);
    assert_eq!(lowerer.engine.resolve(&result), reference);
    assert!(matches!(lowerer.engine.resolve(&callable), Type::Function { params, .. }
        if params == vec![Type::Unit]));
    let wrong_result = lowerer.engine.fresh_type_var();
    let wrong = Type::function(vec![Type::I64, Type::Unit], Type::I64);
    let wrong_bound = TraitBound { trait_id: id(3), type_args: vec![Type::Unit, wrong_result.clone()] };
    assert!(!crate::infer::solve::propagate_native_callable_bound(
        &mut lowerer.engine, &wrong, &wrong_bound, &lowerer.language_items,
    ));
    assert_eq!(lowerer.engine.resolve(&wrong_result), wrong_result);
}

#[test]
fn inferred_lambda_results_do_not_relax_universal_declared_unit_returns() {
    for declaration in ["bad: T -> T\nbad = value -> ()", "bad: T -> ()\nbad = value -> value"] {
        let path = PathBuf::from("/virtual/callable_borrow_inference/invalid.rk");
        let source = format!("{declaration}\nmain = !->\n    bad 1\n");
        let diagnostics = crate::compile_frontend(&Config {
            entry_file: path.clone(),
            current_crate_name: Some("invalid_callable_borrow_inference".into()),
            no_std: true,
            no_prelude: true,
            source_providers: vec![SourceProvider::Virtual { path, text: source }],
            ..Config::default()
        }).err().expect("a caller specialization cannot justify a universally invalid body");
        assert!(diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(crate::diagnostic::DiagnosticCode::Type)
                && diagnostic.primary.is_some()
        }), "{diagnostics:?}");
    }
}
