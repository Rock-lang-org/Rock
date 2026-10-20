use std::{fs, path::Path, process::Command};

use crate::rockc::{ensure_dev_binary_built, resolve_rockc_path};

use super::support::{run_command_output, temp_test_dir, write_package_with_options};

fn rock(root: &Path, arguments: &[&str]) -> std::process::Output {
    ensure_dev_binary_built("rock").unwrap();
    run_command_output(Command::new(rock_shared::process::dev_target_binary_from_current_exe("rock").unwrap())
        .current_dir(root).args(arguments).env("ROCKC", resolve_rockc_path().unwrap()).env("NO_COLOR", "1"))
}

fn success(output: &std::process::Output) {
    assert!(output.status.success(), "stdout:\n{}\nstderr:\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn macro_b_16_real_rock_build_run_imports_only_execute_invocations() {
    let root = temp_test_dir("macro_b16_package"); let provider = root.join("provider"); let app = root.join("app");
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("rock_macro");
    write_package_with_options(&provider, "provider", "src/lib.rk", &[("rock_macro", sdk.display().to_string())],
        "> rock_macro::MacroResult\n< macro identity = context, input -> MacroResult::Expand input\n< mod tools\n", true);
    fs::write(provider.join("src/tools.rk"), "> rock_macro::MacroResult\n< macro identity = context, input -> MacroResult::Expand input\n").unwrap();
    let manifest = provider.join("rock.toml");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
    write_package_with_options(&app, "app", "src/main.rk", &[("provider", provider.display().to_string())],
        "> provider::identity\nmain = -> %identity 42\n", true);
    success(&rock(&app, &["build", "--debug"]));
    for source in [
        "> provider::identity\nmain = -> %identity 42\n",
        "main = -> %provider::identity 42\n",
        "> provider::tools\nmain = -> %tools::identity 42\n",
    ] {
        fs::write(app.join("src/main.rk"), source).unwrap();
        let output = rock(&app, &["run", "--debug"]);
        assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    }
    fs::write(app.join("src/main.rk"), "> provider::identity\nmain = !-> ()\n").unwrap();
    success(&rock(&app, &["build", "--debug", "--no-proc-execution"]));
    fs::write(app.join("src/main.rk"), "> provider::identity\nmain = -> %identity 42\n").unwrap();
    let disabled = rock(&app, &["build", "--debug", "--no-proc-execution"]);
    assert!(!disabled.status.success());
    assert!(format!("{}{}", String::from_utf8_lossy(&disabled.stdout), String::from_utf8_lossy(&disabled.stderr)).contains("execution is disabled"), "{disabled:?}");
    let runtime = root.join("runtime");
    write_package_with_options(&runtime, "runtime", "lib.rk", &[], "< answer = -> 42\n", true);
    let mut manifest_text = fs::read_to_string(&manifest).unwrap().replace("no_std = true", "no_std = false");
    manifest_text.push_str(&format!("runtime = {{ path = \"{}\" }}\n", runtime.display()));
    fs::write(&manifest, manifest_text).unwrap();
    let mut source = fs::read_to_string(provider.join("src/lib.rk")).unwrap();
    source.push_str(r#"
> rock_macro::unknown_entry
< macro helper = context, input ->
    i = 0
    while i < context.definition_dependencies.len!
        if context.definition_dependencies[i].name.as_str! == "runtime"
            match context.builder &input
                Result::Ok value =>
                    mut builder = value
                    mut names: Vec String = Vec::new!
                    names.push (String::from "answer")
                    match builder.dependency_path (i as U32), names
                        Result::Ok tokens => return MacroResult::Expand tokens
                        Result::Err _ => return unknown_entry context, input
                Result::Err _ => return unknown_entry context, input
        i = i + 1
    unknown_entry context, input
"#);
    fs::write(provider.join("src/lib.rk"), source).unwrap();
    fs::write(app.join("src/main.rk"), "main = -> (%provider::helper!)!\n").unwrap();
    let missing = rock(&app, &["build", "--debug"]);
    assert!(!missing.status.success());
    assert!(format!("{}{}", String::from_utf8_lossy(&missing.stdout), String::from_utf8_lossy(&missing.stderr)).contains("required runtime dependency runtime@0.1.0"), "{missing:?}");
    let app_manifest = app.join("rock.toml");
    fs::write(&app_manifest, format!("{}runtime = {{ path = \"{}\" }}\n", fs::read_to_string(&app_manifest).unwrap(), runtime.display())).unwrap();
    let result = rock(&app, &["run", "--debug"]);
    assert_eq!(result.status.code(), Some(42), "{result:?}");
    let plan = crate::dependency_plan::BuildPlan::new(&super::support::load_package(app.clone()), crate::dependency_plan::BuildTriples::native()).unwrap();
    let runtimes: Vec<_> = plan.units.values().filter(|unit| unit.package.root_dir == runtime).collect();
    assert_eq!(runtimes.len(), 2);
    assert_ne!(runtimes[0].package.object_path(), runtimes[1].package.object_path());
    for unit in runtimes { assert!(unit.package.artifact_path().is_file() && unit.package.object_path().is_file()); }
    let provider_unit = plan.units.values().find(|unit| unit.package.root_dir == provider).unwrap();
    assert!(provider_unit.package.worker_path().is_file());
    assert!(!provider_unit.package.object_path().exists(), "provider is not an ordinary runtime object");
    fs::remove_dir_all(root).unwrap();
}
