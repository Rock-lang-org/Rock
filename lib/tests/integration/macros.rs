use crate::support::*;
use bincode::Options;
use serde::Serialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[test]
fn test_declarative_macro_expands_user_program() {
    let (_stdout, success) = compile_and_run_with_status(
        r#"macro make_main
    =>
        main = -> 0
%make_main"#,
    );

    assert!(success);
}

#[test]
fn test_declarative_macro_can_produce_call_argument_holes() {
    let output = compile_and_run(
        r#"macro make_main
    =>
        combine = a, b, c -> a * 100 + b * 10 + c
        main = ->
            section = combine 1, _, 3
            section 2 .println!
            0
%make_main"#,
    );

    assert_eq!(output.trim(), "123");
}

#[test]
fn test_declarative_macros_expand_in_expression_position() {
    let output = compile_and_run(
        r#"macro format
    $first:expr $( , $rest:expr )* =>
        ($first).show! $( .concat (($rest).show!) )*

macro println
    $first:expr $( , $rest:expr )* =>
        (%format $first $( , $rest )*).println!

add = a, b -> a + b

main = !->
    name = String::from "rock"
    greeting = %format "Hello ", name, ", you are ", 42
    greeting.println!
    name.println!
    %println "sum: ", (add 1, 2), " bool: ", true
    %println "alone"
    joined = (%format "a", 1).concat (%format "b", 2)
    joined.println!
    for x in [1, 2]
        %println "x = ", x
"#,
    );

    assert_eq!(
        output,
        "Hello rock, you are 42\nrock\nsum: 3 bool: true\nalone\na1b2\nx = 1\nx = 2\n"
    );
}

#[test]
fn test_std_format_and_println_macros_come_from_the_prelude() {
    let output = compile_and_run(
        r#"add = a, b -> a + b

main = !->
    name = String::from "rock"
    greeting = %format "Hello ", name, ", you are ", 42
    greeting.println!
    name.println!
    %println "sum: ", (add 1, 2), " bool: ", true, " char: ", 'c'
    %println "alone"
"#,
    );

    assert_eq!(
        output,
        "Hello rock, you are 42\nrock\nsum: 3 bool: true char: c\nalone\n"
    );
}

#[test]
fn test_local_macro_shadows_std_prelude_macro() {
    let output = compile_and_run(
        r#"macro format
    $x:expr =>
        ("local ").show! .concat (($x).show!)

main = !->
    (%format 1).println!
"#,
    );

    assert_eq!(output, "local 1\n");
}

#[test]
fn test_exported_macros_are_imported_across_crates() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(dir.join("mylib")).unwrap();
    let _cleanup = TestDirCleanup(dir.clone());

    let library_entry = dir.join("mylib").join("lib.rk");
    fs::write(&library_entry, "< mod text\n").unwrap();
    fs::write(
        dir.join("mylib").join("text.rk"),
        r#"< macro join
    $first:expr $( , $rest:expr )* =>
        ($first).show! $( .concat (($rest).show!) )*

macro secret
    =>
        1
"#,
    )
    .unwrap();
    let mut library_config = test_config(library_entry, dir.join("mylib_build"));
    library_config.current_crate_name = Some("mylib".to_string());
    library_config.no_link = true;
    let products = rock_lib::compile_with_products(&library_config)
        .expect("library with an exported macro should compile")
        .products
        .expect("library products");
    let exported = products
        .interface
        .macros
        .iter()
        .map(|export| export.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(exported, vec!["text::join"]);
    let artifact = dir.join("mylib.rkca");
    products.write_artifact_to_path(&artifact).unwrap();

    let consumer = |name: &str, source: &str| {
        let entry = dir.join(format!("{name}.rk"));
        fs::write(&entry, source).unwrap();
        let mut config = test_config(entry, dir.clone());
        config
            .extern_artifacts
            .push(("mylib".to_string(), artifact.clone()));
        rock_lib::compile(&config).map(|_| {
            let output = Command::new(dir.join(name)).output().unwrap();
            String::from_utf8_lossy(&output.stdout).to_string()
        })
    };
    let main_body = "main = !->\n    (%join \"a\", 1, true).println!\n";

    let imported = consumer("imported", &format!("> mylib::text::join\n{main_body}"))
        .expect("an imported macro should expand");
    assert_eq!(imported, "a1true\n");

    let missing = consumer("missing", main_body)
        .expect_err("a dependency macro is not visible without an import");
    assert!(missing.contains("Unknown macro '%join'"), "{missing:?}");

    let private = consumer(
        "private",
        "> mylib::text::secret\nmain = !->\n    x = %secret\n",
    )
    .expect_err("a macro declared without `<` is not exported");
    assert!(private.contains("Unknown macro '%secret'"), "{private:?}");
}

fn proc_macro_host_with_response(
    response: rock_lib::macro_expansion::proc_macro::ProcMacroResponse,
) -> (
    rock_lib::macro_expansion::proc_macro::ProcMacroArtifact,
    PathBuf,
) {
    let response = rock_lib::macro_expansion::proc_macro::encode_response(&response).unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "rock-integration-proc-macro-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&temp_dir).unwrap();
    let host = temp_dir.join("proc_macro_host");
    fs::write(host.with_extension("response"), response).unwrap();
    fs::write(&host, "#!/bin/sh\ncat >/dev/null\ncat \"$0.response\"\n").unwrap();
    let mut permissions = fs::metadata(&host).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&host, permissions).unwrap();

    let artifact = rock_lib::macro_expansion::proc_macro::ProcMacroArtifact {
        artifact_format_version: rock_lib::products::PRODUCT_ARTIFACT_FORMAT_VERSION,
        crate_identity: "integration_macros".to_string(),
        protocol_version: rock_lib::macro_expansion::proc_macro::PROC_MACRO_PROTOCOL_VERSION,
        host_triple: "x86_64-unknown-linux-gnu".to_string(),
        executable: host,
        capabilities: vec![rock_lib::macro_expansion::proc_macro::ProcMacroCapability::Stdio],
        exports: vec![rock_lib::macro_expansion::proc_macro::ProcMacroExport {
            name: "make_main".to_string(),
            identity: "integration_macros::make_main".to_string(),
            kind: rock_lib::macro_expansion::proc_macro::ProcMacroKind::FunctionLike,
            input_shape: rock_lib::macro_expansion::proc_macro::ProcMacroInputShape::TokenStream,
        }],
    };

    (artifact, temp_dir)
}

#[derive(Serialize)]
struct ProcMacroToken {
    token_type: ProcMacroTokenType,
    span: ProcMacroSpan,
}

#[derive(Serialize)]
struct ProcMacroSpan {
    file_path: PathBuf,
    start: usize,
    end: usize,
}

#[derive(Serialize)]
#[allow(dead_code)]
enum ProcMacroTokenType {
    Ident(String),
    Type(String),
    Number(String),
    Float(String),
    Operator(String),
    Comment(String),
    StuckOperator(String),
    NativeOperator(String),
    Keyword(String),
    MacroVar(String),
    MacroInvoc(String),
    MacroRepeatOpen,
    MacroRepeatClose,
    Equal,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Char(String),
    String(String),
    Arrow,
    CurriedArrow,
    TypeLambda,
    FatArrow,
    Coma,
    Colon,
    DoubleColon,
    Dot,
    DoubleDot,
    SpacedDot,
    Caret,
    Tilde,
    Arobase,
    Interogation,
    Ampersand,
    Indent(u8),
    Underscore,
    Eol,
    Eof,
}

fn proc_macro_token(token_type: ProcMacroTokenType) -> ProcMacroToken {
    ProcMacroToken {
        token_type,
        span: ProcMacroSpan {
            file_path: PathBuf::from("<proc-macro-output>"),
            start: 0,
            end: 0,
        },
    }
}

fn encode_generated_main_tokens() -> Vec<u8> {
    let generated_tokens = vec![
        proc_macro_token(ProcMacroTokenType::Indent(0)),
        proc_macro_token(ProcMacroTokenType::Ident("main".to_string())),
        proc_macro_token(ProcMacroTokenType::Equal),
        proc_macro_token(ProcMacroTokenType::Arrow),
        proc_macro_token(ProcMacroTokenType::Number("0".to_string())),
        proc_macro_token(ProcMacroTokenType::Eol),
        proc_macro_token(ProcMacroTokenType::Indent(0)),
        proc_macro_token(ProcMacroTokenType::Eol),
        proc_macro_token(ProcMacroTokenType::Eof),
    ];

    bincode::DefaultOptions::new()
        .with_limit(rock_lib::macro_expansion::proc_macro::MAX_PROC_MACRO_MESSAGE_BYTES)
        .serialize(&generated_tokens)
        .unwrap()
}

#[test]
fn test_function_like_proc_macro_expands_through_public_macro_api() {
    let config = rock_lib::Config {
        entry_file: PathBuf::new(),
        output_dir: PathBuf::new(),
        debug_print: Vec::new(),
        meta_files: Vec::new(),
        extern_artifacts: Vec::new(),
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: false,
        emit_object: None,
        no_prelude: false,
        no_std: false,
        sysroot: None,
    };
    let response = rock_lib::macro_expansion::proc_macro::ProcMacroResponse::Expand {
        output: encode_generated_main_tokens(),
    };
    let (artifact, temp_dir) = proc_macro_host_with_response(response);
    let expanded = (|| {
        let input_program = rock_lib::parser::parse_string("%make_main", &config).unwrap();
        let context = rock_lib::macro_expansion::MacroExpansionContext::new(&config)
            .with_proc_macro_artifact(artifact);

        rock_lib::macro_expansion::expand_macros_with_context(input_program, &context)
    })();

    let _ = fs::remove_dir_all(temp_dir);
    let expanded = expanded.unwrap();

    assert!(expanded.module.top_level_from_ident("main").is_some());
}

#[test]
fn test_same_crate_macro_import_ignores_module_load_order() {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = test_temp_dir(id);
    fs::create_dir_all(&dir).unwrap();

    // `user` loads before `defs`, and the macro is imported from inside it.
    fs::write(
        dir.join("user.rk"),
        "> defs::twice\n\n< doubled = x -> %twice x\n",
    )
    .unwrap();
    fs::write(
        dir.join("defs.rk"),
        "< macro twice\n    $x:expr =>\n        ($x) + ($x)\n",
    )
    .unwrap();
    fs::write(
        dir.join("test.rk"),
        "mod user\nmod defs\n> user::doubled\n\nmain = ->\n    (doubled 21).println!\n    0\n",
    )
    .unwrap();

    let mut config = test_config(dir.join("test.rk"), dir.clone());
    config.no_prelude = true;
    rock_lib::compile(&config).expect("same-crate macro import must resolve");
    let output = Command::new(dir.join("test")).output().unwrap();
    let _ = fs::remove_dir_all(&dir);

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout), "42\n");
}

#[test]
fn test_std_io_error_show_formats_each_variant() {
    let output = compile_and_run(
        r#"> std::io::IoError

main = !->
    (IoError::Os 2).show!.println!
    IoError::InvalidPath .show!.println!
    IoError::WriteZero .show!.println!
"#,
    );

    assert_eq!(output.lines().next(), Some("IoError(Os(2))"), "{output}");
    assert_eq!(output.lines().nth(1), Some("IoError(InvalidPath)"), "{output}");
    assert!(output.contains("IoError(WriteZero)"), "{output}");
    assert_eq!(output.lines().count(), 3, "{output}");
}
