use crate::support::*;

#[test]
fn test_enums() {
    let output = compile_example("enums");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "enum test");
    assert_eq!(lines[1], "42");
}

#[test]
fn test_enum_match() {
    let output = compile_example("enum_match");
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_enum_string_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

main = ->
    token = Token::Text "x"
    match token
        Token::Text "x" => 1
        _ => 0
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_impl_method_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper
    < token: Token

impl Wrapper
    @check = ->
        match @token
            Token::Text "x" => 1
            _ => 0

main = ->
    wrapper = Wrapper
        token: Token::Text "x"
    wrapper.check!
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_trait_default_enum_payload_pattern_reports_diagnostic() {
    compile_should_fail(
        r#"
enum Token
    Text &Str
    Other

struct Wrapper

trait ChecksToken
    @check: Token -> I64
    @check = token ->
        match token
            Token::Text "x" => 1
            _ => 0

impl ChecksToken for Wrapper

main = ->
    wrapper = Wrapper
    wrapper.check (Token::Text "x")
"#,
        "unsupported enum payload literal pattern",
    );
}

#[test]
fn test_guarded_noncopy_enum_payload_binding_reports_diagnostic() {
    compile_should_fail(
        r#"
struct Boxed
    < value: I64

enum MaybeBoxed
    Some Boxed
    None

main = ->
    item = MaybeBoxed::Some (Boxed
        value: 1)
    match item
        MaybeBoxed::Some value if 1 == 1 => value.value
        _ => 0
"#,
        "unsupported guarded non-copy enum payload binding",
    );
}

#[test]
fn test_enum_pattern_matching() {
    let output = compile_and_run(
        r#"
enum Option
    Some I64
    None

unwrap_or = x, default ->
    match x
        Option::Some val => val
        Option::None => default

main = ->
    a = Option::Some 42
    b = Option::None
    r1 = unwrap_or a, 0
    r2 = unwrap_or b, 99
    r1.println!
    r2.println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "42");
    assert_eq!(lines[1], "99");
}

#[test]
fn test_enum_multiple_variants() {
    let output = compile_and_run(
        r#"
enum Color
    Red
    Green
    Blue

to_num = c ->
    match c
        Color::Red => 1
        Color::Green => 2
        Color::Blue => 3

main = ->
    r = Color::Red
    g = Color::Green
    b = Color::Blue
    to_num r .println!
    to_num g .println!
    to_num b .println!
    0
"#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "2");
    assert_eq!(lines[2], "3");
}

#[test]
fn test_unqualified_same_named_enum_variant_pattern_uses_scrutinee_owner() {
    let output = compile_and_run(
        r#"
enum AlphaChoice
    Hit Bool
    Miss

enum DecoyChoiceA
    Hit Bool
    Miss

enum DecoyChoiceB
    Hit Bool
    Miss

enum DecoyChoiceC
    Hit Bool
    Miss

enum DecoyChoiceD
    Hit Bool
    Miss

enum DecoyChoiceE
    Hit Bool
    Miss

enum DecoyChoiceF
    Hit Bool
    Miss

enum DecoyChoiceG
    Hit Bool
    Miss

enum DecoyChoiceH
    Hit Bool
    Miss

enum DecoyChoiceI
    Hit Bool
    Miss

enum DecoyChoiceJ
    Hit Bool
    Miss

enum BetaChoice
    Hit I64
    Miss

score: BetaChoice -> I64
score = choice ->
    match choice
        Hit value => value + 1
        Miss => 0

main = ->
    score (BetaChoice::Hit 41) .println!
    0
"#,
    );
    assert_eq!(output.trim(), "42");
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_known_non_enum_scrutinee() {
    compile_should_fail(
        r#"
enum Choice
    Hit

main = ->
    match 1
        Hit => 1
        _ => 0
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_qualified_enum_variant_pattern_rejects_different_enum_with_same_variant_name() {
    compile_should_fail(
        r#"
enum A
    Hit I64
    Miss

enum B
    Hit I64
    Miss

score: A -> I64
score = value ->
    match value
        B::Hit n => n
        A::Hit n => n + 1
        A::Miss => 0

main = ->
    score (A::Hit 41)
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_unqualified_enum_variant_pattern_rejects_variant_missing_from_known_enum() {
    compile_should_fail(
        r#"
enum A
    Hit

enum B
    OnlyOnB

score: A -> I64
score = value ->
    match value
        OnlyOnB => 1
        Hit => 0

main = ->
    score A::Hit
"#,
        "pattern type mismatch",
    );
}

#[test]
fn test_bare_variant_constructor_names_the_qualified_path() {
    compile_should_fail(
        r#"
enum Shape
    Dot
    Line I64

wrap: I64 -> Shape
wrap = x -> Line x

main = !->
    wrap 1
    ()
"#,
        "unknown type 'Line'; enum variants are constructed with their enum path, such as `Shape::Line`",
    );
}

#[test]
fn test_prelude_variants_construct_and_match_without_enum_path() {
    let output = compile_and_run(
        r#"
wrap: I64 -> Option I64
wrap = x -> Some x

describe: Option I64 -> I64
describe = o -> match o
    Some v => v
    None => 0

main = !->
    (wrap 1).println!
    (describe (Some 4)).println!
    (describe None).println!
    r: Result I64, I64 = Err 3
    r.println!
"#,
    );
    assert_eq!(output.trim(), "Some(1)\n4\n0\nErr(3)");
}

#[test]
fn test_imported_variants_of_a_generic_enum() {
    let output = compile_and_run(
        r#"
enum Pair T
    Both T, T
    Neither

> Pair::Both
> Pair::Neither

sum: Pair I64 -> I64
sum = p -> match p
    Both a, b => a + b
    Neither => 0

main = !->
    (sum (Both 2, 3)).println!
    (sum Neither).println!
"#,
    );
    assert_eq!(output.trim(), "5\n0");
}

#[test]
fn test_imported_variant_as_payload_of_imported_variant() {
    let output = compile_and_run(
        r#"
enum Light
    On
    Off

> Light::Off

describe: Option Light -> &Str
describe = slot -> match slot
    Some Off => "off"
    Some Light::On => "on"
    None => "empty"

main = !->
    (describe (Some Off)).println!
    (describe (Some Light::On)).println!
    (describe None).println!
"#,
    );
    assert_eq!(output.trim(), "off\non\nempty");
}

#[test]
fn test_import_of_unknown_variant_is_reported() {
    compile_should_fail(
        r#"
enum Light
    On
    Off

> Light::Dim

main = !->
    ()
"#,
        "unresolved import 'Light::Dim'",
    );
}

#[test]
fn test_struct_shadows_prelude_variant_of_same_name() {
    let output = compile_and_run(
        r#"
struct Ok
    < v: I64

main = !->
    o = Ok
        v: 5
    o.v.println!
"#,
    );
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_bare_payloadless_variant_is_reported_instead_of_crashing() {
    compile_should_fail(
        r#"
enum Light
    On
    Off

main = !->
    light: Light = Off
    ()
"#,
        "unknown type 'Off'; enum variants are constructed with their enum path, such as `Light::Off`",
    );
}

#[test]
fn test_unknown_struct_literal_is_reported() {
    compile_should_fail(
        r#"
main = !->
    point = Pointt
        x: 1
    ()
"#,
        "unknown type 'Pointt'",
    );
}
