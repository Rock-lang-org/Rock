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

#[test]
fn macro_b_17_freshness_and_relocated_source_free_proc_bundle() {
    let root = temp_test_dir("macro_b17_bundle"); let provider = root.join("provider");
    let sdk = root.join("sdk"); let helper = root.join("helper"); let app = root.join("app");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    write_package_with_options(&sdk, "rock_macro", "src/lib.rk", &[("std", workspace.join("std").display().to_string())], "", true);
    for entry in fs::read_dir(workspace.join("rock_macro/src")).unwrap() {
        let entry = entry.unwrap(); if entry.path().extension().is_some_and(|extension| extension == "rk") {
            fs::copy(entry.path(), sdk.join("src").join(entry.file_name())).unwrap();
        }
    }
    write_package_with_options(&helper, "helper", "lib.rk", &[], "< answer = -> 42\n", true);
    write_package_with_options(&provider, "provider", "src/lib.rk",
        &[("rock_macro", sdk.display().to_string()), ("helper", helper.display().to_string())],
        "> rock_macro::MacroResult\n< macro identity = context, input -> MacroResult::Expand input\n", true);
    let manifest = provider.join("rock.toml");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
    write_package_with_options(&app, "app", "src/main.rk", &[("provider", provider.display().to_string())], "main = -> %provider::identity 42\n", true);
    success(&rock(&app, &["build", "--debug"]));
    let plan = crate::dependency_plan::BuildPlan::new(&super::support::load_package(app.clone()), crate::dependency_plan::BuildTriples::native()).unwrap();
    let package = &plan.units.values().find(|unit| unit.package.root_dir == provider).unwrap().package;
    let artifact = package.artifact_path(); let worker = package.worker_path();
    let mut modified = fs::metadata(&artifact).unwrap().modified().unwrap();
    success(&rock(&app, &["build", "--debug"]));
    assert_eq!(fs::metadata(&artifact).unwrap().modified().unwrap(), modified, "unchanged host is reused");
    for source in [provider.join("src/lib.rk"), helper.join("lib.rk"), sdk.join("src/lib.rk")] {
        fs::write(&source, format!("{}\n// freshness input changed\n", fs::read_to_string(&source).unwrap())).unwrap();
        success(&rock(&app, &["build", "--debug"]));
        let next = fs::metadata(&artifact).unwrap().modified().unwrap();
        assert!(next > modified, "provider was not rebuilt for {}", source.display()); modified = next;
    }
    let original_worker = fs::read(&worker).unwrap(); let mut corrupt = original_worker.clone(); corrupt.extend_from_slice(b"changed");
    fs::write(&worker, corrupt).unwrap();
    success(&rock(&app, &["build", "--debug"]));
    assert!(fs::metadata(&artifact).unwrap().modified().unwrap() > modified);
    assert_eq!(fs::read(&worker).unwrap(), original_worker);

    let bundle = root.join("bundle"); fs::create_dir_all(bundle.join("artifacts")).unwrap(); fs::create_dir_all(bundle.join("worker")).unwrap();
    let bundle_artifact = bundle.join("artifacts/provider.rkca");
    fs::copy(&artifact, &bundle_artifact).unwrap(); fs::copy(&worker, bundle.join("worker/lib")).unwrap();
    let bytes = fs::read(&bundle_artifact).unwrap();
    let moved = root.join("relocated"); fs::rename(bundle, &moved).unwrap();
    fs::remove_dir_all(provider.join("src")).unwrap(); fs::remove_file(&manifest).unwrap();
    fs::remove_dir_all(sdk.join("src")).unwrap(); fs::remove_file(sdk.join("rock.toml")).unwrap();
    for unit in plan.units.values().filter(|unit| unit.id.platform == crate::dependency_plan::BuildPlatform::Host
        && unit.id.role() == rock_shared::manifest::TargetRole::Library) {
        fs::remove_file(unit.package.artifact_path()).unwrap(); fs::remove_file(unit.package.object_path()).unwrap();
    }
    let artifact = moved.join("artifacts/provider.rkca"); let rockc = resolve_rockc_path().unwrap();
    let inspect = || run_command_output(Command::new(&rockc).args(["--validate-artifact", artifact.to_str().unwrap(), "--expected-artifact-role", "proc-macro", "--no-proc-execution"]));
    success(&inspect());
    let source = root.join("source-free.rk"); fs::write(&source, "main = -> %provider::identity 42\n").unwrap();
    let output_dir = root.join("source-free-output");
    let output = run_command_output(Command::new(&rockc).args(["--entry-file", source.to_str().unwrap(), "--output-dir", output_dir.to_str().unwrap(), "--no-std", "--no-prelude", "--extern-artifact", &format!("provider={}", artifact.display())]));
    success(&output);
    super::support::assert_executable_exit_code(&output_dir.join("source-free"), 42);
    assert_eq!(fs::read(&artifact).unwrap(), bytes, "source-free consumption never rewrites provider metadata");
    let mut obsolete = bytes.clone(); let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    obsolete[8..12].copy_from_slice(&(version - 1).to_le_bytes()); fs::write(&artifact, obsolete).unwrap(); assert!(!inspect().status.success());
    fs::write(&artifact, b"corrupt product").unwrap(); assert!(!inspect().status.success()); fs::write(&artifact, &bytes).unwrap();
    let worker = moved.join("worker/lib"); let mut changed = fs::read(&worker).unwrap(); changed.push(0); fs::write(&worker, changed).unwrap();
    assert!(!inspect().status.success(), "host content identity is checked even without execution");
    fs::write(&worker, original_worker).unwrap(); success(&inspect());
    assert_eq!(fs::read(&artifact).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn macro_b_20_external_block_dsl_runs_and_diagnoses_through_rock() {
    let root = temp_test_dir("macro_b20_dsl"); let provider = root.join("provider"); let app = root.join("app");
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("rock_macro");
    write_package_with_options(&provider, "block_dsl", "src/lib.rk", &[("rock_macro", sdk.display().to_string())],
        include_str!("../../../lib/tests/fixtures/macros/block_dsl/src/lib.rk"), false);
    let manifest = provider.join("rock.toml");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
    write_package_with_options(&app, "app", "src/main.rk", &[("block_dsl", provider.display().to_string())], "", true);
    for source in [
        include_str!("../../../lib/tests/fixtures/macros/block_dsl/expression.rk"),
        include_str!("../../../lib/tests/fixtures/macros/block_dsl/items.rk"),
        include_str!("../../../lib/tests/fixtures/macros/block_dsl/forward.rk"),
    ] {
        fs::write(app.join("src/main.rk"), source).unwrap();
        let output = rock(&app, &["run", "--debug"]);
        assert_eq!(output.status.code(), Some(42), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
    }
    fs::write(app.join("src/main.rk"), include_str!("../../../lib/tests/fixtures/macros/block_dsl/error.rk")).unwrap();
    let error = rock(&app, &["build", "--debug"]);
    assert!(!error.status.success());
    let report = format!("{}{}", String::from_utf8_lossy(&error.stdout), String::from_utf8_lossy(&error.stderr));
    assert!(report.contains("block fixture forbids '@'") && report.contains("雪🦀 @reserved"), "{report}");
    let raw = include_str!("../../../lib/tests/fixtures/macros/block_dsl/expression.rk").replace("render", "raw");
    fs::write(app.join("src/main.rk"), raw).unwrap();
    let error = rock(&app, &["build", "--debug"]);
    assert!(!error.status.success(), "uninterpreted DSL is not Rock output");
    let report = format!("{}{}", String::from_utf8_lossy(&error.stdout), String::from_utf8_lossy(&error.stderr));
    assert!(report.contains("Unexpected") && report.contains("expanded from macro 'raw'"), "{report}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn macro_b_21_external_interpolation_resolves_caller_holes_and_runtime_authority() {
    let root = temp_test_dir("macro_b21_interpolation");
    let provider = root.join("provider"); let runtime = root.join("runtime"); let app = root.join("app");
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("rock_macro");
    let std = sdk.parent().unwrap().join("std");
    write_package_with_options(&runtime, "ink_runtime", "src/lib.rk", &[("std", std.display().to_string())],
        include_str!("../../../lib/tests/fixtures/macros/interpolation/runtime/src/lib.rk"), false);
    write_package_with_options(&provider, "weave", "src/lib.rk", &[
        ("rock_macro", sdk.display().to_string()), ("ink_runtime", runtime.display().to_string()),
    ], include_str!("../../../lib/tests/fixtures/macros/interpolation/provider/src/lib.rk"), false);
    let manifest = provider.join("rock.toml");
    fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
    write_package_with_options(&app, "app", "src/main.rk", &[
        ("weave", provider.display().to_string()), ("ink_runtime", runtime.display().to_string()), ("std", std.display().to_string()),
    ], include_str!("../../../lib/tests/fixtures/macros/interpolation/consumer.rk"), false);
    let output = rock(&app, &["run", "--debug"]);
    success(&output);
    assert_eq!(output.stdout, b"Hello Ada: 3\n");
    let manifest = app.join("rock.toml");
    let original = fs::read_to_string(&manifest).unwrap();
    fs::write(&manifest, original.lines().filter(|line| !line.starts_with("ink_runtime = ")).collect::<Vec<_>>().join("\n")).unwrap();
    let output = rock(&app, &["build", "--debug"]);
    assert!(!output.status.success());
    let report = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(report.contains("required runtime dependency ink_runtime@0.1.0"), "{report}");
    fs::write(&manifest, original).unwrap();
    fs::write(app.join("src/main.rk"), "> weave::weave\nmain = !-> (%weave \"é\\n{missing}\").println!\n").unwrap();
    let output = rock(&app, &["build", "--debug"]);
    assert!(!output.status.success());
    let report = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(report.contains("Unknown variable: missing"), "{report}");
    fs::write(app.join("src/main.rk"), "> weave::weave\nstruct Unsupported\nmain = !->\n    item = Unsupported\n    (%weave \"{item}\").println!\n").unwrap();
    let output = rock(&app, &["build", "--debug"]);
    assert!(!output.status.success());
    let report = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(report.contains("Paint") && report.contains("Unsupported"), "{report}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn macro_b_22_nontrivial_consumers_execute_from_relocated_source_free_bundle() {
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap(); let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() { copy_tree(&entry.path(), &target); }
            else { fs::copy(entry.path(), target).unwrap(); }
        }
    }
    let root = temp_test_dir("macro_b22_bundle");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let std = root.join("std"); let sdk = root.join("sdk"); let runtime = root.join("runtime");
    let provider = root.join("provider"); let dsl = root.join("dsl"); let app = root.join("app");
    write_package_with_options(&std, "std", "src/lib.rk", &[], "", true);
    copy_tree(&workspace.join("std/src"), &std.join("src"));
    write_package_with_options(&sdk, "rock_macro", "src/lib.rk", &[("std", std.display().to_string())], "", true);
    copy_tree(&workspace.join("rock_macro/src"), &sdk.join("src"));
    write_package_with_options(&runtime, "ink_runtime", "src/lib.rk", &[("std", std.display().to_string())],
        include_str!("../../../lib/tests/fixtures/macros/interpolation/runtime/src/lib.rk"), false);
    write_package_with_options(&provider, "weave", "src/lib.rk", &[
        ("rock_macro", sdk.display().to_string()), ("ink_runtime", runtime.display().to_string()),
    ], include_str!("../../../lib/tests/fixtures/macros/interpolation/provider/src/lib.rk"), false);
    write_package_with_options(&dsl, "block_dsl", "src/lib.rk", &[("rock_macro", sdk.display().to_string())],
        include_str!("../../../lib/tests/fixtures/macros/block_dsl/src/lib.rk"), false);
    for package in [&provider, &dsl] {
        let manifest = package.join("rock.toml");
        fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
    }
    write_package_with_options(&app, "app", "src/main.rk", &[
        ("weave", provider.display().to_string()), ("block_dsl", dsl.display().to_string()),
        ("ink_runtime", runtime.display().to_string()), ("std", std.display().to_string()),
    ], include_str!("../../../lib/tests/fixtures/macros/interpolation/packaged.rk"), false);
    let output = rock(&app, &["run", "--debug"]); success(&output);
    assert_eq!(output.stdout, b"source-free 42\n");
    let plan = crate::dependency_plan::BuildPlan::new(&super::support::load_package(app.clone()), crate::dependency_plan::BuildTriples::native()).unwrap();
    let bundle = root.join("bundle");
    let mut expected = Vec::new();
    let mut names = Vec::new();
    for unit in plan.units.values().filter(|unit|
        unit.id.role() == rock_shared::manifest::TargetRole::ProcMacro
            || (unit.id.platform == crate::dependency_plan::BuildPlatform::Target && unit.id.role() == rock_shared::manifest::TargetRole::Library)) {
        let name = &unit.package.manifest.crate_.name;
        names.push(name.clone());
        let relative = Path::new(name).join("artifacts").join(format!("{name}.rkca"));
        fs::create_dir_all(bundle.join(&relative).parent().unwrap()).unwrap();
        fs::copy(unit.package.artifact_path(), bundle.join(&relative)).unwrap();
        expected.push((relative, fs::read(unit.package.artifact_path()).unwrap()));
        let (original, relative) = if unit.id.role() == rock_shared::manifest::TargetRole::ProcMacro {
            (unit.package.worker_path(), Path::new(name).join("worker/lib"))
        } else {
            (unit.package.object_path(), Path::new(name).join("objects/lib.o"))
        };
        fs::create_dir_all(bundle.join(&relative).parent().unwrap()).unwrap();
        fs::copy(&original, bundle.join(&relative)).unwrap();
        expected.push((relative, fs::read(original).unwrap()));
    }
    names.sort(); assert_eq!(names, ["block_dsl", "ink_runtime", "std", "weave"]);
    let moved = root.join("relocated"); fs::rename(&bundle, &moved).unwrap();
    for package in [&provider, &dsl, &sdk, &runtime, &std] {
        fs::remove_dir_all(package.join("src")).unwrap();
        fs::remove_file(package.join("rock.toml")).unwrap();
    }
    fs::remove_dir_all(app.join("build")).unwrap();
    let source = root.join("packaged.rk");
    fs::write(&source, include_str!("../../../lib/tests/fixtures/macros/interpolation/packaged.rk")).unwrap();
    let output_dir = root.join("source-free-output");
    let mut command = Command::new(resolve_rockc_path().unwrap());
    command.args(["--entry-file", source.to_str().unwrap(), "--output-dir", output_dir.to_str().unwrap(), "--no-std"]);
    for name in &names {
        command.args(["--extern-artifact", &format!("{name}={}", moved.join(name).join("artifacts").join(format!("{name}.rkca")).display())]);
    }
    success(&run_command_output(&mut command));
    let output = run_command_output(&mut Command::new(output_dir.join("packaged")));
    success(&output); assert_eq!(output.stdout, b"source-free 42\n");
    for (relative, bytes) in expected { assert_eq!(fs::read(moved.join(relative)).unwrap(), bytes, "source-free consumption changed a bundled product"); }
    fs::remove_dir_all(root).unwrap();
}
