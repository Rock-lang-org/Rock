use crate::ast::*;
use crate::language_items::LanguageItemRole;
use crate::lexer::Span;
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;
use std::path::PathBuf;

fn innermost_parse_error(error: &ParseError) -> &ParseError {
    match error {
        ParseError::WithContext { error, .. } => innermost_parse_error(error),
        error => error,
    }
}

fn assert_marker_error(source: &str, expected_message: &str) {
    let error = parse_string(source, &Config::default()).unwrap_err();
    let ParseError::HardError(message, _) = innermost_parse_error(&error) else {
        panic!("expected a language item marker error, got {error:?}");
    };

    assert!(
        message.contains(expected_message),
        "expected marker diagnostic containing {expected_message:?}, got {message:?}",
    );
}

#[test]
fn language_item_marker_reports_invalid_unknown_role() {
    assert_marker_error(
        "lang unrecognized\ntrait Marker\n    item: I64\n",
        "unknown language item role 'unrecognized'",
    );
}

#[test]
fn language_item_marker_reports_child_only_root_role() {
    assert_marker_error(
        "lang method\ntrait Marker\n    item: I64\n",
        "may only mark a language item member",
    );
    assert_marker_error(
        "lang owner_cancel\ntrait Reservation\n",
        "may only mark a language item member",
    );
}

#[test]
fn language_item_marker_reports_incompatible_target_roles() {
    assert_marker_error(
        "lang control_flow\ntrait Marker\n    item: I64\n",
        "may only mark an enum",
    );
    assert_marker_error(
        "lang sized\nenum Marker\n    Value\n",
        "may only mark a trait",
    );
}

#[test]
fn language_item_marker_reports_non_trait_or_enum_target() {
    assert_marker_error(
        "lang sized\nstruct Marker\n    item: I64\n",
        "must apply to a trait or enum declaration",
    );
}

#[test]
fn language_item_marker_reports_eof_after_marker() {
    let error = parse_string("lang sized\n", &Config::default()).unwrap_err();

    assert!(matches!(
        innermost_parse_error(&error),
        ParseError::UnexpectedEOF(_),
    ));
}

#[test]
fn language_item_marker_attaches_to_exported_trait_without_name_inference() {
    let program = parse_string(
        "lang sized\n< trait StaticLayout\n    layout: I64\n",
        &Config::default(),
    )
    .unwrap();

    let TopLevel::TraitDecl(trait_decl) = &program.module.top_levels[0] else {
        panic!("expected a trait declaration");
    };

    assert!(trait_decl.exported);
    assert_eq!(trait_decl.name.name, "StaticLayout");
    assert_eq!(
        trait_decl.language_items.root.as_ref().unwrap().role,
        LanguageItemRole::Sized,
    );
}

#[test]
fn language_item_marker_attaches_index_mut_trait_without_name_inference() {
    let program = parse_string(
        "lang index_mut\n< trait WriteAt Key\n    lang output\n    type Value\n    lang method\n    ^@write_at: Key -> &mut Self::Value\n",
        &Config::default(),
    )
    .unwrap();

    let TopLevel::TraitDecl(trait_decl) = &program.module.top_levels[0] else {
        panic!("expected a trait declaration");
    };

    assert_eq!(trait_decl.name.name, "WriteAt");
    assert_eq!(
        trait_decl
            .language_items
            .root
            .as_ref()
            .map(|marker| marker.role),
        Some(LanguageItemRole::IndexMut),
    );
}

#[test]
fn language_item_marker_attaches_to_exported_enum_without_name_inference() {
    let program = parse_string(
        "lang control_flow\n< enum Flow\n    KeepGoing\n",
        &Config::default(),
    )
    .unwrap();

    let TopLevel::EnumDecl(enum_decl) = &program.module.top_levels[0] else {
        panic!("expected an enum declaration");
    };

    assert!(enum_decl.exported);
    assert_eq!(enum_decl.name.name, "Flow");
    assert_eq!(
        enum_decl.language_items.root.as_ref().unwrap().role,
        LanguageItemRole::ControlFlow,
    );
}

#[test]
fn language_item_markers_attach_to_trait_members_in_source_order() {
    let program = parse_string(
        "lang try\ntrait Carrier\n    lang output\n    // the successful value\n\n    type Value\n    lang residual\n\n    type Remainder\n    lang branch\n    split : I64\n",
        &Config::default(),
    )
    .unwrap();

    let TopLevel::TraitDecl(trait_decl) = &program.module.top_levels[0] else {
        panic!("expected a trait declaration");
    };

    assert_eq!(trait_decl.name.name, "Carrier");
    assert_eq!(
        trait_decl.language_items.members,
        vec![
            LanguageItemMemberMarker {
                marker: LanguageItemMarker {
                    role: LanguageItemRole::Output,
                    span: trait_decl.language_items.members[0].marker.span.clone(),
                },
                kind: LanguageItemMemberKind::AssociatedType,
                member_name: "Value".to_string(),
            },
            LanguageItemMemberMarker {
                marker: LanguageItemMarker {
                    role: LanguageItemRole::Residual,
                    span: trait_decl.language_items.members[1].marker.span.clone(),
                },
                kind: LanguageItemMemberKind::AssociatedType,
                member_name: "Remainder".to_string(),
            },
            LanguageItemMemberMarker {
                marker: LanguageItemMarker {
                    role: LanguageItemRole::Branch,
                    span: trait_decl.language_items.members[2].marker.span.clone(),
                },
                kind: LanguageItemMemberKind::Method,
                member_name: "split".to_string(),
            },
        ],
    );
}

#[test]
fn language_item_markers_attach_to_enum_variants_in_source_order() {
    let program = parse_string(
        "lang control_flow\nenum Flow\n    lang break\n    Stop\n    lang continue\n    Next\n",
        &Config::default(),
    )
    .unwrap();

    let TopLevel::EnumDecl(enum_decl) = &program.module.top_levels[0] else {
        panic!("expected an enum declaration");
    };

    assert_eq!(enum_decl.name.name, "Flow");
    assert_eq!(
        enum_decl.language_items.members,
        vec![
            LanguageItemMemberMarker {
                marker: LanguageItemMarker {
                    role: LanguageItemRole::Break,
                    span: enum_decl.language_items.members[0].marker.span.clone(),
                },
                kind: LanguageItemMemberKind::Variant,
                member_name: "Stop".to_string(),
            },
            LanguageItemMemberMarker {
                marker: LanguageItemMarker {
                    role: LanguageItemRole::Continue,
                    span: enum_decl.language_items.members[1].marker.span.clone(),
                },
                kind: LanguageItemMemberKind::Variant,
                member_name: "Next".to_string(),
            },
        ],
    );
}

#[test]
fn language_item_marker_rejects_index_mut_on_trait_member() {
    assert_marker_error(
        "trait WriteAt Key\n    lang index_mut\n    type Value\n",
        "may only mark a trait root",
    );
}

#[test]
fn language_item_marker_rejects_index_mut_on_enum_variant() {
    let result = parse_string(
        "enum WriteAt\n    lang index_mut\n    Value\n",
        &Config::default(),
    );

    assert!(result.is_err(), "IndexMut must not mark an enum variant");
}

#[test]
fn language_item_marker_reports_second_marker_before_trait_member() {
    assert_marker_error(
        "trait Carrier\n    lang output\n    lang residual\n    type Value\n",
        "language item marker must apply to exactly one member",
    );
}

#[test]
fn language_item_marker_reports_orphaned_trait_member_marker() {
    assert_marker_error(
        "trait Carrier\n    lang output\ntype Value\n",
        "language item marker must be followed by a member at the same indentation",
    );
}

#[test]
fn parse_top_level() {
    use crate::lexer::Lexer;

    let mut tokens = Lexer::new(std::path::PathBuf::from("/test.rk"), "a = foo -> foo\n")
        .unwrap()
        .with_newline_at_end(false)
        .collect()
        .unwrap();

    tokens.pop();

    // let tokens = lex_test();
    let config = Config::default();

    let (tokens, top_level) = top_level(ParseCtx::from(&tokens, &config)).unwrap();

    assert_eq!(
        top_level,
        TopLevel::FunctionDecl(FunctionDecl {
            name: Ident {
                name: "a".to_string(),
                span: Span {
                    start: 0,
                    end: 1,
                    file_path: PathBuf::from("/test.rk"),
                },
            },
            lambda: LambdaDecl {
                parameters: vec![Pattern {
                    binding: None,
                    kind: PatternKind::Ident(IdentPattern {
                        name: Ident {
                            name: "foo".to_string(),
                            span: Span {
                                start: 4,
                                end: 7,
                                file_path: PathBuf::from("/test.rk"),
                            },
                        },
                        mut_: false,
                    }),
                },],
                body: Block {
                    statements: vec![Statement::Expression(Expression::UnaryExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Ident(IdentifierPath {
                                path: vec![IdentOrType::Ident(Ident {
                                    name: "foo".to_string(),
                                    span: Span {
                                        start: 11,
                                        end: 14,
                                        file_path: PathBuf::from("/test.rk"),
                                    },
                                })]
                            }),
                            secondaries: None,
                            type_annotation: None,
                        })
                    ))]
                },
                arrow_kind: LambdaArrowKind::Normal,
                span: Span {
                    start: 8,
                    end: 10,
                    file_path: PathBuf::from("/test.rk"),
                },
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }),
    );

    assert_eq!(tokens.len(), 0);
}
