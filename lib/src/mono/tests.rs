use super::*;
use super::{
    CompilationIdentityContext, InstanceImplOwner, InstanceKey, InstanceOrigin, MonoError,
    MonoErrorKind, Monomorphizer,
};

use crate::diagnostic::{DiagnosticCode, DiagnosticLocation};
use crate::hir::{
    AcceptedHirProgram, HirAssociatedTypeDef, HirImplOwner, HirImplReceiverPattern, HirParam,
};
use crate::ids::{AssocTypeId, CrateId, DefId, HirLocalId, LocalDefId, TypeId};
use crate::mono::hir_types::{HirBlock, HirFunction, HirImpl, HirProgram};
use crate::products::{ProductCrateIdentity, ProductSourceFingerprint};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{GenericParamDecl, GenericParamId, NominalTypeKind, Type};

use std::collections::HashMap;

#[test]
fn mono_receiver_diagnostic_uses_user_type_names_and_mono_code() {
    let span = crate::lexer::Span::new("/virtual/main.rk".into(), 2, 7);
    let receiver = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(7)),
        args: vec![Type::I64],
    };
    let diagnostic = MonoError {
        kind: MonoErrorKind::ReceiverMismatch {
            impl_id: DefId::new(CrateId(0), LocalDefId(8)),
            receiver,
            pattern: None,
        },
        span: span.clone(),
    }
    .diagnostic(&crate::collect::resolver::ResolverTables::default());

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Mono));
    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert!(!diagnostic.message.contains("struct#"));
    assert!(!diagnostic.message.contains("DefId"));
}

#[test]
fn monomorphizer_display_context_renders_generic_projection_names() {
    let mut mono = Monomorphizer::new();
    let function_id = DefId::new(CrateId(0), LocalDefId(81));
    mono.generic_functions.insert(
        function_id,
        test_function(function_id, "identity", vec!["T".to_string()]),
    );

    let trait_id = DefId::new(CrateId(0), LocalDefId(82));
    mono.resolver
        .item_names_by_id
        .insert(trait_id, "Iterator".to_string());
    let impl_id = DefId::new(CrateId(0), LocalDefId(83));
    let impl_generic = GenericParamDecl::type_param(
        GenericParamId {
            owner: impl_id,
            index: 0,
        },
        "T",
    );
    let mut impl_def = concrete_impl_with_methods(impl_id, &[]);
    impl_def.type_generics = vec![impl_generic];
    impl_def.trait_id = Some(trait_id);
    impl_def.associated_types = vec![HirAssociatedTypeDef {
        id: AssocTypeId(0),
        name: "Item".to_string(),
        kind: Kind::Type,
        ty: Type::I64,
    }];
    mono.generic_impls.insert(impl_id, impl_def);

    let generic = Type::Generic(GenericParamId {
        owner: impl_id,
        index: 0,
    });
    assert_eq!(mono.display_type_for_diagnostic(&generic), "T");
    let projection = Type::Projection {
        ty: Box::new(generic),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    assert_eq!(
        mono.display_type_for_diagnostic(&projection),
        "<T as Iterator>::Item"
    );
}

fn test_function(id: DefId, name: &str, generic_params: Vec<String>) -> HirFunction {
    let generic_params = generic_params
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: index as u32,
                },
                name,
            )
        })
        .collect();

    HirFunction {
        id,
        name: name.to_string(),
        generic_params,
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn test_method(id: DefId, name: &str) -> HirFunction {
    let mut method = test_function(id, name, Vec::new());
    method.is_method = true;
    method
}

fn concrete_function_with_param(id: DefId, name: &str, ty: Type) -> HirFunction {
    let mut function = test_function(id, name, Vec::new());
    function.params = vec![HirParam {
        name: "value".to_string(),
        local_id: HirLocalId(0),
        ty,
        mutable: false,
        is_ref: false,
    }];
    function
}

fn monomorphize_functions_in_order(order: &[DefId]) -> (Vec<InstanceOrigin>, Vec<Type>) {
    let first_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_id = DefId::new(CrateId(0), LocalDefId(20));
    let mut functions = HashMap::new();
    for id in order {
        let function = match *id {
            id if id == first_id => concrete_function_with_param(id, "first", Type::I64),
            id if id == second_id => concrete_function_with_param(id, "second", Type::Bool),
            _ => panic!("unexpected fixture DefId"),
        };
        functions.insert(*id, function);
    }
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        functions,
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        crate::hir::HirNameTables::default(),
        &HashMap::from([
            (first_id, "first".to_string()),
            (second_id, "second".to_string()),
        ]),
    );
    let program = AcceptedHirProgram::revalidate_for_test(program)
        .expect("mono fixture must satisfy accepted HIR invariants");
    let monomorphized = monomorphize_for_tests(program).expect("monomorphization succeeds");

    let origins = monomorphized
        .instances
        .values()
        .map(|record| record.origin.clone())
        .collect();
    let types = (0..monomorphized.type_context.len())
        .map(|index| monomorphized.type_context.type_for(TypeId(index as u32)))
        .collect();
    (origins, types)
}

fn concrete_impl_with_methods(id: DefId, method_ids: &[DefId]) -> HirImpl {
    let methods = method_ids
        .iter()
        .map(|method_id| {
            (
                format!("method_{}", method_id.local.0),
                test_method(*method_id, "method"),
            )
        })
        .collect();
    HirImpl {
        id,
        owner: HirImplOwner::Named(format!("Impl_{}", id.local.0)),
        type_name: format!("Impl_{}", id.local.0),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods,
    }
}

fn monomorphize_impls_in_order(order: &[DefId], first_method_ids: &[DefId]) -> Vec<InstanceOrigin> {
    let first_impl_id = DefId::new(CrateId(0), LocalDefId(30));
    let second_impl_id = DefId::new(CrateId(0), LocalDefId(40));
    let mut impls = HashMap::new();
    for id in order {
        let imp = match *id {
            id if id == first_impl_id => concrete_impl_with_methods(id, first_method_ids),
            id if id == second_impl_id => {
                concrete_impl_with_methods(id, &[DefId::new(CrateId(0), LocalDefId(41))])
            }
            _ => panic!("unexpected fixture DefId"),
        };
        impls.insert(*id, imp);
    }
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        impls,
        HashMap::new(),
        crate::hir::HirNameTables::default(),
        &HashMap::new(),
    );
    let program = AcceptedHirProgram::revalidate_for_test(program)
        .expect("mono fixture must satisfy accepted HIR invariants");
    monomorphize_for_tests(program)
        .expect("monomorphization succeeds")
        .instances
        .values()
        .map(|record| record.origin.clone())
        .collect()
}

#[test]
fn backend_symbol_for_origin_uses_ids_not_display_names() {
    let function_id = DefId::new(CrateId(0), LocalDefId(42));
    let origin = InstanceOrigin::Function(function_id);
    let mut mono = Monomorphizer::new();
    let symbol = mono.backend_symbol_for_origin(&origin, &[]);

    assert!(symbol.starts_with("__rock_fn_d42_h"));
    assert!(symbol.ends_with("_none"));
}

fn product_identity(name: &str, source_hash: &str) -> ProductCrateIdentity {
    ProductCrateIdentity::local(name.to_string()).with_source_fingerprint(
        ProductSourceFingerprint {
            manifest_hash: None,
            source_hash: Some(source_hash.to_string()),
            loaded_files: vec!["src/main.rk".into()],
        },
    )
}

#[test]
fn backend_symbol_fingerprint_ignores_type_id_allocation_order() {
    let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(1)));
    let ty = Type::Enum {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        args: vec![Type::I64],
    };
    let mut first = Monomorphizer::new();
    first.intern_type(&Type::Bool);
    let first_ty = first.intern_type(&ty);
    let mut second = Monomorphizer::new();
    second.intern_type(&Type::U8);
    second.intern_type(&Type::Bool);
    let second_ty = second.intern_type(&ty);

    assert_ne!(first_ty, second_ty);
    assert_eq!(
        first.backend_symbol_for_origin(&origin, &[first_ty]),
        second.backend_symbol_for_origin(&origin, &[second_ty])
    );
}

#[test]
fn backend_symbol_fingerprint_ignores_dependency_load_order() {
    let current = product_identity("app", "app-hash");
    let dependency = product_identity("dep", "dep-hash");
    let mut first = Monomorphizer::new();
    first.identity_context = CompilationIdentityContext::new(
        CrateId(0),
        current.clone(),
        std::collections::BTreeMap::from([(CrateId(1), dependency.clone())]),
    );
    let mut second = Monomorphizer::new();
    second.identity_context = CompilationIdentityContext::new(
        CrateId(0),
        current,
        std::collections::BTreeMap::from([(CrateId(9), dependency)]),
    );
    let first_ty = first.intern_type(&Type::Struct {
        id: DefId::new(CrateId(1), LocalDefId(7)),
        args: vec![],
    });
    let second_ty = second.intern_type(&Type::Struct {
        id: DefId::new(CrateId(9), LocalDefId(7)),
        args: vec![],
    });
    let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(3)));

    assert_eq!(
        first.backend_symbol_for_origin(&origin, &[first_ty]),
        second.backend_symbol_for_origin(&origin, &[second_ty])
    );
}

#[test]
fn backend_symbol_fingerprint_delimits_nested_nominal_arguments() {
    let outer = DefId::new(CrateId(0), LocalDefId(20));
    let inner = DefId::new(CrateId(0), LocalDefId(21));
    let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(22)));
    let mut mono = Monomorphizer::new();
    let siblings = mono.intern_type(&Type::Struct {
        id: outer,
        args: vec![
            Type::Struct {
                id: inner,
                args: vec![],
            },
            Type::I64,
        ],
    });
    let nested = mono.intern_type(&Type::Struct {
        id: outer,
        args: vec![Type::Struct {
            id: inner,
            args: vec![Type::I64],
        }],
    });

    assert_ne!(
        mono.backend_symbol_for_origin(&origin, &[siblings]),
        mono.backend_symbol_for_origin(&origin, &[nested])
    );
}

#[test]
fn constructor_instance_keys_distinguish_families_and_reuse_eta_equivalents() {
    let option = DefId::new(CrateId(0), LocalDefId(10));
    let vector = DefId::new(CrateId(0), LocalDefId(11));
    let unary = Kind::arrow(Kind::Type, Kind::Type);
    let mut env = TypeNormalizationEnv::new();
    env.register_constructor(option, NominalTypeKind::Enum, unary.clone());
    env.register_constructor(vector, NominalTypeKind::Struct, unary);
    let mut mono = Monomorphizer::new();
    mono.normalization_env = env;
    let option_ty = Type::Constructor {
        id: option,
        flavor: NominalTypeKind::Enum,
    };
    let option_id = mono.intern_type(&option_ty);
    let eta_option_id = mono.intern_type(&Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Apply {
            constructor: Box::new(option_ty),
            args: vec![Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            }],
        }),
    });
    let vector_id = mono.intern_type(&Type::Constructor {
        id: vector,
        flavor: NominalTypeKind::Struct,
    });
    let origin = InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(12)));

    assert_eq!(option_id, eta_option_id);
    assert_eq!(
        InstanceKey::new(origin.clone(), vec![option_id]),
        InstanceKey::new(origin.clone(), vec![eta_option_id])
    );
    assert_ne!(
        InstanceKey::new(origin, vec![option_id]),
        InstanceKey::new(
            InstanceOrigin::Function(DefId::new(CrateId(0), LocalDefId(12))),
            vec![vector_id]
        )
    );
}

#[test]
fn monomorphize_processes_alias_duplicate_function_once_by_def_id() {
    let id = DefId::new(CrateId(0), LocalDefId(1));
    let program = HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
        HashMap::from([(id, test_function(id, "canonical", Vec::new()))]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        crate::hir::HirNameTables {
            functions_by_name: HashMap::from([
                ("alias".to_string(), id),
                ("canonical".to_string(), id),
            ]),
            ..crate::hir::HirNameTables::default()
        },
        &HashMap::from([(id, "canonical".to_string())]),
    );

    let program = AcceptedHirProgram::revalidate_for_test(program)
        .expect("mono fixture must satisfy accepted HIR invariants");
    let monomorphized = monomorphize_for_tests(program).expect("monomorphization succeeds");

    assert_eq!(monomorphized.program.functions.len(), 1);
    assert!(monomorphized.program.functions.contains_key(&id));
    assert!(monomorphized
        .program
        .names
        .functions_by_name
        .contains_key("canonical"));
    assert!(!monomorphized
        .program
        .names
        .functions_by_name
        .contains_key("alias"));
    assert_eq!(
        monomorphized.program.indexes.functions_by_id.get(&id),
        Some(&"canonical".to_string())
    );
}

#[test]
fn monomorphize_allocates_top_level_instances_and_types_by_def_id() {
    let first_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_id = DefId::new(CrateId(0), LocalDefId(20));
    let expected_origins = vec![
        InstanceOrigin::Function(first_id),
        InstanceOrigin::Function(second_id),
    ];
    let expected_types = vec![Type::I64, Type::Bool];

    for order in [[first_id, second_id], [second_id, first_id]] {
        for _ in 0..16 {
            let (origins, types) = monomorphize_functions_in_order(&order);
            assert_eq!(origins, expected_origins);
            assert_eq!(types, expected_types);
        }
    }
}

#[test]
fn monomorphize_allocates_impl_method_instances_by_impl_and_method_def_id() {
    let first_impl_id = DefId::new(CrateId(0), LocalDefId(30));
    let second_impl_id = DefId::new(CrateId(0), LocalDefId(40));
    let first_method_id = DefId::new(CrateId(0), LocalDefId(31));
    let second_method_id = DefId::new(CrateId(0), LocalDefId(32));
    let third_method_id = DefId::new(CrateId(0), LocalDefId(41));
    let expected = vec![
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(first_impl_id),
            method: first_method_id,
        },
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(first_impl_id),
            method: second_method_id,
        },
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(second_impl_id),
            method: third_method_id,
        },
    ];

    for impl_order in [
        [first_impl_id, second_impl_id],
        [second_impl_id, first_impl_id],
    ] {
        for method_order in [
            [first_method_id, second_method_id],
            [second_method_id, first_method_id],
        ] {
            assert_eq!(
                monomorphize_impls_in_order(&impl_order, &method_order),
                expected
            );
        }
    }
}

fn generic_impl(owner: &str, type_name: &str, id: DefId) -> HirImpl {
    HirImpl {
        id,
        owner: HirImplOwner::Named(owner.to_string()),
        type_name: type_name.to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
            index: 0,
        })]
        .into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    }
}

#[test]
fn collect_impls_preserves_same_short_name_generic_impls_by_owner() {
    let mut mono = Monomorphizer::new();
    let first_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_id = DefId::new(CrateId(0), LocalDefId(11));

    mono.collect_impls(&[
        generic_impl("a::Foo", "Foo", first_id),
        generic_impl("b::Foo", "Foo", second_id),
    ]);

    assert_eq!(mono.generic_impls.len(), 2);
}

#[test]
fn receiver_type_match_uses_impl_owner_identity_for_nominals() {
    let mono = Monomorphizer::new();
    let first_id = DefId::new(CrateId(0), LocalDefId(10));
    let second_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut imp = generic_impl("b::Foo", "Foo", second_id);
    imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Struct {
        id: second_id,
        args: vec![Type::Generic(crate::types::GenericParamId {
            owner: second_id,
            index: 0,
        })],
    });
    let recv_ty = Type::Struct {
        id: first_id,
        args: vec![Type::I64],
    };

    assert!(!mono.impl_receiver_pattern_matches(&imp, &recv_ty));
}

#[test]
fn receiver_type_match_accepts_artifact_owner_alias_for_canonical_receiver_path() {
    let mono = Monomorphizer::new();
    let option_id = DefId::new(CrateId(1), LocalDefId(10));
    let mut imp = generic_impl("std::Option", "Option", option_id);
    imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Enum {
        id: option_id,
        args: vec![Type::Generic(crate::types::GenericParamId {
            owner: option_id,
            index: 0,
        })],
    });
    let recv_ty = Type::Enum {
        id: option_id,
        args: vec![Type::I64],
    };

    assert!(mono.impl_receiver_pattern_matches(&imp, &recv_ty));
}

#[test]
fn receiver_type_match_rejects_ambiguous_artifact_owner_alias() {
    let mono = Monomorphizer::new();
    let first_id = DefId::new(CrateId(1), LocalDefId(10));
    let second_id = DefId::new(CrateId(1), LocalDefId(11));
    let mut imp = generic_impl("dep::Box", "Box", second_id);
    imp.receiver_pattern = HirImplReceiverPattern::Exact(Type::Struct {
        id: second_id,
        args: vec![Type::Generic(crate::types::GenericParamId {
            owner: second_id,
            index: 0,
        })],
    });
    let recv_ty = Type::Struct {
        id: first_id,
        args: vec![Type::I64],
    };

    assert!(!mono.impl_receiver_pattern_matches(&imp, &recv_ty));
}

#[test]
fn builtin_slice_owner_identity_is_structural() {
    let mono = Monomorphizer::new();
    let method_id = DefId::new(CrateId(0), LocalDefId(1));
    let method = test_method(method_id, "println");
    let imp = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[T]".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: DefId::new(CrateId(0), LocalDefId(0)),
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
            index: 0,
        })]
        .into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(20))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };

    assert_eq!(
        mono.method_instance_origin(&imp, &method),
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::BuiltinSlice,
            method: method_id,
        }
    );
}

#[test]
fn named_impl_method_origin_uses_impl_and_method_def_ids() {
    let mut mono = Monomorphizer::new();
    let foo_type_id = DefId::new(CrateId(0), LocalDefId(10));
    let display_impl_id = DefId::new(CrateId(0), LocalDefId(11));
    let debug_impl_id = DefId::new(CrateId(0), LocalDefId(12));
    let display_method_id = DefId::new(CrateId(0), LocalDefId(13));
    let debug_method_id = DefId::new(CrateId(0), LocalDefId(14));
    let display_method = test_method(display_method_id, "fmt");
    let debug_method = test_method(debug_method_id, "fmt");
    mono.resolver
        .item_paths
        .insert("Foo".to_string(), foo_type_id);

    let display_impl = HirImpl {
        id: display_impl_id,
        owner: HirImplOwner::Named("Foo".to_string()),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: Some("Display".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(30))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };
    let debug_impl = HirImpl {
        id: debug_impl_id,
        owner: HirImplOwner::Named("Foo".to_string()),
        type_name: "Foo".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: Some("Debug".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(31))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::new(),
    };

    assert_eq!(
        mono.method_instance_origin(&display_impl, &display_method),
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(display_impl_id),
            method: display_method_id,
        }
    );
    assert_eq!(
        mono.method_instance_origin(&debug_impl, &debug_method),
        InstanceOrigin::ImplMethod {
            owner: InstanceImplOwner::Named(debug_impl_id),
            method: debug_method_id,
        }
    );
}

pub(crate) fn monomorphize_for_tests(
    program: AcceptedHirProgram,
) -> Result<MonomorphizedProgram, Diagnostics> {
    monomorphize(program, SemanticSourceMap::default())
}
