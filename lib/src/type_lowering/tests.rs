use std::collections::HashMap;

use super::*;

use crate::ast::{GenericParamDecl as AstGenericParamDecl, Ident, ParseType, ParseTypeInner};
use crate::hir::{HirAssociatedTypeDecl, HirEnum, HirStruct, HirTrait};
use crate::ids::{AssocTypeId, CrateId, DefId, Idx, LocalDefId, TypeVarId};
use crate::lexer::Span;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, NominalTypeKind};

#[derive(Default)]
struct TestTypeContext {
    errors: Vec<String>,
    structs: HashMap<String, HirStruct>,
    enums: HashMap<String, HirEnum>,
    traits: HashMap<String, HirTrait>,
    current_trait: Option<String>,
    generic_owner: Option<DefId>,
    generic_params: Vec<String>,
    generic_kinds: HashMap<String, Kind>,
    /// `Some` when the context can create inference variables for `_`; the
    /// kinds of the variables handed out so far.
    hole_kinds: Option<Vec<Kind>>,
}

impl TypeLoweringContext for TestTypeContext {
    fn push_type_error(&mut self, message: String, _span: crate::lexer::Span) {
        self.errors.push(message);
    }

    fn fresh_type_hole(&mut self, _span: crate::lexer::Span, kind: Kind) -> Option<Type> {
        let kinds = self.hole_kinds.as_mut()?;
        kinds.push(kind);
        Some(Type::TypeVar(TypeVarId::from_raw(kinds.len() as u32 - 1)))
    }

    fn current_module_prefix(&self) -> Option<String> {
        None
    }

    fn current_trait_name(&self) -> Option<String> {
        self.current_trait.clone()
    }

    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
        self.structs
            .get(name)
            .cloned()
            .map(ResolvedNominalType::Struct)
            .or_else(|| self.enums.get(name).cloned().map(ResolvedNominalType::Enum))
    }

    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
        self.traits.get(name).cloned()
    }

    fn resolve_type_alias(&self, _name: &str) -> Option<HirTypeAlias> {
        None
    }

    fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type {
        let Some(owner) = self.generic_owner else {
            self.push_type_error(format!("unknown type '{}'", name), span.clone());
            return Type::Error;
        };
        let Some(index) = self
                .generic_params
                .iter()
                .position(|param| param == name)
                .or_else(|| {
                    let mut chars = name.chars();
                    let is_implicit_generic = matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
                        && chars.next().is_none();
                    if is_implicit_generic {
                        self.generic_params.push(name.to_string());
                        Some(self.generic_params.len() - 1)
                    } else {
                        None
                    }
                })
            else {
                self.push_type_error(format!("unknown type '{}'", name), span);
                return Type::Error;
            };

        Type::Generic(GenericParamId {
            owner,
            index: index as u32,
        })
    }

    fn populate_type_normalization_env(&self, env: &mut TypeNormalizationEnv) {
        for structure in self.structs.values() {
            env.register_constructor(
                structure.id,
                NominalTypeKind::Struct,
                constructor_kind(&structure.generic_params),
            );
        }
        for enumeration in self.enums.values() {
            env.register_constructor(
                enumeration.id,
                NominalTypeKind::Enum,
                constructor_kind(&enumeration.generic_params),
            );
        }
        for trait_def in self.traits.values() {
            for associated_type in &trait_def.associated_types {
                env.register_projection_kind(
                    AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
        }
        if let Some(owner) = self.generic_owner {
            for (index, name) in self.generic_params.iter().enumerate() {
                env.register_generic_kind(
                    GenericParamId {
                        owner,
                        index: index as u32,
                    },
                    self.generic_kinds.get(name).cloned().unwrap_or(Kind::Type),
                );
            }
        }
    }
}

fn named_type(name: &str) -> ParseType {
    ParseType::Type(ParseTypeInner {
        name: name.to_string(),
        generics: vec![],
        span: Span::test(),
    })
}

fn application(constructor: ParseType, args: Vec<ParseType>) -> ParseType {
    ParseType::Application(ast::TypeApplication {
        constructor: Box::new(constructor),
        args,
        span: Span::test(),
    })
}

#[test]
fn constructor_generic_application_preserves_its_argument() {
    let owner = def_id(21);
    let mut context = TestTypeContext {
        generic_owner: Some(owner),
        generic_params: vec!["F".to_string(), "A".to_string()],
        generic_kinds: HashMap::from([("F".to_string(), Kind::arrow(Kind::Type, Kind::Type))]),
        ..TestTypeContext::default()
    };

    let ty = TypeLowerer::lower_parse_type(
        &mut context,
        &application(named_type("F"), vec![named_type("A")]),
    );

    assert_eq!(
        ty,
        Type::Apply {
            constructor: Box::new(Type::Generic(GenericParamId { owner, index: 0 })),
            args: vec![Type::Generic(GenericParamId { owner, index: 1 })],
        }
    );
    assert!(context.errors.is_empty());
}

#[test]
fn result_section_and_explicit_lambda_lower_identically() {
    let result_id = def_id(22);
    let owner = def_id(23);
    let mut context = TestTypeContext {
        generic_owner: Some(owner),
        generic_params: vec!["E".to_string()],
        ..TestTypeContext::default()
    };
    context.enums.insert(
        "Result".to_string(),
        HirEnum {
            id: result_id,
            name: "Result".to_string(),
            generic_params: GenericParamDecl::type_params(result_id, ["T", "E"]),
            variants: Vec::new(),
        },
    );
    let section = application(
        named_type("Result"),
        vec![
            ParseType::Hole(ast::TypeHole { span: Span::test() }),
            named_type("E"),
        ],
    );
    let explicit = ParseType::Lambda(ast::TypeLambda {
        params: vec![AstGenericParamDecl {
            default: None,
            name: Ident {
                name: "T".to_string(),
                span: Span::test(),
            },
            kind: None,
            span: Span::test(),
        }],
        body: Box::new(application(
            named_type("Result"),
            vec![named_type("T"), named_type("E")],
        )),
        span: Span::test(),
    });

    let section_ty = TypeLowerer::lower_parse_type(&mut context, &section);
    let explicit_ty = TypeLowerer::lower_parse_type(&mut context, &explicit);

    assert_eq!(section_ty, explicit_ty);
    assert!(context.errors.is_empty());
}

#[test]
fn type_lowerer_consumes_canonical_nominal_resolution_result() {
    let struct_id = def_id(30);
    let mut context = TestTypeContext::default();
    context.structs.insert(
        "Alias".to_string(),
        HirStruct {
            id: struct_id,
            name: "dep::Actual".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        },
    );

    let ty = TypeLowerer::lower_parse_type(&mut context, &named_type("Alias"));

    assert_eq!(
        ty,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        }
    );
    assert!(context.errors.is_empty());
}

#[test]
fn kind_check_rejects_nominal_arity_overapplication() {
    let struct_id = def_id(31);
    let mut context = TestTypeContext::default();
    context.structs.insert(
        "Box".to_string(),
        HirStruct {
            id: struct_id,
            name: "Box".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        },
    );

    let ty = TypeLowerer::lower_parse_type(
        &mut context,
        &application(
            named_type("Box"),
            vec![named_type("I64"), named_type("Bool")],
        ),
    );

    assert_eq!(ty, Type::Error);
    assert!(context
        .errors
        .iter()
        .any(|message| message.contains("not applicable")));
}

#[test]
fn kind_check_rejects_nominal_argument_kind_mismatch() {
    let struct_id = def_id(32);
    let mut context = TestTypeContext::default();
    context.structs.insert(
        "Higher".to_string(),
        HirStruct {
            id: struct_id,
            name: "Higher".to_string(),
            generic_params: vec![GenericParamDecl::new(
                GenericParamId {
                    owner: struct_id,
                    index: 0,
                },
                "F",
                Kind::arrow(Kind::Type, Kind::Type),
            )],
            fields: Vec::new(),
        },
    );

    let ty = TypeLowerer::lower_parse_type(
        &mut context,
        &application(named_type("Higher"), vec![named_type("I64")]),
    );

    assert_eq!(ty, Type::Error);
    assert!(context
        .errors
        .iter()
        .any(|message| message.contains("kind mismatch")));
}

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn type_lowerer_reports_bare_slice_error() {
    let mut context = TestTypeContext::default();

    let ty =
        TypeLowerer::lower_parse_type(&mut context, &ParseType::Slice(Box::new(named_type("I64"))));

    assert_eq!(ty, Type::Error);
    assert!(context
        .errors
        .iter()
        .any(|message| message.contains("bare slice type [T]")));
}

#[test]
fn type_lowerer_lowers_associated_projection_with_trait_identity() {
    let trait_id = def_id(10);
    let assoc_type_id = AssocTypeId(2);
    let mut context = TestTypeContext {
        current_trait: Some("Iterator".to_string()),
        generic_owner: Some(trait_id),
        generic_params: vec!["Self".to_string()],
        ..TestTypeContext::default()
    };
    context.traits.insert(
        "Iterator".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Iterator".to_string(),
            generic_params: GenericParamDecl::type_params(trait_id, ["Self"]),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Item".to_string(),
                kind: Kind::Type,
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let ty = TypeLowerer::lower_parse_type(
        &mut context,
        &ParseType::Associated {
            base: ParseTypeInner {
                name: "Self".to_string(),
                generics: vec![],
                span: Span::test(),
            },
            member: Ident {
                name: "Item".to_string(),
                span: Span::test(),
            },
        },
    );

    assert_eq!(
        ty,
        Type::Projection {
            ty: Box::new(Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            })),
            trait_id,
            assoc_type: AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id,
            },
            trait_args: vec![Type::Generic(GenericParamId {
                owner: trait_id,
                index: 0,
            })],
        }
    );
    assert!(context.errors.is_empty());
}

#[test]
fn type_lowerer_applies_constructor_valued_associated_projection() {
    let trait_id = def_id(11);
    let assoc_type_id = AssocTypeId(0);
    let mut context = TestTypeContext {
        current_trait: Some("Families".to_string()),
        generic_owner: Some(trait_id),
        generic_params: vec!["Self".to_string()],
        ..TestTypeContext::default()
    };
    context.traits.insert(
        "Families".to_string(),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Families".to_string(),
            generic_params: GenericParamDecl::type_params(trait_id, ["Self"]),
            associated_types: vec![HirAssociatedTypeDecl {
                id: assoc_type_id,
                name: "Family".to_string(),
                kind: Kind::arrow(Kind::Type, Kind::Type),
            }],
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );
    let projection = ParseType::Associated {
        base: ParseTypeInner {
            name: "Self".to_string(),
            generics: Vec::new(),
            span: Span::test(),
        },
        member: Ident {
            name: "Family".to_string(),
            span: Span::test(),
        },
    };

    let ty = TypeLowerer::lower_parse_type(
        &mut context,
        &application(projection, vec![named_type("I64")]),
    );

    assert!(matches!(
        ty,
        Type::Apply {
            constructor,
            args
        } if matches!(*constructor, Type::Projection { assoc_type, .. }
            if assoc_type == (AssociatedTypeKey { owner: trait_id, assoc_type_id }))
            && args == vec![Type::I64]
    ));
    assert!(context.errors.is_empty());
}

#[test]
fn type_lowerer_creates_implicit_generic_in_context() {
    let owner = def_id(20);
    let mut context = TestTypeContext {
        generic_owner: Some(owner),
        ..TestTypeContext::default()
    };

    let ty = TypeLowerer::lower_parse_type(&mut context, &named_type("T"));

    assert_eq!(ty, Type::Generic(GenericParamId { owner, index: 0 }));
    assert_eq!(context.generic_params, vec!["T".to_string()]);
    assert!(context.errors.is_empty());
}

fn hole() -> ParseType {
    ParseType::Hole(ast::TypeHole { span: Span::test() })
}

fn context_with_result(result_id: DefId) -> TestTypeContext {
    let mut context = TestTypeContext {
        hole_kinds: Some(Vec::new()),
        ..TestTypeContext::default()
    };
    context.enums.insert(
        "Result".to_string(),
        HirEnum {
            id: result_id,
            name: "Result".to_string(),
            generic_params: GenericParamDecl::type_params(result_id, ["T", "E"]),
            variants: Vec::new(),
        },
    );
    context
}

#[test]
fn annotation_hole_in_proper_type_position_is_an_inference_variable() {
    let result_id = def_id(30);
    let mut context = context_with_result(result_id);

    let ty = TypeLowerer::lower_annotation_type(
        &mut context,
        &application(named_type("Result"), vec![hole(), named_type("I64")]),
    );

    assert_eq!(
        ty,
        Type::Enum {
            id: result_id,
            args: vec![Type::TypeVar(TypeVarId::from_raw(0)), Type::I64],
        }
    );
    assert_eq!(context.hole_kinds, Some(vec![Kind::Type]));
    assert!(context.errors.is_empty());
}

#[test]
fn bare_annotation_hole_is_an_inference_variable() {
    let mut context = TestTypeContext {
        hole_kinds: Some(Vec::new()),
        ..TestTypeContext::default()
    };

    let ty = TypeLowerer::lower_annotation_type(
        &mut context,
        &ParseType::Tuple(vec![hole(), named_type("Bool")]),
    );

    assert_eq!(
        ty,
        Type::Tuple(vec![Type::TypeVar(TypeVarId::from_raw(0)), Type::Bool])
    );
    assert!(context.errors.is_empty());
}

#[test]
fn annotation_hole_under_constructor_kinded_parameter_stays_a_section() {
    let result_id = def_id(31);
    let owner = def_id(32);
    let mut context = TestTypeContext {
        generic_owner: Some(owner),
        generic_params: vec!["F".to_string()],
        generic_kinds: HashMap::from([(
            "F".to_string(),
            Kind::arrow(Kind::arrow(Kind::Type, Kind::Type), Kind::Type),
        )]),
        ..context_with_result(result_id)
    };

    // `F (Result _, I64)` with `F: (Type -> Type) -> Type`: the argument of
    // `F` must be a constructor, so the hole keeps its section meaning even
    // inside a local annotation.
    let ty = TypeLowerer::lower_annotation_type(
        &mut context,
        &application(
            named_type("F"),
            vec![application(
                named_type("Result"),
                vec![hole(), named_type("I64")],
            )],
        ),
    );

    let Type::Apply { args, .. } = ty else {
        panic!("expected an application, got {ty:?}");
    };
    assert!(matches!(args.as_slice(), [Type::Lambda { .. }]), "{args:?}");
    assert_eq!(context.hole_kinds, Some(Vec::new()));
    assert!(context.errors.is_empty());
}

#[test]
fn hole_outside_local_annotations_is_rejected() {
    let mut context = TestTypeContext {
        hole_kinds: Some(Vec::new()),
        ..TestTypeContext::default()
    };

    let ty = TypeLowerer::lower_parse_type(&mut context, &hole());

    assert_eq!(ty, Type::Error);
    assert_eq!(
        context.errors,
        vec!["the type placeholder `_` is only allowed in local type annotations".to_string()]
    );
    assert_eq!(context.hole_kinds, Some(Vec::new()));
}
