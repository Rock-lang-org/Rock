use std::collections::HashMap;

use super::*;

use crate::hir::HirParam;
use crate::ids::{AssocTypeId, CrateId, LocalDefId};
use crate::mono::hir_types::{
    HirBlock, HirExpr, HirExprKind, HirImpl, HirImplOwner, HirImplReceiverPattern,
};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, NominalTypeKind};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn generic_function(id: DefId, param_ty: Type, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: id,
                index: 0,
            },
            "T",
        )],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: param_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type,
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

fn expr_with_type(ty: Type) -> HirExpr {
    HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty,
        span: crate::lexer::Span::test(),
    }
}

#[test]
fn extract_type_args_uses_bound_impl_constructor_section() {
    let mut mono = Monomorphizer::new();
    let function_id = def_id(1);
    let trait_id = def_id(2);
    let impl_id = def_id(3);
    let result_id = def_id(4);
    let f = GenericParamDecl::new(
        GenericParamId {
            owner: function_id,
            index: 0,
        },
        "F",
        Kind::arrow(Kind::Type, Kind::Type),
    );
    let a = GenericParamDecl::type_param(
        GenericParamId {
            owner: function_id,
            index: 1,
        },
        "A",
    );
    let function = HirFunction {
        id: function_id,
        name: "map_op".to_string(),
        generic_params: vec![f.clone(), a.clone()],
        generic_bounds: HashMap::from([(
            f.id,
            vec![crate::types::TraitBound {
                trait_id,
                type_args: Vec::new(),
            }],
        )])
        .into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Apply {
                constructor: Box::new(Type::Generic(f.id)),
                args: vec![Type::Generic(a.id)],
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let impl_error = GenericParamDecl::type_param(
        GenericParamId {
            owner: impl_id,
            index: 0,
        },
        "E",
    );
    let section = |error| Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Enum {
            id: result_id,
            args: vec![
                Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                error,
            ],
        }),
    };
    mono.trait_impls.insert(
        trait_id,
        vec![HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("ResultSection".to_string()),
            type_name: "ResultSection".to_string(),
            type_generics: vec![impl_error.clone()],
            receiver_pattern: HirImplReceiverPattern::Constructor(section(Type::Generic(
                impl_error.id,
            ))),
            trait_name: Some("Functor".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::new(),
        }],
    );

    let type_args = mono.extract_type_args(
        &function,
        &[expr_with_type(Type::Enum {
            id: result_id,
            args: vec![Type::I64, Type::Bool],
        })],
    );

    assert_eq!(mono.type_for(type_args[0]), section(Type::Bool));
    assert_eq!(mono.type_for(type_args[1]), Type::I64);

    let function_value_args = mono
        .extract_type_args_from_expr_type(
            &function,
            &Type::function(
                vec![Type::Enum {
                    id: result_id,
                    args: vec![Type::I64, Type::Bool],
                }],
                Type::Unit,
            ),
        )
        .expect("concrete function value should infer all generic arguments");
    assert_eq!(mono.type_for(function_value_args[0]), section(Type::Bool));
    assert_eq!(mono.type_for(function_value_args[1]), Type::I64);

    // A constructor may appear only in the return type (for example pure).
    let mut producer = function;
    producer.ret_type = producer.params[0].ty.clone();
    producer.params[0].ty = Type::Generic(a.id);
    let producer_args = mono
        .extract_type_args_from_expr_type(
            &producer,
            &Type::function(
                vec![Type::I64],
                Type::Enum {
                    id: result_id,
                    args: vec![Type::I64, Type::Bool],
                },
            ),
        )
        .expect("return context should select the bound constructor section");
    assert_eq!(mono.type_for(producer_args[0]), section(Type::Bool));
    assert_eq!(mono.type_for(producer_args[1]), Type::I64);
}

#[test]
fn monomorphize_call_returns_instance_id_for_new_specialization() {
    let mut mono = Monomorphizer::new();
    let id = def_id(10);
    let generic = generic_function(
        id,
        Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        }),
        Type::Generic(GenericParamId {
            owner: id,
            index: 0,
        }),
    );
    mono.resolver.item_paths.insert("identity".to_string(), id);
    let args = vec![HirExpr {
        kind: HirExprKind::IntLiteral(21),
        ty: Type::I64,
        span: crate::lexer::Span::test(),
    }];

    let (instance_id, func_ty, ret_ty) = mono
        .monomorphize_call("identity", &generic, &args)
        .expect("generic call should specialize");

    assert_eq!(ret_ty, Type::I64);
    assert_eq!(func_ty, Type::function(vec![Type::I64], Type::I64));
    let record = mono
        .instances
        .record(instance_id)
        .expect("instance is recorded");
    assert_eq!(record.origin, crate::mono::InstanceOrigin::Function(id));
    assert_eq!(record.substitution.len(), 1);
    assert_eq!(mono.type_for(record.substitution[0]), Type::I64);
}

#[test]
fn generic_ids_for_function_uses_function_owner_not_foreign_occurrences() {
    let function_owner = def_id(10);
    let foreign_owner = def_id(20);
    let generic_func = generic_function(
        function_owner,
        Type::Generic(GenericParamId {
            owner: foreign_owner,
            index: 0,
        }),
        Type::I64,
    );

    let generic_ids = Monomorphizer::generic_ids_for_function(&generic_func);

    assert_eq!(
        generic_ids,
        vec![GenericParamId {
            owner: function_owner,
            index: 0,
        }]
    );
}

#[test]
fn contains_generic_detects_projection_generic_inputs() {
    let mono = Monomorphizer::new();
    let generic_id = GenericParamId {
        owner: def_id(10),
        index: 0,
    };

    assert!(mono.contains_generic(&Type::Projection {
        ty: Box::new(Type::Generic(generic_id)),
        trait_id: def_id(1),
        assoc_type: AssociatedTypeKey {
            owner: def_id(1),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    }));
    assert!(mono.contains_generic(&Type::Projection {
        ty: Box::new(Type::I64),
        trait_id: def_id(1),
        assoc_type: AssociatedTypeKey {
            owner: def_id(1),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::Generic(generic_id)],
    }));
}

#[test]
fn extract_type_args_ignores_same_index_foreign_generic_owner() {
    let mut mono = Monomorphizer::new();
    let function_owner = def_id(10);
    let foreign_owner = def_id(20);
    let generic_func = generic_function(
        function_owner,
        Type::Generic(GenericParamId {
            owner: foreign_owner,
            index: 0,
        }),
        Type::Generic(GenericParamId {
            owner: function_owner,
            index: 0,
        }),
    );

    let type_args = mono.extract_type_args(&generic_func, &[expr_with_type(Type::Bool)]);

    assert_eq!(
        type_args
            .into_iter()
            .map(|ty_id| mono.type_for(ty_id))
            .collect::<Vec<_>>(),
        vec![Type::I64]
    );
}

#[test]
fn extract_type_args_does_not_infer_across_projection_identity_mismatch() {
    let mut mono = Monomorphizer::new();
    let function_owner = def_id(10);
    let generic_id = GenericParamId {
        owner: function_owner,
        index: 0,
    };
    let generic_func = generic_function(
        function_owner,
        Type::Projection {
            ty: Box::new(Type::Generic(generic_id)),
            trait_id: def_id(1),
            assoc_type: AssociatedTypeKey {
                owner: def_id(1),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(generic_id)],
        },
        Type::I64,
    );

    let type_args = mono.extract_type_args(
        &generic_func,
        &[expr_with_type(Type::Projection {
            ty: Box::new(Type::Bool),
            trait_id: def_id(2),
            assoc_type: AssociatedTypeKey {
                owner: def_id(2),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![Type::Bool],
        })],
    );

    assert_eq!(
        type_args
            .into_iter()
            .map(|ty_id| mono.type_for(ty_id))
            .collect::<Vec<_>>(),
        vec![Type::I64]
    );
}

#[test]
fn constructor_specialization_normalizes_nested_generic_application() {
    let function_id = def_id(30);
    let option_id = def_id(31);
    let constructor_param = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let element_param = GenericParamId {
        owner: function_id,
        index: 1,
    };
    let unary = Kind::arrow(Kind::Type, Kind::Type);
    let applied = Type::Apply {
        constructor: Box::new(Type::Generic(constructor_param)),
        args: vec![Type::Generic(element_param)],
    };
    let generic = HirFunction {
        id: function_id,
        name: "lift".to_string(),
        generic_params: vec![
            GenericParamDecl::new(constructor_param, "F", unary.clone()),
            GenericParamDecl::type_param(element_param, "A"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: applied.clone(),
            mutable: false,
            is_ref: false,
        }],
        ret_type: applied,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    };
    let concrete = Type::Enum {
        id: option_id,
        args: vec![Type::I64],
    };
    let mut env = TypeNormalizationEnv::new();
    env.register_constructor(option_id, NominalTypeKind::Enum, unary);
    env.register_generic_kind(constructor_param, Kind::arrow(Kind::Type, Kind::Type));
    env.register_generic_kind(element_param, Kind::Type);
    let mut mono = Monomorphizer::new();
    mono.normalization_env = env;

    let (instance, _, ret) = mono
        .monomorphize_call("lift", &generic, &[expr_with_type(concrete.clone())])
        .expect("constructor application should specialize");
    let record = mono.instances.record(instance).unwrap();

    assert_eq!(
        mono.type_for(record.substitution[0]),
        Type::Constructor {
            id: option_id,
            flavor: NominalTypeKind::Enum,
        }
    );
    assert_eq!(mono.type_for(record.substitution[1]), Type::I64);
    assert_eq!(ret, concrete);
    assert_eq!(
        mono.instances.pre_mir_body(instance).unwrap().params[0].ty,
        concrete
    );
}

#[test]
fn specialization_beta_reduces_result_section_after_error_substitution() {
    let function_id = def_id(40);
    let result_id = def_id(41);
    let error_param = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let section = Type::Lambda {
        params: vec![Kind::Type],
        body: Box::new(Type::Enum {
            id: result_id,
            args: vec![
                Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                Type::Generic(error_param),
            ],
        }),
    };
    let application = Type::Apply {
        constructor: Box::new(section),
        args: vec![Type::I64],
    };
    let mut env = TypeNormalizationEnv::new();
    env.register_nominal(result_id, NominalTypeKind::Enum, 2);
    env.register_generic_kind(error_param, Kind::Type);
    let mut mono = Monomorphizer::new();
    mono.normalization_env = env;
    let bool_id = mono.intern_type(&Type::Bool);

    assert_eq!(
        mono.substitute_type_with_map(&application, &HashMap::from([(error_param, bool_id)])),
        Type::Enum {
            id: result_id,
            args: vec![Type::I64, Type::Bool],
        }
    );
}

#[test]
fn extract_type_args_binds_const_param_from_array_length() {
    let mut mono = Monomorphizer::new();
    let function_id = def_id(40);
    let element = GenericParamDecl::type_param(
        GenericParamId {
            owner: function_id,
            index: 0,
        },
        "T",
    );
    let length = GenericParamDecl::new(
        GenericParamId {
            owner: function_id,
            index: 1,
        },
        "N",
        Kind::Usize,
    );
    let mut function = generic_function(
        function_id,
        Type::Array(
            Box::new(Type::Generic(element.id)),
            crate::types::ArrayLen::Param(length.id),
        ),
        Type::Generic(element.id),
    );
    function.generic_params = vec![element, length.clone()];
    let arg = expr_with_type(Type::Array(
        Box::new(Type::I64),
        crate::types::ArrayLen::Value(3),
    ));

    let type_args = mono.extract_type_args(&function, &[arg]);

    assert_eq!(type_args.len(), 2);
    assert_eq!(mono.type_for(type_args[0]), Type::I64);
    assert_eq!(mono.type_for(type_args[1]), Type::ConstValue(3));
}

#[test]
fn apply_substitution_replaces_array_length_param() {
    let mut mono = Monomorphizer::new();
    let length = GenericParamId {
        owner: def_id(41),
        index: 0,
    };
    let three = mono.intern_type(&Type::ConstValue(3));
    let ty = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Param(length));

    let substituted = mono.apply_substitution(&ty, &HashMap::from([(length, three)]));

    assert_eq!(
        substituted,
        Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(3))
    );
}
