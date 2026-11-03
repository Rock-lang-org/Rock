use super::*;

#[test]
fn test_product_artifact_preserves_trait_default_methods() {
    let temp_dir = temp_test_dir("product_trait_defaults");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "trait Animal\n    @legs = -> 4\n< Animal\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let animal = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::Animal")
        .unwrap();
    let legs = animal.methods.get("legs").unwrap();
    let legs_body = products
        .bodies
        .trait_default_methods
        .values()
        .find(|function| function.id == legs.id)
        .unwrap();
    assert_eq!(legs_body.name, "legs");
    assert!(!legs_body.body.stmts.is_empty());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_file_module_trait_default_methods() {
    let temp_dir = temp_test_dir("product_file_module_trait_defaults");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&dep_dir, "dep", "", "< mod animals\n");
    fs::write(
        dep_dir.join("animals.rk"),
        "< trait Animal\n    @legs = -> 4\n< Animal\n",
    )
    .unwrap();

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();

    let animal = products
        .interface
        .traits
        .values()
        .find(|trait_| trait_.name == "dep::animals::Animal")
        .unwrap();
    let legs = animal.methods.get("legs").unwrap();
    let legs_body = products
        .bodies
        .trait_default_methods
        .values()
        .find(|function| function.id == legs.id)
        .unwrap();
    assert_eq!(legs_body.name, "legs");
    assert!(!legs_body.body.stmts.is_empty());

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_default_method_from_product_artifact() {
    let temp_dir = temp_test_dir("product_trait_default_downstream");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "< trait Animal\n    @legs = -> 4\n< Animal\n",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Animal\n\nstruct Dog\n\nimpl Animal for Dog\n\nmain = ->\n    d = Dog\n    d.legs!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 4);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_trait_generic_default_for_downstream_impl_from_product_artifact() {
    let temp_dir = temp_test_dir("product_trait_generic_default_downstream_impl");
    let dep_dir = temp_dir.join("dep");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(
        &dep_dir,
        "dep",
        "",
        "trait Value T\n    @get: T\n    @same: T\n    @same = -> self.get!\n< Value\n",
    );
    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Value\n\nstruct Number\n    < value: I64\n\nimpl Value I64 for Number\n    @get = -> @value\n\nmain = ->\n    number = Number\n        value: 42\n    number.same!\n",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_compile_generic_trait_default_method_from_product_artifact() {
    let temp_dir = temp_test_dir("product_generic_trait_default_downstream");
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
    new = value ->
        Box
            value: value

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

impl Value T for Box T
    @get = -> @value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);

    let exit_code = write_and_run_artifact_app(
        "> dep::Box
> dep::Value

main = ->
    b = Box::new 42
    b.same!
",
        vec![("dep".to_string(), artifact_path)],
    );
    assert_eq!(exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_preserves_generic_impl_inherited_trait_default_methods() {
    let temp_dir = temp_test_dir("product_generic_impl_inherited_trait_default");
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
    new = value ->
        Box
            value: value

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

impl Value T for Box T
    @get = -> @value
",
    );

    build_product_artifact(&dep_dir, "dep", &artifact_path, true, true);
    let products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    let generic_impl = products
        .bodies
        .generic_impls
        .values()
        .find(|imp| imp.methods.contains_key("same"))
        .expect("generic impl should preserve inherited default method");

    let same = generic_impl.methods.get("same").unwrap();
    assert!(!same.body.stmts.is_empty());
    assert!(generic_impl.methods.contains_key("get"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_product_artifact_remaps_trait_args_in_default_method_body() {
    let temp_dir = temp_test_dir("product_default_method_target_trait_args");
    let marker_dir = temp_dir.join("marker");
    let dep_dir = temp_dir.join("dep");
    let marker_artifact = temp_dir.join("marker.rkca");
    let artifact_path = temp_dir.join("dep.rkca");

    write_crate(&marker_dir, "marker", "", "struct Token\n< Token\n");
    build_product_artifact(&marker_dir, "marker", &marker_artifact, true, true);

    write_crate(
        &dep_dir,
        "dep",
        "",
        "> marker::Token

trait Value T
    @get: T
    @same: T
    @same = -> self.get!
< Value

keep_token: Token -> Token
keep_token = value -> value
< keep_token
",
    );

    build_product_artifact_with_extern_artifacts(
        &dep_dir,
        "dep",
        &artifact_path,
        true,
        true,
        vec![("marker".to_string(), marker_artifact.clone())],
    );

    let mut products = CompilerProducts::read_artifact_from_path(&artifact_path).unwrap();
    let keep_token_id = products.identity_table.export_names["keep_token"];
    let token_ty = products.interface.functions[&keep_token_id]
        .ret_type
        .clone();
    assert!(matches!(token_ty, Type::Struct { .. }));

    let value_trait_id = products.identity_table.export_names["Value"];
    let same_method_id = products.interface.traits[&value_trait_id].methods["same"].id;
    let body = products
        .bodies
        .trait_default_methods
        .values_mut()
        .find(|method| method.id == same_method_id)
        .unwrap();
    let HirStmt::Expr(expr) = &mut body.body.stmts[0] else {
        panic!("default method body should contain method call expression");
    };
    let HirExprKind::MethodCall(_, _, _, _, target) = &mut expr.kind else {
        panic!("default method body should call required trait method");
    };
    *target
        .trait_args_mut()
        .expect("trait method target has trait arguments") = vec![token_ty];
    products.write_artifact_to_path(&artifact_path).unwrap();

    let mut ctx = CrateContext::new();
    ctx.load_product_artifact_from_path_as("marker", marker_artifact)
        .unwrap();
    ctx.load_product_artifact_from_path_as("dep", artifact_path)
        .unwrap();
    let marker_token_id = ctx
        .extern_crate("marker")
        .unwrap()
        .metadata()
        .resolver()
        .item_paths["marker::Token"];
    let dep = ctx.extern_crate("dep").unwrap();
    let value_trait_id = dep
        .metadata()
        .interface()
        .trait_by_canonical_name("dep::Value")
        .expect("trait interface should exist")
        .id;
    let value_trait = dep
        .body_providers()
        .trait_with_defaults(value_trait_id)
        .unwrap();
    let same = value_trait.methods.get("same").unwrap();
    let HirStmt::Expr(expr) = &same.body.stmts[0] else {
        panic!("default method body should contain method call expression");
    };
    let HirExprKind::MethodCall(_, method_name, _, _, target) = &expr.kind else {
        panic!("default method body should call required trait method");
    };

    assert_eq!(method_name, "get");
    assert_eq!(target.trait_id(), Some(value_trait.id));
    assert_eq!(
        target.trait_args(),
        &[Type::Struct {
            id: marker_token_id,
            args: Vec::new(),
        }]
    );

    let _ = fs::remove_dir_all(temp_dir);
}
