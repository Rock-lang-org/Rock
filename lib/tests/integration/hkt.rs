use crate::support::*;

#[test]
fn test_functor_operator_on_array_names_trait_and_implementors() {
    compile_should_fail(
        r#"
main = !->
    values = [1, 2, 3]
    (values <&> (+ 1)).println!
"#,
        "`<&>` needs its argument to be a type constructor applied to a type (like `F A`) \
         implementing `Functor`, but `[I64; 3]` is a built-in array type, which cannot be \
         used that way; `Functor` is implemented for Option, Result, Vec",
    );
}

#[test]
fn test_constructor_pattern_rejects_tuple_with_clear_message() {
    compile_should_fail(
        r#"
main = !->
    (fmap (+ 1), (1, 2)).println!
"#,
        "`(I64, I64)` is a tuple type, not a type constructor applied to a type, \
         so it cannot match `_ I64`",
    );
}

#[test]
fn constructor_trait_selection_option_pure_compiles() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

main = ->
    value = Maybe::pure 42
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_generic_call_specializes_before_mir() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Maybe T
    None
    Some T

impl Applicative for Maybe
    pure = value -> Maybe::Some value

repure: F I64 -> F I64 where F _: Applicative
repure = ignored -> F::pure 42

main = ->
    value = repure (Maybe::Some 1)
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_result_section_specializes_before_llvm() {
    compile_should_pass(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

struct IoError

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

repure: Outcome I64, E -> Outcome I64, E
repure = ignored -> (Outcome _, E)::pure 42

main = ->
    input: Outcome I64, IoError = Outcome::Ok 1
    value = repure input
    0
"#,
    );
}

#[test]
fn constructor_trait_selection_rejects_bare_binary_constructor_for_unary_trait() {
    compile_should_fail(
        r#"
trait Applicative for F _
    pure: A -> F A

enum Outcome T, E
    Ok T
    Err E

impl Applicative for Outcome _, E
    pure = value -> Outcome::Ok value

main = ->
    value = Outcome::pure 42
    0
"#,
        "has kind Type -> Type -> Type",
    );
}

#[test]
fn test_hkt_functor_option_result_vec() {
    let products =
        rock_lib::products::CompilerProducts::read_artifact_from_path(&std_artifact_path())
            .expect("read std artifact");
    let functional_trait_ids = products
        .interface
        .traits
        .iter()
        .filter_map(|(id, trait_interface)| {
            [
                "Bifunctor",
                "Functor",
                "Applicative",
                "Monad",
                "Foldable",
                "Traversable",
            ]
            .iter()
            .any(|name| trait_interface.name.ends_with(name))
            .then_some(*id)
        })
        .collect::<Vec<_>>();
    assert_eq!(functional_trait_ids.len(), 6);
    let language_items = &products.interface.language_items;
    let language_trait_ids = [
        language_items.sized.as_ref().map(|items| items.trait_id),
        language_items.drop.as_ref().map(|items| items.trait_id),
        language_items.index.as_ref().map(|items| items.trait_id),
        language_items
            .index_mut
            .as_ref()
            .map(|items| items.trait_id),
        language_items.fn_once.as_ref().map(|items| items.trait_id),
        language_items.fn_mut.as_ref().map(|items| items.trait_id),
        language_items.fn_trait.as_ref().map(|items| items.trait_id),
        language_items.send.as_ref().map(|items| items.trait_id),
        language_items.sync.as_ref().map(|items| items.trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.try_trait_id),
        language_items
            .try_protocol
            .as_ref()
            .map(|items| items.from_residual_trait_id),
    ];
    assert!(functional_trait_ids
        .iter()
        .all(|id| !language_trait_ids.contains(&Some(*id))));

    let output = compile_and_run(
        r#"
> std::io::IoError

type IoResult = Result _, IoError

map_any: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_any = mapper, value -> F::Functor::fmap mapper, value

map_result: M -> Result A, E -> Result B, E where M: FnMut A, B
map_result = mapper, value -> (Result _, E)::Functor::fmap mapper, value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

map_values: Vec I64 -> Vec I64
map_values = values -> map_any double, values

main = ->
    mapped_option: Option I64 = map_any inc, (Option::Some 4)
    mapped_result: Result I64, I64 = map_result inc, (Result::Ok 5)
    mapped_vec: Vec I64 = (make_values!) <&> double
    qualified: Option I64 = Option::Functor::fmap inc, (Option::Some 6)
    fixed_error: Result I64, IoError = (Result _, IoError)::Applicative::pure 8
    alias_error: Result I64, IoError = IoResult::Applicative::pure 9
    nested: Option (Vec I64) = Option::Some (make_values!)
    mapped_nested: Option (Vec I64) = map_any map_values, nested

    mapped_option.println!
    mapped_result.println!
    mapped_vec.println!
    qualified.println!
    fixed_error.println!
    alias_error.println!
    mapped_nested.println!
    0
"#,
    );

    assert_eq!(
        output,
        "Some(5)\nOk(6)\n[2, 4, 6]\nSome(7)\nOk(8)\nOk(9)\nSome([2, 4, 6])\n"
    );
}

#[test]
fn test_hkt_applicative_and_monad_laws() {
    let output = compile_and_run(
        r#"
repure_any: F I64 -> F I64 where F _: Applicative
repure_any = ignored -> F::Applicative::pure 2

ap_any: F (A -> B) -> F A -> F B where F _: Applicative
ap_any = wrapped_function, wrapped_value ->
    F::Applicative::ap wrapped_function, wrapped_value

bind_any: F A -> M -> F B where F _: Monad, M: FnMut A, (F B)
bind_any = value, callback -> F::Monad::bind value, callback

id_i64: I64 -> I64
id_i64 = value -> value

inc: I64 -> I64
inc = value -> value + 1

double: I64 -> I64
double = value -> value * 2

inc_after_double: I64 -> I64
inc_after_double = value -> inc (double value)

apply_five: (I64 -> I64) -> I64
apply_five = function -> function 5

option_add_two: I64 -> Option I64
option_add_two = value -> Option::Some (value + 2)

option_double: I64 -> Option I64
option_double = value -> Option::Some (value * 2)

option_pure: I64 -> Option I64
option_pure = value -> Option::Applicative::pure value

result_add_two: I64 -> Result I64, I64
result_add_two = value -> Result::Ok (value + 2)

result_double: I64 -> Result I64, I64
result_double = value -> Result::Ok (value * 2)

result_pure: I64 -> Result I64, I64
result_pure = value -> (Result _, I64)::Applicative::pure value

main = ->
    generic_pure: Option I64 = repure_any (Option::Some 0)
    generic_function: Option (I64 -> I64) = Option::Some inc
    generic_ap: Option I64 = ap_any generic_function, generic_pure
    generic_bind: Option I64 = bind_any generic_ap, option_add_two

    option_value: Option I64 = Option::Some 3
    option_identity_function: Option (I64 -> I64) = Option::Applicative::pure id_i64
    option_identity_l: Option I64 = Option::Applicative::ap option_identity_function, option_value
    option_identity_r: Option I64 = Option::Some 3

    option_composition_l: Option I64 = Option::Applicative::ap (Option::Some inc_after_double), (Option::Some 3)
    option_composition_inner: Option I64 = Option::Applicative::ap (Option::Some double), (Option::Some 3)
    option_composition_r: Option I64 = Option::Applicative::ap (Option::Some inc), option_composition_inner

    option_inc: Option (I64 -> I64) = Option::Applicative::pure inc
    option_four: Option I64 = Option::Applicative::pure 4
    option_homomorphism_l: Option I64 = Option::Applicative::ap option_inc, option_four
    option_homomorphism_r: Option I64 = Option::Applicative::pure (inc 4)
    option_five: Option I64 = Option::Applicative::pure 5
    option_interchange_l: Option I64 = Option::Applicative::ap (Option::Some inc), option_five
    option_apply_five: Option ((I64 -> I64) -> I64) = Option::Applicative::pure apply_five
    option_interchange_r: Option I64 = Option::Applicative::ap option_apply_five, (Option::Some inc)

    option_six: Option I64 = Option::Applicative::pure 6
    option_left_identity_l: Option I64 = Option::Monad::bind option_six, option_add_two
    option_left_identity_r: Option I64 = option_add_two 6
    option_right_identity_l: Option I64 = Option::Monad::bind (Option::Some 7), option_pure
    option_right_identity_r: Option I64 = Option::Some 7
    option_associativity_inner: Option I64 = Option::Monad::bind (Option::Some 8), option_add_two
    option_associativity_l: Option I64 = Option::Monad::bind option_associativity_inner, option_double
    option_associativity_r: Option I64 = Option::Monad::bind (Option::Some 8), (value -> Option::Monad::bind (option_add_two value), option_double)

    result_value: Result I64, I64 = Result::Ok 3
    result_identity_function: Result (I64 -> I64), I64 = (Result _, I64)::Applicative::pure id_i64
    result_identity_l: Result I64, I64 = (Result _, I64)::Applicative::ap result_identity_function, result_value
    result_identity_r: Result I64, I64 = Result::Ok 3

    result_composition_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc_after_double), (Result::Ok 3)
    result_composition_inner: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok double), (Result::Ok 3)
    result_composition_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), result_composition_inner

    result_homomorphism_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 4)
    result_homomorphism_r: Result I64, I64 = Result::Ok (inc 4)
    result_interchange_l: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok inc), (Result::Ok 5)
    result_interchange_r: Result I64, I64 = (Result _, I64)::Applicative::ap (Result::Ok apply_five), (Result::Ok inc)

    result_left_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 6), result_add_two
    result_left_identity_r: Result I64, I64 = result_add_two 6
    result_right_identity_l: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 7), result_pure
    result_right_identity_r: Result I64, I64 = Result::Ok 7
    result_associativity_l: Result I64, I64 = (Result _, I64)::Monad::bind ((Result _, I64)::Monad::bind (Result::Ok 8), result_add_two), result_double
    result_associativity_r: Result I64, I64 = (Result _, I64)::Monad::bind (Result::Ok 8), (value -> (Result _, I64)::Monad::bind (result_add_two value), result_double)

    first_error: Result (I64 -> I64), I64 = Result::Err 1
    second_error: Result I64, I64 = Result::Err 2
    result_error_precedence: Result I64, I64 = (Result _, I64)::Applicative::ap first_error, second_error

    generic_bind.println!
    option_identity_l.println!
    option_identity_r.println!
    option_composition_l.println!
    option_composition_r.println!
    option_homomorphism_l.println!
    option_homomorphism_r.println!
    option_interchange_l.println!
    option_interchange_r.println!
    option_left_identity_l.println!
    option_left_identity_r.println!
    option_right_identity_l.println!
    option_right_identity_r.println!
    option_associativity_l.println!
    option_associativity_r.println!
    result_identity_l.println!
    result_identity_r.println!
    result_composition_l.println!
    result_composition_r.println!
    result_homomorphism_l.println!
    result_homomorphism_r.println!
    result_interchange_l.println!
    result_interchange_r.println!
    result_left_identity_l.println!
    result_left_identity_r.println!
    result_right_identity_l.println!
    result_right_identity_r.println!
    result_associativity_l.println!
    result_associativity_r.println!
    result_error_precedence.println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Some(5)", "Some(3)", "Some(3)", "Some(7)", "Some(7)", "Some(5)", "Some(5)", "Some(6)",
            "Some(6)", "Some(8)", "Some(8)", "Some(7)", "Some(7)", "Some(20)", "Some(20)", "Ok(3)",
            "Ok(3)", "Ok(7)", "Ok(7)", "Ok(5)", "Ok(5)", "Ok(6)", "Ok(6)", "Ok(8)", "Ok(8)",
            "Ok(7)", "Ok(7)", "Ok(20)", "Ok(20)", "Err(1)",
        ]
    );
}

#[test]
fn test_hkt_standalone_and_receiver_operations() {
    let output = compile_and_run(
        r#"
increment: I64 -> I64
increment = value -> value + 1

some_increment: I64 -> Option I64
some_increment = value -> pure (value + 1)

digits: (I64, I64) -> I64
digits = pair -> pair.0 * 10 + pair.1

values: () -> Vec I64
values = ->
    mut result = Vec::new!
    result.push 1
    result.push 2
    result

effects: () -> Vec (Option I64)
effects = -> fmap some_increment, values!

map_generic: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_generic = mapper, value -> value.fmap mapper

result_value: () -> Result I64, I64
result_value = -> pure 4

main = !->
    (fmap increment, Option::Some 4).println!
    (Option::Some 4).fmap increment .println!
    (map_generic increment, result_value!).println!
    (map_generic increment, values!).println!
    (ap (Option::Some increment), (Option::Some 4)).println!
    (Option::Some increment).ap (Option::Some 4) .println!
    (bind (Option::Some 4), some_increment).println!
    (Option::Some 4).bind some_increment .println!
    (foldl digits, 0, values!).println!
    values!.foldl digits, 0 .println!
    (traverse some_increment, values!).println!
    values!.traverse some_increment .println!
    (traverse_m some_increment, values!).println!
    values!.traverse_m some_increment .println!
    (sequence effects!).println!
    effects!.sequence!.println!
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Some(5)",
            "Some(5)",
            "Ok(5)",
            "[2, 3]",
            "Some(5)",
            "Some(5)",
            "Some(5)",
            "Some(5)",
            "12",
            "12",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
            "Some([2, 3])",
        ]
    );
}

#[test]
fn test_hkt_receiver_custom_carrier_and_owned_mutable_callbacks() {
    let output = compile_and_run(
        r#"
enum Parcel T
    Item T

impl Functor for Parcel
    fmap = mut mapper, value ->
        match value
            Parcel::Item item => Parcel::Item (mapper item)

string_length: String -> I64
string_length = value -> value.len!

main = !->
    parcel = Parcel::Item (String::from "hello")
    match parcel.fmap string_length
        Parcel::Item length => length.println!
    match fmap string_length, Parcel::Item (String::from "world")
        Parcel::Item length => length.println!

    mut total = 0
    mut values = Vec::new!
    values.push 1
    values.push 2
    mapped = values.fmap (value ->
        total = total + value
        total)
    mapped.println!
    total.println!

    mut words = Vec::new!
    words.push (String::from "one")
    words.push (String::from "four")
    words.fmap string_length .println!
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec!["5", "5", "[1, 3]", "3", "[3, 4]"]
    );
}

#[test]
fn test_hkt_receiver_failure_paths_and_constructor_sections() {
    let output = compile_and_run(
        r#"
increment: I64 -> I64
increment = value -> value + 1

success: I64 -> Result I64, String
success = value -> pure value

some: I64 -> Option I64
some = value -> pure value

stop: I64 -> Result I64, String
stop = value ->
    value.println!
    if value == 2
        Result::Err (String::from "stop")
    else
        pure (value + 1)

values: () -> Vec I64
values = ->
    mut items = Vec::new!
    items.push 1
    items.push 2
    items.push 3
    items

bind_generic: F A -> M -> F B where F _: Monad, M: FnMut A, (F B)
bind_generic = value, callback -> value.bind callback

main = !->
    failed: Result I64, String = Result::Err (String::from "error")
    failed.fmap increment .println!
    absent = Option::None
    absent.fmap increment .println!
    absent_bind = Option::None
    absent_bind.bind some .println!
    (bind_generic (success 4), success).println!
    wrapped: Result (I64 -> I64), String = pure increment
    wrapped.ap (success 4) .println!
    (success 4).traverse success .unwrap_or (success 0) .unwrap_or 0 .println!
    (success 4).traverse_m success .unwrap_or (success 0) .unwrap_or 0 .println!
    values!.traverse_m stop .println!
    mut effects = Vec::new!
    effects.push (success 1)
    effects.push (Result::Err (String::from "error"))
    effects.sequence!.println!
"#,
    );
    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "Err(error)",
            "None",
            "None",
            "Ok(4)",
            "Ok(5)",
            "4",
            "4",
            "1",
            "2",
            "Err(stop)",
            "Err(error)",
        ]
    );
}

#[test]
fn test_hkt_pure_requires_constructor_context() {
    compile_should_fail(
        r#"
main = !->
    value = pure 4
"#,
        "ambiguous",
    );
}

#[test]
fn test_hkt_receiver_fmap_consumes_owned_source() {
    compile_should_fail(
        r#"
main = !->
    mut values = Vec::new!
    values.push 1
    mapped = values.fmap (value -> value + 1)
    values.len!.println!
    mapped.println!
"#,
        "borrow of moved value",
    );
}

#[test]
fn test_hkt_foldable_and_traversable() {
    let output = compile_and_run(
        r#"
fold_digits: (I64, I64) -> I64
fold_digits = pair -> pair.0 * 10 + pair.1

option_pure_inc: I64 -> Option I64
option_pure_inc = value -> Option::Some (value + 1)

option_pure_double: I64 -> Option I64
option_pure_double = value -> Option::Some (value * 2)

option_pure_double_then_inc: I64 -> Option I64
option_pure_double_then_inc = value -> Option::Some (value * 2 + 1)

result_pure_inc: I64 -> Result I64, I64
result_pure_inc = value -> Result::Ok (value + 1)

option_to_result: Option A -> Result A, I64
option_to_result = value ->
    match value
        Option::Some item => Result::Ok item
        Option::None => Result::Err 99

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

traverse_option_inc: Option I64 -> Option (Option I64)
traverse_option_inc = value -> Option::Traversable::traverse option_pure_inc, value

traverse_result_inc: Result I64, I64 -> Option (Result I64, I64)
traverse_result_inc = value -> (Result _, I64)::Traversable::traverse option_pure_inc, value

traverse_vec_inc: Vec I64 -> Option (Vec I64)
traverse_vec_inc = value -> Vec::Traversable::traverse option_pure_inc, value

option_stop: I64 -> Option I64
option_stop = value ->
    value.println!
    if value == 2
        Option::None
    else
        Option::Some (value + 1)

result_stop: I64 -> Result I64, I64
result_stop = value ->
    value.println!
    if value == 2
        Result::Err 9
    else
        Result::Ok (value + 1)

encode_option_option: Option (Option I64) -> I64
encode_option_option = outer ->
    match outer
        Option::Some inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Option::None => 0 - 1

encode_option_result: Option (Result I64, I64) -> I64
encode_option_result = outer ->
    match outer
        Option::Some inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Option::None => 0 - 1

encode_option_option_option: Option (Option (Option I64)) -> I64
encode_option_option_option = outer ->
    match outer
        Option::Some middle => encode_option_option middle
        Option::None => 0 - 1

encode_option_option_result: Option (Option (Result I64, I64)) -> I64
encode_option_option_result = outer ->
    match outer
        Option::Some middle => encode_option_result middle
        Option::None => 0 - 1

encode_vec: Vec I64 -> I64
encode_vec = values -> Vec::Foldable::foldl fold_digits, 0, values

encode_option_vec: Option (Vec I64) -> I64
encode_option_vec = outer ->
    match outer
        Option::Some values => encode_vec values
        Option::None => 0 - 1

encode_option_option_vec: Option (Option (Vec I64)) -> I64
encode_option_option_vec = outer ->
    match outer
        Option::Some middle => encode_option_vec middle
        Option::None => 0 - 1

encode_result_option: Result (Option I64), I64 -> I64
encode_result_option = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Option::Some value => value
                Option::None => 0 - 2
        Result::Err error => 0 - 100 - error

encode_result_result: Result (Result I64, I64), I64 -> I64
encode_result_result = outer ->
    match outer
        Result::Ok inner =>
            match inner
                Result::Ok value => value
                Result::Err error => 0 - 100 - error
        Result::Err error => 0 - 200 - error

encode_result_vec: Result (Vec I64), I64 -> I64
encode_result_vec = outer ->
    match outer
        Result::Ok values => encode_vec values
        Result::Err error => 0 - error

main = ->
    fold_option: I64 = Option::Foldable::foldl fold_digits, 0, (Option::Some 4)
    fold_ok_input: Result I64, I64 = Result::Ok 5
    fold_ok: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_ok_input
    fold_error_input: Result I64, I64 = Result::Err 7
    fold_error: I64 = (Result _, I64)::Foldable::foldl fold_digits, 0, fold_error_input
    fold_vec: I64 = Vec::Foldable::foldl fold_digits, 0, (make_values!)
    mut calls = 0
    captured_step = pair ->
        calls = calls + 1
        pair.0 + pair.1
    captured_fold: I64 = Vec::Foldable::foldl captured_step, 0, (make_values!)
    fold_option.println!
    fold_ok.println!
    fold_error.println!
    fold_vec.println!
    captured_fold.println!
    calls.println!
    option_identity_l: Option (Option I64) = Option::Traversable::traverse option_pure_inc, (Option::Some 1)
    option_identity_r: Option (Option I64) = Option::Applicative::pure (Option::Some 2)
    encode_option_option option_identity_l .println!
    encode_option_option option_identity_r .println!
    result_identity_input: Result I64, I64 = Result::Ok 1
    result_identity_l: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_identity_input
    result_identity_r: Option (Result I64, I64) = Option::Applicative::pure (Result::Ok 2)
    encode_option_result result_identity_l .println!
    encode_option_result result_identity_r .println!
    vec_identity_l: Option (Vec I64) = Vec::Traversable::traverse (value -> Option::Some value), (make_values!)
    vec_identity_r: Option (Vec I64) = Option::Applicative::pure (make_values!)
    encode_option_vec vec_identity_l .println!
    encode_option_vec vec_identity_r .println!

    option_composition_inner: Option (Option I64) = Option::Traversable::traverse option_pure_double_then_inc, (Option::Some 1)
    option_composition_l: Option (Option (Option I64)) = Option::Some option_composition_inner
    option_first: Option (Option I64) = Option::Traversable::traverse option_pure_double, (Option::Some 1)
    option_composition_r: Option (Option (Option I64)) = Option::Functor::fmap traverse_option_inc, option_first

    result_composition_input_l: Result I64, I64 = Result::Ok 1
    result_composition_inner: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double_then_inc, result_composition_input_l
    result_composition_l: Option (Option (Result I64, I64)) = Option::Some result_composition_inner
    result_composition_input_r: Result I64, I64 = Result::Ok 1
    result_first: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_double, result_composition_input_r
    result_composition_r: Option (Option (Result I64, I64)) = Option::Functor::fmap traverse_result_inc, result_first

    vec_composition_inner: Option (Vec I64) = Vec::Traversable::traverse option_pure_double_then_inc, (make_values!)
    vec_composition_l: Option (Option (Vec I64)) = Option::Some vec_composition_inner
    vec_first: Option (Vec I64) = Vec::Traversable::traverse option_pure_double, (make_values!)
    vec_composition_r: Option (Option (Vec I64)) = Option::Functor::fmap traverse_vec_inc, vec_first
    encode_option_option_option option_composition_l .println!
    encode_option_option_option option_composition_r .println!
    encode_option_option_result result_composition_l .println!
    encode_option_option_result result_composition_r .println!
    encode_option_option_vec vec_composition_l .println!
    encode_option_option_vec vec_composition_r .println!

    option_natural_input_l: Option I64 = Option::Some 1
    option_natural_effect: Option (Option I64) = Option::Traversable::traverse option_pure_inc, option_natural_input_l
    option_naturality_l: Result (Option I64), I64 = option_to_result option_natural_effect
    option_natural_input_r: Option I64 = Option::Some 1
    option_naturality_r: Result (Option I64), I64 = Option::Traversable::traverse result_pure_inc, option_natural_input_r

    result_natural_input_l: Result I64, I64 = Result::Ok 1
    result_natural_effect: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse option_pure_inc, result_natural_input_l
    result_naturality_l: Result (Result I64, I64), I64 = option_to_result result_natural_effect
    result_natural_input_r: Result I64, I64 = Result::Ok 1
    result_naturality_r: Result (Result I64, I64), I64 = (Result _, I64)::Traversable::traverse result_pure_inc, result_natural_input_r

    vec_natural_effect: Option (Vec I64) = Vec::Traversable::traverse option_pure_inc, (make_values!)
    vec_naturality_l: Result (Vec I64), I64 = option_to_result vec_natural_effect
    vec_naturality_r: Result (Vec I64), I64 = Vec::Traversable::traverse result_pure_inc, (make_values!)
    encode_result_option option_naturality_l .println!
    encode_result_option option_naturality_r .println!
    encode_result_result result_naturality_l .println!
    encode_result_result result_naturality_r .println!
    encode_result_vec vec_naturality_l .println!
    encode_result_vec vec_naturality_r .println!

    "app-option".println!
    vec_app_option: Option (Vec I64) = Vec::Traversable::traverse option_stop, (make_values!)
    encode_option_vec vec_app_option .println!
    "monad-option".println!
    vec_monad_option: Option (Vec I64) = Vec::Traversable::traverse_m option_stop, (make_values!)
    encode_option_vec vec_monad_option .println!
    "app-result".println!
    vec_app_result: Result (Vec I64), I64 = Vec::Traversable::traverse result_stop, (make_values!)
    encode_result_vec vec_app_result .println!
    "monad-result".println!
    vec_monad_result: Result (Vec I64), I64 = Vec::Traversable::traverse_m result_stop, (make_values!)
    encode_result_vec vec_monad_result .println!

    "container-short".println!
    none_input: Option I64 = Option::None
    none_traverse: Option (Option I64) = Option::Traversable::traverse_m option_stop, none_input
    failed_input: Result I64, I64 = Result::Err 7
    failed_traverse: Option (Result I64, I64) = (Result _, I64)::Traversable::traverse_m option_stop, failed_input
    encode_option_option none_traverse .println!
    encode_option_result failed_traverse .println!
    0
"#,
    );

    assert_eq!(
        output.lines().collect::<Vec<_>>(),
        vec![
            "4",
            "5",
            "0",
            "123",
            "6",
            "3",
            "2",
            "2",
            "2",
            "2",
            "123",
            "123",
            "3",
            "3",
            "3",
            "3",
            "357",
            "357",
            "2",
            "2",
            "2",
            "2",
            "234",
            "234",
            "app-option",
            "1",
            "2",
            "3",
            "-1",
            "monad-option",
            "1",
            "2",
            "-1",
            "app-result",
            "1",
            "2",
            "3",
            "-9",
            "monad-result",
            "1",
            "2",
            "-9",
            "container-short",
            "-2",
            "-107",
        ]
    );
}

#[test]
fn test_std_traversable_sequence() {
    let output = compile_and_run(
        r#"
main = ->
    mut effects: Vec (Option I64) = Vec::new!
    effects.push (Option::Some 4)
    effects.push (Option::Some 5)
    sequenced: Option (Vec I64) = sequence effects
    match sequenced
        Option::Some values =>
            values.len!.println!
            match (values.get 0)
                Option::Some value => *value .println!
                Option::None => 0.println!
        Option::None => 0.println!

    mut failed_effects: Vec (Option I64) = Vec::new!
    failed_effects.push (Option::Some 1)
    failed_effects.push Option::None
    failed_effects.push (Option::Some 3)
    failed: Option (Vec I64) = sequence failed_effects
    match failed
        Option::Some _ => 1.println!
        Option::None => -1 .println!
    0
"#,
    );

    assert_eq!(output.lines().collect::<Vec<_>>(), vec!["2", "4", "-1"]);
}
