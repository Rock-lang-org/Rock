use crate::ast::{ParseType, TopLevel};
use crate::fmt::{format, FormatInput};
use crate::parser::items::*;
use crate::parser::*;
use crate::Config;

fn parse_type_text(input: &str) -> ParseType {
    let tokens = lex_test(input);
    let config = Config::default();
    let (rest, ty) = parse_type
        .process(ParseCtx::from(&tokens, &config))
        .expect("type should parse");
    assert!(rest.is_empty(), "unparsed tokens: {:?}", rest.tokens);
    ty
}

#[test]
fn type_application_preserves_holes_and_comma_arguments() {
    let ty = parse_type_text("Result _, E");
    let ParseType::Application(application) = ty else {
        panic!("expected a type application");
    };
    assert_eq!(application.args.len(), 2);
    assert!(matches!(&application.args[0], ParseType::Hole(_)));
    assert_eq!(application.constructor.type_name(), "Result");
}

#[test]
fn explicit_type_lambdas_support_nesting_and_roundtrip() {
    let ty = parse_type_text("\\A -> \\B -> Either A, B");
    assert_eq!(ty.to_string(), "\\A -> \\B -> Either A, B");
    assert!(matches!(ty, ParseType::Lambda(_)));
}

#[test]
fn type_application_binds_tighter_than_function_arrow() {
    let ty = parse_type_text("F A -> G B");
    let ParseType::Function(types) = ty else {
        panic!("expected function type");
    };
    assert!(matches!(&types[0], ParseType::Application(_)));
    assert!(matches!(&types[1], ParseType::Application(_)));
}

#[test]
fn parenthesized_function_return_remains_nested() {
    let ty = parse_type_text("A -> (() -> A)");
    let ParseType::Function(types) = ty else {
        panic!("expected function type");
    };
    assert_eq!(types.len(), 2);
    assert!(matches!(&types[1], ParseType::Function(_)));

    let ty = parse_type_text("Option A -> (A -> B) -> Option B");
    let ParseType::Function(types) = ty else {
        panic!("expected function type");
    };
    assert_eq!(types.len(), 3);
    assert!(matches!(&types[1], ParseType::Function(_)));
}

#[test]
fn parenthesized_type_lambda_application_keeps_precedence() {
    let ty = parse_type_text("(\\T -> Result T, E) I64");
    assert_eq!(ty.to_string(), "(\\T -> Result T, E) I64");
}

#[test]
fn malformed_type_lambda_body_is_rejected() {
    let tokens = lex_test("\\T ->");
    let config = Config::default();
    assert!(parse_type
        .process(ParseCtx::from(&tokens, &config))
        .is_err());
}

#[test]
fn malformed_trailing_type_hole_separator_is_rejected() {
    assert!(parse_string("identity: F _,\n", &Config::default()).is_err());
}

#[test]
fn declaration_binders_are_explicit_and_parenthesized() {
    let tokens = lex_test("struct Compose (F _), (G _), A\n");
    let config = Config::default();
    let (rest, declaration) = struct_decl
        .process(ParseCtx::from(&tokens, &config))
        .expect("struct binder list should parse");
    assert!(rest.is_empty());
    assert_eq!(declaration.generic_params.len(), 3);
    assert!(declaration.generic_params[0].kind.is_some());
    assert!(declaration.generic_params[1].kind.is_some());
    assert!(declaration.generic_params[2].kind.is_none());
}

#[test]
fn default_binders_parse_for_all_declaration_owners() {
    let program = parse_string(
        "struct Pair A, {B = A}\nenum Choice {T = U8}\n    Empty\ntrait Compare {Rhs = Self}\n    @compare: &Rhs -> Bool\ntype Items {T = U8} = Pair T\nstruct Higher T, {F _ = Pair U8, _}\n",
        &Config::default(),
    ).expect("default binders should parse");
    match &program.module.top_levels[0] {
        crate::ast::TopLevel::StructDecl(decl) => {
            assert!(decl.generic_params[0].default.is_none());
            assert!(decl.generic_params[1].default.is_some());
        }
        _ => panic!("expected struct"),
    }
    match &program.module.top_levels[3] {
        crate::ast::TopLevel::NewType(_, params, _) => assert!(params[0].default.is_some()),
        _ => panic!("expected alias"),
    }
}

#[test]
fn higher_order_kind_binder_parses_and_roundtrips() {
    let source = "struct Higher (H: (Type -> Type) -> Type)\n";
    let program = parse_string(source, &Config::default()).expect("higher-order kind binder");
    let TopLevel::StructDecl(declaration) = &program.module.top_levels[0] else {
        panic!("expected struct declaration");
    };
    let kind = declaration.generic_params[0]
        .kind
        .as_ref()
        .expect("explicit kind syntax");
    assert!(kind.args.is_empty());
    assert!(matches!(kind.constructor.as_ref(), ParseType::Function(_)));
    let lowered = crate::type_lowering::lower_generic_param_decls(
        crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
        &declaration.generic_params,
    );
    assert_eq!(
        lowered[0].kind,
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
            crate::type_services::kind::Kind::Type,
        )
    );

    let formatted = format(FormatInput::program(&program));
    assert_eq!(formatted, source);
    parse_string(&formatted, &Config::default()).expect("formatted binder should parse");
}

#[test]
fn trait_higher_order_kind_binders_parse() {
    let source = "trait Higher (H: (Type -> Type) -> Type) for (F: (Type -> Type) -> Type)\n";
    let program =
        parse_string(source, &Config::default()).expect("trait higher-order kind binders");
    let TopLevel::TraitDecl(declaration) = &program.module.top_levels[0] else {
        panic!("expected trait declaration");
    };
    assert!(declaration.generic_params[0]
        .kind
        .as_ref()
        .is_some_and(|kind| kind.args.is_empty()));
    assert!(declaration
        .for_
        .as_ref()
        .and_then(|target| target.kind.as_ref())
        .is_some_and(|kind| kind.args.is_empty()));
    assert_eq!(format(FormatInput::program(&program)), source);
}

#[test]
fn explicit_kind_binder_rejects_type_expression_syntax() {
    assert!(parse_string("struct Invalid (H: Option)\n", &Config::default()).is_err());
}

#[test]
fn type_lambda_preserves_explicit_higher_order_kind() {
    let parsed = parse_type_text("\\(H: (Type -> Type) -> Type) -> H");
    let mut context = crate::lower::Lowerer::new();
    let lowered = crate::type_lowering::TypeLowerer::lower_parse_type(&mut context, &parsed);
    let crate::types::Type::Lambda { params, .. } = lowered else {
        panic!("expected lowered type lambda");
    };
    assert_eq!(
        params,
        vec![crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
            crate::type_services::kind::Kind::Type,
        )]
    );
}

#[test]
fn associated_type_higher_order_kind_roundtrips() {
    let source = "trait Families for F _\n    type (Family: (Type -> Type) -> Type)\n";
    let program = parse_string(source, &Config::default()).expect("associated constructor kind");
    let TopLevel::TraitDecl(declaration) = &program.module.top_levels[0] else {
        panic!("expected trait declaration");
    };
    let kind = declaration.associated_types[0]
        .kind
        .as_ref()
        .expect("associated type kind");
    assert_eq!(
        crate::type_lowering::lower_associated_type_kind(Some(kind)),
        crate::type_services::kind::Kind::arrow(
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
            crate::type_services::kind::Kind::Type,
        )
    );
    assert_eq!(format(FormatInput::program(&program)), source);

    let impl_source =
        "impl Families for Option\n    type (Family: (Type -> Type) -> Type) = Option\n";
    let impl_program =
        parse_string(impl_source, &Config::default()).expect("associated constructor definition");
    assert_eq!(format(FormatInput::program(&impl_program)), impl_source);
}

#[test]
fn trait_constructor_target_keeps_multiple_holes_as_one_binder() {
    let tokens = lex_test("trait Bifunctor for F _, _\n");
    let config = Config::default();
    let (rest, declaration) = r#trait
        .process(ParseCtx::from(&tokens, &config))
        .expect("trait constructor target should parse");
    assert!(rest.is_empty());
    let target = declaration.for_.expect("trait target binder");
    assert_eq!(target.name.name, "F");
    assert_eq!(target.kind.expect("constructor kind").args.len(), 2);
}

#[test]
fn trait_target_and_where_subject_preserve_constructor_holes() {
    let program = parse_string(
        "trait Functor for F _\n\napply_f: F A -> A where F _: Functor\n",
        &Config::default(),
    )
    .expect("trait target and where subject should parse");
    let TopLevel::TraitDecl(trait_decl) = &program.module.top_levels[0] else {
        panic!("expected trait declaration");
    };
    assert!(trait_decl.for_.as_ref().unwrap().kind.is_some());
    let TopLevel::FunctionSig(signature) = &program.module.top_levels[1] else {
        panic!("expected function signature");
    };
    assert!(matches!(
        &signature.where_clauses[0].subject,
        ParseType::Application(_)
    ));
}

#[test]
fn unbounded_constructor_where_subject_is_preserved() {
    let program = parse_string(
        "constructor_identity: F A -> F A where F _\n",
        &Config::default(),
    )
    .expect("unbounded constructor subject should parse");
    let TopLevel::FunctionSig(signature) = &program.module.top_levels[0] else {
        panic!("expected function signature");
    };
    assert!(signature.where_clauses[0].trait_bound.is_none());
}

#[test]
fn formatter_roundtrips_alias_and_type_qualified_owner_path() {
    let source = "type ResultWith E = \\T -> Result T, E\nmain = -> (Result _, IoError)::Applicative::pure 42\n";
    let program = parse_string(source, &Config::default()).expect("HKT source should parse");
    let formatted = format(FormatInput::program(&program));
    let reparsed = parse_string(&formatted, &Config::default()).expect("formatted source parses");
    assert_eq!(format(FormatInput::program(&reparsed)), formatted);
}

#[test]
fn implicit_uppercase_identity_remains_an_ordinary_function_signature() {
    let program = parse_string("identity: T -> T\n", &Config::default())
        .expect("ordinary uppercase generic should parse");
    assert!(matches!(
        &program.module.top_levels[0],
        TopLevel::FunctionSig(_)
    ));
}
