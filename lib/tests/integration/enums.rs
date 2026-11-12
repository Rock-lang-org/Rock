use std::process::Command;
use std::sync::atomic::Ordering;

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

#[test]
fn test_explicit_variant_import_shadows_prelude_struct() {
    let output = compile_and_run(
        r#"
enum Event
    String

> Event::String

describe: Event -> I64
describe = event -> match event
    String => 7

main = !->
    event: Event = String
    (describe event).println!
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_variant_reexport_introduces_its_short_name_over_prelude() {
    let output = compile_and_run(
        r#"
enum Boxed
    Some I64

< Boxed::Some

main = !->
    boxed: Boxed = Some 7
    match boxed
        Some value => value.println!
"#,
    );
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_invalid_variant_import_is_checked_before_alias_overwrite() {
    compile_should_fail(
        r#"
enum Light
    Off
enum Dark
    Dim

> Light::Dim
> Dark::Dim

main = !->
    ()
"#,
        "unresolved import 'Light::Dim'",
    );
}

#[test]
fn test_explicit_variant_import_conflicts_with_local_struct() {
    compile_should_fail(
        r#"
struct Some
    < value: I64

> std::control::option::Option::Some

main = !->
    ()
"#,
        "conflicting explicit declarations or imports for 'Some'",
    );
}

#[test]
fn test_explicit_variant_and_struct_imports_conflict_in_either_order() {
    for imports in [
        "> Event::String\n> std::string::owned::String",
        "> std::string::owned::String\n> Event::String",
    ] {
        compile_should_fail(
            &format!("enum Event\n    String\n\n{imports}\n\nmain = !->\n    ()\n"),
            "conflicting explicit declarations or imports for 'String'",
        );
    }
}

#[test]
fn test_variant_reexports_through_source_modules() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(&dir).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    std::fs::write(dir.join("lamps.rk"), "enum Light\n    Off\n    On\n\n< Light::Off\n").unwrap();
    std::fs::write(dir.join("facade.rk"), "> lamps::Off\n< Off\n").unwrap();
    std::fs::write(
        dir.join("test.rk"),
        r#"
mod facade
mod lamps
> facade::Off

main = !->
    light = Off
    match light
        Off => 7.println!
        _ => 0.println!
"#,
    ).unwrap();
    let config = test_config(dir.join("test.rk"), dir.clone());
    rock_lib::compile(&config).expect("variant re-exports should resolve through source facades");
    let output = Command::new(dir.join("test")).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
}

#[test]
fn test_variant_reexports_survive_deleted_source_artifacts() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    std::fs::create_dir_all(dir.join("producer")).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());
    let entry = dir.join("producer/lib.rk");
    std::fs::write(&entry, "mod signals\n< signals::Off\n").unwrap();
    std::fs::write(dir.join("producer/signals.rk"), "enum Light\n    Off\n    On\n\n< Light::Off\n").unwrap();
    let mut config = test_config(entry, dir.join("producer-build"));
    config.current_crate_name = Some("lamps".to_string());
    config.no_link = true;
    let products = rock_lib::compile_with_products(&config).expect("producer should compile")
        .products.expect("producer products");
    let artifact = dir.join("lamps.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    std::fs::remove_dir_all(dir.join("producer")).unwrap();

    let facade_entry = dir.join("facade.rk");
    std::fs::write(&facade_entry, "> lamps::Off\n< Off\n").unwrap();
    let mut facade_config = test_config(facade_entry.clone(), dir.join("facade-build"));
    facade_config.current_crate_name = Some("facade".to_string());
    facade_config.no_link = true;
    facade_config.extern_artifacts.push(("lamps".to_string(), artifact.clone()));
    let facade_products = rock_lib::compile_with_products(&facade_config)
        .expect("facade should re-export a dependency variant").products.expect("facade products");
    let facade_artifact = dir.join("facade.rkca");
    facade_products.write_artifact_to_path(&facade_artifact).unwrap();
    std::fs::remove_file(facade_entry).unwrap();

    for (name, import) in [
        ("explicit", "> lamps::Off"),
        ("glob", "> lamps::*"),
        ("nested", "> lamps::signals::Off"),
        ("transitive", "> facade::Off"),
    ] {
        let entry = dir.join(format!("{name}.rk"));
        std::fs::write(&entry, format!(
            "{import}\n\nmain = !->\n    light = Off\n    match light\n        Off => 7.println!\n        _ => 0.println!\n"
        )).unwrap();
        let mut consumer = test_config(entry, dir.clone());
        consumer.extern_artifacts.push(("lamps".to_string(), artifact.clone()));
        consumer.extern_artifacts.push(("facade".to_string(), facade_artifact.clone()));
        rock_lib::compile(&consumer).expect("source-free variant export should import");
        let output = Command::new(dir.join(name)).output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
    }

    let qualified_entry = dir.join("qualified.rk");
    std::fs::write(&qualified_entry, "main = !->\n    light = facade::Off\n    match light\n        facade::Off => 7.println!\n        _ => 0.println!\n").unwrap();
    let mut qualified = test_config(qualified_entry, dir.clone());
    qualified.extern_artifacts.push(("lamps".to_string(), artifact));
    qualified.extern_artifacts.push(("facade".to_string(), facade_artifact));
    rock_lib::compile(&qualified).expect("variant exports should also resolve by qualified path");
    let output = Command::new(dir.join("qualified")).output().unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "7");
}
