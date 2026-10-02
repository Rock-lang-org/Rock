use std::sync::atomic::{AtomicUsize, Ordering};

use rock_lib::{Config, SourceProvider};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const OWNER: &str = r#"
lang sized
< trait KnownSize
lang drop
< trait Destroy
    lang method
    ~@destroy: ()
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
extern malloc: I64 -> *U8
extern free: *U8 -> ()
struct State
    < base: *U8
    < releases: *I64
struct Own (T: ?KnownSize)
    raw: *T
    base: *U8
    releases: *I64
impl Own T
    new: T -> *I64 -> Own T
    new = value, releases ->
        base = unsafe malloc (~SizeOf value)
        pointer: *T = base as *T
        unsafe *pointer = value
        Own T
            raw: pointer
            base: base
            releases: releases
impl Own T where T: ?KnownSize
    @borrow: &T
    @borrow = -> unsafe &*self.raw
impl Transport State for Own
    unsafe split = owner ->
        state = State
            base: owner.base
            releases: owner.releases
        parts = (owner.raw, state)
        ~Forget owner
        parts
    unsafe rebuild = pointer, state -> Own
        raw: pointer
        base: state.base
        releases: state.releases
impl Destroy for Own T where T: ?KnownSize
    ~@destroy = !->
        unsafe
            ~DropInPlace self.raw
            *self.releases = ~I64Add (*self.releases), 1
            free self.base
"#;

const UNIQUE: &str = r#"
impl Unique State for Own
    unsafe release = _, state !->
        *state.releases = ~I64Add (*state.releases), 1
        free state.base
"#;

const PAYLOAD: &str = r#"
trait Take
    ~@take: I64
    @peek: I64
struct Payload
    < answer: I64
    < drops: *I64
impl Take for Payload
    ~@take = -> self.answer
    @peek = -> self.answer
impl Destroy for Payload
    ~@destroy = !->
        unsafe
            *self.drops = ~I64Add (*self.drops), 1
"#;

const BORROWED_PAYLOAD: &str = r#"
trait Read
    @peek: I64
    ~@read: I64
trait Take where Self: Read
    ~@take: I64
struct Borrowed
    < answer: &I64
impl Read for Borrowed
    @peek = -> *self.answer
    ~@read = -> *self.answer
impl Take for Borrowed
    ~@take = -> *self.answer
"#;

fn config(source: String) -> Config {
    let directory = std::env::temp_dir().join(format!(
        "rock_owned_calls_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let entry = directory.join("main.rk");
    Config {
        entry_file: entry.clone(),
        output_dir: directory,
        current_crate_name: Some("owned_calls".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path: entry,
            text: source,
        }],
        ..Default::default()
    }
}

fn run(source: String) -> i32 {
    let config = config(source);
    rock_lib::compile(&config).unwrap();
    let result = std::process::Command::new(config.output_dir.join("main"))
        .output()
        .unwrap();
    assert!(
        result.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let code = result.status.code().unwrap();
    std::fs::remove_dir_all(config.output_dir).unwrap();
    code
}

fn assert_borrow_rejected(body: &str) {
    let source = format!("{OWNER}{UNIQUE}{BORROWED_PAYLOAD}\n{body}");
    let mut config = config(source);
    rock_lib::analyze(&config).expect("ownership fixture must pass frontend authority validation");
    config.no_link = true;
    let diagnostics = rock_lib::compile(&config).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| {
            diagnostic.code == Some(rock_lib::diagnostic::DiagnosticCode::Borrow)
                && diagnostic.primary.is_some()
        }),
        "expected a source-backed ownership rejection, got {diagnostics:?}"
    );
    std::fs::remove_dir_all(config.output_dir).unwrap();
}

#[test]
fn custom_owner_cannot_return_an_erased_payload_borrowing_a_local() {
    assert_borrow_rejected(
        r#"
escape: *I64 -> Own Take
escape = releases ->
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, releases
    owner
main = ->
    mut releases = 0
    owner = escape ((&mut releases) as *I64)
    owner.take!
"#,
    );
}

#[test]
fn custom_owner_cannot_return_a_borrowed_object_view_of_a_local() {
    assert_borrow_rejected(
        r#"
escape: *I64 -> &Take
escape = releases ->
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, releases
    owner.borrow!
main = ->
    mut releases = 0
    object = escape ((&mut releases) as *I64)
    object.peek!
"#,
    );
}

#[test]
fn live_payload_view_prevents_owner_split_and_rebuild_for_erasure() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner = Own::new payload, ((&mut releases) as *I64)
    view = owner.borrow!
    erased: Own Take = owner
    erased.take!
    view.peek!
"#,
    );
}

#[test]
fn live_object_view_prevents_owner_upcast_and_consumption() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    view = owner.borrow!
    upcast: Own Read = owner
    upcast.read!
    view.peek!
"#,
    );
}

#[test]
fn conditional_owner_upcast_and_consumption_preserves_the_live_loan() {
    assert_borrow_rejected(
        r#"
consume_branch: Own Take -> Bool -> I64
consume_branch = owner, condition ->
    view = owner.borrow!
    if condition
        upcast: Own Read = owner
        upcast.read!
    else
        0
    view.peek!
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    consume_branch owner, (~I64Eq answer, 42)
"#,
    );
}

#[test]
fn erased_owner_keeps_its_hidden_payload_reference_live_through_upcast() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    mut answer = 41
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    upcast: Own Read = owner
    answer = 42
    upcast.read!
"#,
    );
}

#[test]
fn owner_is_unusable_after_a_consuming_virtual_call() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    owner.take!
    owner.borrow!.peek!
"#,
    );
}

#[test]
fn binding_a_consuming_method_value_moves_the_owner_immediately() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    consume = owner.take
    owner.borrow!.peek!
    consume!
"#,
    );
}

#[test]
fn consuming_method_value_cannot_be_invoked_twice() {
    assert_borrow_rejected(
        r#"
main = ->
    mut releases = 0
    answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    consume = owner.take
    consume!
    consume!
"#,
    );
}

#[test]
fn hidden_payload_loan_ends_after_last_view_use_and_owner_consumption() {
    let source = format!(
        "{OWNER}{UNIQUE}{BORROWED_PAYLOAD}{}",
        r#"
main = ->
    mut releases = 0
    mut answer = 42
    payload = Borrowed
        answer: &answer
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    upcast: Own Read = owner
    view = upcast.borrow!
    observed = view.peek!
    consumed = upcast.read!
    answer = ~I64Add observed, (~I64Sub consumed, 42)
    if ~I64Eq releases, 1 then answer else 1
"#
    );
    assert_eq!(run(source), 42);
}

#[test]
fn owner_constructor_kind_and_state_binding_survive_declaration_scope() {
    use rock_lib::type_services::kind::Kind;
    use rock_lib::types::Type;

    let source = format!("{OWNER}{UNIQUE}\nmain = !-> ()\n");
    let analysis = rock_lib::analyze(&config(source)).unwrap();
    let program = &analysis.hir.program;
    let transport = program
        .traits
        .values()
        .find(|definition| definition.name == "Transport")
        .unwrap();
    let target = transport.target.as_ref().unwrap();
    assert_eq!(target.kind, Kind::arrow(Kind::Type, Kind::Type));
    let split = &transport.signatures["split"];
    let Type::Apply { constructor, args } = &split.params[0] else {
        panic!("the owner constructor must remain bound to the trait target")
    };
    assert_eq!(constructor.as_ref(), &Type::Generic(target.id));
    assert_eq!(args, &[Type::Generic(split.generic_params[0].id)]);
    let Type::Tuple(parts) = &split.ret else {
        panic!("transport parts")
    };
    assert_eq!(parts[1], Type::Generic(transport.generic_params[0].id));

    // A nominal declaration with the same spelling must not replace the trait binder,
    // but it must become the State argument when specializing the implementation.
    let state = program
        .structs
        .values()
        .find(|definition| definition.name == "State")
        .unwrap();
    let implementation = program
        .impls
        .values()
        .find(|implementation| implementation.trait_id == Some(transport.id))
        .unwrap();
    let Type::Tuple(parts) = &implementation.methods["split"].ret_type else {
        panic!("specialized transport parts")
    };
    assert_eq!(
        parts[1],
        Type::Struct {
            id: state.id,
            args: vec![],
        }
    );
}

#[test]
fn owner_free_constructor_signature_retains_kinds_after_header_consumption() {
    use rock_lib::type_services::kind::Kind;
    use rock_lib::types::Type;

    let source = format!(
        "{OWNER}{UNIQUE}\nidentity_owner: F T -> F T where F _: Transport State\nidentity_owner = owner -> owner\nmain = !-> ()\n"
    );
    let analysis = rock_lib::analyze(&config(source)).unwrap();
    let function = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "identity_owner")
        .unwrap();
    let constructor = function
        .generic_params
        .iter()
        .find(|parameter| parameter.name == "F")
        .unwrap();
    assert_eq!(constructor.kind, Kind::arrow(Kind::Type, Kind::Type));
    let Type::Apply {
        constructor: applied,
        ..
    } = &function.params[0].ty
    else {
        panic!("a free constructor signature must retain its application")
    };
    assert_eq!(applied.as_ref(), &Type::Generic(constructor.id));
    assert_eq!(function.params[0].ty, function.ret_type);
}

#[test]
fn owner_conversion_and_consumption_keep_canonical_protocol_members() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}\nerase: Own Payload -> Own Take\nerase = owner -> owner\nconsume: Own Take -> I64\nconsume = owner -> owner.take!\nmain = !-> ()\n"
    );
    let analysis = rock_lib::analyze(&config(source)).unwrap();
    let program = &analysis.hir.program;
    let consume = program
        .functions
        .values()
        .find(|function| function.name == "consume")
        .unwrap();
    let Some(rock_lib::hir::HirStmtFor::Expr(expression)) = consume.body.stmts.last() else {
        panic!("consuming tail")
    };
    let rock_lib::hir::HirExprKindFor::OwnedObjectCall { call, .. } = &expression.kind else {
        panic!("canonical owned call")
    };
    for (operation, trait_name, member_name) in [
        (&call.into_parts, "Transport", "split"),
        (&call.release, "Unique", "release"),
    ] {
        let definition = program
            .traits
            .values()
            .find(|definition| definition.name == trait_name)
            .unwrap();
        let implementation = program
            .impls
            .values()
            .find(|implementation| implementation.trait_id == Some(definition.id))
            .unwrap();
        assert_eq!(operation.method.impl_id(), Some(implementation.id));
        let constructor_id = analysis
            .hir
            .type_context
            .id_for_type(&operation.owner_ty)
            .expect("canonical owner constructor evidence must be interned");
        assert_eq!(
            analysis.hir.type_context.kind(constructor_id),
            &rock_lib::type_services::kind::Kind::arrow(
                rock_lib::type_services::kind::Kind::Type,
                rock_lib::type_services::kind::Kind::Type,
            )
        );
        assert!(analysis.hir.type_ids.iter().any(|(location, id)| {
            matches!(
                location,
                rock_lib::hir::HirTypeLocation::OwnedObjectCallEvidence { owner, .. }
                    if *owner == consume.id
            ) && id == constructor_id
        }));
        assert_eq!(
            operation.method.method_id(),
            Some(implementation.methods[member_name].id)
        );
        assert_eq!(
            operation.method.trait_args(),
            implementation.trait_arg_types.as_slice()
        );
    }
    let erase = program
        .functions
        .values()
        .find(|function| function.name == "erase")
        .unwrap();
    assert!(analysis.hir.type_ids.iter().any(|(location, _)| {
        matches!(
            location,
            rock_lib::hir::HirTypeLocation::ObjectCoercionEvidence { owner, .. }
                if *owner == erase.id
        )
    }));
}

#[test]
fn custom_unique_owner_consumes_payload_and_releases_storage_once() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
main = ->
    mut drops: I64 = 0
    mut releases: I64 = 0
    payload = Payload
        answer: 42
        drops: (&mut drops) as *I64
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    result = owner.take!
    if ~I64Eq drops, 1
        if ~I64Eq releases, 1 then result else 1
    else
        2
"#
    );
    let analysis = rock_lib::analyze(&config(source.clone())).unwrap();
    let main = analysis
        .hir
        .program
        .functions
        .values()
        .find(|function| function.name == "main")
        .unwrap();
    let call = main
        .body
        .stmts
        .iter()
        .find_map(|statement| match statement {
            rock_lib::hir::HirStmtFor::Let { name, value, .. } if name == "result" => Some(value),
            _ => None,
        })
        .unwrap();
    let rock_lib::hir::HirExprKindFor::OwnedObjectCall { owner, call, .. } = &call.kind else {
        panic!("owning operand must remain in HIR")
    };
    assert!(matches!(owner.ty, rock_lib::types::Type::Struct { .. }));
    assert_eq!(call.into_parts.owner_ty, call.release.owner_ty);
    assert_eq!(
        call.into_parts.method.trait_args(),
        call.release.method.trait_args()
    );
    assert_eq!(run(source), 42);
}

#[test]
fn references_do_not_authorize_consuming_virtual_calls() {
    for reference in ["&Take", "&mut Take", "*Take"] {
        let source = format!("{OWNER}{UNIQUE}{PAYLOAD}\nbad: {reference} -> I64\nbad = value -> value.take!\nmain = !-> ()\n");
        assert!(rock_lib::analyze(&config(source)).is_err());
    }
}

#[test]
fn unique_release_must_match_the_transport_state() {
    let source = format!(
        "{OWNER}{PAYLOAD}{}",
        r#"
impl Unique () for Own
    unsafe release = _, _ !-> ()
bad: Own Take -> I64
bad = owner -> owner.take!
main = !-> ()
"#
    );
    let diagnostics = rock_lib::analyze(&config(source)).unwrap_err();
    assert!(
        diagnostics.0.iter().any(|diagnostic| diagnostic
            .message
            .contains("same owner constructor and State")),
        "{diagnostics:?}"
    );
}

#[test]
fn consuming_owner_with_a_live_borrowed_view_is_rejected() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
main = ->
    mut drops: I64 = 0
    mut releases: I64 = 0
    payload = Payload
        answer: 42
        drops: (&mut drops) as *I64
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    view = owner.borrow!
    owner.take!
    view.peek!
"#
    );
    let diagnostics = rock_lib::compile(&config(source)).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("borrow")
                || diagnostic.message.contains("move")),
        "{diagnostics:?}"
    );
}

#[test]
fn consuming_member_can_return_a_noncopy_field_without_dropping_it_twice() {
    let source = format!(
        "{OWNER}{UNIQUE}{}",
        r#"
struct Piece
    < answer: I64
    < drops: *I64
impl Destroy for Piece
    ~@destroy = !->
        unsafe
            *self.drops = ~I64Add (*self.drops), 1
trait Take
    ~@take: Piece
struct Payload
    < piece: Piece
impl Take for Payload
    ~@take = -> self.piece
finish: Piece -> ()
finish = piece !-> ()
main = ->
    mut drops: I64 = 0
    mut releases: I64 = 0
    piece = Piece
        answer: 42
        drops: (&mut drops) as *I64
    payload = Payload
        piece: piece
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    returned = owner.take!
    answer = returned.answer
    finish returned
    if ~I64Eq drops, 1
        if ~I64Eq releases, 1 then answer else 1
    else
        2
"#
    );
    assert_eq!(run(source), 42);
}

#[test]
fn by_value_hidden_self_still_requires_explicit_packaging() {
    let source = format!("{OWNER}{UNIQUE}\ntrait Rebuild\n    ~@rebuild: Self\nbad: Own Rebuild -> ()\nbad = owner !-> owner.rebuild!\nmain = !-> ()\n");
    assert!(rock_lib::analyze(&config(source)).is_err());
}

#[test]
fn inferred_owner_consumption_uses_the_authority_worklist() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
main = ->
    mut drops: I64 = 0
    mut releases: I64 = 0
    payload = Payload
        answer: 42
        drops: (&mut drops) as *I64
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    answer = consume owner
    if ~I64Eq drops, 1
        if ~I64Eq releases, 1 then answer else 1
    else
        2
consume = owner -> owner.take!
"#
    );
    assert_eq!(run(source), 42);
}

#[test]
fn consuming_method_value_moves_the_owner_into_a_once_callable() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
main = ->
    mut drops: I64 = 0
    mut releases: I64 = 0
    payload = Payload
        answer: 42
        drops: (&mut drops) as *I64
    owner: Own Take = Own::new payload, ((&mut releases) as *I64)
    consume = owner.take
    answer = consume!
    if ~I64Eq drops, 1
        if ~I64Eq releases, 1 then answer else 1
    else
        2
"#
    );
    assert_eq!(run(source), 42);
}

#[test]
fn consuming_method_value_evaluates_its_owner_once_at_binding() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
make: *I64 -> *I64 -> *I64 -> Own Take
make = makes, drops, releases ->
    unsafe *makes = ~I64Add (*makes), 1
    payload = Payload
        answer: 42
        drops: drops
    owner: Own Take = Own::new payload, releases
    owner
main = ->
    mut makes: I64 = 0
    mut drops: I64 = 0
    mut releases: I64 = 0
    consume = (make ((&mut makes) as *I64), ((&mut drops) as *I64), ((&mut releases) as *I64)).take
    if ~I64Eq makes, 1
        answer = consume!
        if ~I64Eq drops, 1
            if ~I64Eq releases, 1 then answer else 1
        else
            2
    else
        3
"#
    );
    assert_eq!(run(source), 42);
}

#[test]
fn artifact_transport_revalidates_unique_owner_state_evidence() {
    let source = format!(
        "{OWNER}{UNIQUE}{PAYLOAD}{}",
        r#"
consume: Own Take -> U -> I64
< consume = owner, unused -> owner.take!
"#
    );
    let mut producer = config(source);
    producer.no_link = true;
    producer.emit_object = Some(producer.output_dir.join("owned_calls.o"));
    let mut products = rock_lib::compile_with_products(&producer)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("owned_calls.o".into());
    let artifact = producer.output_dir.join("owned_calls.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let mut consumer = config("main = !-> ()\n".into());
    consumer.current_crate_name = Some("owned_call_consumer".into());
    consumer
        .extern_artifacts
        .push(("owned_calls".into(), artifact.clone()));
    rock_lib::analyze(&consumer).expect("valid remapped owner proof");

    let function = products
        .bodies
        .functions
        .values_mut()
        .find(|function| function.name == "consume")
        .unwrap();
    let Some(rock_lib::hir::HirStmtFor::Expr(expression)) = function.body.stmts.last_mut() else {
        panic!("consuming tail")
    };
    let rock_lib::hir::HirExprKindFor::OwnedObjectCall { call, .. } = &mut expression.kind else {
        panic!("persisted owned call")
    };
    call.state = rock_lib::types::Type::Bool;
    products.write_artifact_to_path(&artifact).unwrap();
    let diagnostics = rock_lib::analyze(&consumer).unwrap_err();
    assert!(
        diagnostics
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("owned-object evidence")),
        "{diagnostics:?}"
    );
    std::fs::remove_dir_all(producer.output_dir).unwrap();
    std::fs::remove_dir_all(consumer.output_dir).unwrap();
}
