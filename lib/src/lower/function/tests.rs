use std::collections::{HashMap, HashSet};

use super::*;

use crate::ast::{
    Block, FunctionDecl, FunctionSig, Ident, IdentPattern, LambdaArrowKind, LambdaDecl, ParseType,
    ParseTypeInner, Pattern, PatternKind, SelfReceiverMode,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
use crate::lexer::Span;
use crate::types::AssociatedTypeKey;

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn unit_signature(name: &str) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig: ParseType::Unit(crate::lexer::Span::test()),
        where_clauses: vec![],
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

fn named_type(name: &str) -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics: Vec::new(),
        span: Span::test(),
    })
}

fn generic_identity_signature(name: &str) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig: ParseType::Function(vec![named_type("T"), named_type("T")]),
        where_clauses: vec![],
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

fn binding_pattern(name: &str) -> Pattern {
    Pattern {
        binding: None,
        kind: PatternKind::Ident(IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn single_param_function_decl(name: &str, param: &str) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: vec![binding_pattern(param)],
            body: Block { statements: vec![] },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: None,
        is_unsafe: false,
        exported: false,
    }
}

fn single_param_method_decl(
    name: &str,
    param: &str,
    self_receiver: SelfReceiverMode,
) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: vec![binding_pattern(param)],
            body: Block { statements: vec![] },
            arrow_kind: LambdaArrowKind::Normal,
            span: crate::lexer::Span::test(),
        },
        self_receiver: Some(self_receiver),
        is_unsafe: false,
        exported: false,
    }
}

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

#[test]
fn lower_method_signature_injects_shared_self_receiver_type() {
    let mut lowerer = Lowerer::new_for_test();
    let signature_id = def_id(30);
    let sig = FunctionSig {
        name: ident("is_valid"),
        sig: named_type("Bool"),
        where_clauses: vec![],
        self_receiver: Some(SelfReceiverMode::Shared),
        is_unsafe: false,
        exported: false,
    };

    let hir_sig = lowerer.lower_function_sig_with_id(&sig, signature_id);

    let expected_self = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Generic(GenericParamId {
            owner: signature_id,
            index: 0,
        })),
    };
    assert_eq!(hir_sig.params, vec![expected_self]);
    assert_eq!(
        hir_sig.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: signature_id,
                index: 0,
            },
            "Self",
        )]
    );
    assert_eq!(hir_sig.ret, Type::Bool);
}

#[test]
fn collect_function_sig_uses_explicit_declaration_id() {
    let mut lowerer = Lowerer::new_for_test();
    let declaration = single_param_function_decl("identity", "value");
    let function_id = def_id(31);

    lowerer.collect_function_sig(&declaration, function_id);

    assert_eq!(lowerer.items.function(function_id).unwrap().id, function_id);
    assert_eq!(lowerer.items.function_sigs().count(), 0);
}

#[test]
fn lower_method_signature_injects_mut_and_move_self_receiver_types() {
    let mut lowerer = Lowerer::new_for_test();
    let mut_sig = FunctionSig {
        name: ident("set"),
        sig: ParseType::Function(vec![
            named_type("I64"),
            ParseType::Unit(crate::lexer::Span::test()),
        ]),
        where_clauses: vec![],
        self_receiver: Some(SelfReceiverMode::Mut),
        is_unsafe: false,
        exported: false,
    };
    let move_sig = FunctionSig {
        name: ident("consume"),
        sig: named_type("Bool"),
        where_clauses: vec![],
        self_receiver: Some(SelfReceiverMode::Move),
        is_unsafe: false,
        exported: false,
    };

    let mut_hir = lowerer.lower_function_sig_with_id(&mut_sig, def_id(31));
    let move_hir = lowerer.lower_function_sig_with_id(&move_sig, def_id(32));

    assert!(matches!(
        mut_hir.params[0],
        Type::Reference { mutable: true, .. }
    ));
    assert_eq!(mut_hir.params[1], Type::I64);
    assert!(matches!(move_hir.params[0], Type::Generic(_)));
    assert_eq!(move_hir.ret, Type::Bool);
}

#[test]
fn lower_signature_backed_mut_method_remaps_hidden_self_receiver_generic() {
    let mut lowerer = Lowerer::new_for_test();
    let signature_id = def_id(33);
    let function_id = def_id(34);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let hir_sig = HirFunctionSig {
        id: signature_id,
        name: "set".to_string(),
        generic_params: vec![GenericParamDecl::type_param(signature_self, "Self")],
        params: vec![
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::I64,
        ],
        ret: Type::Bool,
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };

    let func = lowerer.lower_function_decl_header_with_sig_and_id(
        &single_param_method_decl("set", "value", SelfReceiverMode::Mut),
        &hir_sig,
        function_id,
    );

    let expected_self = GenericParamId {
        owner: function_id,
        index: 0,
    };
    assert_eq!(
        func.generic_params,
        vec![GenericParamDecl::type_param(expected_self, "Self")]
    );
    assert_eq!(func.params[0].local_id, crate::ids::HirLocalId(0));
    assert!(func.params[0].mutable);
    assert_eq!(
        func.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(func.params[1].ty, Type::I64);
}

#[test]
fn generic_param_descriptor_method_keeps_public_name_paired_after_hidden_self() {
    let mut lowerer = Lowerer::new_for_test();
    let signature_id = def_id(35);
    let function_id = def_id(36);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let signature_t = GenericParamId {
        owner: signature_id,
        index: 1,
    };
    let hir_sig = HirFunctionSig {
        id: signature_id,
        name: "set".to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(signature_self, "Self"),
            GenericParamDecl::type_param(signature_t, "T"),
        ],
        params: vec![
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Generic(signature_self)),
            },
            Type::Generic(signature_t),
        ],
        ret: Type::Generic(signature_t),
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Mut),
        is_unsafe: false,
    };

    let func = lowerer.lower_function_decl_header_with_sig_and_id(
        &single_param_method_decl("set", "value", SelfReceiverMode::Mut),
        &hir_sig,
        function_id,
    );

    let expected_t = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let expected_self = GenericParamId {
        owner: function_id,
        index: 1,
    };
    assert_eq!(
        func.generic_params,
        vec![
            GenericParamDecl::type_param(expected_t, "T"),
            GenericParamDecl::type_param(expected_self, "Self"),
        ]
    );
    assert_eq!(
        func.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(func.params[1].ty, Type::Generic(expected_t));
    assert_eq!(func.ret_type, Type::Generic(expected_t));
}

#[test]
fn collect_type_var_ids_visits_projection_base_and_trait_args() {
    let base_var = TypeVarId(20);
    let arg_var = TypeVarId(21);
    let trait_id = DefId::new(CrateId(0), LocalDefId(30));
    let ty = Type::Projection {
        ty: Box::new(Type::TypeVar(base_var)),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::TypeVar(arg_var)],
    };
    let mut type_vars = HashSet::new();

    Lowerer::collect_type_var_ids(&ty, &mut type_vars);

    assert!(type_vars.contains(&base_var));
    assert!(type_vars.contains(&arg_var));
}

#[test]
#[should_panic(expected = "missing canonical function signature identity")]
fn lower_function_sig_panics_without_current_trait_signature_identity() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = DefId::new(CrateId(0), LocalDefId(10));
    lowerer.current_trait = Some("Show".to_string());
    lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
        trait_id,
        vec!["Self".to_string()],
    ));

    lowerer.lower_function_sig(&unit_signature("show"));
}

#[test]
#[should_panic(expected = "missing canonical function signature identity")]
fn lower_function_sig_rejects_top_level_collision_in_trait_context() {
    let mut lowerer = Lowerer::new_for_test();
    let trait_id = DefId::new(CrateId(0), LocalDefId(10));
    let top_level_id = DefId::new(CrateId(0), LocalDefId(11));
    lowerer.current_trait = Some("Show".to_string());
    lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
        trait_id,
        vec!["Self".to_string()],
    ));
    lowerer
        .resolver
        .item_paths
        .insert("show".to_string(), top_level_id);

    lowerer.lower_function_sig(&unit_signature("show"));
}

#[test]
fn lower_function_sig_with_id_uses_explicit_id_for_signature_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let stale_resolver_id = def_id(11);
    let signature_id = def_id(12);
    lowerer
        .resolver
        .item_paths
        .insert("id".to_string(), stale_resolver_id);

    let sig = lowerer.lower_function_sig_with_id(&generic_identity_signature("id"), signature_id);

    let expected_generic = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    assert_eq!(sig.id, signature_id);
    assert_eq!(
        sig.generic_params,
        vec![GenericParamDecl::type_param(expected_generic, "T")]
    );
    assert_eq!(sig.params, vec![Type::Generic(expected_generic)]);
    assert_eq!(sig.ret, Type::Generic(expected_generic));
}

#[test]
fn lower_function_decl_header_remaps_signature_owned_generics_to_function_id() {
    let mut lowerer = Lowerer::new_for_test();
    let signature_id = def_id(13);
    let function_id = def_id(14);
    let signature_generic = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let hir_sig = HirFunctionSig {
        id: signature_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(signature_generic, "T")],
        params: vec![Type::Generic(signature_generic)],
        ret: Type::Generic(signature_generic),
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let func = lowerer.lower_function_decl_header_with_sig_and_id(
        &single_param_function_decl("id", "value"),
        &hir_sig,
        function_id,
    );

    let expected_generic = GenericParamId {
        owner: function_id,
        index: 0,
    };
    assert_eq!(
        func.generic_params,
        vec![GenericParamDecl::type_param(expected_generic, "T")]
    );
    assert_eq!(func.params[0].ty, Type::Generic(expected_generic));
    assert_eq!(func.ret_type, Type::Generic(expected_generic));
}

#[test]
fn lower_function_decl_header_drops_unused_signature_generics() {
    let mut lowerer = Lowerer::new_for_test();
    let function_id = def_id(15);
    let phantom_generic = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let hir_sig = HirFunctionSig {
        id: function_id,
        name: "len".to_string(),
        generic_params: vec![GenericParamDecl::type_param(phantom_generic, "T")],
        params: vec![Type::I64],
        ret: Type::I64,
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let func = lowerer.lower_function_decl_header_with_sig_and_id(
        &single_param_function_decl("len", "value"),
        &hir_sig,
        function_id,
    );

    assert!(func.generic_params.is_empty());
    assert_eq!(func.params[0].ty, Type::I64);
    assert_eq!(func.ret_type, Type::I64);
}
