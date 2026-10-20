use super::local_resolution_tests::module;
use crate::ast;
use crate::lower::Lowerer;
use crate::types::Type;

#[test]
fn macro_a_13_std_frontend_preserves_existing_item_resolution() {
    let config = crate::Config {
        entry_file: std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../std/src/lib.rk"),
        current_crate_name: Some("std".into()),
        no_std: true,
        ..crate::Config::default()
    };
    if let Err(errors) = crate::analyze(&config) {
        panic!("{}", errors.0.iter().take(12).map(|error| format!("{:?}: {}", error.location, error.message)).collect::<Vec<_>>().join("\n"));
    }
}

#[test]
fn macro_a_13_nominal_generic_binders_with_same_spelling_keep_distinct_ids() {
    let module = module(&[
        ("struct Pair T, ", 0), ("T", 7),
        ("\n    first: T\n    second: ", 0), ("T", 7),
        ("\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let pair = partial.structs.values().find(|item| item.name == "Pair").unwrap();
    assert_eq!(pair.generic_params.len(), 2);
    assert_eq!(pair.fields[0].ty, Type::Generic(pair.generic_params[0].id));
    assert_eq!(pair.fields[1].ty, Type::Generic(pair.generic_params[1].id));
}

#[test]
fn macro_a_13_generic_default_resolves_matching_earlier_context() {
    let module = module(&[
        ("struct Pair T, {", 0), ("T", 11),
        (" = T}\n    first: T\n    second: ", 0), ("T", 11),
        ("\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let pair = partial.structs.values().find(|item| item.name == "Pair").unwrap();
    assert_eq!(pair.generic_params[1].default, Some(Type::Generic(pair.generic_params[0].id)));
    assert_eq!(pair.fields[1].ty, Type::Generic(pair.generic_params[1].id));
}

#[test]
fn macro_a_13_generic_default_context_mismatch_cannot_capture_root_parameter() {
    let module = module(&[
        ("struct Pair T, {U = ", 0), ("T", 13),
        ("}\nmain = !-> ()\n", 0),
    ]);
    let result = Lowerer::new_for_test().lower_program(&ast::Program { module });
    assert!(result.is_err(), "a different context cannot read root T in a default");
}

#[test]
fn macro_a_13_function_signature_and_body_annotations_share_contextual_generics() {
    let module = module(&[
        ("keep: T -> ", 0), ("T", 17),
        (" -> T\nkeep = first, second ->\n    original: T = first\n    generated: ", 0),
        ("T", 17), (" = second\n    original\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let keep = partial.functions.values().find(|item| item.name == "keep").unwrap();
    assert_eq!(keep.generic_params.len(), 2);
    assert_ne!(keep.params[0].ty, keep.params[1].ty);
    assert_eq!(keep.ret_type, keep.params[0].ty);
}

#[test]
fn macro_a_13_nested_type_lambda_keeps_outer_context_reference() {
    let module = module(&[
        ("type Select = \\T -> ", 0), ("\\T -> ", 19),
        ("T\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let alias = partial.type_aliases.values().find(|item| item.name == "Select").unwrap();
    let Type::Lambda { body, .. } = &alias.ty else { panic!("outer lambda") };
    let Type::Lambda { body, .. } = body.as_ref() else { panic!("inner lambda") };
    assert!(matches!(body.as_ref(), Type::BoundVar { depth: 1, index: 0, .. }));
}

#[test]
fn macro_a_13_default_binder_does_not_hide_different_context_nominal() {
    let module = module(&[
        ("struct Value\n    value: I64\nstruct Holder {", 0),
        ("Value", 23), (" = Value}\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let holder = partial.structs.values().find(|item| item.name == "Holder").unwrap();
    let value = partial.structs.values().find(|item| item.name == "Value").unwrap();
    assert!(matches!(holder.generic_params[0].default, Some(Type::Struct { id, .. }) if id == value.id));
}

#[test]
fn macro_a_13_nonroot_type_reference_cannot_fall_back_to_root_nominal() {
    let module = module(&[
        ("struct Value\n    value: I64\nstruct Holder\n    field: ", 0),
        ("Value", 29), ("\nmain = !-> ()\n", 0),
    ]);
    assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).is_err());
}

#[test]
fn macro_a_13_enum_and_alias_binders_keep_contexts() {
    let module = module(&[
        ("enum Choice T, ", 0), ("T", 31),
        ("\n    First T\n    Second ", 0), ("T", 31),
        ("\ntype Pair T, ", 0), ("T", 31),
        (" = (T, ", 0), ("T", 31), (" )\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let choice = partial.enums.values().find(|item| item.name == "Choice").unwrap();
    let crate::hir::HirVariantFields::Positional(first) = &choice.variants[0].fields else { panic!("first") };
    let crate::hir::HirVariantFields::Positional(second) = &choice.variants[1].fields else { panic!("second") };
    assert_eq!(first, &[Type::Generic(choice.generic_params[0].id)]);
    assert_eq!(second, &[Type::Generic(choice.generic_params[1].id)]);
    let alias = partial.type_aliases.values().find(|item| item.name == "Pair").unwrap();
    let Type::Lambda { body, .. } = &alias.ty else { panic!("alias lambda") };
    let Type::Tuple(elements) = body.as_ref() else { panic!("alias tuple") };
    assert!(matches!(elements[0], Type::BoundVar { depth: 0, index: 0, .. }));
    assert!(matches!(elements[1], Type::BoundVar { depth: 0, index: 1, .. }));
}

#[test]
fn macro_a_13_impl_generic_discovery_and_method_body_keep_contexts() {
    let module = module(&[
        ("struct Pair A, B\n    first: A\n    second: B\nimpl Pair T, ", 0),
        ("T", 37),
        ("\n    @second = ->\n        value: ", 0), ("T", 37),
        (" = self.second\n        value\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let implementation = partial.impls.values().next().unwrap();
    assert_eq!(implementation.type_generics.len(), 2);
    assert_ne!(implementation.type_generics[0].id, implementation.type_generics[1].id);
    let method = implementation.methods.get("second").unwrap();
    assert_eq!(method.ret_type, Type::Generic(implementation.type_generics[1].id));
}

#[test]
fn macro_a_13_trait_generic_default_and_default_method_keep_contexts() {
    let module = module(&[
        ("trait Choose T, {", 0), ("T", 41),
        (" = T}\n    choose: T -> ", 0), ("T", 41),
        (" -> T\n    choose = first, second ->\n        value: ", 0),
        ("T", 41), (" = second\n        first\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let definition = partial.traits.values().find(|item| item.name == "Choose").unwrap();
    assert_eq!(definition.generic_params.len(), 2);
    assert_eq!(definition.generic_params[1].default, Some(Type::Generic(definition.generic_params[0].id)));
    let method = definition.methods.get("choose").unwrap();
    assert_eq!(method.params[0].ty, Type::Generic(definition.generic_params[0].id));
    assert_eq!(method.params[1].ty, Type::Generic(definition.generic_params[1].id));
}

#[test]
fn macro_a_13_constructor_binder_context_survives_signature_reordering() {
    let module = module(&[
        ("trait Marker for F _\n    marker: () -> I64\nkeep: T -> (", 0),
        ("F", 43), (" T) -> T where ", 0), ("F", 43),
        (" _: Marker\nkeep = first, second ->\n    generated: ", 0),
        ("F", 43), (" T = second\n    first\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let keep = partial.functions.values().find(|item| item.name == "keep").unwrap();
    assert_eq!(keep.generic_params.len(), 2);
    let constructor = keep.generic_params.iter().find(|param| param.name == "F").unwrap();
    assert_eq!(constructor.kind, crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type, crate::type_services::kind::Kind::Type,
    ));
    assert_eq!(partial.resolver.generic_context(constructor.id), super::SyntaxContext(43));
}

#[test]
fn macro_a_13_context_remapping_snapshots_cyclic_permutations() {
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::types::GenericParamId;
    let owner = DefId::new(CrateId(0), LocalDefId(1));
    let first = GenericParamId { owner, index: 0 };
    let second = GenericParamId { owner, index: 1 };
    let mut contexts = std::collections::HashMap::from([
        (first, super::SyntaxContext(47)), (second, super::SyntaxContext(53)),
    ]);
    crate::collect::resolver::remap_generic_contexts(&mut contexts, &[second, first], owner);
    assert_eq!(contexts[&first], super::SyntaxContext(53));
    assert_eq!(contexts[&second], super::SyntaxContext(47));
}

#[test]
fn macro_a_13_static_constructor_inherits_expected_generic_argument() {
    let module = module(&[("struct Buffer T\n    < value: I64\nimpl Buffer T\n    new: () -> Buffer T\n    new = -> Buffer T\n        value: 0\nstruct Holder T\n    < buffer: Buffer T\nkeep: T -> Holder T\nkeep = value -> Holder T\n    buffer: Buffer::new!\nmain = !-> ()\n", 0)]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    crate::infer::finalize(partial).unwrap();
}

#[test]
fn macro_a_13_same_spelling_nominal_definitions_keep_distinct_contexts() {
    let module = module(&[
        ("struct Value\n    caller: I64\n", 0),
        ("struct Value\n    generated: Bool\n", 59),
        ("struct Holder\n    caller: Value\n    generated: ", 0),
        ("Value", 59), ("\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let values = partial.structs.values().filter(|item| item.name == "Value").collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    let caller = values.iter().find(|item| item.fields[0].name == "caller").unwrap();
    let generated = values.iter().find(|item| item.fields[0].name == "generated").unwrap();
    let holder = partial.structs.values().find(|item| item.name == "Holder").unwrap();
    assert!(matches!(holder.fields[0].ty, Type::Struct { id, .. } if id == caller.id));
    assert!(matches!(holder.fields[1].ty, Type::Struct { id, .. } if id == generated.id));
}

#[test]
fn macro_a_13_root_type_reference_cannot_capture_generated_definition() {
    let module = module(&[
        ("struct Value\n    field: I64\n", 61),
        ("struct Holder\n    value: Value\nmain = !-> ()\n", 0),
    ]);
    assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).is_err());
}

#[test]
fn macro_a_13_same_spelling_functions_and_constructors_resolve_by_context() {
    let module = module(&[
        ("answer = -> 1\nstruct Value\n    < value: I64\n", 0),
        ("answer = -> true\nstruct Value\n    < value: Bool\n", 67),
        ("main = ->\n    caller = Value\n        value: answer!\n    generated = ", 0),
        ("Value", 67), ("\n        value: ", 0), ("answer", 67),
        ("!\n    (caller, generated)\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let main = partial.functions.values().find(|item| item.name == "main").unwrap();
    let Type::Tuple(elements) = &main.ret_type else { panic!("tuple return") };
    let (Type::Struct { id: caller, .. }, Type::Struct { id: generated, .. }) = (&elements[0], &elements[1]) else { panic!("nominal returns") };
    assert_ne!(caller, generated);
    assert_eq!(partial.structs[caller].fields[0].ty, Type::I64);
    assert_eq!(partial.structs[generated].fields[0].ty, Type::Bool);
}

#[test]
fn macro_a_13_module_path_segments_preserve_independent_contexts() {
    let mut root = module(&[
        ("main = -> (tools::answer!, ", 0), ("tools", 71),
        ("::answer!)\n", 0),
    ]);
    for (context, source) in [(0, "answer = -> 1\n"), (71, "answer = -> true\n")] {
        let mut child = module(&[(source, 0)]);
        child.name = Some(ast::Ident {
            name: "tools".into(), context: super::SyntaxContext(context), span: crate::lexer::Span::test(),
        });
        root.top_levels.push(ast::TopLevel::Module(ast::ModuleDecl(child)));
    }
    for crate_name in [None, Some("app")] {
        let program = ast::Program { module: root.clone() };
        let crates = crate::crate_system::CrateContext::new();
        let declarations = crate::collect::collect(&program, &crates, false, crate_name).unwrap();
        let partial = crate::lower::pipeline::LoweringPipeline::new(&program, &crates, crate_name).lower_from_declarations(declarations).unwrap();
        let resolved = crate::infer::finalize(partial).unwrap();
        let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
        assert_eq!(main.ret_type, Type::Tuple(vec![Type::I64, Type::Bool]));
    }
}

#[test]
fn macro_a_13_definition_source_scope_does_not_capture_caller_helper() {
    let mut root = module(&[("answer = -> 1\nmain = -> ", 0), ("answer", 107), ("!\n", 0)]);
    let mut child = module(&[("answer = -> true\n", 0)]);
    child.name = Some(ast::Ident { name: "tools".into(), context: super::SyntaxContext::ROOT, span: crate::lexer::Span::test() });
    root.top_levels.push(ast::TopLevel::Module(ast::ModuleDecl(child)));
    let program = ast::Program { module: root };
    let crates = crate::crate_system::CrateContext::new();
    let mut environments = crate::macro_expansion::binding_environment::BindingEnvironments::default();
    environments.record_source_scope(super::SyntaxContext(107), vec![("tools".into(), super::SyntaxContext::ROOT)]).unwrap();
    let declarations = crate::collect::collect_with_binding_environments(&program, None, &crates, false, None, &environments).unwrap();
    let partial = crate::lower::pipeline::LoweringPipeline::new(&program, &crates, None).lower_from_declarations(declarations).unwrap();
    let resolved = crate::infer::finalize(partial).unwrap();
    let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
    assert_eq!(main.ret_type, Type::Bool);
}

#[test]
fn macro_a_13_associated_type_member_requires_matching_context() {
    for reference_context in [0, 109] {
        let module = module(&[
            ("trait Source\n    type ", 0), ("Item", 109),
            ("\n    take: () -> Self::", 0), ("Item", reference_context),
            ("\nmain = !-> ()\n", 0),
        ]);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module });
        if reference_context == 109 {
            result.unwrap();
        } else {
            assert!(result.is_err(), "root reference cannot capture generated associated type");
        }
    }
}

#[test]
fn macro_a_13_contextual_type_argument_and_trait_impl_keep_captured_identities() {
    let module = module(&[
        ("struct Value\n    field: I64\ntrait Marker\n    mark: () -> I64\n", 0),
        ("struct Value\n    field: Bool\ntrait Marker\n    mark: () -> Bool\n", 113),
        ("struct Boxed T\n    value: T\nstruct Holder\n    value: Boxed ", 0), ("Value", 113),
        ("\nimpl ", 0), ("Marker", 113), (" for ", 0), ("Value", 113),
        ("\n    mark = -> true\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let generated_value = partial.structs.values().find(|item| item.name == "Value" && item.fields[0].ty == Type::Bool).unwrap();
    let generated_trait = partial.traits.values().find(|item| item.name == "Marker" && item.signatures["mark"].ret == Type::Bool).unwrap();
    let implementation = partial.impls.values().next().unwrap();
    assert_eq!(implementation.trait_id, Some(generated_trait.id));
    assert!(matches!(&implementation.receiver_pattern, crate::hir::HirImplReceiverPattern::Exact(Type::Struct { id, .. }) if *id == generated_value.id));
    let holder = partial.structs.values().find(|item| item.name == "Holder").unwrap();
    assert!(matches!(&holder.fields[0].ty, Type::Struct { args, .. } if matches!(args.as_slice(), [Type::Struct { id, .. }] if *id == generated_value.id)));
}

#[test]
fn macro_a_13_generated_function_uses_its_contextual_signature() {
    let module = module(&[
        ("answer: I64 -> I64\nanswer = value -> value\n", 0),
        ("answer: Bool -> Bool\nanswer = value -> value\n", 127),
        ("main = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let answers = partial.functions.values().filter(|function| function.name == "answer").collect::<Vec<_>>();
    assert_eq!(answers.len(), 2);
    assert!(answers.iter().any(|function| function.params[0].ty == Type::I64 && function.ret_type == Type::I64));
    assert!(answers.iter().any(|function| function.params[0].ty == Type::Bool && function.ret_type == Type::Bool));
}

#[test]
fn macro_a_13_import_alias_keeps_module_and_alias_contexts() {
    let module = module(&[
        ("mod tools\n    < answer = -> 1\n", 0),
        ("mod tools\n    < answer = -> true\n", 137),
        ("> ", 0), ("tools", 137), ("::", 0), ("answer", 137),
        ("\nmain = -> ", 0), ("answer", 137), ("!\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let resolved = crate::infer::finalize(partial).unwrap();
    let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
    assert_eq!(main.ret_type, Type::Bool);
}

#[test]
fn macro_a_13_imported_contextual_module_keeps_member_context_independent() {
    let module = module(&[
        ("mod outer\n    mod ", 0), ("tools", 139),
        ("\n        < answer = -> true\n> outer::", 0), ("tools", 139),
        ("\nmain = -> ", 0), ("tools", 139), ("::answer!\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let resolved = crate::infer::finalize(partial).unwrap();
    let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
    assert_eq!(main.ret_type, Type::Bool);
}

#[test]
fn macro_a_13_imported_generated_type_does_not_leak_to_root() {
    for reference_context in [0, 149] {
        let module = module(&[
            ("mod tools\n    < struct ", 0), ("Value", 149),
            ("\n        value: Bool\n> tools::", 0), ("Value", 149),
            ("\nstruct Holder\n    value: ", 0), ("Value", reference_context),
            ("\nmain = !-> ()\n", 0),
        ]);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module });
        if reference_context == 149 { result.unwrap(); }
        else { assert!(result.is_err(), "contextual import must not publish a root alias"); }
    }
}

#[test]
fn macro_a_13_type_lookup_preserves_contextual_containing_module() {
    let module = module(&[
        ("mod tools\n    struct Value\n        value: I64\n", 0),
        ("mod ", 0), ("tools", 151),
        ("\n    struct Value\n        value: Bool\n    struct Holder\n        value: Value\nmain = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    let value = partial.structs.values().find(|item| item.name == "Value" && item.fields[0].ty == Type::Bool).unwrap();
    let holder = partial.structs.values().find(|item| item.name == "Holder").unwrap();
    assert!(matches!(&holder.fields[0].ty, Type::Struct { id, .. } if *id == value.id));
}

#[test]
fn macro_a_13_self_associated_owner_uses_trait_identity_under_shadowing() {
    let module = module(&[
        ("trait Source\n    type Item\n    take: () -> Self::Item\n", 0),
        ("trait Source\n    type Item\n    take: () -> Self::Item\n", 157),
        ("main = !-> ()\n", 0),
    ]);
    let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
    assert_eq!(partial.traits.len(), 2);
    for owner in partial.traits.values() {
        assert!(matches!(&owner.signatures["take"].ret, Type::Projection { trait_id, assoc_type, .. }
            if *trait_id == owner.id && assoc_type.owner == owner.id));
    }
}

#[test]
fn macro_a_13_where_bound_cannot_resolve_trait_by_spelling_fallback() {
    let module = module(&[
        ("trait Marker\n    mark: () -> I64\nkeep: T -> T where T: ", 0), ("Marker", 163),
        ("\nkeep = value -> value\nmain = !-> ()\n", 0),
    ]);
    let errors = Lowerer::new_for_test().lower_program(&ast::Program { module }).err().unwrap();
    assert!(errors.iter().any(|error| error.message.contains("unknown trait 'Marker'")), "{errors:?}");
}

#[test]
fn macro_a_13_contextual_module_cannot_capture_root_sibling_members() {
    let module = module(&[
        ("mod tools\n    < answer = -> 1\nmod ", 0), ("tools", 167),
        ("\n    probe = -> answer!\nmain = !-> ()\n", 0),
    ]);
    assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).is_err());
}

#[test]
fn macro_a_13_contextual_module_cannot_capture_root_sibling_type() {
    let module = module(&[
        ("mod tools\n    < struct Value\n        value: I64\nmod ", 0), ("tools", 173),
        ("\n    struct Holder\n        value: Value\nmain = !-> ()\n", 0),
    ]);
    assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).is_err());
}

#[test]
fn macro_a_13_enum_variant_path_keeps_member_context() {
    for reference_context in [0, 179] {
        let module = module(&[
            ("enum Value\n    ", 0), ("Choice", 179),
            ("\nmain = -> Value::", 0), ("Choice", reference_context), ("\n", 0),
        ]);
        let declarations = crate::collect::collect(&ast::Program { module: module.clone() }, &crate::crate_system::CrateContext::new(), false, None).unwrap();
        assert!(declarations.resolver.variant_contexts.values().any(|context| *context == super::SyntaxContext(179)), "{:?}", declarations.resolver.variant_contexts);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module }).and_then(crate::infer::finalize);
        if reference_context == 179 { result.unwrap(); }
        else { assert!(result.is_err(), "root variant reference cannot capture generated member"); }
    }
}

#[test]
fn macro_a_13_static_method_path_keeps_member_context() {
    for reference_context in [0, 181] {
        let module = module(&[
            ("struct Value\n    value: I64\nimpl Value\n    ", 0), ("answer", 181),
            (" = -> true\nmain = -> Value::", 0), ("answer", reference_context), ("!\n", 0),
        ]);
        let declarations = crate::collect::collect(&ast::Program { module: module.clone() }, &crate::crate_system::CrateContext::new(), false, None).unwrap();
        assert!(declarations.resolver.member_contexts.values().any(|context| *context == super::SyntaxContext(181)), "{:?}", declarations.resolver.member_contexts);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module }).and_then(crate::infer::finalize);
        if reference_context == 181 { result.unwrap(); }
        else { assert!(result.is_err(), "root static method reference cannot capture generated member"); }
    }
}

#[test]
fn macro_a_13_enum_pattern_uses_contextual_owner_and_member() {
    for reference_context in [0, 191] {
        let module = module(&[
            ("enum Value\n    Choice I64\n", 0),
            ("enum Value\n    Choice Bool\n", 191),
            ("convert: ", 0), ("Value", 191),
            (" -> Bool\nconvert = value ->\n    match value\n        ", 0),
            ("Value::Choice", reference_context), (" inner => inner\nmain = !-> ()\n", 0),
        ]);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module }).and_then(crate::infer::finalize);
        if reference_context == 191 { result.unwrap(); }
        else { assert!(result.is_err(), "pattern cannot select a root sibling enum"); }
    }
}

#[test]
fn macro_a_13_root_function_reference_cannot_capture_generated_definition() {
    let module = module(&[
        ("answer = -> true\n", 193),
        ("main = -> answer!\n", 0),
    ]);
    assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).and_then(crate::infer::finalize).is_err());
}

#[test]
fn macro_a_13_contextual_globs_and_reexports_preserve_selected_module() {
    for export in ["< tools::*", "< tools::answer"] {
        let module = module(&[
            ("mod tools\n    < answer = -> 1\n", 0),
            ("mod tools\n    < answer = -> true\n", 197),
            ("mod facade\n    ", 0), (export, 197),
            ("\n> facade::*\nmain = -> ", 0), ("answer", 197), ("!\n", 0),
        ]);
        let partial = Lowerer::new_for_test().lower_program(&ast::Program { module }).unwrap();
        let resolved = crate::infer::finalize(partial).unwrap();
        let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
        assert_eq!(main.ret_type, Type::Bool);
    }
}

#[test]
fn macro_a_13_contextual_glob_excludes_private_items_and_root_aliases() {
    for (name, context, accepted) in [("answer", 199, true), ("answer", 0, false), ("secret", 199, false)] {
        let module = module(&[
            ("mod tools\n    < answer = -> true\n    secret = -> true\n", 199),
            ("> ", 0), ("tools", 199), ("::*\nmain = -> ", 0), (name, context), ("!\n", 0),
        ]);
        let result = Lowerer::new_for_test().lower_program(&ast::Program { module }).and_then(crate::infer::finalize);
        assert_eq!(result.is_ok(), accepted, "{name} in {context}");
    }
}

#[test]
fn macro_a_13_definition_scope_uses_its_import_alias_not_caller_binding() {
    for import in ["> support::answer", "> support::*"] {
        let mut root = module(&[
            ("mod support\n    < answer = -> true\nmod tools\n    ", 0), (import, 0),
            ("\nanswer = -> 1\nmain = -> ", 0), ("answer", 211), ("!\n", 0),
        ]);
        root.filepath = Some("/virtual/macro-test.rk".into());
        let program = ast::Program { module: root };
        let crates = crate::crate_system::CrateContext::new();
        let mut environments = crate::macro_expansion::binding_environment::BindingEnvironments::default();
        environments.record_source_scope(super::SyntaxContext(211), vec![("tools".into(), super::SyntaxContext::ROOT)]).unwrap();
        let declarations = crate::collect::collect_with_binding_environments(&program, None, &crates, false, None, &environments).unwrap();
        let partial = crate::lower::pipeline::LoweringPipeline::new(&program, &crates, None).lower_from_declarations(declarations).unwrap();
        let resolved = crate::infer::finalize(partial).unwrap();
        let main = resolved.program.functions.values().find(|item| item.name == "main").unwrap();
        assert_eq!(main.ret_type, Type::Bool);
    }
}

#[test]
fn macro_a_13_contextual_import_conflicts_and_unknown_globs_reject() {
    for source in [
        "mod tools\n    < answer = -> true\nanswer = -> 1\n> tools::answer\nmain = !-> ()\n",
        "> missing::*\nmain = !-> ()\n",
    ] {
        let module = module(&[(source, 223)]);
        assert!(Lowerer::new_for_test().lower_program(&ast::Program { module }).is_err());
    }
}
