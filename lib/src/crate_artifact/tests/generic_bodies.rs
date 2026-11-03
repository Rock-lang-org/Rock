use super::*;

#[test]
fn test_hkt_receiver_authority_survives_artifact_round_trip() {
    let temp_dir = temp_test_dir("hkt_receiver_artifact");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");
    let std = shared_std_product_artifact();
    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
some_increment: I64 -> Option I64
some_increment = value -> pure (value + 1)

compute: () -> I64
< compute = ->
    mut values = Vec::new!
    values.push 4
    values.push 5
    traversed = values.traverse some_increment
    match traversed
        Option::Some items => items.foldl (pair -> pair.0 + pair.1), 0
        Option::None => 0
"#,
    );
    build_product_artifact_with_extern_artifacts(
        &dep_dir,
        "dep",
        &artifact_path,
        true,
        false,
        vec![("std".to_string(), std.clone())],
    );
    delete_crate_source_files(&dep_dir);
    assert_eq!(
        write_and_run_artifact_app(
            "> dep::compute\n\nmain = -> compute!\n",
            vec![("std".to_string(), std), ("dep".to_string(), artifact_path)],
        ),
        11
    );
}

#[test]
fn test_compile_generic_function_from_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_function_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "identity = x -> x\n< identity\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::identity\n\nmain = -> identity 7\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_cross_crate_hkt_selection_and_generic_body_specialization() {
    let temp_dir = temp_test_dir("cross_crate_hkt_specialization");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
trait Functor for F _
    keep: F A -> F A
< Functor

trait Monad for F _
    pure: A -> F A
< Monad

enum Maybe T
    None
    Some T
< Maybe

impl Functor for Maybe
    keep = value -> value

impl Monad for Maybe
    pure = value -> Maybe::Some value

keep_generic: F I64 -> F I64 where F _: Functor
keep_generic = value -> F::keep value
< keep_generic

repure: F I64 -> F I64 where F _: Monad
repure = ignored -> F::pure 42
< repure
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app_with_options(
        r#"
> dep::Maybe
> dep::keep_generic
> dep::repure

main = ->
    kept = keep_generic (Maybe::Some 7)
    result = repure kept
    match result
        Maybe::Some value => value
        Maybe::None => 1
"#,
        vec![("dep".to_string(), artifact_path)],
        None,
        true,
        true,
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_function_from_file_module_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_function_file_module_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod generic\n");
    fs::write(
        dep_dir.join("generic.rk"),
        "< identity = x -> x\n< identity\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::generic::identity\n\nmain = -> identity 7\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 7);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_impl_from_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_impl_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "identity = x -> x\n< identity\n\nstruct Box T\n    value: T\n< Box\n\nimpl Box T\n    new = value ->\n        Box\n            value: value\n\n    @apply = f -> f self.value\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::Box\n> dep::identity\n\nmain = ->\n    b = Box::new 21\n    b.apply identity\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 21);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_backed_structural_static_method_from_artifact() {
    let temp_dir = temp_test_dir("trait_backed_structural_static_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "struct Box T
    value: T
< Box

impl Box T
    wrap = value ->
        Box
            value: value

    @get = -> self.value

trait Factory T
    create: Box T -> Box T
< Factory

impl Factory T for Box T
    create = value -> value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Box\n\nmain = ->\n    input = Box::wrap 42\n    output = Box::create input\n    output.get!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_impl_from_file_module_artifact_hir_bundle() {
    let temp_dir = temp_test_dir("generic_impl_file_module_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod boxes\n");
    fs::write(
        dep_dir.join("boxes.rk"),
        "< identity = x -> x\n< identity\n\n< struct Box T\n    value: T\n< Box\n\nimpl Box T\n    new = value ->\n        Box\n            value: value\n\n    @apply = f -> f self.value\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::boxes::Box\n> dep::boxes::identity\n\nmain = ->\n    b = Box::new 21\n    b.apply identity\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 21);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_parameterized_equality_survives_deleted_source_product_artifact() {
    let temp_dir = temp_test_dir("parameterized_equality_artifact");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");
    let std_artifact = shared_std_product_artifact();
    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
< struct Tag
    < id: I64

impl PartialEq I64 for Tag
    @== = other -> self.id == *other

impl PartialEq Bool for Tag
    @== = other -> (self.id != 0) == *other

both: T -> U -> V -> Bool where T: PartialEq U, T: PartialEq V
< both = left, first, second -> left == first && left != second
"#,
    );
    build_product_artifact_with_extern_artifacts(
        &dep_dir,
        "dep",
        &artifact_path,
        false,
        false,
        vec![("std".to_string(), std_artifact.clone())],
    );
    delete_crate_source_files(&dep_dir);
    let exit_code = write_and_run_artifact_app(
        r#"
> dep::Tag
> dep::both

main = ->
    tag = Tag
        id: 7
    if tag == 7 && tag != false && both tag, 7, false
        0
    else
        1
"#,
        vec![
            ("std".to_string(), std_artifact),
            ("dep".to_string(), artifact_path),
        ],
    );
    assert_eq!(exit_code, 0);
}

#[test]
fn test_default_type_arguments_survive_deleted_source_product_artifact() {
    let temp_dir = temp_test_dir("default_type_arguments_artifact");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");
    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"
< struct Pair A, {B = A}
    < first: A
    < second: B

type Chosen {T = I64} = Pair T
< Chosen

struct Seed
    < value: I64

< struct Envelope {T = Seed}
    < value: T

< enum Choice {T = I64}
    Empty
    Value T

< struct Hold T, {F _ = Choice _}
    < value: F T

< trait Compare {Rhs = Self}
    @same: &Rhs -> Bool

same: T -> T -> Bool where T: Compare
< same = left, right -> left.same &right

read: Chosen -> I64
< read = value -> value.second

read_envelope: Envelope -> I64
< read_envelope = value -> value.value.value
"#,
    );
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    delete_crate_source_files(&dep_dir);
    let exit_code = write_and_run_artifact_app(
        r#"
> dep::Pair
> dep::Chosen
> dep::Envelope
> dep::Choice
> dep::Hold
> dep::Compare
> dep::same
> dep::read

struct Seed
    value: Bool

struct Local
    < value: I64

impl Compare for Local
    @same = rhs -> true

consume: Choice -> I64
consume = value -> match value
    Choice::Empty => 0
    Choice::Value item => item

scope_check: Envelope -> I64
scope_check = value -> value.value.value

read_hold: Hold I64 -> I64
read_hold = value -> match value.value
    Choice::Empty => 0
    Choice::Value item => item

main = ->
    pair: Chosen = Pair
        first: 7
        second: 42
    left = Local
        value: 9
    right = Local
        value: 9
    empty = Choice::Empty
    consume empty
    held = Hold
        value: Choice::Value 42
    if same left, right
        if ~I64Eq (~I64Add (read pair), (read_hold held)), 84
            0
        else
            1
    else
        2
"#,
        vec![("dep".into(), artifact_path)],
    );
    assert_eq!(exit_code, 0);
}
