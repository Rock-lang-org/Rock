use rock_lib::types::Type;
use rock_lib::{Config, SourceProvider};

fn analyze(
    source: &str,
) -> Result<rock_lib::analysis::Analysis, rock_lib::diagnostic::Diagnostics> {
    let path = std::env::temp_dir()
        .join("rock_object_type_analysis")
        .join("main.rk");
    rock_lib::analyze(&Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        current_crate_name: Some("objects".to_string()),
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: source.to_string(),
        }],
        ..Config::default()
    })
}

#[test]
fn trait_names_in_signatures_are_objects_without_dyn_or_implicit_generics() {
    let analysis = analyze("ignore: &Value -> ()\nignore = value !-> ()\ntrait Value\n    @value: I64\nmain = !-> ()\n").unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "ignore")
        .unwrap();
    assert!(function.generic_params.is_empty());
    let Type::Reference {
        inner,
        mutable: false,
    } = &function.params[0].ty
    else {
        panic!("shared object reference expected")
    };
    let Type::Object(object) = inner.as_ref() else {
        panic!("object type expected")
    };
    let definition = analysis
        .hir
        .program
        .traits
        .get(&object.principal.trait_id)
        .unwrap();
    assert_eq!(definition.name, "Value");
}

#[test]
fn forward_object_signatures_use_complete_instantiated_supertraits() {
    let analysis = analyze("ignore: &(Derived I64) -> ()\nignore = value !-> ()\ntrait Derived A where Self: Base A\ntrait Base A\n    @value: I64\nmain = !-> ()\n").unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "ignore")
        .unwrap();
    let Type::Reference { inner, .. } = &function.params[0].ty else {
        panic!("reference expected")
    };
    let Type::Object(object) = inner.as_ref() else {
        panic!("object expected")
    };
    assert_eq!(object.principal.type_args, vec![Type::I64]);
    assert!(object
        .guarantees
        .iter()
        .any(
            |bound| analysis.hir.program.traits[&bound.trait_id].name == "Base"
                && bound.type_args == vec![Type::I64]
        ));
}

#[test]
fn missing_object_bindings_point_to_the_type_use() {
    let source = "trait Source\n    type Item\n    @get: Self::Item\nignore: &Source -> ()\nignore = value !-> ()\nmain = !-> ()\n";
    let diagnostics = analyze(source).unwrap_err();
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("missing required object associated binding")
        })
        .expect("missing binding diagnostic");
    let span = &diagnostic.primary.as_ref().expect("source span").span;
    assert_eq!(&source[span.start..span.end], "Source");
}

#[test]
fn forward_qualified_objects_resolve_inherited_instantiated_members() {
    let analysis = analyze(
        r#"
ignore: &(Derived I64 { (Base I64)::Item = I64 }) -> ()
ignore = source !-> ()
trait Derived A where Self: Base A
trait Base A
    type Item
    @get: Self::Item
main = !-> ()
"#,
    )
    .unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "ignore")
        .unwrap();
    let Type::Reference { inner, .. } = &function.params[0].ty else {
        panic!("reference expected")
    };
    let Type::Object(object) = inner.as_ref() else {
        panic!("object expected")
    };
    let binding = object.bindings.iter().next().expect("inherited binding");
    assert_eq!(binding.ty, Type::I64);
    assert_eq!(binding.key.trait_ref.type_args, vec![Type::I64]);
    assert_eq!(
        analysis.hir.program.traits[&binding.key.trait_ref.trait_id].name,
        "Base"
    );
}

#[test]
fn borrowed_object_creation_retains_impl_evidence_and_virtual_authority() {
    use rock_lib::hir::{
        HirExprKindFor as E, HirObjectEvidence, HirObjectWitnessOrigin, HirSelectedMethodTarget,
        HirStmtFor as S, HirTraitDispatchKind,
    };
    let analysis = analyze(
        r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
read: &Value -> I64
read = object -> object.value!
main = !->
    number = Number
        inner: 42
    object: &Value = &number
    read object
"#,
    )
    .unwrap();
    let program = &analysis.hir.program;
    let main = program
        .functions
        .values()
        .find(|function| function.name == "main")
        .unwrap();
    let coercion = main
        .body
        .stmts
        .iter()
        .find_map(|statement| match statement {
            S::Let { value, .. } => match &value.kind {
                E::ObjectCoercion(_, coercion) => Some(coercion),
                _ => None,
            },
            _ => None,
        })
        .expect("explicit object coercion");
    let Some(HirObjectEvidence::Concrete { views, source }) = &coercion.evidence else {
        panic!("concrete proof expected")
    };
    assert!(matches!(source, Type::Struct { .. }));
    assert_eq!(views.len(), 1);
    let HirObjectWitnessOrigin::Impl { impl_id, .. } = &views[0].origin else {
        panic!("impl proof expected")
    };
    assert!(program.impls.contains_key(impl_id));
    let read = program
        .functions
        .values()
        .find(|function| function.name == "read")
        .unwrap();
    let Some(S::Expr(expression)) = read.body.stmts.last() else {
        panic!("method call expected")
    };
    let E::MethodCall(receiver, _, _, _, target) = &expression.kind else {
        panic!("virtual call expected")
    };
    assert!(
        matches!(&receiver.ty, Type::Reference { mutable: false, inner } if matches!(inner.as_ref(), Type::Object(_)))
    );
    assert!(matches!(
        target.target,
        HirSelectedMethodTarget::TraitMethod {
            dispatch: HirTraitDispatchKind::Object,
            ..
        }
    ));
}

#[test]
fn inferred_object_receiver_keeps_its_borrow_through_deferred_selection() {
    let analysis = analyze(
        r#"
trait Value
    @value: I64
read: &Value -> I64
read = object -> (identity object).value!
identity = value -> value
main = !-> ()
"#,
    )
    .unwrap();
    let read = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "read")
        .unwrap();
    let Some(rock_lib::hir::HirStmtFor::Expr(call)) = read.body.stmts.last() else {
        panic!("call tail expected")
    };
    let rock_lib::hir::HirExprKindFor::MethodCall(receiver, _, _, _, _) = &call.kind else {
        panic!("virtual call expected")
    };
    assert!(
        matches!(&receiver.ty, Type::Reference { mutable: false, inner } if matches!(inner.as_ref(), Type::Object(_)))
    );
}

#[test]
fn signatureless_relay_does_not_select_the_only_concrete_self_impl() {
    analyze(
        r#"
trait View
    @view: &Self
struct Number
    < inner: I64
impl View for Number
    @view = -> self
main = !->
    number = Number
        inner: 42
    object: &View = &number
    derived = relay object
    derived.view!
relay = value -> value
"#,
    )
    .unwrap();
}

#[test]
fn signatureless_relay_method_value_keeps_virtual_authority() {
    analyze(
        r#"
trait View
    @view: &Self
struct Number
    < inner: I64
impl View for Number
    @view = -> self
main = !->
    number = Number
        inner: 42
    object: &View = &number
    derived = relay object
    view = derived.view
    view!
relay = value -> value
"#,
    )
    .unwrap();
}

#[test]
fn shared_object_cannot_call_an_exclusive_virtual_member() {
    let diagnostics = analyze(
        r#"
trait Cell
    ^@set: I64 -> ()
write: &Cell -> ()
write = cell !-> cell.set 7
main = !-> ()
"#,
    )
    .unwrap_err();
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics
        .0
        .iter()
        .all(|diagnostic| diagnostic.primary.is_some()));
}

#[test]
fn borrowed_objects_coerce_at_returns_arguments_fields_and_typed_joins() {
    analyze(
        r#"
trait Value
    @value: I64
struct Number
    < inner: I64
struct Other
    < inner: I64
struct Holder
    < object: &Value
impl Value for Number
    @value = -> self.inner
impl Value for Other
    @value = -> self.inner
erase: &Number -> &Value
erase = number -> number
read: &Value -> I64
read = object -> object.value!
main = !->
    number = Number
        inner: 1
    other = Other
        inner: 2
    object: &Value = if true
        &number
    else
        &other
    holder = Holder
        object: &number
    read &number
    erase &number
    read object
"#,
    )
    .unwrap();
}

#[test]
fn object_creation_rejects_missing_implementation_and_mutability_upgrade() {
    assert!(analyze("trait Value\n    @value: I64\nmain = !->\n    number: I64 = 1\n    object: &Value = &number\n").is_err());
    assert!(analyze("trait Value\n    @value: I64\nbad: &Value -> &mut Value\nbad = value -> value\nmain = !-> ()\n").is_err());
}

#[test]
fn object_upcast_cannot_add_guarantees() {
    assert!(analyze("trait Value\n    @value: I64\ntrait Extra\nbad: &Value -> &(Value { Extra })\nbad = value -> value\nmain = !-> ()\n").is_err());
}

#[test]
fn borrowed_generic_supertrait_upcast_retains_instantiation() {
    analyze(
        r#"
trait Base A
    @value: A
trait Derived A where Self: Base A
upcast: &(Derived I64) -> &(Base I64)
upcast = value -> value
read: &(Derived I64) -> I64
read = value -> value.value!
main = !-> ()
"#,
    )
    .unwrap();
}

#[test]
fn mutable_borrowed_object_selects_exclusive_receiver() {
    let analysis = analyze(
        r#"
trait Cell
    ^@set: I64 -> ()
struct Number
    < value: I64
impl Cell for Number
    ^@set = value !->
        self.value = value
write: &mut Cell -> ()
write = cell !-> cell.set 7
main = !->
    mut number = Number
        value: 1
    object: &mut Cell = &mut number
    write object
"#,
    )
    .unwrap();
    let write = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "write")
        .unwrap();
    let receiver = write
        .body
        .stmts
        .iter()
        .find_map(|statement| match statement {
            rock_lib::hir::HirStmtFor::Expr(expression) => match &expression.kind {
                rock_lib::hir::HirExprKindFor::MethodCall(receiver, _, _, _, _) => Some(receiver),
                _ => None,
            },
            _ => None,
        })
        .expect("virtual call expected");
    assert!(
        matches!(&receiver.ty, Type::Reference { mutable: true, inner } if matches!(inner.as_ref(), Type::Object(_)))
    );
}

#[test]
fn associated_object_method_uses_binding_without_exposing_hidden_self() {
    analyze(
        r#"
trait Source A
    type Item
    @get: Self::Item
struct Number
    < value: I64
impl Source Bool for Number
    type Item = I64
    @get = -> self.value
read: &(Source Bool { Item = I64 }) -> I64
read = source -> source.get!
main = !->
    number = Number
        value: 7
    read &number
"#,
    )
    .unwrap();
}

#[test]
fn object_method_cannot_treat_an_independent_object_as_hidden_self() {
    assert!(analyze(
        r#"
trait Same
    @accept: &Self -> ()
bad: &Same -> &Same -> ()
bad = left, right !-> left.accept right
main = !-> ()
"#
    )
    .is_err());
}

#[test]
fn short_object_member_names_reject_inherited_ambiguity() {
    let diagnostics = analyze("trait Left\n    type Item\ntrait Right\n    type Item\ntrait Both where Self: Left, Self: Right\nignore: &(Both { Item = I64 }) -> ()\nignore = value !-> ()\nmain = !-> ()\n").unwrap_err();
    assert!(diagnostics
        .0
        .iter()
        .any(|diagnostic| diagnostic.message.contains("ambiguous")));
}

#[test]
fn conflicting_object_bindings_retain_the_second_member_span() {
    let source = "trait Source\n    type Item\nignore: &(Source { Item = I64, Item = Bool }) -> ()\nignore = source !-> ()\nmain = !-> ()\n";
    let diagnostics = analyze(source).unwrap_err();
    let diagnostic = diagnostics
        .0
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("conflicting object associated binding")
        })
        .expect("conflicting binding diagnostic");
    let span = &diagnostic.primary.as_ref().expect("source span").span;
    assert_eq!(span.start, source.rfind("Item = Bool").unwrap());
    assert_eq!(&source[span.start..span.end], "Item");
}

#[test]
fn deferred_object_coercion_preserves_forward_inferred_source() {
    analyze(
        r#"
trait Value
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
read: &Value -> I64
read = object -> object.value!
main = !->
    number = make!
    object: &Value = borrow &number
    read (borrow &number)
make = -> Number
    inner: 42
borrow = value -> value
"#,
    )
    .unwrap();
}

#[test]
fn deferred_object_coercion_uses_generic_assumptions() {
    analyze(
        r#"
trait Value
    @value: I64
erase: &T -> &Value where T: Value
erase = value -> value
main = !-> ()
"#,
    )
    .unwrap();
}

#[test]
fn deferred_object_coercion_preserves_typed_branch_sources() {
    analyze(
        r#"
trait Value
    @value: I64
struct Number
    < inner: I64
struct Other
    < inner: I64
impl Value for Number
    @value = -> self.inner
impl Value for Other
    @value = -> self.inner
main = !->
    number = Number
        inner: 1
    other = Other
        inner: 2
    object: &Value = if true
        identity &number
    else
        identity &other
identity = value -> value
"#,
    )
    .unwrap();
}

#[test]
fn borrowed_self_result_retains_the_receiver_object_view() {
    use rock_lib::hir::{HirExprKindFor as E, HirStmtFor as S};
    let analysis = analyze(
        r#"
trait Extra
trait Again
    @again: &Self
repeat: &(Again { Extra }) -> &(Again { Extra })
repeat = object -> object.again!
main = !-> ()
"#,
    )
    .unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "repeat")
        .unwrap();
    let Some(S::Expr(call)) = function.body.stmts.last() else {
        panic!("tail call")
    };
    let E::MethodCall(receiver, _, _, _, _) = &call.kind else {
        panic!("virtual edge")
    };
    assert_eq!(call.ty, receiver.ty);
    assert_eq!(call.ty, function.ret_type);
}

#[test]
fn object_auto_guarantees_use_renamed_language_items() {
    analyze(
        r#"
lang sized
< trait Layout
lang send
< trait Transfer
lang sync
< trait Share
trait Value where Self: Layout, Self: Transfer, Self: Share
    @value: I64
struct Number
    < inner: I64
impl Value for Number
    @value = -> self.inner
main = !->
    number = Number
        inner: 7
    object: &Value = &number
"#,
    )
    .unwrap();
}

#[test]
fn qualifier_type_lambda_keeps_captured_arguments_through_beta_reduction() {
    let analysis = analyze(
        r#"
type Read = \A -> Source { Item = A }
ignore: &(Read I64) -> ()
ignore = source !-> ()
trait Source
    type Item
main = !-> ()
"#,
    )
    .unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "ignore")
        .unwrap();
    let Type::Reference { inner, .. } = &function.params[0].ty else {
        panic!("reference")
    };
    let Type::Object(object) = inner.as_ref() else {
        panic!("object")
    };
    assert_eq!(object.bindings.iter().next().unwrap().ty, Type::I64);
}

#[test]
fn nested_qualifier_lambdas_preserve_each_captured_binder() {
    let analysis = analyze(
        r#"
type Read = \A -> \B -> Source { Item = (A, B) }
ignore: &((Read I64) Bool) -> ()
ignore = source !-> ()
trait Source
    type Item
main = !-> ()
"#,
    )
    .unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "ignore")
        .unwrap();
    let Type::Reference { inner, .. } = &function.params[0].ty else {
        panic!("reference")
    };
    let Type::Object(object) = inner.as_ref() else {
        panic!("object")
    };
    assert_eq!(
        object.bindings.iter().next().unwrap().ty,
        Type::Tuple(vec![Type::I64, Type::Bool])
    );
}

#[test]
fn borrowed_object_method_value_retains_virtual_self_result() {
    analyze(
        r#"
trait Again
    @again: &Self
repeat: &Again -> &Again
repeat = object ->
    again = object.again
    again!
main = !-> ()
"#,
    )
    .unwrap();
}

#[test]
fn auto_guarantee_rejects_payload_with_raw_pointer() {
    assert!(analyze(
        r#"
lang send
< trait Transfer
trait Value
    @value: I64
struct UnsafePayload
    pointer: *I64
impl Value for UnsafePayload
    @value = -> 0
erase: &UnsafePayload -> &(Value { Transfer })
erase = payload -> payload
main = !-> ()
"#
    )
    .is_err());
}

#[test]
fn native_callable_object_uses_marker_owned_output_and_virtual_application() {
    analyze(r#"
lang fn_once
< trait Once Args, Ret
    lang output
    type Result
    lang method
    ~@invoke_once: Args -> Ret
lang fn_mut
< trait Mut Args, Ret where Self: Once Args, Ret
    lang output
    type Result
    lang method
    ^@invoke_mut: Args -> Ret
lang fn
< trait Shared Args, Ret where Self: Mut Args, Ret
    lang output
    type Result
    lang method
    @invoke: Args -> Ret
type Callback = Shared I64, I64 { (Once I64, I64)::Result = I64, (Mut I64, I64)::Result = I64, (Shared I64, I64)::Result = I64 }
call: &Callback -> I64
call = callback -> callback 7
identity: I64 -> I64
identity = value -> value
main = !->
    callback: &Callback = &identity
    call callback
"#).unwrap();
}

#[test]
fn non_reference_call_arguments_keep_static_trait_inference() {
    analyze(
        r#"
trait Convert A
    from: A -> Self
impl Convert T for T
    from = value -> value
struct Text
    < inner: I64
impl Convert I64 for Text
    from = value -> Text
        inner: value
convert: I64 -> Text
convert = value -> Text::from value
main = !->
    convert 42
"#,
    )
    .unwrap();
}
