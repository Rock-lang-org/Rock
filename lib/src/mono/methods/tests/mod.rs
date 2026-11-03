mod impl_collection;
mod impl_materialization;
mod method_origins;
mod no_name_specialization;
mod selected_targets;
mod slice_receivers;

use std::collections::HashMap;

use crate::hir::{
    HirImplOwner, HirImplReceiverPattern, HirMethodCallTarget, HirParam, HirVarTarget,
};

use crate::ids::{CrateId, DefId, LocalDefId};

use crate::lexer::Span;

use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirStmt};

use crate::types::ReceiverMode;

use crate::types::Type;

use crate::types::{GenericParamDecl, GenericParamId};

use crate::mono::{InstanceImplOwner, InstanceOrigin};

use super::Monomorphizer;

fn empty_body(ty: Type) -> HirBlock {
    HirBlock { stmts: vec![], ty }
}

fn enum_ty(type_name: &str, args: Vec<Type>) -> Type {
    let local = match type_name {
        "Option" | "std::option::Option" => 10,
        "Result" => 11,
        _ => 12,
    };
    Type::Enum {
        id: DefId::new(CrateId(0), LocalDefId(local)),
        args,
    }
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn type_generic(owner: DefId, name: &str) -> Vec<GenericParamDecl> {
    vec![GenericParamDecl::type_param(
        GenericParamId { owner, index: 0 },
        name,
    )]
}

fn option_map_method(impl_id: DefId, method_id: DefId) -> HirFunction {
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    let method_generic = GenericParamId {
        owner: method_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "map".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(impl_generic, "T"),
            GenericParamDecl::type_param(method_generic, "U"),
        ],
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "f".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::function(
                    vec![Type::Generic(impl_generic)],
                    Type::Generic(method_generic),
                ),
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: enum_ty("Option", vec![Type::Generic(method_generic)]),
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn map_method_for_type(type_name: &str, impl_id: DefId, method_id: DefId) -> HirFunction {
    let mut method = option_map_method(impl_id, method_id);
    method.params[0].ty = enum_ty(
        type_name,
        vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })],
    );
    method.ret_type = enum_ty(
        type_name,
        vec![Type::Generic(GenericParamId {
            owner: method_id,
            index: 0,
        })],
    );
    method
}

fn map_expr_for_type(type_name: &str) -> (HirExpr, HirExpr, HirExpr) {
    let recv = HirExpr {
        kind: HirExprKind::Var("value".to_string()),
        ty: enum_ty(type_name, vec![Type::I64]),
        span: Span::test(),
    };
    let func_arg = HirExpr {
        kind: HirExprKind::Var("inc".to_string()),
        ty: Type::function(vec![Type::I64], Type::I64),
        span: Span::test(),
    };
    let expr = HirExpr {
        kind: HirExprKind::MethodCall(
            Box::new(recv.clone()),
            "map".to_string(),
            vec![func_arg.clone()],
            Some(ReceiverMode::Move),
            HirMethodCallTarget::trait_method(
                def_id(999),
                def_id(998),
                Vec::new(),
                crate::hir::HirTraitDispatchKind::TraitBound,
            ),
        ),
        ty: enum_ty(type_name, vec![Type::I64]),
        span: Span::test(),
    };

    (recv, func_arg, expr)
}

fn generic_map_impl(type_name: &str, impl_id: DefId, method_id: DefId) -> HirImpl {
    HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named(type_name.to_string()),
        type_name: type_name.to_string(),
        type_generics: type_generic(impl_id, "T"),
        receiver_pattern: vec![Type::Generic(GenericParamId {
            owner: impl_id,
            index: 0,
        })]
        .into(),
        trait_name: None,
        trait_id: None,
        trait_generics: vec![],
        trait_arg_types: vec![],
        associated_types: vec![],
        bounds: std::collections::HashMap::new().into(),
        methods: HashMap::from([(
            "map".to_string(),
            map_method_for_type(type_name, impl_id, method_id),
        )]),
    }
}

fn seed_generic_enum_receiver_pattern(mono: &mut Monomorphizer, impl_id: DefId, type_name: &str) {
    mono.set_impl_receiver_pattern_for_test(
        impl_id,
        crate::hir::HirImplReceiverPattern::Exact(enum_ty(
            type_name,
            vec![Type::Generic(GenericParamId {
                owner: impl_id,
                index: 0,
            })],
        )),
    );
}

fn option_println_method(impl_id: DefId, method_id: DefId) -> HirFunction {
    let impl_generic = GenericParamId {
        owner: impl_id,
        index: 0,
    };
    HirFunction {
        id: method_id,
        name: "println".to_string(),
        generic_params: vec![GenericParamDecl::type_param(impl_generic, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: enum_ty("Option", vec![Type::Generic(impl_generic)]),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn byte_slice_println_method() -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "println".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::U8))),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: empty_body(Type::Unit),
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn qualified_option_receiver() -> HirExpr {
    HirExpr {
        kind: HirExprKind::Var("some".to_string()),
        ty: enum_ty("std::option::Option", vec![Type::I64]),
        span: Span::test(),
    }
}

fn seed_owner(mono: &mut Monomorphizer, name: &str) {
    let local = match name {
        "Option" | "std::option::Option" => 10,
        "Result" => 11,
        _ => 0,
    };
    let def_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(local));
    mono.resolver.item_paths.insert(name.to_string(), def_id);
    mono.resolver
        .item_names_by_id
        .insert(def_id, name.to_string());
}

fn selected_impl_target(impl_id: DefId, method_id: DefId, owner_ty: Type) -> HirMethodCallTarget {
    let mut target = HirMethodCallTarget::impl_method(impl_id, method_id, None);
    target.owner_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: impl_id,
            index: 0,
        },
        ty: owner_ty,
    }];
    target
}

fn selected_map_target(
    impl_id: DefId,
    method_id: DefId,
    owner_ty: Type,
    method_ty: Type,
) -> HirMethodCallTarget {
    let mut target = selected_impl_target(impl_id, method_id, owner_ty);
    target.method_substitution = vec![crate::hir::HirTypeBinding {
        param: GenericParamId {
            owner: method_id,
            index: 0,
        },
        ty: method_ty,
    }];
    target
}
