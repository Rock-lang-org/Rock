use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir};
use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{
    HirCallTarget, HirImplOwner, HirMethodCallTarget, HirParam, HirVarRef, HirVarTarget,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lexer::Span;
use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirStmt};
use crate::types::Type;
use crate::types::{GenericParamDecl, ReceiverMode};

use super::*;

fn println_method(receiver_ty: Type) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "println".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: receiver_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn concrete_function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: ret_type,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn loaded_object_backed_crate(
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
) -> ExternCrateRecord {
    loaded_object_backed_crate_with_backend_symbols(
        interface_impls,
        cross_crate_impls,
        BTreeMap::new(),
    )
}

fn loaded_object_backed_crate_with_backend_symbols(
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(1),
        "std",
        interface_impls,
        cross_crate_impls,
        backend_symbols,
    )
}

fn loaded_object_backed_crate_named_with_backend_symbols(
    crate_id: CrateId,
    crate_name: &str,
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    let interface_impls = interface_impls;

    let mut resolver = ResolverTables::default();
    let mut next_local_id = 0u32;
    for imp in &interface_impls {
        let owner_name = format!("{}::{}", crate_name, imp.type_name);
        let owner_def_id = DefId::new(CrateId(0), LocalDefId(next_local_id));
        next_local_id += 1;
        resolver.item_paths.insert(owner_name.clone(), owner_def_id);
        resolver.item_names_by_id.insert(owner_def_id, owner_name);

        for method in imp.methods.values() {
            let Some(backend_name) = backend_symbols.get(&method.id).cloned() else {
                continue;
            };
            let def_id = DefId::new(CrateId(0), LocalDefId(next_local_id));
            next_local_id += 1;
            let canonical_name = format!("{}::{}", crate_name, backend_name);
            resolver.item_paths.insert(canonical_name.clone(), def_id);
            resolver.item_names_by_id.insert(def_id, canonical_name);
        }
    }

    let mut interface = ArtifactCrateInterface::default();
    for imp in interface_impls {
        interface.insert_impl(imp);
    }

    ExternCrateRecord::new(
        crate_id,
        crate_name.to_string(),
        ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: cross_crate_impls,
        }),
        ExternCrateLink::object(PathBuf::from("std.o"), backend_symbols),
    )
}

fn simple_hir_function(name: &str, _canonical_name: Option<&str>, id: DefId) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn loaded_object_backed_function_crate_with_backend_symbols(
    crate_name: &str,
    canonical_name: &str,
    func: HirFunction,
    object_path: PathBuf,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    let mut interface = ArtifactCrateInterface::default();
    interface.insert_function(canonical_name.to_string(), func);

    let mut resolver = ResolverTables::default();
    let def_id = DefId::new(CrateId(0), LocalDefId(0));
    resolver
        .item_paths
        .insert(canonical_name.to_string(), def_id);
    resolver
        .item_names_by_id
        .insert(def_id, canonical_name.to_string());

    ExternCrateRecord::new(
        CrateId(1),
        crate_name.to_string(),
        ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: vec![],
        }),
        ExternCrateLink::object(object_path, backend_symbols),
    )
}

#[test]
fn process_with_crates_uses_link_provider_for_object_backed_functions() {
    let answer_id = DefId::new(CrateId(0), LocalDefId(0));
    let func = simple_hir_function("answer", Some("dep::answer"), answer_id);
    let loaded = loaded_object_backed_function_crate_with_backend_symbols(
        "dep",
        "dep::answer",
        func,
        PathBuf::from("dep.o"),
        BTreeMap::from([(answer_id, "dep::answer".to_string())]),
    );
    let mut crate_ctx = CrateContext::new();
    crate_ctx.add_extern_crate(loaded).unwrap();

    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &crate_ctx);

    assert!(output
        .instances
        .values()
        .any(|record| record.symbols.backend_symbol == "dep::answer" && record.provided_by_object));
}

#[test]
fn process_with_crates_uses_artifact_backend_symbols_for_object_backed_functions() {
    let answer_id = DefId::new(CrateId(0), LocalDefId(7));
    let func = simple_hir_function("answer", Some("dep::answer"), answer_id);
    let loaded = loaded_object_backed_function_crate_with_backend_symbols(
        "dep",
        "dep::answer",
        func,
        PathBuf::from("dep.o"),
        BTreeMap::from([(answer_id, "artifact_dep_answer".to_string())]),
    );
    let mut crate_ctx = CrateContext::new();
    crate_ctx.add_extern_crate(loaded).unwrap();

    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &crate_ctx);

    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed function instance should be recorded");
    assert_eq!(record.symbols.source_name, "dep::answer");
    assert_eq!(record.symbols.backend_symbol, "artifact_dep_answer");
}

#[test]
fn test_load_external_generic_functions_keeps_concrete_object_backed_trait_impls() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let byte_slice_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::U8))),
    };

    let concrete_method_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut concrete_method = println_method(byte_slice_ty.clone());
    concrete_method.id = concrete_method_id;
    let concrete_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), concrete_method)]),
    };
    let generic_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::BuiltinSlice,
        type_name: "Array".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
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
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "println".to_string(),
            println_method(Type::Slice(Box::new(Type::Generic(
                crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                },
            )))),
        )]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl.clone()],
        vec![generic_impl],
        BTreeMap::from([(concrete_method_id, "&[U8]_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    let show_impls = mono.trait_impls.get(&trait_id).unwrap();
    assert!(show_impls.iter().any(|imp| imp.type_name == "&[U8]"));

    let recv = HirExpr {
        kind: HirExprKind::Var("bytes".to_string()),
        ty: byte_slice_ty,
        span: Span::test(),
    };
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                DefId::new(CrateId(0), LocalDefId(999)),
                DefId::new(CrateId(0), LocalDefId(998)),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: Type::I32,
        span: Span::test(),
    };

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    assert!(matches!(expr.kind, HirExprKind::MethodCall(..)));
}

#[test]
fn load_external_generic_functions_keeps_generic_trait_impls_out_of_inherent_table() {
    let generic_trait_impl = HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(0)),
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
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "println".to_string(),
            println_method(Type::Struct {
                id: DefId::new(CrateId(0), LocalDefId(10)),
                args: vec![Type::Generic(crate::types::GenericParamId {
                    owner: crate::ids::DefId::new(
                        crate::ids::CrateId(0),
                        crate::ids::LocalDefId(0),
                    ),
                    index: 0,
                })],
            }),
        )]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(vec![], vec![generic_trait_impl]))
        .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    assert!(!mono
        .generic_impls
        .values()
        .any(|imp| imp.type_name == "Box"));
    assert!(mono
        .trait_impls
        .get(&DefId::new(CrateId(0), LocalDefId(99)))
        .is_some_and(|impls| impls.iter().any(|imp| imp.type_name == "Box")));
}

#[test]
fn load_external_generic_functions_ignores_module_aliases_for_generic_functions() {
    let function_id = DefId::new(CrateId(0), LocalDefId(80));
    let generic_param = crate::types::GenericParamId {
        owner: function_id,
        index: 0,
    };
    let mut function = simple_hir_function("identity", Some("dep::generic::identity"), function_id);
    function.generic_params = vec![GenericParamDecl::type_param(generic_param, "T")];
    function.ret_type = Type::Generic(generic_param);
    function.body.ty = Type::Generic(generic_param);
    let canonical_name = "dep::generic::identity".to_string();

    let mut resolver = ResolverTables::default();
    resolver
        .item_paths
        .insert(canonical_name.clone(), function_id);
    resolver
        .item_names_by_id
        .insert(function_id, canonical_name.clone());
    resolver
        .module_aliases
        .insert("identity".to_string(), function_id);
    let loaded = ExternCrateRecord::new(
        CrateId(1),
        "dep".to_string(),
        ExternCrateMetadata::new(ArtifactCrateInterface::default(), resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::from([(function_id, function)]),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: Vec::new(),
        }),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    );
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded).unwrap();
    let mut mono = Monomorphizer::new();

    mono.load_external_generic_functions(&ctx);

    assert!(mono.generic_functions.contains_key(&function_id));
}

#[test]
fn load_external_generic_functions_skips_concrete_object_backed_inherent_impls() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(70));
    let method_id = DefId::new(CrateId(0), LocalDefId(71));
    let mut method = simple_hir_function("from_str", Some("std::String_from_str"), method_id);
    method.params = vec![HirParam {
        name: "s".to_string(),
        local_id: crate::ids::HirLocalId(0),
        ty: Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        },
        mutable: false,
        is_ref: false,
    }];
    method.ret_type = Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(72)),
        args: Vec::new(),
    };
    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("String".to_string()),
        type_name: "String".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("from_str".to_string(), method)]),
    };
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(
        vec![concrete_impl.clone()],
        vec![concrete_impl],
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    mono.load_external_generic_functions(&ctx);

    assert!(!mono
        .generic_impls
        .values()
        .any(|imp| imp.type_name == "String"));
}

fn wrapped_process_with_crates(
    mono: &mut Monomorphizer,
    program: crate::mono::hir_types::HirProgram,
    crate_ctx: &CrateContext,
) -> crate::mono::MonomorphizedProgram {
    let program = mono.process_with_crates(program, crate_ctx);
    let (instances, pre_mir_instance_bodies) =
        std::mem::replace(&mut mono.instances, crate::mono::InstanceRegistry::new()).into_parts();
    crate::mono::MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context: mono.type_context.clone(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    }
}

#[test]
fn process_with_crates_records_current_crate_concrete_function_instance() {
    let function_id = DefId::new(CrateId(0), LocalDefId(30));
    let function = concrete_function(function_id, "main", Type::I32);
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::from([(function_id, function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: HashMap::from([("main".to_string(), function_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([(function_id, "main".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_paths
        .insert("main".to_string(), function_id);

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| record.origin == crate::mono::InstanceOrigin::Function(function_id))
        .expect("current-crate concrete function should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "main");
    assert_eq!(record.symbols.backend_symbol, "main");
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_records_current_crate_concrete_impl_method_instance() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(40));
    let method_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut method = println_method(Type::I64);
    method.id = method_id;
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, imp)]),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::from([(impl_id, "Box".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::ImplMethod {
                    owner: crate::mono::InstanceImplOwner::Named(impl_id),
                    method: method_id,
                }
        })
        .expect("current-crate concrete impl method should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "Box::println");
    assert!(record
        .symbols
        .backend_symbol
        .starts_with("__rock_impl_d40_h"));
    assert!(record.symbols.backend_symbol.ends_with("_none"));
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_does_not_eagerly_emit_dependency_generic_impl_methods() {
    let impl_id = DefId::new(CrateId(1), LocalDefId(10));
    let method_id = DefId::new(CrateId(1), LocalDefId(11));
    let mut method = concrete_function(method_id, "capacity", Type::I32);
    method.is_method = true;
    let generic_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Vec".to_string()),
        type_name: "Vec".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("capacity".to_string(), method)]),
    };
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(
        Vec::new(),
        vec![generic_impl.clone()],
    ))
    .unwrap();
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(impl_id, generic_impl)]),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::from([(impl_id, "Vec".to_string())]),
        );

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    assert!(
        output.instances.is_empty(),
        "dependency generic impl methods should specialize only when called: {:?}",
        output.instances
    );
}

#[test]
fn process_with_crates_does_not_emit_unused_dependency_drop_glue_for_interned_types() {
    let drop_trait_id = DefId::new(CrateId(1), LocalDefId(1));
    let impl_id = DefId::new(CrateId(1), LocalDefId(10));
    let method_id = DefId::new(CrateId(1), LocalDefId(11));
    let box_id = DefId::new(CrateId(1), LocalDefId(20));
    let generic = crate::types::GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let box_generic = Type::Struct {
        id: box_id,
        args: vec![Type::Generic(generic)],
    };
    let drop_method = HirFunction {
        id: method_id,
        name: "drop".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: box_generic.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let drop_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(generic, "T")],
        receiver_pattern: vec![Type::Generic(generic)].into(),
        trait_name: Some("Drop".to_string()),
        trait_id: Some(drop_trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("drop".to_string(), drop_method)]),
    };
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(box_id, "std::Box".to_string());
    let loaded = ExternCrateRecord::new(
        CrateId(1),
        "std".to_string(),
        ExternCrateMetadata::new(ArtifactCrateInterface::default(), resolver, BTreeMap::new())
            .with_language_items(crate::hir::HirLanguageItems {
                drop: Some(crate::language_items::DropLanguageItems {
                    trait_id: drop_trait_id,
                    method_id,
                }),
                ..crate::hir::HirLanguageItems::default()
            }),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: vec![drop_impl],
        }),
        ExternCrateLink::object(PathBuf::from("std.o"), BTreeMap::new()),
    );
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded).unwrap();
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        );

    let mut mono = Monomorphizer::new();
    mono.type_context.intern_type(&Type::Struct {
        id: box_id,
        args: vec![Type::I32],
    });
    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    assert!(
        output.instances.is_empty(),
        "unused dependency Drop impls should not be emitted from the global type table: {:?}",
        output.instances
    );
}

#[test]
fn process_with_crates_records_concrete_trait_default_instance() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(50));
    let method_id = DefId::new(CrateId(0), LocalDefId(51));
    let default_method = concrete_function(method_id, "default_value", Type::I32);
    let trait_def = crate::hir::HirTraitFor::<crate::hir::AcceptedHir> {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Provider".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default_value".to_string(), default_method)]),
        signatures: HashMap::new(),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                traits_by_name: HashMap::from([("Provider".to_string(), trait_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([(trait_id, "Provider".to_string())]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::TraitDefault {
                    trait_id,
                    method: method_id,
                }
        })
        .expect("concrete trait default should be an instance");
    assert!(record.substitution.is_empty());
    assert_eq!(record.symbols.source_name, "Provider::default_value");
    assert!(record
        .symbols
        .backend_symbol
        .starts_with("__rock_trait_default_d50_h"));
    assert!(record.symbols.backend_symbol.ends_with("_none"));
    assert!(output.pre_mir_instance_bodies.contains(record.id));
    assert!(!record.provided_by_object);
}

#[test]
fn process_with_crates_processes_concrete_trait_default_body_before_registration() {
    let generic_id = DefId::new(CrateId(0), LocalDefId(60));
    let generic_param_id = crate::types::GenericParamId {
        owner: generic_id,
        index: 0,
    };
    let generic_type = Type::Generic(generic_param_id);
    let generic_function = HirFunction {
        id: generic_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_param_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: generic_type.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: generic_type.clone(),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: generic_type.clone(),
                span: Span::test(),
            }))],
            ty: generic_type,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };

    let trait_id = DefId::new(CrateId(0), LocalDefId(61));
    let method_id = DefId::new(CrateId(0), LocalDefId(62));
    let mut default_method = concrete_function(method_id, "default_value", Type::I32);
    default_method.body = HirBlock {
        stmts: vec![HirStmt::Return(Some(HirExpr {
            kind: HirExprKind::Call(
                Box::new(HirExpr {
                    kind: HirExprKind::ResolvedVar(HirVarRef {
                        name: "identity".to_string(),
                        target: HirVarTarget::Function(generic_id),
                    }),
                    ty: Type::function(vec![Type::I32], Type::I32),
                    span: Span::test(),
                }),
                vec![HirExpr {
                    kind: HirExprKind::IntLiteral(7),
                    ty: Type::I32,
                    span: Span::test(),
                }],
                Some(HirCallTarget::Function(generic_id)),
            ),
            ty: Type::I32,
            span: Span::test(),
        }))],
        ty: Type::I32,
    };
    let trait_def = crate::hir::HirTraitFor::<crate::hir::AcceptedHir> {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "Provider".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([("default_value".to_string(), default_method)]),
        signatures: HashMap::new(),
    };
    let program =
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::from([(generic_id, generic_function)]),
            HashMap::new(),
            HashMap::new(),
            HashMap::from([(trait_id, trait_def)]),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables {
                functions_by_name: HashMap::from([("identity".to_string(), generic_id)]),
                traits_by_name: HashMap::from([("Provider".to_string(), trait_id)]),
                ..crate::hir::HirNameTables::default()
            },
            &HashMap::from([
                (generic_id, "identity".to_string()),
                (trait_id, "Provider".to_string()),
            ]),
        );
    let ctx = CrateContext::new();
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_paths
        .insert("identity".to_string(), generic_id);

    let output = wrapped_process_with_crates(&mut mono, program, &ctx);

    let trait_record = output
        .instances
        .values()
        .find(|record| {
            record.origin
                == crate::mono::InstanceOrigin::TraitDefault {
                    trait_id,
                    method: method_id,
                }
        })
        .expect("concrete trait default should be an instance");
    let trait_body = output
        .pre_mir_instance_bodies
        .get(trait_record.id)
        .expect("trait default has body");
    let HirStmt::Return(Some(returned)) = &trait_body.body.stmts[0] else {
        panic!("trait default should return the generic call");
    };
    let HirExprKind::Call(callee, _, _) = &returned.kind else {
        panic!("trait default return should remain a call");
    };
    let HirExprKind::ResolvedVar(reference) = &callee.kind else {
        panic!("trait default call callee should be a resolved instance target");
    };
    let instance_id = match reference.target {
        HirVarTarget::Instance(instance_id) => instance_id,
        HirVarTarget::Function(id) if id == generic_id => output
            .instances
            .values()
            .find(|record| record.origin == crate::mono::InstanceOrigin::Function(generic_id))
            .map(|record| record.id)
            .expect("generic call specialization should be registered"),
        ref target => panic!("trait default call callee has unexpected target {target:?}"),
    };
    assert_eq!(reference.name, "identity");

    let specialization = output
        .instances
        .get(&instance_id)
        .expect("trait default should register the generic specialization it calls");
    assert_eq!(
        specialization.origin,
        crate::mono::InstanceOrigin::Function(generic_id)
    );
    assert_eq!(specialization.substitution.len(), 1);
    assert_eq!(
        output.type_context.type_for(specialization.substitution[0]),
        Type::I32
    );
    assert_eq!(
        specialization.symbols.backend_symbol,
        mono.backend_symbol_for_origin(&specialization.origin, &specialization.substitution)
    );
}

#[test]
fn process_with_crates_records_object_backed_instances_without_re_emitting() {
    let byte_slice_ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::U8))),
    };
    let impl_id = DefId::new(CrateId(0), LocalDefId(0));
    let method_id = DefId::new(CrateId(0), LocalDefId(1));
    let mut method = println_method(byte_slice_ty);
    method.id = method_id;

    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::BuiltinSlice,
        type_name: "&[U8]".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![Type::U8].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl],
        vec![],
        BTreeMap::from([(method_id, "&[U8]_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    assert!(output
        .instances
        .values()
        .any(|record| record.provided_by_object));
    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed method instance should be recorded");
    assert_eq!(
        record.origin,
        crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::BuiltinSlice,
            method: method_id,
        }
    );
    assert_eq!(record.symbols.backend_symbol, "&[U8]_println");
    assert!(!output
        .program
        .names
        .functions_by_name
        .keys()
        .any(|name| name.starts_with("std::")));
}

#[test]
fn process_with_crates_records_concrete_method_from_mixed_object_backed_impl() {
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let impl_id = DefId::new(CrateId(0), LocalDefId(20));
    let concrete_method_id = DefId::new(CrateId(0), LocalDefId(21));
    let generic_method_id = DefId::new(CrateId(0), LocalDefId(22));
    let generic_param = crate::types::GenericParamId {
        owner: generic_method_id,
        index: 0,
    };

    let mut concrete_method = println_method(Type::I64);
    concrete_method.id = concrete_method_id;
    let generic_method = HirFunction {
        id: generic_method_id,
        name: "identity".to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_param, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_param),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_param),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_param),
                span: Span::test(),
            }))],
            ty: Type::Generic(generic_param),
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    };
    let mixed_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([
            ("println".to_string(), concrete_method),
            ("identity".to_string(), generic_method),
        ]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![mixed_impl],
        Vec::new(),
        BTreeMap::from([(concrete_method_id, "artifact_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let object_records = output
        .instances
        .values()
        .filter(|record| record.provided_by_object)
        .collect::<Vec<_>>();
    assert_eq!(object_records.len(), 1);
    let concrete_record = object_records[0];
    assert_eq!(
        concrete_record.origin,
        crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::Named(impl_id),
            method: concrete_method_id,
        }
    );
    assert_eq!(
        concrete_record.symbols.backend_symbol,
        "artifact_box_println"
    );
    assert!(!output.instances.values().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: generic_method_id,
            }
    }));
}

#[test]
fn record_imported_impl_allocates_object_method_instances_by_method_def_id() {
    fn imported_origins(
        method_ids: &[DefId],
    ) -> Vec<(crate::mono::InstanceId, crate::mono::InstanceOrigin)> {
        let impl_id = DefId::new(CrateId(1), LocalDefId(30));
        let trait_id = DefId::new(CrateId(1), LocalDefId(31));
        let methods = method_ids
            .iter()
            .map(|method_id| {
                let mut method = println_method(Type::I64);
                method.id = *method_id;
                (format!("method_{}", method_id.local.0), method)
            })
            .collect::<HashMap<_, _>>();
        let imp = HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods,
        };
        let backend_symbols = method_ids
            .iter()
            .map(|method_id| (*method_id, format!("artifact_{}", method_id.local.0)))
            .collect();
        let mut ctx = CrateContext::new();
        ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
            vec![imp.clone()],
            Vec::new(),
            backend_symbols,
        ))
        .expect("object-backed dependency should load");
        let dep = ctx
            .extern_crates()
            .next()
            .expect("dependency should be available");
        let mut mono = Monomorphizer::new();

        mono.record_imported_impl("std", &imp, Some(dep));

        mono.instances
            .records()
            .map(|record| (record.id, record.origin.clone()))
            .collect()
    }

    let first_method_id = DefId::new(CrateId(1), LocalDefId(32));
    let second_method_id = DefId::new(CrateId(1), LocalDefId(33));
    let impl_id = DefId::new(CrateId(1), LocalDefId(30));
    let expected = vec![
        (
            crate::mono::InstanceId(0),
            crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: first_method_id,
            },
        ),
        (
            crate::mono::InstanceId(1),
            crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(impl_id),
                method: second_method_id,
            },
        ),
    ];

    for method_order in [
        [first_method_id, second_method_id],
        [second_method_id, first_method_id],
    ] {
        for _ in 0..16 {
            assert_eq!(imported_origins(&method_order), expected);
        }
    }
}

#[test]
fn process_with_crates_uses_artifact_backend_symbols_for_object_backed_methods() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let mut method = println_method(Type::I64);
    method.id = method_id;

    let concrete_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![],
        receiver_pattern: vec![].into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(99))),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_with_backend_symbols(
        vec![concrete_impl],
        vec![],
        BTreeMap::from([(method_id, "artifact_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let record = output
        .instances
        .values()
        .find(|record| record.provided_by_object)
        .expect("object-backed method instance should be recorded");
    assert_eq!(record.symbols.source_name, "std::println");
    assert_eq!(record.symbols.backend_symbol, "artifact_box_println");
}

#[test]
fn object_backed_impls_with_same_display_names_use_distinct_ids() {
    fn object_backed_box_impl(
        crate_id: CrateId,
        impl_local: u32,
        method_local: u32,
        trait_local: u32,
    ) -> HirImpl {
        let impl_id = DefId::new(crate_id, LocalDefId(impl_local));
        let method_id = DefId::new(crate_id, LocalDefId(method_local));
        let trait_id = DefId::new(crate_id, LocalDefId(trait_local));
        let mut method = println_method(Type::I64);
        method.id = method_id;

        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("Box".to_string()),
            type_name: "Box".to_string(),
            type_generics: vec![],
            receiver_pattern: vec![].into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(trait_id),
            trait_generics: vec![],
            trait_arg_types: vec![],
            associated_types: vec![],
            bounds: std::collections::HashMap::new().into(),
            methods: HashMap::from([("println".to_string(), method)]),
        }
    }

    let first_impl = object_backed_box_impl(CrateId(1), 10, 11, 12);
    let second_impl = object_backed_box_impl(CrateId(2), 10, 11, 12);
    let first_method_id = first_impl.methods["println"].id;
    let second_method_id = second_impl.methods["println"].id;
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(1),
        "first_dep",
        vec![first_impl.clone()],
        vec![],
        BTreeMap::from([(first_method_id, "first_box_println".to_string())]),
    ))
    .unwrap();
    ctx.add_extern_crate(loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(2),
        "second_dep",
        vec![second_impl.clone()],
        vec![],
        BTreeMap::from([(second_method_id, "second_box_println".to_string())]),
    ))
    .unwrap();

    let mut mono = Monomorphizer::new();
    let output = wrapped_process_with_crates(
        &mut mono,
        crate::mono::hir_types::HirProgram::from_accepted_id_parts_with_names_and_canonical_names(
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            crate::hir::HirNameTables::default(),
            &HashMap::new(),
        ),
        &ctx,
    );

    let object_records = output
        .instances
        .values()
        .filter(|record| record.provided_by_object)
        .collect::<Vec<_>>();
    assert_eq!(object_records.len(), 2);
    assert!(object_records.iter().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(first_impl.id),
                method: first_method_id,
            }
            && record.symbols.backend_symbol == "first_box_println"
    }));
    assert!(object_records.iter().any(|record| {
        record.origin
            == crate::mono::InstanceOrigin::ImplMethod {
                owner: crate::mono::InstanceImplOwner::Named(second_impl.id),
                method: second_method_id,
            }
            && record.symbols.backend_symbol == "second_box_println"
    }));
}

#[test]
fn generic_artifact_method_specialization_records_method_def_id_origin() {
    let impl_id = DefId::new(CrateId(0), LocalDefId(10));
    let method_id = DefId::new(CrateId(0), LocalDefId(11));
    let trait_id = DefId::new(CrateId(0), LocalDefId(99));
    let mut method = println_method(Type::Struct {
        id: DefId::new(CrateId(0), LocalDefId(10)),
        args: vec![Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        })],
    });
    method.id = method_id;
    let generic_trait_impl = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: vec![GenericParamDecl::type_param(
            crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "T",
        )],
        receiver_pattern: vec![Type::Generic(crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(trait_id),
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([("println".to_string(), method)]),
    };

    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(loaded_object_backed_crate(vec![], vec![generic_trait_impl]))
        .unwrap();

    let recv = HirExpr {
        kind: HirExprKind::Var("boxed".to_string()),
        ty: Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: vec![Type::I64],
        },
        span: Span::test(),
    };
    let mut selected_target = HirMethodCallTarget::impl_method(
        impl_id,
        method_id,
        Some(crate::hir::HirSelectedTraitMember {
            trait_id,
            member_id: method_id,
            trait_args: Vec::new(),
        }),
    );
    selected_target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: crate::types::GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: Type::I64,
    }];
    let mut expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "println".to_string(),
            vec![],
            Some(ReceiverMode::Move),
            selected_target,
        ),
        ty: Type::I32,
        span: Span::test(),
    };
    let mut mono = Monomorphizer::new();
    mono.resolver
        .item_names_by_id
        .insert(impl_id, "Box".to_string());
    mono.load_external_generic_functions(&ctx);
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(Type::Struct {
            id: DefId::new(CrateId(0), LocalDefId(10)),
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        }),
    );

    let _ = mono.monomorphize_trait_method_call("println", &[recv], &mut expr);

    let origins = mono
        .instances
        .records()
        .map(|record| record.origin.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        origins,
        vec![crate::mono::InstanceOrigin::ImplMethod {
            owner: crate::mono::InstanceImplOwner::Named(impl_id),
            method: method_id,
        }]
    );
}
