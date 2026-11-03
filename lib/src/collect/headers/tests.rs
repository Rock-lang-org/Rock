use std::collections::{HashMap, HashSet};

use super::*;

use crate::ast::{
    self, AssociatedTypeDecl, AssociatedTypeDef, EnumDecl, EnumVariant, FunctionDecl, FunctionSig,
    Ident, Impl, LambdaArrowKind, LambdaDecl, NamedFieldsOrTypesList, ParseType, ParseTypeInner,
    Pattern, PatternKind, SelfReceiverMode, StructDecl, StructDeclField, TraitDecl, WhereClause,
};
use crate::collect::context::CollectContext;
use crate::hir::{HirFunctionSig, HirVariantFields};
use crate::ids::{CrateId, LocalDefId};
use crate::language_items::SizedLanguageItems;
use crate::lexer::Span;
use crate::types::{AssociatedTypeKey, GenericParamId, Type};

fn build_struct_with_id(context: &mut CollectContext, decl: &StructDecl, id: DefId) -> HirStruct {
    let params = build_nominal_generic_params(context, &decl.generic_params, id);
    super::build_struct_with_id(context, decl, id, params)
}

fn build_enum_with_id(context: &mut CollectContext, decl: &EnumDecl, id: DefId) -> HirEnum {
    let params = build_nominal_generic_params(context, &decl.generic_params, id);
    super::build_enum_with_id(context, decl, id, params)
}

fn ident(name: &str) -> Ident {
    Ident {
        name: name.to_string(),
        span: Span::test(),
    }
}

fn type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

fn named_type(name: &str) -> ParseType {
    ParseType::Type(type_inner(name))
}

fn generic_type(name: &str) -> ParseType {
    ParseType::Type(type_inner(name))
}

fn generic_param(name: &str) -> ast::GenericParamDecl {
    ast::GenericParamDecl {
        default: None,
        name: ident(name),
        kind: None,
        span: Span::test(),
    }
}

fn constructor_kind_syntax(name: &str, arity: usize) -> ast::TypeApplication {
    ast::TypeApplication {
        constructor: Box::new(named_type(name)),
        args: (0..arity)
            .map(|_| ast::ParseType::Hole(ast::TypeHole { span: Span::test() }))
            .collect(),
        span: Span::test(),
    }
}

fn generic_type_inner(name: &str) -> ParseTypeInner {
    ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    }
}

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn trait_member_ids(
    methods: &[(&str, DefId)],
    signatures: &[(&str, DefId)],
) -> CollectedTraitMemberIds {
    CollectedTraitMemberIds {
        methods: methods
            .iter()
            .map(|(name, id)| ((*name).to_string(), *id))
            .collect(),
        signatures: signatures
            .iter()
            .map(|(name, id)| ((*name).to_string(), *id))
            .collect(),
    }
}

fn impl_method_ids(methods: &[(&str, DefId)]) -> HashMap<String, DefId> {
    methods
        .iter()
        .map(|(name, id)| ((*name).to_string(), *id))
        .collect()
}

fn field(name: &str, ty: ParseType, public: bool) -> StructDeclField {
    StructDeclField {
        name: ident(name),
        ty,
        public,
        default: None,
    }
}

fn ident_pattern(name: &str) -> Pattern {
    Pattern {
        binding: None,
        kind: PatternKind::Ident(ast::IdentPattern {
            name: ident(name),
            mut_: false,
        }),
    }
}

fn function_decl(
    name: &str,
    params: &[&str],
    self_receiver: Option<SelfReceiverMode>,
    arrow_kind: LambdaArrowKind,
) -> FunctionDecl {
    FunctionDecl {
        name: ident(name),
        lambda: LambdaDecl {
            parameters: params.iter().map(|name| ident_pattern(name)).collect(),
            body: ast::Block { statements: vec![] },
            arrow_kind,
            span: crate::lexer::Span::test(),
        },
        self_receiver,
        is_unsafe: false,
        exported: false,
    }
}

fn function_sig(
    name: &str,
    sig: ParseType,
    self_receiver: Option<SelfReceiverMode>,
    where_clauses: Vec<WhereClause>,
) -> FunctionSig {
    FunctionSig {
        name: ident(name),
        sig,
        where_clauses,
        self_receiver,
        is_unsafe: false,
        exported: false,
    }
}

#[test]
fn collect_structural_inherent_method_has_declared_return_and_receiver() {
    let mut context = CollectContext::new();
    let impl_id = def_id(100);
    let method_id = def_id(101);
    let implementation = Impl {
        name: ParseType::Reference {
            is_mut: false,
            pointee: Box::new(ParseType::Slice(Box::new(named_type("T")))),
        },
        for_: None,
        associated_types: vec![],
        methods: HashMap::from([(
            ident("pointer"),
            function_decl(
                "pointer",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::from([(
            ident("pointer"),
            function_sig(
                "pointer",
                ParseType::Pointer(Box::new(named_type("T"))),
                Some(SelfReceiverMode::Shared),
                vec![],
            ),
        )]),
        where_clauses: vec![],
    };
    let collected = build_impl_with_id(
        &mut context,
        &implementation,
        impl_id,
        &impl_method_ids(&[("pointer", method_id)]),
    );
    assert!(context.errors.is_empty(), "{:?}", context.errors);
    let element = Type::Generic(GenericParamId {
        owner: impl_id,
        index: 0,
    });
    let target = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(element.clone()))),
    };
    assert_eq!(collected.type_generics.len(), 1);
    assert_eq!(
        collected.receiver_pattern,
        crate::hir::HirImplReceiverPattern::Exact(target.clone())
    );
    let method = &collected.methods["pointer"];
    assert_eq!(method.ret_type, Type::Pointer(Box::new(element)));
    assert_eq!(
        method.params[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(target)
        }
    );
    assert_eq!(method.id, method_id);
}

#[test]
fn collect_type_var_ids_visits_projection_base_and_trait_args() {
    let base_var = TypeVarId(10);
    let arg_var = TypeVarId(11);
    let ty = Type::Projection {
        ty: Box::new(Type::TypeVar(base_var)),
        trait_id: DefId::new(CrateId(0), LocalDefId(20)),
        assoc_type: AssociatedTypeKey {
            owner: DefId::new(CrateId(0), LocalDefId(20)),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![Type::TypeVar(arg_var)],
    };
    let mut type_vars = HashSet::new();

    collect_type_var_ids(&ty, &mut type_vars);

    assert!(type_vars.contains(&base_var));
    assert!(type_vars.contains(&arg_var));
}

#[test]
fn test_build_struct_lowers_struct_field_types() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "Buffer".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        fields: vec![
            field(
                "bytes",
                ParseType::Reference {
                    is_mut: false,
                    pointee: Box::new(ParseType::Slice(Box::new(named_type("U8")))),
                },
                true,
            ),
            field(
                "cursor",
                ParseType::Reference {
                    is_mut: true,
                    pointee: Box::new(named_type("I64")),
                },
                false,
            ),
        ],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_struct.name, "Buffer");
    assert_eq!(
        hir_struct.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(1),
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(hir_struct.fields.len(), 2);
    assert_eq!(hir_struct.fields[0].name, "bytes");
    assert_eq!(
        hir_struct.fields[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Slice(Box::new(Type::U8))),
        }
    );
    assert!(hir_struct.fields[0].public);
    assert_eq!(hir_struct.fields[1].name, "cursor");
    assert_eq!(
        hir_struct.fields[1].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::I64),
        }
    );
    assert!(!hir_struct.fields[1].public);
}

#[test]
fn build_struct_preserves_constructor_generic_kind_and_application() {
    let owner = def_id(90);
    let constructor_param = ast::GenericParamDecl {
        default: None,
        name: ident("F"),
        kind: Some(ast::TypeApplication {
            constructor: Box::new(named_type("F")),
            args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
            span: Span::test(),
        }),
        span: Span::test(),
    };
    let decl = StructDecl {
        name: type_inner("Wrapper"),
        generic_params: vec![constructor_param, generic_param("A")],
        fields: vec![field(
            "value",
            ast::ParseType::Application(ast::TypeApplication {
                constructor: Box::new(named_type("F")),
                args: vec![named_type("A")],
                span: Span::test(),
            }),
            true,
        )],
        exported: false,
    };

    let strukt = build_struct_with_id(&mut CollectContext::new(), &decl, owner);

    assert_eq!(
        strukt.generic_params[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        strukt.fields[0].ty,
        Type::Apply {
            constructor: Box::new(Type::Generic(GenericParamId { owner, index: 0 })),
            args: vec![Type::Generic(GenericParamId { owner, index: 1 })],
        }
    );
}

#[test]
fn build_struct_assigns_source_order_field_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = StructDecl {
        name: type_inner("Left"),
        generic_params: vec![],
        fields: vec![field("value", named_type("I32"), true)],
        exported: false,
    };
    let right = StructDecl {
        name: type_inner("Right"),
        generic_params: vec![],
        fields: vec![
            field("value", named_type("I32"), true),
            field("other", named_type("I64"), true),
        ],
        exported: false,
    };

    let left = build_struct_with_id(&mut context, &left, def_id(1));
    let right = build_struct_with_id(&mut context, &right, def_id(2));

    assert_eq!(left.fields[0].id, FieldId(0));
    assert_eq!(right.fields[0].id, FieldId(0));
    assert_eq!(right.fields[1].id, FieldId(1));
}

#[test]
fn test_build_struct_rejects_bare_str_field_type() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "TextHolder".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![],
        fields: vec![field("text", named_type("Str"), true)],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_struct.fields[0].ty, Type::Error);
    assert!(context.errors.iter().any(|err| err
        .message
        .contains("bare string slice type Str must be written behind a reference")));
}

#[test]
fn test_build_struct_accepts_borrowed_str_field_type() {
    let mut context = CollectContext::new();
    let decl = StructDecl {
        name: ParseTypeInner {
            name: "TextHolder".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![],
        fields: vec![field(
            "text",
            ParseType::Reference {
                is_mut: false,
                pointee: Box::new(named_type("Str")),
            },
            true,
        )],
        exported: false,
    };

    let hir_struct = build_struct_with_id(&mut context, &decl, def_id(1));

    assert_eq!(
        hir_struct.fields[0].ty,
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::Str),
        }
    );
    assert!(context.errors.is_empty());
}

#[test]
fn test_build_enum_mirrors_variant_header_construction() {
    let mut lowerer = CollectContext::new();
    let decl = EnumDecl {
        generic_params: vec![generic_param("T")],
        name: ParseTypeInner {
            name: "Message".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        variants: vec![
            EnumVariant {
                name: generic_type_inner("Data"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                    "payload",
                    ParseType::Pointer(Box::new(named_type("U8"))),
                    true,
                )]),
            },
            EnumVariant {
                name: generic_type_inner("Pair"),
                fields: NamedFieldsOrTypesList::TypesList(vec![
                    named_type("I32"),
                    ParseType::Array {
                        inner: Box::new(named_type("Bool")),
                        len: 2,
                    },
                ]),
            },
            EnumVariant {
                name: generic_type_inner("Empty"),
                fields: NamedFieldsOrTypesList::TypesList(vec![]),
            },
        ],
        exported: false,
        language_items: Default::default(),
    };

    let hir_enum = build_enum_with_id(&mut lowerer, &decl, def_id(1));

    assert_eq!(hir_enum.name, "Message");
    assert_eq!(
        hir_enum.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(1),
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(hir_enum.variants.len(), 3);
    match &hir_enum.variants[0].fields {
        HirVariantFields::Named(fields) => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, "payload");
            assert_eq!(fields[0].ty, Type::Pointer(Box::new(Type::U8)));
            assert!(fields[0].public);
        }
        other => panic!("expected named fields, got {other:?}"),
    }
    match &hir_enum.variants[1].fields {
        HirVariantFields::Positional(types) => {
            assert_eq!(
                types,
                &vec![Type::I32, Type::Array(Box::new(Type::Bool), 2)]
            );
        }
        other => panic!("expected positional fields, got {other:?}"),
    }
    assert!(matches!(
        hir_enum.variants[2].fields,
        HirVariantFields::Unit
    ));
}

#[test]
fn build_enum_assigns_variant_ids_and_owner_unique_named_field_ids() {
    let mut context = CollectContext::new();
    let decl = EnumDecl {
        generic_params: Vec::new(),
        name: type_inner("Message"),
        variants: vec![
            EnumVariant {
                name: generic_type_inner("First"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![
                    field("value", named_type("I32"), true),
                    field("shared", named_type("I64"), true),
                ]),
            },
            EnumVariant {
                name: generic_type_inner("Second"),
                fields: NamedFieldsOrTypesList::NamedFields(vec![field(
                    "value",
                    named_type("Bool"),
                    true,
                )]),
            },
        ],
        exported: false,
        language_items: Default::default(),
    };

    let hir_enum = build_enum_with_id(&mut context, &decl, def_id(1));

    assert_eq!(hir_enum.variants[0].id, VariantId(0));
    assert_eq!(hir_enum.variants[1].id, VariantId(1));
    let HirVariantFields::Named(first_fields) = &hir_enum.variants[0].fields else {
        panic!("expected first variant to have named fields");
    };
    let HirVariantFields::Named(second_fields) = &hir_enum.variants[1].fields else {
        panic!("expected second variant to have named fields");
    };
    assert_eq!(first_fields[0].id, FieldId(0));
    assert_eq!(first_fields[1].id, FieldId(1));
    assert_eq!(second_fields[0].id, FieldId(2));
}

#[test]
fn test_build_function_sig_handles_curried_types_and_where_bounds() {
    let mut lowerer = CollectContext::new();
    let show_id = DefId::new(CrateId(0), LocalDefId(11));
    let eq_id = DefId::new(CrateId(0), LocalDefId(12));
    lowerer.traits.insert(
        "Show".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: show_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    lowerer.traits.insert(
        "Eq".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: eq_id,
            name: "Eq".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let sig = function_sig(
        "compose",
        ParseType::Function(vec![
            generic_type("T"),
            named_type("I32"),
            named_type("Bool"),
            named_type("U8"),
        ]),
        Some(SelfReceiverMode::Shared),
        vec![
            WhereClause {
                subject: generic_type("T"),
                trait_bound: Some(named_type("Show")),
            },
            WhereClause {
                subject: generic_type("T"),
                trait_bound: Some(named_type("Eq")),
            },
        ],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(20));

    assert_eq!(hir_sig.name, "compose");
    assert_eq!(
        hir_sig.generic_params,
        vec![
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(20),
                    index: 0,
                },
                "T",
            ),
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(20),
                    index: 1,
                },
                "Self",
            ),
        ]
    );
    let t_id = hir_sig.generic_params[0].id;
    let self_id = hir_sig.generic_params[1].id;
    assert_eq!(
        hir_sig.params,
        vec![
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Generic(self_id)),
            },
            Type::Generic(t_id),
            Type::I32,
            Type::Bool
        ]
    );
    assert_eq!(hir_sig.ret, Type::U8);
    assert_eq!(
        hir_sig.self_receiver,
        Some(crate::types::ReceiverMode::Shared)
    );
    assert_eq!(
        hir_sig.generic_bounds.get(&t_id),
        Some(&vec![
            TraitBound {
                trait_id: show_id,
                type_args: Vec::new(),
            },
            TraitBound {
                trait_id: eq_id,
                type_args: Vec::new(),
            },
        ])
    );
}

#[test]
fn test_build_function_sig_preserves_returned_function_types() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "make",
        ParseType::Function(vec![
            named_type("I64"),
            ParseType::Function(vec![
                ParseType::Unit(crate::lexer::Span::test()),
                named_type("I64"),
            ]),
        ]),
        None,
        Vec::new(),
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(21));

    assert_eq!(hir_sig.params, vec![Type::I64]);
    assert_eq!(hir_sig.ret, Type::function(Vec::new(), Type::I64));
}

#[test]
fn test_build_function_sig_omits_unknown_where_clause_trait_bounds() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "identity",
        ParseType::Function(vec![generic_type("T"), generic_type("T")]),
        None,
        vec![WhereClause {
            subject: generic_type("T"),
            trait_bound: Some(named_type("Missing")),
        }],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(22));

    assert!(hir_sig.generic_bounds.is_empty());
    assert!(lowerer
        .errors
        .iter()
        .any(|error| error.message == "unknown trait 'Missing' in where clause"));
    assert!(lowerer.errors.iter().any(|error| {
        error.message == "unknown trait 'Missing' in where clause"
            && error.span() == Some(Span::test())
    }));
}

#[test]
fn test_build_function_sig_preserves_self_receiver_for_signature_backed_method() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "render",
        ParseType::Function(vec![named_type("I64"), named_type("Bool")]),
        Some(SelfReceiverMode::Mut),
        vec![],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(23));

    assert_eq!(hir_sig.self_receiver, Some(crate::types::ReceiverMode::Mut));
    assert!(matches!(
        hir_sig.params[0],
        Type::Reference { mutable: true, .. }
    ));
    assert_eq!(
        hir_sig.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(23),
                index: 0,
            },
            "Self",
        )]
    );
    assert_eq!(hir_sig.params[1], Type::I64);
    assert_eq!(hir_sig.ret, Type::Bool);
}

#[test]
fn test_build_function_sig_preserves_explicit_generic_names() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "identity",
        ParseType::Function(vec![named_type("A"), named_type("A")]),
        None,
        vec![],
    );

    let hir_sig = build_function_sig_with_id(&mut lowerer, &sig, def_id(24));

    assert_eq!(
        hir_sig.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: def_id(24),
                index: 0,
            },
            "A",
        )]
    );
}

#[test]
fn test_build_function_header_tracks_function_type_vars() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);

    let function = build_function_header_with_id(&mut lowerer, &decl, def_id(40));

    assert_eq!(function.name, "map");
    assert!(function.is_curried);
    assert_eq!(function.params.len(), 1);
    let mut expected = HashSet::new();
    collect_type_var_ids(&function.params[0].ty, &mut expected);
    collect_type_var_ids(&function.ret_type, &mut expected);
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&expected)
    );
}

#[test]
fn test_build_function_header_uses_provided_function_id() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("map", &["f", "x"], None, LambdaArrowKind::Curried);
    let function_id = def_id(44);

    let function = build_function_header_with_id(&mut lowerer, &decl, function_id);

    assert_eq!(function.id, function_id);
}

#[test]
fn test_build_function_header_with_sig_uses_provided_function_id_for_generic_owner() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
    let signature_id = def_id(11);
    let function_id = def_id(45);
    let sig_generic = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let sig = HirFunctionSig {
        id: signature_id,
        name: "id".to_string(),
        generic_params: vec![GenericParamDecl::type_param(sig_generic, "T")],
        params: vec![Type::Generic(sig_generic)],
        ret: Type::Generic(sig_generic),
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    assert_eq!(function.id, function_id);
    assert_eq!(
        function.generic_params,
        vec![GenericParamDecl::type_param(
            GenericParamId {
                owner: function_id,
                index: 0,
            },
            "T",
        )]
    );
    assert_eq!(
        function.params[0].ty,
        Type::Generic(GenericParamId {
            owner: function_id,
            index: 0,
        })
    );
    assert_eq!(
        function.ret_type,
        Type::Generic(GenericParamId {
            owner: function_id,
            index: 0,
        })
    );
}

#[test]
fn test_build_function_header_with_sig_uses_signature_types_and_tracks_type_vars() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("id", &["value"], None, LambdaArrowKind::Normal);
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(1)),
        name: "id".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::TypeVar(TypeVarId(41))],
        ret: Type::TypeVar(TypeVarId(99)),
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(41));

    assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(41)));
    assert_eq!(function.ret_type, Type::TypeVar(TypeVarId(99)));
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&HashSet::from([TypeVarId(41), TypeVarId(99)]))
    );
}

#[test]
fn test_build_function_header_with_sig_keeps_self_receiver_for_signature_backed_method() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl(
        "render",
        &["value"],
        Some(SelfReceiverMode::Shared),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        name: "render".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::TypeVar(TypeVarId(7)), Type::I64],
        ret: Type::Bool,
        generic_bounds: HashMap::new().into(),
        self_receiver: Some(crate::types::ReceiverMode::Shared),
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(42));

    assert!(function.is_method);
    assert_eq!(
        function.self_receiver,
        Some(crate::types::ReceiverMode::Shared)
    );
    assert_eq!(function.params.len(), 2);
    assert_eq!(function.params[0].ty, Type::TypeVar(TypeVarId(7)));
    assert_eq!(function.params[1].name, "value");
    assert_eq!(function.params[1].ty, Type::I64);
    assert_eq!(function.ret_type, Type::Bool);
    assert_eq!(
        lowerer.function_type_vars.get(&function.id),
        Some(&HashSet::from([TypeVarId(7)]))
    );
}

#[test]
fn test_build_function_header_with_sig_remaps_hidden_mut_self_receiver_generic() {
    let mut lowerer = CollectContext::new();
    let signature_id = def_id(46);
    let function_id = def_id(47);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let decl = function_decl(
        "set",
        &["value"],
        Some(SelfReceiverMode::Mut),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
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

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    let expected_self = GenericParamId {
        owner: function_id,
        index: 0,
    };
    assert_eq!(
        function.generic_params,
        vec![GenericParamDecl::type_param(expected_self, "Self")]
    );
    assert_eq!(function.params[0].local_id, crate::ids::HirLocalId(0));
    assert!(function.params[0].mutable);
    assert_eq!(
        function.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(function.params[1].ty, Type::I64);
}

#[test]
fn test_build_function_header_with_sig_keeps_public_generic_name_paired_after_hidden_self() {
    let mut lowerer = CollectContext::new();
    let signature_id = def_id(48);
    let function_id = def_id(49);
    let signature_self = GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let signature_t = GenericParamId {
        owner: signature_id,
        index: 1,
    };
    let decl = function_decl(
        "set",
        &["value"],
        Some(SelfReceiverMode::Mut),
        LambdaArrowKind::Normal,
    );
    let sig = HirFunctionSig {
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

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, function_id);

    let expected_t = GenericParamId {
        owner: function_id,
        index: 0,
    };
    let expected_self = GenericParamId {
        owner: function_id,
        index: 1,
    };
    assert_eq!(
        function.generic_params,
        vec![
            GenericParamDecl::type_param(expected_t, "T"),
            GenericParamDecl::type_param(expected_self, "Self"),
        ]
    );
    assert_eq!(
        function.params[0].ty,
        Type::Reference {
            mutable: true,
            inner: Box::new(Type::Generic(expected_self)),
        }
    );
    assert_eq!(function.params[1].ty, Type::Generic(expected_t));
    assert_eq!(function.ret_type, Type::Generic(expected_t));
}

#[test]
fn test_build_function_header_with_sig_keeps_curried_signature_shape() {
    let mut lowerer = CollectContext::new();
    let decl = function_decl("compose", &["f", "x"], None, LambdaArrowKind::Curried);
    let sig = HirFunctionSig {
        id: DefId::new(CrateId(0), LocalDefId(3)),
        name: "compose".to_string(),
        generic_params: Vec::new(),
        params: vec![Type::I32, Type::Bool],
        ret: Type::U8,
        generic_bounds: HashMap::new().into(),
        self_receiver: None,
        is_unsafe: false,
    };

    let function = build_function_header_with_sig(&mut lowerer, &decl, &sig, def_id(43));

    assert!(function.is_curried);
    assert_eq!(function.params.len(), 1);
    assert_eq!(function.params[0].name, "f");
    assert_eq!(function.params[0].ty, Type::I32);
    assert_eq!(
        function.ret_type,
        Type::function(vec![Type::Bool], Type::U8)
    );
}

#[test]
fn test_build_trait_uses_temporary_trait_context_and_restores_it() {
    let mut lowerer = CollectContext::new();
    lowerer.current_trait = Some("Outer".to_string());
    lowerer.current_trait_generics = vec!["X".to_string()];

    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Iterable".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![AssociatedTypeDecl {
            name: ident("Item"),
            kind: None,
        }],
        methods: HashMap::from([(
            ident("next"),
            function_decl(
                "next",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::from([(
            ident("collect"),
            function_sig(
                "collect",
                ParseType::Associated {
                    base: type_inner("Self"),
                    member: ident("Item"),
                },
                None,
                vec![],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let member_ids = trait_member_ids(&[("next", def_id(2))], &[("collect", def_id(3))]);
    let hir_trait = build_trait_with_id(&mut lowerer, &decl, def_id(1), &member_ids);

    assert_eq!(hir_trait.name, "Iterable");
    assert!(hir_trait.methods.contains_key("next"));
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_trait.methods["next"].id));
    assert_eq!(lowerer.current_trait.as_deref(), Some("Outer"));
    assert_eq!(lowerer.current_trait_generics, vec!["X".to_string()]);
    assert_eq!(
        hir_trait.signatures["collect"].ret,
        Type::Projection {
            ty: Box::new(Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 1
            })),
            trait_id: def_id(1),
            assoc_type: crate::types::AssociatedTypeKey {
                owner: def_id(1),
                assoc_type_id: crate::ids::AssocTypeId(0),
            },
            trait_args: vec![Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 0
            })],
        }
    );
}

#[test]
fn build_trait_lowers_constructor_target_and_typed_supertrait_predicate() {
    let functor_id = def_id(70);
    let applicative_id = def_id(71);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Functor".to_string(),
        HirTrait {
            id: functor_id,
            name: "Functor".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: functor_id,
                    index: 0,
                },
                "F",
                crate::type_services::kind::Kind::arrow(
                    crate::type_services::kind::Kind::Type,
                    crate::type_services::kind::Kind::Type,
                ),
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let target = ast::GenericParamDecl {
        default: None,
        name: ident("F"),
        kind: Some(ast::TypeApplication {
            constructor: Box::new(named_type("F")),
            args: vec![ast::ParseType::Hole(ast::TypeHole { span: Span::test() })],
            span: Span::test(),
        }),
        span: Span::test(),
    };
    let declaration = TraitDecl {
        name: type_inner("Applicative"),
        generic_params: Vec::new(),
        for_: Some(target),
        where_clauses: vec![ast::WhereClause {
            subject: named_type("F"),
            trait_bound: Some(named_type("Functor")),
        }],
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };

    let trait_def = build_trait_with_id(
        &mut context,
        &declaration,
        applicative_id,
        &trait_member_ids(&[], &[]),
    );

    let target = trait_def.target.expect("constructor target");
    assert_eq!(
        target.kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        trait_def.predicates,
        vec![crate::types::Predicate::Trait {
            subject: Type::Generic(target.id),
            trait_id: functor_id,
            args: Vec::new(),
        }]
    );
}

#[test]
fn typed_predicate_preserves_arbitrary_subjects() {
    let owner = def_id(72);
    let trait_id = def_id(73);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Show".to_string(),
        HirTrait {
            id: trait_id,
            name: "Show".to_string(),
            generic_params: Vec::new(),
            target: None,
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let previous = context.push_generic_context(owner, vec!["T".to_string()]);
    let clauses = vec![ast::WhereClause {
        subject: ast::ParseType::Tuple(vec![named_type("T"), named_type("I64")]),
        trait_bound: Some(named_type("Show")),
    }];

    let bounds = lower_where_clauses(&mut context, &clauses);
    context.pop_generic_context(previous.0, previous.1);

    assert!(bounds.is_empty());
    assert_eq!(
        bounds.predicates,
        vec![crate::types::Predicate::Trait {
            subject: Type::Tuple(vec![
                Type::Generic(GenericParamId { owner, index: 0 }),
                Type::I64,
            ]),
            trait_id,
            args: Vec::new(),
        }]
    );
}

#[test]
fn build_trait_method_reuses_trait_generic_without_redeclaring_it() {
    let mut context = CollectContext::new();
    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Mapper".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("map"),
            function_sig(
                "map",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let member_ids = trait_member_ids(&[], &[("map", def_id(2))]);
    let hir_trait = build_trait_with_id(&mut context, &decl, def_id(1), &member_ids);
    let expected_generic = crate::types::GenericParamId {
        owner: hir_trait.id,
        index: 0,
    };
    let sig = &hir_trait.signatures["map"];

    assert!(sig.generic_params.is_empty());
    assert_eq!(sig.params[0], Type::Generic(expected_generic));
    assert_eq!(sig.ret, Type::Generic(expected_generic));
}

#[test]
fn trait_member_owned_generics_do_not_collide_across_members() {
    let mut context = CollectContext::new();
    let trait_id = def_id(1);
    let first_id = def_id(2);
    let second_id = def_id(3);
    let signature = |name| {
        function_sig(
            name,
            ParseType::Function(vec![generic_type("T"), generic_type("U")]),
            None,
            vec![],
        )
    };
    let decl = TraitDecl {
        where_clauses: Vec::new(),
        name: ParseTypeInner {
            name: "Mapper".to_string(),
            generics: vec![],
            span: Span::test(),
        },
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([
            (ident("first"), signature("first")),
            (ident("second"), signature("second")),
        ]),
        exported: false,
        language_items: Default::default(),
    };
    let member_ids = trait_member_ids(&[], &[("first", first_id), ("second", second_id)]);

    let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
    let inherited = Type::Generic(GenericParamId {
        owner: trait_id,
        index: 0,
    });
    for (name, member_id) in [("first", first_id), ("second", second_id)] {
        let signature = &hir_trait.signatures[name];
        let member_generic = GenericParamId {
            owner: member_id,
            index: 0,
        };
        assert_eq!(
            signature.generic_params,
            vec![GenericParamDecl::type_param(member_generic, "U")]
        );
        assert_eq!(signature.params, vec![inherited.clone()]);
        assert_eq!(signature.ret, Type::Generic(member_generic));
    }
}

#[test]
fn trait_member_bound_only_generic_uses_member_owner() {
    let mut context = CollectContext::new();
    let trait_id = def_id(1);
    let member_id = def_id(2);
    let marker_id = def_id(3);
    context.traits.insert(
        "Marker".to_string(),
        HirTrait {
            id: marker_id,
            name: "Marker".to_string(),
            generic_params: vec![],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let decl = TraitDecl {
        where_clauses: vec![],
        name: type_inner("Bounded"),
        generic_params: vec![generic_param("T")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("check"),
            function_sig(
                "check",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![WhereClause {
                    subject: generic_type("U"),
                    trait_bound: Some(named_type("Marker")),
                }],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };
    let member_ids = trait_member_ids(&[], &[("check", member_id)]);

    let hir_trait = build_trait_with_id(&mut context, &decl, trait_id, &member_ids);
    let signature = &hir_trait.signatures["check"];
    let member_generic = GenericParamId {
        owner: member_id,
        index: 0,
    };
    assert_eq!(
        signature.generic_params,
        vec![GenericParamDecl::type_param(member_generic, "U")]
    );
    assert_eq!(
        signature.generic_bounds[&member_generic][0].trait_id,
        marker_id
    );
}

#[test]
fn constructor_kind_discovery_requires_explicit_holes() {
    let application = |arg| {
        ParseType::Application(ast::TypeApplication {
            constructor: Box::new(generic_type("F")),
            args: vec![arg],
            span: Span::test(),
        })
    };
    let mut kinds = HashMap::new();
    collect_constructor_generic_kinds(&application(generic_type("A")), &mut kinds, &|_| false);
    assert!(!kinds.contains_key("F"));

    collect_constructor_generic_kinds(
        &application(ParseType::Hole(ast::TypeHole { span: Span::test() })),
        &mut kinds,
        &|_| false,
    );
    assert_eq!(
        kinds["F"],
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
}

#[test]
fn constructor_binder_does_not_rekind_existing_ordinary_generic() {
    let mut context = CollectContext::new();
    let marker_id = def_id(3);
    context.traits.insert(
        "Marker".to_string(),
        HirTrait {
            id: marker_id,
            name: "Marker".to_string(),
            generic_params: vec![],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let constructor_subject = ParseType::Application(ast::TypeApplication {
        constructor: Box::new(generic_type("F")),
        args: vec![ParseType::Hole(ast::TypeHole { span: Span::test() })],
        span: Span::test(),
    });
    let decl = TraitDecl {
        where_clauses: vec![],
        name: type_inner("Ordinary"),
        generic_params: vec![generic_param("F")],
        for_: None,
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::from([(
            ident("check"),
            function_sig(
                "check",
                ParseType::Function(vec![generic_type("T"), generic_type("T")]),
                None,
                vec![WhereClause {
                    subject: constructor_subject,
                    trait_bound: Some(named_type("Marker")),
                }],
            ),
        )]),
        exported: false,
        language_items: Default::default(),
    };

    let hir_trait = build_trait_with_id(
        &mut context,
        &decl,
        def_id(1),
        &trait_member_ids(&[], &[("check", def_id(2))]),
    );

    assert_eq!(
        hir_trait.generic_params[0].kind,
        crate::type_services::kind::Kind::Type
    );
    assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but its constructor binder requires Type -> Type"
        )));
}

#[test]
fn build_impl_resolves_trait_id_through_import_alias() {
    let mut context = CollectContext::new();
    let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
    let assoc_id = crate::ids::AssocTypeId(7);
    context
        .import_aliases
        .insert("Deref".to_string(), "std::deref::Deref".to_string());
    context.traits.insert(
        "std::deref::Deref".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Deref".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_id,
                name: "Target".to_string(),
                kind: crate::type_services::kind::Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let imp = Impl {
        name: ParseType::Type(type_inner("Deref")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Target"),
            kind: None,
            ty: ParseType::Reference {
                is_mut: false,
                pointee: Box::new(ParseType::Slice(Box::new(generic_type("T")))),
            },
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let method_ids = impl_method_ids(&[]);
    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.trait_id, Some(trait_id));
    assert_eq!(hir_impl.associated_types[0].id, assoc_id);
}

#[test]
fn build_impl_resolves_trait_id_through_canonical_import_alias() {
    let mut context = CollectContext::new();
    let trait_id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(10));
    context
        .canonical_import_aliases
        .insert("Clone".to_string(), trait_id);
    context.traits.insert(
        "std::clone::Clone".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Clone".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let imp = Impl {
        name: ParseType::Type(type_inner("Clone")),
        for_: Some(ParseType::Type(type_inner("String"))),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let method_ids = impl_method_ids(&[]);
    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.trait_id, Some(trait_id));
}

#[test]
fn build_trait_assigns_source_order_associated_type_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner("Left"),
        generic_params: vec![],
        for_: None,
        associated_types: vec![AssociatedTypeDecl {
            name: ident("Item"),
            kind: None,
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };
    let right = TraitDecl {
        where_clauses: Vec::new(),
        name: type_inner("Right"),
        generic_params: vec![],
        for_: None,
        associated_types: vec![
            AssociatedTypeDecl {
                name: ident("Item"),
                kind: None,
            },
            AssociatedTypeDecl {
                name: ident("Error"),
                kind: None,
            },
        ],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        exported: false,
        language_items: Default::default(),
    };

    let left_member_ids = trait_member_ids(&[], &[]);
    let right_member_ids = trait_member_ids(&[], &[]);
    let left = build_trait_with_id(&mut context, &left, def_id(1), &left_member_ids);
    let right = build_trait_with_id(&mut context, &right, def_id(2), &right_member_ids);

    assert_eq!(left.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[1].id, AssocTypeId(1));
}

#[test]
fn test_build_extern_registers_scope_entry() {
    let mut lowerer = CollectContext::new();
    let sig = function_sig(
        "puts",
        ParseType::Function(vec![
            ParseType::Pointer(Box::new(named_type("U8"))),
            named_type("I32"),
        ]),
        None,
        vec![],
    );

    let hir_extern = build_extern_with_id(&mut lowerer, &sig, sig.name.name.clone(), def_id(1));

    assert_eq!(hir_extern.name, "puts");
    assert_eq!(hir_extern.params, vec![Type::Pointer(Box::new(Type::U8))]);
    assert_eq!(hir_extern.ret, Type::I32);
    assert!(!hir_extern.variadic);
    let binding = lowerer.scope.lookup("puts").expect("extern is registered");
    assert_eq!(
        binding.ty,
        Type::function(vec![Type::Pointer(Box::new(Type::U8))], Type::I32)
    );
    assert!(!binding.mutable);
}

#[test]
fn test_build_impl_registers_methods_and_tracks_function_type_vars_by_id() {
    let mut lowerer = CollectContext::new();
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Show".to_string(),
            generics: vec![],
            span: Span::test(),
        }),
        for_: Some(named_type("Point")),
        associated_types: vec![],
        methods: HashMap::from([(
            ident("render"),
            function_decl(
                "render",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let method_ids = impl_method_ids(&[("render", def_id(2))]);
    let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.type_name, "Point");
    assert_eq!(hir_impl.trait_name.as_deref(), Some("Show"));
    assert!(hir_impl.methods.contains_key("render"));
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_impl.methods["render"].id));
}

#[test]
fn build_impl_assigns_source_order_associated_type_def_ids_per_owner() {
    let mut context = CollectContext::new();
    let left = Impl {
        name: ParseType::Type(type_inner("Iterable")),
        for_: Some(named_type("List")),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Item"),
            kind: None,
            ty: named_type("I32"),
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };
    let right = Impl {
        name: ParseType::Type(type_inner("Iterable")),
        for_: Some(named_type("Tree")),
        associated_types: vec![
            AssociatedTypeDef {
                name: ident("Item"),
                kind: None,
                ty: named_type("I32"),
            },
            AssociatedTypeDef {
                name: ident("Error"),
                kind: None,
                ty: named_type("Bool"),
            },
        ],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let left_method_ids = impl_method_ids(&[]);
    let right_method_ids = impl_method_ids(&[]);
    let left = build_impl_with_id(&mut context, &left, def_id(1), &left_method_ids);
    let right = build_impl_with_id(&mut context, &right, def_id(2), &right_method_ids);

    assert_eq!(left.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[0].id, AssocTypeId(0));
    assert_eq!(right.associated_types[1].id, AssocTypeId(1));
}

#[test]
fn build_impl_preserves_constructor_valued_associated_type_kind() {
    let mut context = CollectContext::new();
    let box_id = def_id(210);
    let trait_id = def_id(211);
    context.structs.insert(
        "Box".to_string(),
        HirStruct {
            id: box_id,
            name: "Box".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: box_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    context.traits.insert(
        "Families".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Families".to_string(),
            generic_params: Vec::new(),
            associated_types: vec![HirAssociatedTypeDecl {
                id: AssocTypeId(0),
                name: "Family".to_string(),
                kind: crate::type_services::kind::Kind::arrow(
                    crate::type_services::kind::Kind::Type,
                    crate::type_services::kind::Kind::Type,
                ),
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Families")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Box".to_string(),
            generics: vec![named_type("I64")],
            span: Span::test(),
        })),
        associated_types: vec![AssociatedTypeDef {
            name: ident("Family"),
            kind: Some(constructor_kind_syntax("Family", 1)),
            ty: named_type("Box"),
        }],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(212), &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(hir_impl.associated_types[0].id, AssocTypeId(0));
    assert_eq!(
        hir_impl.associated_types[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(
        hir_impl.associated_types[0].ty,
        Type::Constructor {
            id: box_id,
            flavor: crate::types::NominalTypeKind::Struct,
        }
    );
}

#[test]
fn build_impl_accepts_constructor_generic_in_receiver_pattern() {
    let impl_id = def_id(213);
    let receiver = ParseType::Application(ast::TypeApplication {
        constructor: Box::new(named_type("F")),
        args: vec![named_type("A")],
        span: Span::test(),
    });
    let imp = Impl {
        name: ParseType::Type(type_inner("Functor")),
        for_: Some(receiver),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![ast::WhereClause {
            subject: ParseType::Application(constructor_kind_syntax("F", 1)),
            trait_bound: None,
        }],
    };
    let mut context = CollectContext::new();

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(
        hir_impl
            .type_generics
            .iter()
            .find(|param| param.name == "F")
            .unwrap()
            .kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::Type,
            crate::type_services::kind::Kind::Type,
        )
    );
    assert!(matches!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Apply { .. })
    ));
}

#[test]
fn build_impl_does_not_rekind_existing_ordinary_generic() {
    let trait_id = def_id(214);
    let impl_id = def_id(215);
    let mut context = CollectContext::new();
    context.traits.insert(
        "Holder".to_string(),
        HirTrait {
            id: trait_id,
            name: "Holder".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
            )],
            target: None,
            predicates: vec![],
            associated_types: vec![],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Holder".to_string(),
            generics: vec![generic_type("F")],
            span: Span::test(),
        }),
        for_: Some(named_type("I64")),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![WhereClause {
            subject: ParseType::Application(constructor_kind_syntax("F", 1)),
            trait_bound: None,
        }],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert_eq!(
        hir_impl
            .type_generics
            .iter()
            .find(|param| param.name == "F")
            .expect("impl F generic")
            .kind,
        crate::type_services::kind::Kind::Type
    );
    assert!(context.errors.iter().any(|error| error.message.contains(
            "generic parameter 'F' was declared with kind Type, but a later binder requires Type -> Type"
        )));
}

#[test]
fn build_impl_records_constructor_trait_target_explicitly() {
    let trait_id = def_id(214);
    let option_id = def_id(215);
    let impl_id = def_id(216);
    let mut context = CollectContext::new();
    context.enums.insert(
        "Option".to_string(),
        HirEnum {
            id: option_id,
            name: "Option".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: option_id,
                    index: 0,
                },
                "T",
            )],
            variants: Vec::new(),
        },
    );
    context.traits.insert(
        "Functor".to_string(),
        HirTrait {
            id: trait_id,
            name: "Functor".to_string(),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
                crate::type_services::kind::Kind::arrow(
                    crate::type_services::kind::Kind::Type,
                    crate::type_services::kind::Kind::Type,
                ),
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Functor")),
        for_: Some(named_type("Option")),
        associated_types: Vec::new(),
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: Vec::new(),
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, impl_id, &HashMap::new());

    assert!(context.errors.is_empty(), "{:?}", context.errors);
    assert_eq!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Constructor(Type::Constructor {
            id: option_id,
            flavor: crate::types::NominalTypeKind::Enum,
        })
    );
}

#[test]
fn test_build_impl_tracks_receiver_and_trait_generics() {
    let mut lowerer = CollectContext::new();
    let vec_id = def_id(3);
    lowerer.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(ParseTypeInner {
            name: "Show".to_string(),
            generics: vec![named_type("U8")],
            span: Span::test(),
        }),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::from([(
            ident("render"),
            function_decl(
                "render",
                &[],
                Some(SelfReceiverMode::Shared),
                LambdaArrowKind::Normal,
            ),
        )]),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let method_ids = impl_method_ids(&[("render", def_id(2))]);
    let hir_impl = build_impl_with_id(&mut lowerer, &imp, def_id(1), &method_ids);

    assert_eq!(hir_impl.type_name, "Vec");
    assert_eq!(
        hir_impl.receiver_pattern,
        HirImplReceiverPattern::Exact(Type::Struct {
            id: vec_id,
            args: vec![Type::Generic(crate::types::GenericParamId {
                owner: def_id(1),
                index: 0
            })],
        })
    );
    assert_eq!(hir_impl.trait_arg_types, vec![Type::U8]);
    assert!(lowerer
        .function_type_vars
        .contains_key(&hir_impl.methods["render"].id));
}

#[test]
fn build_impl_uses_marked_renamed_sized_trait_id() {
    let mut context = CollectContext::new();
    let sized_id = def_id(90);
    let vec_id = def_id(91);
    context.language_items.sized = Some(SizedLanguageItems { trait_id: sized_id });
    context.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    context.traits.insert(
        "StaticLayout".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: sized_id,
            name: "StaticLayout".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Vec")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(92), &impl_method_ids(&[]));

    assert_eq!(
        hir_impl.bounds[&GenericParamId {
            owner: hir_impl.id,
            index: 0,
        }],
        vec![TraitBound {
            trait_id: sized_id,
            type_args: Vec::new(),
        }]
    );
}

#[test]
fn build_impl_does_not_inject_sized_bound_without_marked_bundle() {
    let mut context = CollectContext::new();
    let vec_id = def_id(93);
    context.structs.insert(
        "Vec".to_string(),
        HirStruct {
            id: vec_id,
            name: "Vec".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: vec_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );
    let imp = Impl {
        name: ParseType::Type(type_inner("Vec")),
        for_: Some(ParseType::Type(ParseTypeInner {
            name: "Vec".to_string(),
            generics: vec![generic_type("T")],
            span: Span::test(),
        })),
        associated_types: vec![],
        methods: HashMap::new(),
        signatures: HashMap::new(),
        where_clauses: vec![],
    };

    let hir_impl = build_impl_with_id(&mut context, &imp, def_id(94), &impl_method_ids(&[]));

    assert!(hir_impl.bounds.is_empty());
}
