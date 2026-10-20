use rock_lib::crate_system::TargetRole;
use rock_lib::macro_expansion::host::ProcMacroSdk;
use rock_lib::macro_expansion::proc_macro::*;
use rock_lib::{Config, SourceProvider};

pub fn verify(config: &Config, golden: &ProcMacroRequest) {
    let mut host = config.clone();
    host.current_crate_name = Some("generated_provider".into());
    host.target_role = TargetRole::ProcMacro;
    host.output_dir = config.output_dir.parent().unwrap().join("generated-host");
    host.source_providers = vec![
        SourceProvider::Virtual { path: host.entry_file.clone(), text: r#"
> rock_macro::MacroResult
< mod tools
struct MacroContext
identity = -> 17
run_worker = -> 29
__rock_entry_0 = -> 41
mod __rock_proc_wrapper
    __rock_dispatch = -> 31
    __rock_boot = -> 37
macro private = context, input -> MacroResult::Expand input
< macro identity = context, input -> MacroResult::Expand input
macro publish
    =>
        < macro generated = context, input -> MacroResult::Expand input
%publish
"#.into() },
        SourceProvider::Virtual { path: host.entry_file.parent().unwrap().join("tools.rk"), text:
            "> rock_macro::MacroResult\nwrap = input -> MacroResult::Expand input\n< macro identity = context, input -> wrap input\n".into() },
    ];
    let sdk = ProcMacroSdk { dependency: "rock_macro".into() };
    let output = rock_lib::compile_proc_macro(&host, &sdk).unwrap_or_else(|errors| panic!("generated host: {errors:?}"));
    assert!(output.products.is_none(), "proc-only packaging is separate from ordinary runtime products");
    assert_eq!(output.proc_macro_exports.iter().map(|export| export.name.as_str()).collect::<Vec<_>>(), ["generated", "identity", "tools::identity"]);
    let executable = host.output_dir.join("main");
    for export in &output.proc_macro_exports {
        let mut request = golden.clone(); request.macro_name = export.name.clone(); request.macro_identity = export.identity.clone();
        let output = super::run_client_with_input(&executable, &encode_request(&request).unwrap());
        assert!(output.status.success(), "{}: {output:?}", export.identity);
        assert!(output.stderr.is_empty(), "no handwritten main or table and no incidental logging");
        let response = decode_response(&output.stdout).unwrap(); response.validate_for(&request).unwrap();
        assert_eq!(response, ProcMacroResponse::expanded(request.input));
    }
    for (identity, kind) in [
        ("generated_provider::missing", ProcMacroKind::FunctionLike),
        ("generated_provider::private", ProcMacroKind::FunctionLike),
        ("generated_provider::run_worker", ProcMacroKind::FunctionLike),
        ("generated_provider::identity", ProcMacroKind::AttributeLike),
    ] {
        let mut request = golden.clone(); request.macro_identity = identity.into(); request.kind = kind;
        let output = super::run_client_with_input(&executable, &encode_request(&request).unwrap());
        assert!(output.status.success(), "unknown export is a structured macro error: {output:?}");
        let response = decode_response(&output.stdout).unwrap(); response.validate_for(&request).unwrap();
        assert!(response.output.is_none());
        assert_eq!(response.diagnostics[0].message, "unknown procedural macro identity or kind");
    }
    for source in [
        "> rock_macro::MacroResult\n< macro bad = input -> MacroResult::Expand input\n",
        "< macro bad = context, input -> 42\n",
        "< macro bad = context, input -> &input\n",
    ] {
        let mut bad = host.clone(); bad.source_providers = vec![SourceProvider::Virtual { path: bad.entry_file.clone(), text: source.into() }];
        let errors = rock_lib::compile_proc_macro(&bad, &sdk).unwrap_err();
        assert!(!errors.0.is_empty());
        assert!(errors.0.iter().any(|diagnostic| matches!(diagnostic.code,
            Some(rock_lib::diagnostic::DiagnosticCode::Type | rock_lib::diagnostic::DiagnosticCode::Resolve))),
            "ordinary signature checking required: {errors:?}");
        assert!(!format!("{errors:?}").contains("not implemented"), "normal signature/type errors required: {errors:?}");
        assert!(errors.0.iter().any(|diagnostic| diagnostic.source.is_some()), "retained author/wrapper sources: {errors:?}");
    }
    let errors = rock_lib::compile_proc_macro(&host, &ProcMacroSdk { dependency: "missing".into() }).unwrap_err();
    assert!(format!("{errors:?}").contains("explicitly supplied SDK artifact 'missing'"), "{errors:?}");
    let errors = rock_lib::compile_proc_macro(&host, &ProcMacroSdk { dependency: "std".into() }).unwrap_err();
    assert!(format!("{errors:?}").contains("has no public contract export 'MacroContext'"), "{errors:?}");
    let errors = rock_lib::compile_proc_macro(config, &sdk).unwrap_err();
    assert!(format!("{errors:?}").contains("requires the ProcMacro target role"), "{errors:?}");

    let mut renamed = host.clone(); renamed.output_dir = host.output_dir.with_file_name("renamed-host");
    let sources = host.output_dir.with_file_name("renamed-sdk-source");
    std::fs::create_dir_all(&sources).unwrap();
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for entry in std::fs::read_dir(workspace.join("rock_macro/src")).unwrap() {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|extension| extension == "rk") {
            let text = std::fs::read_to_string(entry.path()).unwrap().replace("rock_macro::", "renamed_sdk::");
            std::fs::write(sources.join(entry.file_name()), text).unwrap();
        }
    }
    let dependencies = config.extern_artifacts.iter().filter(|(name, _)| name == "std").cloned().collect();
    let artifact = super::build_library(sources.join("lib.rk"), "renamed_sdk", &sources.join("build"), dependencies);
    for (name, path) in &mut renamed.extern_artifacts {
        if name == "rock_macro" { *name = "renamed_sdk".into(); *path = artifact.clone(); }
    }
    for provider in &mut renamed.source_providers {
        if let SourceProvider::Virtual { text, .. } = provider { *text = text.replace("rock_macro::", "renamed_sdk::"); }
    }
    rock_lib::compile_proc_macro(&renamed, &ProcMacroSdk { dependency: "renamed_sdk".into() })
        .unwrap_or_else(|errors| panic!("explicit SDK with a different package name: {errors:?}"));
    let mut request = golden.clone(); request.macro_identity = "generated_provider::identity".into();
    let output = super::run_client_with_input(&renamed.output_dir.join("main"), &encode_request(&request).unwrap());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(decode_response(&output.stdout).unwrap(), ProcMacroResponse::expanded(request.input));
    eprintln!("SDK acceptance: 14 generated-host dispatch/signature/dependency controls passed");
}
