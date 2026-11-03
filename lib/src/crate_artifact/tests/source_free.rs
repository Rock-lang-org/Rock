use super::*;

#[test]
fn test_nested_facade_imports_resolve_headers_without_dependency_sources() {
    let dir = temp_test_dir("nested_facade_headers");
    let _cleanup = TestDirCleanup(dir.clone());
    let dependency = dir.join("dependency");
    write_crate(&dependency, "dependency", "", "< mod values\n");
    fs::create_dir_all(dependency.join("values")).unwrap();
    fs::write(
        dependency.join("values/mod.rk"),
        "mod data\n< data::Value\n< data::extract\n",
    )
    .unwrap();
    fs::write(
        dependency.join("values/data.rk"),
        "< struct Value\n    < number: I32\nextract: Value -> I32\n< extract = value -> value.number\n",
    )
    .unwrap();
    let artifact = dir.join("dependency.rkca");
    build_product_artifact(&dependency, "dependency", &artifact, true, true);
    delete_crate_source_files(&dependency);
    let exit = write_and_run_artifact_app(
        r#"
> dependency::values::Value
> dependency::values::extract

apply: Value -> I32
apply = value -> extract value

main = ->
    value = Value
        number: 42
    apply value
"#,
        vec![("dependency".to_string(), artifact)],
    );
    assert_eq!(exit, 42);
}

#[test]
fn test_product_artifact_preserves_concrete_function_bodies() {
    let temp_dir = temp_test_dir("source_backed_artifact_bodies");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< answer = -> 5\n< answer\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let exit_code = write_and_run_artifact_app(
        "> dep::answer\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_associated_types() {
    let temp_dir = temp_test_dir("product_artifact_associated_types");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        r#"< trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Target = T
    @deref = -> &@value
"#,
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let deref_trait = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::Deref")
        .unwrap();
    assert_eq!(deref_trait.associated_types.len(), 1);
    assert_eq!(deref_trait.associated_types[0].name, "Target");
    assert_eq!(
        deref_trait.signatures.get("deref").unwrap().ret,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Projection {
                ty: Box::new(Type::Generic(crate::types::GenericParamId {
                    owner: deref_trait.id,
                    index: 0
                })),
                trait_id: deref_trait.id,
                assoc_type: crate::types::AssociatedTypeKey {
                    owner: deref_trait.id,
                    assoc_type_id: deref_trait.associated_types[0].id,
                },
                trait_args: vec![],
            }),
        }
    );

    let deref_impl = products
        .interface
        .impls
        .values()
        .find(|imp| imp.trait_name.as_deref() == Some("Deref") && imp.type_name == "Box")
        .unwrap();
    assert_eq!(deref_impl.associated_types.len(), 1);
    assert_eq!(deref_impl.associated_types[0].name, "Target");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_with_source_free_product_artifact() {
    let temp_dir = temp_test_dir("interface_only_artifact");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod math\n");
    fs::write(dep_dir.join("math.rk"), "< answer = -> 5\n< answer\n").unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app(
        "> dep::math::answer\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_root_glob_import_from_source_free_product_artifact() {
    let temp_dir = temp_test_dir("product_artifact_root_glob_import");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "answer = -> 5\n< answer\n");

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    delete_crate_source_files(&dep_dir);

    let exit_code = write_and_run_artifact_app(
        "> dep::*\n\nmain = -> answer!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 5);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_rejects_changed_dependency_fingerprint() {
    let temp_dir = temp_test_dir("changed_dependency_fingerprint");
    let dep_dir = temp_dir.join("dep");
    let app_dir = temp_dir.join("app");
    let dep_artifact = temp_dir.join("dep.rkca");
    let app_artifact = temp_dir.join("app.rkca");

    write_crate(&dep_dir, "dep", "", "answer = -> 5\n< answer\n");
    build_product_artifact(&dep_dir, "dep", &dep_artifact, true, true);

    write_crate(
        &app_dir,
        "app",
        "",
        "> dep::answer\n\nmain = -> answer!\n< main\n",
    );
    build_product_artifact_with_extern_artifacts(
        &app_dir,
        "app",
        &app_artifact,
        true,
        true,
        vec![("dep".to_string(), dep_artifact.clone())],
    );

    fs::write(dep_dir.join("lib.rk"), "answer = -> 7\n< answer\n").unwrap();
    build_product_artifact(&dep_dir, "dep", &dep_artifact, true, true);

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("dep", dep_artifact)
        .unwrap();
    let err = ctx
        .load_product_artifact_from_path_as("app", app_artifact)
        .unwrap_err();

    assert!(
        err.contains("dependency identity mismatch"),
        "unexpected error: {err}"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_projection_trait_identity_from_product_artifact() {
    let temp_dir = temp_test_dir("product_projection_trait_identity");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< trait HasValue\n    type Output\n    @get: Self::Output\n< HasValue\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::HasValue\n\nstruct Local\n    < value: I64\n\nimpl HasValue for Local\n    type Output = I64\n    @get = -> @value\n\nmain = ->\n    local = Local\n        value: 33\n    local.get!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 33);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn root_glob_import_keeps_local_direct_calls_direct_in_artifact_app() {
    let temp_dir = temp_test_dir("root_glob_local_callable_app");
    let dependency_dir = temp_dir.join("dep");
    let dependency_artifact = temp_dir.join("dep.rkca");
    write_crate(&dependency_dir, "dep", "", "unused = -> 0\n< unused\n");
    build_product_artifact(&dependency_dir, "dep", &dependency_artifact, true, true);

    let (stdout, exit_code) = compile_and_run_artifact_app(
        r#"> dep::*

local: I64
local = -> 83

identity: T -> T
identity = value -> value

main: () -> I64
main = -> identity (local!)
"#,
        vec![("dep".to_string(), dependency_artifact)],
    );

    assert_eq!(stdout, "");
    assert_eq!(exit_code, 83);

    let _ = fs::remove_dir_all(temp_dir);
}
