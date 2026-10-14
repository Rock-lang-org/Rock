//! Comprehensive AST validation tests.
//!
//! These tests parse complete programs and walk the resulting AST to verify
//! logical invariants rather than exact structural equality. This catches
//! classes of bugs where the parser produces an AST that "looks OK" in a
//! unit test but is nonsensical when you inspect the relationships between
//! nodes (empty bodies, missing identifiers, wrong nesting, etc.).

use crate::ast::tree::*;
use crate::ast::visit::Visitor;
use crate::parser::parse_string;
use crate::Config;

// ---------------------------------------------------------------------------
// Helpers: AST invariant checker via the Visitor pattern
// ---------------------------------------------------------------------------

/// Collects every invariant violation found in the AST.
struct AstValidator {
    errors: Vec<String>,
    /// Stack tracking what context we are in for better error messages.
    context: Vec<String>,
}

impl AstValidator {
    fn new() -> Self {
        Self {
            errors: Vec::new(),
            context: Vec::new(),
        }
    }

    fn ctx(&self) -> String {
        self.context.join(" > ")
    }

    fn push_ctx(&mut self, name: &str) {
        self.context.push(name.to_string());
    }

    fn pop_ctx(&mut self) {
        self.context.pop();
    }

    fn error(&mut self, msg: impl Into<String>) {
        self.errors.push(format!("[{}] {}", self.ctx(), msg.into()));
    }
}

impl<'ast> Visitor<'ast> for AstValidator {
    fn visit_module(&mut self, node: &'ast Module) {
        self.push_ctx("Module");

        if node.top_levels.is_empty() {
            self.error("Module has no top-level items");
        }

        // Visit children
        crate::ast::visit::walk_module(self, node);
        self.pop_ctx();
    }

    fn visit_function_decl(&mut self, node: &'ast FunctionDecl) {
        self.push_ctx(&format!("FunctionDecl({})", node.name.name));

        if node.name.name.is_empty() {
            self.error("Function declaration has empty name");
        }

        // Function body must have at least one statement
        if node.lambda.body.statements.is_empty() {
            self.error("Function body is empty (no statements)");
        }

        crate::ast::visit::walk_function_decl(self, node);
        self.pop_ctx();
    }

    fn visit_function_sig(&mut self, node: &'ast FunctionSig) {
        self.push_ctx(&format!("FunctionSig({})", node.name.name));

        if node.name.name.is_empty() {
            self.error("Function signature has empty name");
        }

        crate::ast::visit::walk_function_sig(self, node);
        self.pop_ctx();
    }

    fn visit_lambda_decl(&mut self, node: &'ast LambdaDecl) {
        self.push_ctx("LambdaDecl");

        // A lambda with an empty body is suspicious; shorthand parses to a real body.
        if node.body.statements.is_empty() {
            self.error("Lambda has empty body");
        }

        crate::ast::visit::walk_lambda_decl(self, node);
        self.pop_ctx();
    }

    fn visit_block(&mut self, node: &'ast Block) {
        // Blocks are allowed to be empty in some contexts (e.g. the validator
        // above already checks function/lambda bodies), so we just walk.
        crate::ast::visit::walk_block(self, node);
    }

    fn visit_struct_decl(&mut self, node: &'ast StructDecl) {
        self.push_ctx(&format!("StructDecl({})", node.name.name));

        if node.name.name.is_empty() {
            self.error("Struct has empty name");
        }
        // A struct with zero fields is valid but worth noting in
        // a strongly-typed language – we do NOT flag it.

        // Check for duplicate field names
        let mut seen = std::collections::HashSet::new();
        for field in &node.fields {
            if !seen.insert(&field.name.name) {
                self.error(format!("Duplicate struct field '{}'", field.name.name));
            }
        }

        crate::ast::visit::walk_struct_decl(self, node);
        self.pop_ctx();
    }

    fn visit_enum_decl(&mut self, node: &'ast EnumDecl) {
        self.push_ctx(&format!("EnumDecl({})", node.name.name));

        if node.name.name.is_empty() {
            self.error("Enum has empty name");
        }
        if node.variants.is_empty() {
            self.error("Enum has no variants");
        }

        // Check for duplicate variant names
        let mut seen = std::collections::HashSet::new();
        for variant in &node.variants {
            if !seen.insert(&variant.name.name) {
                self.error(format!("Duplicate enum variant '{}'", variant.name.name));
            }
        }

        crate::ast::visit::walk_enum_decl(self, node);
        self.pop_ctx();
    }

    fn visit_trait_decl(&mut self, node: &'ast TraitDecl) {
        self.push_ctx(&format!("TraitDecl({})", node.name.name));

        if node.name.name.is_empty() {
            self.error("Trait has empty name");
        }
        if node.methods.is_empty() && node.signatures.is_empty() {
            self.error("Trait has no methods or signatures");
        }

        crate::ast::visit::walk_trait_decl(self, node);
        self.pop_ctx();
    }

    fn visit_impl(&mut self, node: &'ast Impl) {
        let label = if let Some(for_) = &node.for_ {
            format!("Impl({} for {})", node.name.type_name(), for_.type_name())
        } else {
            format!("Impl({})", node.name.type_name())
        };
        self.push_ctx(&label);

        if node.name.type_name().is_empty() {
            self.error("Impl block has empty type name");
        }
        if node.methods.is_empty() && node.signatures.is_empty() {
            self.error("Impl block has no methods or signatures");
        }

        crate::ast::visit::walk_impl(self, node);
        self.pop_ctx();
    }

    fn visit_if(&mut self, node: &'ast If) {
        self.push_ctx("If");

        // Then-block must not be empty
        if node.then.statements.is_empty() {
            self.error("If-then block is empty");
        }

        // If there's an else block, it must also be non-empty
        if let Some(Else::Block(block)) = &node.else_ {
            if block.statements.is_empty() {
                self.error("Else block is empty");
            }
        }

        crate::ast::visit::walk_if(self, node);
        self.pop_ctx();
    }

    fn visit_match(&mut self, node: &'ast Match) {
        self.push_ctx("Match");

        if node.arms.is_empty() {
            self.error("Match expression has no arms");
        }

        // Each arm must have a non-empty body
        for (i, arm) in node.arms.iter().enumerate() {
            if arm.body.statements.is_empty() {
                self.error(format!("Match arm {} has empty body", i));
            }
        }

        crate::ast::visit::walk_match(self, node);
        self.pop_ctx();
    }

    fn visit_loop(&mut self, node: &'ast Loop) {
        self.push_ctx("Loop");

        match node {
            Loop::For(_, _, block, _) | Loop::While(_, block, _) | Loop::Loop(block, _) => {
                if block.statements.is_empty() {
                    self.error("Loop body is empty");
                }
            }
        }

        crate::ast::visit::walk_loop(self, node);
        self.pop_ctx();
    }

    fn visit_assignment(&mut self, node: &'ast Assignment) {
        self.push_ctx("Assignment");
        crate::ast::visit::walk_assignment(self, node);
        self.pop_ctx();
    }

    fn visit_identifier_path(&mut self, node: &'ast IdentifierPath) {
        if node.path.is_empty() {
            self.error("IdentifierPath is empty");
        }
        crate::ast::visit::walk_identifier_path(self, node);
    }

    fn visit_type_path(&mut self, node: &'ast TypePath) {
        if node.path.is_empty() {
            self.error("TypePath is empty");
        }
        crate::ast::visit::walk_type_path(self, node);
    }
}

/// Parse `input` and run the AST validator. Panics with all violations on failure.
fn validate(input: &str) {
    let config = Config::default();
    let program = parse_string(input, &config).unwrap_or_else(|e| {
        panic!("parse_string failed: {:?}", e);
    });

    let mut validator = AstValidator::new();
    program.visit(&mut validator);

    if !validator.errors.is_empty() {
        panic!(
            "AST validation found {} error(s):\n{}",
            validator.errors.len(),
            validator.errors.join("\n")
        );
    }
}

/// Counts the number of specific top-level item kinds.
struct TopLevelCounter {
    functions: usize,
    structs: usize,
    enums: usize,
    traits: usize,
    impls: usize,
    externs: usize,
    imports: usize,
    exports: usize,
}

impl TopLevelCounter {
    fn count(program: &Program) -> Self {
        let mut c = Self {
            functions: 0,
            structs: 0,
            enums: 0,
            traits: 0,
            impls: 0,
            externs: 0,
            imports: 0,
            exports: 0,
        };
        for tl in &program.module.top_levels {
            match tl {
                TopLevel::FunctionDecl(_) => c.functions += 1,
                TopLevel::StructDecl(_) => c.structs += 1,
                TopLevel::EnumDecl(_) => c.enums += 1,
                TopLevel::TraitDecl(_) => c.traits += 1,
                TopLevel::Impl(_) => c.impls += 1,
                TopLevel::Extern(_) => c.externs += 1,
                TopLevel::Import(_) | TopLevel::GlobImport(_) => c.imports += 1,
                TopLevel::Export(_) => c.exports += 1,
                _ => {}
            }
        }
        c
    }
}

/// Helper: parse and return the Program for direct inspection.
fn parse(input: &str) -> Program {
    let config = Config::default();
    parse_string(input, &config).unwrap_or_else(|e| {
        panic!("parse_string failed: {:?}", e);
    })
}

/// Helper: extract the first function declaration from a program.
fn first_fn(program: &Program) -> &FunctionDecl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::FunctionDecl(fd) => Some(fd),
            _ => None,
        })
        .expect("no FunctionDecl found")
}

/// Helper: find a function by name.
fn find_fn<'a>(program: &'a Program, name: &str) -> &'a FunctionDecl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::FunctionDecl(fd) if fd.name.name == name => Some(fd),
            _ => None,
        })
        .unwrap_or_else(|| panic!("function '{}' not found", name))
}

/// Helper: find a struct by name.
fn find_struct<'a>(program: &'a Program, name: &str) -> &'a StructDecl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::StructDecl(sd) if sd.name.name == name => Some(sd),
            _ => None,
        })
        .unwrap_or_else(|| panic!("struct '{}' not found", name))
}

/// Helper: find an enum by name.
fn find_enum<'a>(program: &'a Program, name: &str) -> &'a EnumDecl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::EnumDecl(ed) if ed.name.name == name => Some(ed),
            _ => None,
        })
        .unwrap_or_else(|| panic!("enum '{}' not found", name))
}

/// Helper: find a trait by name.
fn find_trait<'a>(program: &'a Program, name: &str) -> &'a TraitDecl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::TraitDecl(td) if td.name.name == name => Some(td),
            _ => None,
        })
        .unwrap_or_else(|| panic!("trait '{}' not found", name))
}

/// Helper: find an impl by name.
fn find_impl<'a>(program: &'a Program, name: &str) -> &'a Impl {
    program
        .module
        .top_levels
        .iter()
        .find_map(|tl| match tl {
            TopLevel::Impl(i) if i.name.type_name() == name => Some(i),
            _ => None,
        })
        .unwrap_or_else(|| panic!("impl '{}' not found", name))
}

// ===========================================================================
// Test: Simple function
// ===========================================================================

#[test]
fn simple_function_is_valid() {
    validate("main = -> 1\n");
}

#[test]
fn simple_function_structure() {
    let p = parse("main = -> 1\n");
    let f = first_fn(&p);
    assert_eq!(f.name.name, "main");
    assert_eq!(f.lambda.parameters.len(), 0);
    assert_eq!(f.lambda.body.statements.len(), 1);
    assert_eq!(f.self_receiver, None);
}

// ===========================================================================
// Test: Function with parameters
// ===========================================================================

#[test]
fn function_with_params() {
    let input = "add = a, b -> a + b\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.name.name, "add");
    assert_eq!(f.lambda.parameters.len(), 2);

    // Both parameters should be ident patterns
    for param in &f.lambda.parameters {
        assert!(
            matches!(param.kind, PatternKind::Ident(_)),
            "Expected ident pattern, got {:?}",
            param.kind,
        );
    }
}

// ===========================================================================
// Test: Multiple top-level declarations
// ===========================================================================

#[test]
fn multiple_functions() {
    let input = r#"
foo = -> 1

bar = -> 2

baz = x -> x
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);
}

// ===========================================================================
// Test: Struct declaration
// ===========================================================================

#[test]
fn struct_declaration_valid() {
    let input = "struct Point\n    x: Int\n    y: Int\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Point");
    assert_eq!(s.fields.len(), 2);
    assert_eq!(s.fields[0].name.name, "x");
    assert_eq!(s.fields[1].name.name, "y");

    // Fields should have types
    for field in &s.fields {
        assert!(
            matches!(field.ty, ParseType::Type(_)),
            "Expected named type, got {:?}",
            field.ty,
        );
    }
}

#[test]
fn struct_with_default_field() {
    let input = "struct Point\n    x: Int\n    y: Int = 0\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Point");
    assert!(s.fields[0].default.is_none());
    assert!(s.fields[1].default.is_some());
}

#[test]
fn struct_with_generics() {
    let input = "struct Container T\n    value: T\n";
    validate(input);

    let p = parse(input);
    let s = find_struct(&p, "Container");
    assert_eq!(s.generic_params.len(), 1);
    assert_eq!(s.fields.len(), 1);
}

// ===========================================================================
// Test: Enum declaration
// ===========================================================================

#[test]
fn enum_declaration_valid() {
    let input = "enum Color\n    Red\n    Green\n    Blue\n";
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Color");
    assert_eq!(e.variants.len(), 3);
    assert_eq!(e.variants[0].name.name, "Red");
    assert_eq!(e.variants[1].name.name, "Green");
    assert_eq!(e.variants[2].name.name, "Blue");
}

#[test]
fn enum_with_payload() {
    let input = "enum Result T, E\n    Ok T\n    Err E\n";
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Result");
    assert_eq!(e.name.generics.len(), 2);
    assert_eq!(e.variants.len(), 2);

    // Each variant should have a type payload
    for variant in &e.variants {
        match &variant.fields {
            NamedFieldsOrTypesList::TypesList(types) => {
                assert!(
                    !types.is_empty(),
                    "Variant {} should have types",
                    variant.name.name,
                );
            }
            NamedFieldsOrTypesList::NamedFields(_) => {
                panic!("Expected TypesList for variant {}", variant.name.name);
            }
        }
    }
}

// ===========================================================================
// Test: Trait declaration
// ===========================================================================

#[test]
fn trait_declaration_valid() {
    let input = "trait Display\n    display: () -> String\n";
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Display");
    assert_eq!(t.signatures.len(), 1);
    assert!(t.signatures.contains_key(&Ident {
        name: "display".to_string(),
        span: crate::lexer::Span::test(),
    }));
}

#[test]
fn trait_with_default_method() {
    let input = "trait Greet\n    greet = -> 42\n";
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Greet");
    assert_eq!(t.methods.len(), 1);
}

#[test]
fn trait_with_associated_type_is_valid() {
    let input = r#"trait Deref
    type Target
    @deref: () -> &Self::Target
"#;
    validate(input);

    let p = parse(input);
    let t = find_trait(&p, "Deref");
    assert_eq!(t.associated_types.len(), 1);
    assert_eq!(t.associated_types[0].name.name, "Target");
}

// ===========================================================================
// Test: Impl block
// ===========================================================================

#[test]
fn impl_for_generic_type() {
    let input = r#"trait Show
    show: () -> String

struct Container T
    value: T

impl Show for Container T
    show = -> "Container"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Container");
    assert!(i.is_some(), "Should find impl Show for Container T");
    let i = i.unwrap();
    assert_eq!(i.name.generics().len(), 0, "Show trait has no generics");
    assert_eq!(
        i.for_.as_ref().unwrap().generics().len(),
        1,
        "Container T has one generic param"
    );

    // Check that the generic parameter is just "T"
    let gen = &i.for_.as_ref().unwrap().generics()[0];
    match gen {
        ParseType::Type(inner) => assert_eq!(inner.name, "T"),
        _ => panic!("Expected Type for generic parameter"),
    }
}

#[test]
fn impl_for_array_named_type_is_plain_named_impl_target() {
    let input = r#"trait Show
    show: () -> String

struct Array T
    < value: T

impl Show for Array T
    show = -> "Array"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Array");
    assert!(i.is_some(), "Should find impl Show for user type Array T");
    let i = i.unwrap();
    let for_type = i.for_.as_ref().expect("impl should have a target type");
    let ParseType::Application(application) = for_type else {
        panic!("expected Array type application, got {:?}", for_type);
    };
    assert!(matches!(
        application.constructor.as_ref(),
        ParseType::Type(inner) if inner.name == "Array"
    ));
    assert_eq!(application.args.len(), 1);
}

#[test]
fn impl_for_borrowed_slice_type_parses_reference_to_slice() {
    let input = r#"trait Show
    show: () -> String

impl Show for &[T]
    show = -> "slice"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "&[T]")
        .expect("Should find impl Show for borrowed slice");
    let for_type = i.for_.as_ref().expect("impl should have a target type");
    match for_type {
        ParseType::Reference { is_mut, pointee } => {
            assert!(!is_mut);
            assert!(matches!(pointee.as_ref(), ParseType::Slice(_)));
        }
        other => panic!("expected borrowed slice type, got {:?}", other),
    }
}

#[test]
fn impl_with_associated_type_is_valid() {
    let input = r#"trait Deref
    type Target
    @deref: () -> &Self::Target

struct Box T
    value: T

impl Deref for Box T
    type Target = T
    @deref = -> @value
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Deref", "Box").unwrap();
    assert_eq!(i.associated_types.len(), 1);
    assert_eq!(i.associated_types[0].name.name, "Target");
}

#[test]
fn impl_with_multiple_generic_params() {
    let input = r#"trait PairShow
    show_pair: () -> String

struct Pair A, B
    first: A
    second: B

impl PairShow for Pair A, B
    show_pair = -> "Pair"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "PairShow", "Pair");
    assert!(i.is_some(), "Should find impl PairShow for Pair A, B");
    let i = i.unwrap();
    assert_eq!(
        i.for_.as_ref().unwrap().generics().len(),
        2,
        "Pair A, B has two generic params"
    );
}

#[test]
fn impl_with_where_clause() {
    let input = r#"trait Show
    show: () -> String

struct Container T
    value: T

impl Show for Container T where T: Show
    show = -> "Container"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Container");
    assert!(
        i.is_some(),
        "Should find impl Show for Container T with where clause"
    );
    let i = i.unwrap();
    assert_eq!(i.where_clauses.len(), 1, "Should have one where clause");
    assert_eq!(i.where_clauses[0].subject.type_name(), "T");
    assert_eq!(
        i.where_clauses[0]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
}

#[test]
fn impl_with_multiple_where_clauses() {
    let input = r#"trait Show
    show: () -> String

struct Pair A, B
    first: A
    second: B

impl Show for Pair A, B where A: Show, B: Show
    show = -> "Pair"
"#;
    validate(input);

    let p = parse(input);
    let i = program_find_impl_for(&p, "Show", "Pair");
    assert!(
        i.is_some(),
        "Should find impl Show for Pair A, B with where clauses"
    );
    let i = i.unwrap();
    assert_eq!(i.where_clauses.len(), 2, "Should have two where clauses");
    assert_eq!(i.where_clauses[0].subject.type_name(), "A");
    assert_eq!(
        i.where_clauses[0]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
    assert_eq!(i.where_clauses[1].subject.type_name(), "B");
    assert_eq!(
        i.where_clauses[1]
            .trait_bound
            .as_ref()
            .expect("trait bound")
            .type_name(),
        "Show"
    );
}

#[test]
fn impl_block_valid() {
    let input = r#"struct Foo
    x: Int

impl Foo
    get_x = -> @x
"#;
    validate(input);

    let p = parse(input);
    let i = find_impl(&p, "Foo");
    assert!(i.for_.is_none());
    assert_eq!(i.methods.len(), 1);
}

#[test]
fn impl_for_trait() {
    let input = r#"trait Display
    display: () -> String

struct Foo
    x: Int

impl Display for Foo
    display = -> "Foo"
"#;
    validate(input);

    let p = parse(input);
    // Find the impl-for
    let i = program_find_impl_for(&p, "Display", "Foo");
    assert!(i.is_some(), "Should find impl Display for Foo");
    let i = i.unwrap();
    assert_eq!(i.methods.len(), 1);
}

fn program_find_impl_for<'a>(
    program: &'a Program,
    trait_name: &str,
    for_name: &str,
) -> Option<&'a Impl> {
    program.module.top_levels.iter().find_map(|tl| match tl {
        TopLevel::Impl(i)
            if i.name.type_name() == trait_name
                && i.for_.as_ref().map(|f| f.type_name()) == Some(for_name.to_string()) =>
        {
            Some(i)
        }
        _ => None,
    })
}

// ===========================================================================
// Test: If expressions
// ===========================================================================

#[test]
fn if_then_else_valid() {
    let input = "main = ->\n    if true then 1 else 2\n";
    validate(input);
}

#[test]
fn if_then_multiline_valid() {
    let input = r#"main = ->
    x = if true
        then 1
        else 2
    x
"#;
    validate(input);
}

#[test]
fn if_else_if_chain_valid() {
    let input = r#"main = x ->
    if x > 10 then 3
    else if x > 5 then 2
    else 1
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // The body should have one statement (the if expression)
    assert_eq!(f.lambda.body.statements.len(), 1);
}

#[test]
fn if_let_valid() {
    let input = r#"enum Result T, E
    Ok T
    Err E

main = ->
    result_val: Result Int, String = Result::Ok 42
    if_let_test = if Ok x = result_val
        then x
        else 0
    if_let_test
"#;
    validate(input);
}

// ===========================================================================
// Test: Match expressions
// ===========================================================================

#[test]
fn match_basic_valid() {
    let input = r#"main = ->
    match 1
        0 => "zero"
        1 => "one"
        _ => "other"
"#;
    validate(input);
}

#[test]
fn match_with_guards() {
    let input = r#"main = ->
    match 5
        n if n > 10 => "big"
        n if n > 0 => "positive"
        _ => "non-positive"
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // Walk to the match expression
    let stmt = &f.lambda.body.statements[0];
    if let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe))) = stmt {
        if let Operand::Match(m) = &pe.operand {
            assert_eq!(m.arms.len(), 3);
            // First two arms should have guards
            assert!(m.arms[0].condition.is_some(), "Arm 0 should have a guard");
            assert!(m.arms[1].condition.is_some(), "Arm 1 should have a guard");
            // Last arm (wildcard) has no guard
            assert!(
                m.arms[2].condition.is_none(),
                "Arm 2 should NOT have a guard"
            );
        } else {
            panic!("Expected match expression");
        }
    } else {
        panic!("Expected expression statement");
    }
}

#[test]
fn match_tuple_destructuring() {
    let input = r#"main = ->
    match (1, 2)
        (0, _) => "zero first"
        (_, 0) => "zero second"
        (a, b) => "both"
"#;
    validate(input);
}

#[test]
fn match_array_destructuring() {
    let input = r#"main = ->
    match [1, 2, 3]
        [first, ..rest] => first
        _ => 0
"#;
    validate(input);
}

#[test]
fn match_with_binding() {
    let input = r#"main = ->
    match (1, 2, 3)
        foo @ (x, y, z) if x > 0 => foo
        _ => (0, 0, 0)
"#;
    validate(input);
}

// ===========================================================================
// Test: Loop constructs
// ===========================================================================

#[test]
fn for_loop_valid() {
    let input = r#"main = ->
    for item in [1, 2, 3, 4, 5]
        item
    0
"#;
    validate(input);
}

#[test]
fn while_loop_valid() {
    let input = r#"main = ->
    counter = 0
    while counter < 10
        counter = counter + 1
    counter
"#;
    validate(input);
}

#[test]
fn loop_with_break_valid() {
    let input = r#"main = ->
    loop_result = loop
        if true then break 42
        continue
    loop_result
"#;
    validate(input);
}

#[test]
fn for_with_break_continue() {
    let input = r#"main = ->
    for item in [1, 2, 3, 4, 5]
        if item > 3 then break item
    0
"#;
    validate(input);
}

#[test]
fn while_with_break_continue() {
    let input = r#"main = ->
    counter = 0
    while counter < 10
        counter = counter + 1
        if counter == 5 then continue
        if counter == 8 then break
    counter
"#;
    validate(input);
}

// ===========================================================================
// Test: Assignments and patterns
// ===========================================================================

#[test]
fn simple_assignment() {
    let input = "main = ->\n    x = 42\n    x\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 2);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Assignment(_),
    ));
}

#[test]
fn tuple_pattern_assignment() {
    let input = "main = ->\n    (a, b) = (1, 2)\n    a\n";
    validate(input);
}

#[test]
fn array_destructuring_assignment() {
    let input = r#"main = ->
    x = [1, 2, 3]
    [first, ..rest] = x
    first
"#;
    validate(input);
}

#[test]
fn typed_assignment() {
    let input = r#"main = ->
    x: Int = 42
    x
"#;
    validate(input);
}

// ===========================================================================
// Test: Expressions
// ===========================================================================

#[test]
fn binary_expression() {
    let input = "main = -> 1 + 2\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    let stmt = &f.lambda.body.statements[0];
    assert!(
        matches!(stmt, Statement::Expression(Expression::BinopExpr(_, _, _))),
        "Expected binary expression, got {:?}",
        stmt,
    );
}

#[test]
fn chained_binary_expressions() {
    let input = "main = -> 1 + 2 * 3\n";
    validate(input);
}

#[test]
fn function_call() {
    let input = r#"foo = x -> x

main = -> foo 42
"#;
    validate(input);

    let p = parse(input);
    let f = find_fn(&p, "main");
    let stmt = &f.lambda.body.statements[0];
    if let Statement::Expression(Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe))) = stmt {
        assert!(
            pe.secondaries.is_some(),
            "Function call should have secondaries (arguments)"
        );
        if let Some(secs) = &pe.secondaries {
            assert!(
                matches!(secs[0], SecondaryExpr::Arguments(_)),
                "First secondary should be Arguments",
            );
        }
    } else {
        panic!("Expected primary expression");
    }
}

#[test]
fn method_chaining() {
    let input = r#"main = ->
    chained = foo.bar.baz
    chained
"#;
    validate(input);
}

#[test]
fn index_expression() {
    let input = r#"main = ->
    arr = [1, 2, 3]
    arr[0]
"#;
    validate(input);
}

#[test]
fn nested_function_calls() {
    let input = r#"foo = x -> x
bar = x -> x

main = -> foo (bar 42)
"#;
    validate(input);
}

// ===========================================================================
// Test: Literals
// ===========================================================================

#[test]
fn integer_literal() {
    let input = "main = -> 42\n";
    validate(input);
}

#[test]
fn float_literal() {
    let input = "main = -> 3.14\n";
    validate(input);
}

#[test]
fn string_literal() {
    let input = "main = -> \"hello world\"\n";
    validate(input);
}

#[test]
fn bool_literals() {
    let input = r#"main = ->
    a = true
    b = false
    a
"#;
    validate(input);
}

#[test]
fn array_literal() {
    let input = "main = -> [1, 2, 3]\n";
    validate(input);
}

#[test]
fn nested_array_literal() {
    let input = "main = -> [[1, 2], [3, 4]]\n";
    validate(input);
}

#[test]
fn char_literal() {
    let input = "main = -> 'a'\n";
    validate(input);
}

#[test]
fn tuple_literal() {
    let input = "main = -> (1, 2, 3)\n";
    validate(input);
}

// ===========================================================================
// Test: Lambdas
// ===========================================================================

#[test]
fn inline_lambda() {
    let input = r#"main = ->
    double = x -> x + x
    double 5
"#;
    validate(input);
}

#[test]
fn lambda_as_argument() {
    let input = r#"apply = f, x -> f x

main = -> apply (x -> x + 1), 5
"#;
    validate(input);
}

#[test]
fn shorthand_functions() {
    let input = r#"main = ->
    plus_one = (+1)
    times_two = (*2)
    result = plus_one 5
    result
"#;
    validate(input);
}

// ===========================================================================
// Test: Struct instantiation
// ===========================================================================

#[test]
fn struct_instantiation_multiline() {
    let input = r#"struct Point
    x: Int
    y: Int
    z: Int

main = ->
    point = Point
        x: 10
        y: 20
        z: 30
    point
"#;
    validate(input);
}

#[test]
fn struct_instantiation_inline() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    point = Point x: 10, y: 20
    point
"#;
    validate(input);
}

// ===========================================================================
// Test: Pattern matching in assignments
// ===========================================================================

#[test]
fn struct_pattern_assignment() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    point = Point x: 10, y: 20
    Point x: px, y: py = point
    px
"#;
    validate(input);
}

// ===========================================================================
// Test: Return, break, continue
// ===========================================================================

#[test]
fn return_statement() {
    let input = "main = ->\n    return 42\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Return(Some(_)),
    ));
}

#[test]
fn break_with_value() {
    let input = r#"main = ->
    loop
        break 42
"#;
    validate(input);
}

#[test]
fn continue_statement() {
    let input = r#"main = ->
    loop
        continue
"#;
    // continue without a value should also pass the visitor walk
    let config = Config::default();
    let program = parse_string(input, &config).unwrap();
    let mut validator = AstValidator::new();
    program.visit(&mut validator);
    // We don't flag continue/break, just check it parsed correctly
    assert!(
        parse_string(input, &config).is_ok(),
        "Should parse continue",
    );
}

// ===========================================================================
// Test: Type annotations
// ===========================================================================

#[test]
fn function_type_annotations() {
    let input = r#"add: Int -> Int -> Int
add = a, b -> a + b
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 1, "Should have 1 function decl");
}

#[test]
fn complex_type_annotations() {
    let input = r#"struct Container T
    value: T
    count: Int

main = ->
    c: Container Int = Container value: 42, count: 1
    c
"#;
    validate(input);
}

// ===========================================================================
// Test: Extern declarations
// ===========================================================================

#[test]
fn extern_declaration() {
    let input = "extern printf: *Char -> Int\n";
    let config = Config::default();
    let result = parse_string(input, &config);
    assert!(
        result.is_ok(),
        "Extern decl should parse: {:?}",
        result.err()
    );

    let p = result.unwrap();
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.externs, 1);
}

// ===========================================================================
// Test: Unary operators
// ===========================================================================

#[test]
fn unary_negation() {
    let input = "main = -> -1\n";
    validate(input);
}

#[test]
fn unary_not() {
    let input = "main = -> !true\n";
    validate(input);
}

// ===========================================================================
// Test: Complex/realistic programs
// ===========================================================================

#[test]
fn realistic_program_with_structs_and_methods() {
    let input = r#"struct Point
    x: Int
    y: Int

impl Point
    add = other ->
        Point
            x: @x + other.x
            y: @y + other.y

main = ->
    a = Point x: 1, y: 2
    b = Point x: 3, y: 4
    c = a.add b
    c
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.structs, 1);
    assert_eq!(c.impls, 1);
    assert_eq!(c.functions, 1);
}

#[test]
fn realistic_enum_with_match() {
    let input = r#"enum Shape
    Circle Int
    Rectangle Int, Int
    Triangle Int, Int, Int

area = shape ->
    match shape
        Circle r => r * r * 3
        Rectangle w, h => w * h
        Triangle a, b, c => a
"#;
    validate(input);

    let p = parse(input);
    let e = find_enum(&p, "Shape");
    assert_eq!(e.variants.len(), 3);

    let f = find_fn(&p, "area");
    assert_eq!(f.lambda.parameters.len(), 1);
}

#[test]
fn realistic_trait_implementation() {
    let input = r#"trait Printable
    print: () -> String

struct Dog
    name: String

impl Printable for Dog
    print = -> @name

main = ->
    d = Dog name: "Rex"
    d.print!
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.traits, 1);
    assert_eq!(c.structs, 1);
    assert_eq!(c.impls, 1);
    assert_eq!(c.functions, 1);
}

#[test]
fn realistic_higher_order_functions() {
    let input = r#"apply = f, x -> f x

compose = f, g, x -> f (g x)

main = ->
    inc = x -> x + 1
    double = x -> x * 2
    result = compose inc, double, 5
    result
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);

    // compose should have 3 params
    let compose = find_fn(&p, "compose");
    assert_eq!(compose.lambda.parameters.len(), 3);
}

#[test]
fn realistic_nested_control_flow() {
    let input = r#"main = ->
    result = 0
    for i in [1, 2, 3, 4, 5]
        if i > 3
            then break i
        result = result + i
    result
"#;
    validate(input);
}

#[test]
fn realistic_complex_match() {
    let input = r#"main = ->
    match_test = match (1, "test", [1, 2, 3])
        (0, _, _) => "zero"
        (1, str, arr) if str == "test" => "matched with guard"
        foo @ (x, y, z) if x > 0 => "bound pattern"
        (a, b, [first, ..rest]) => "array destructure"
        _ => "default"
    match_test
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);

    // Should have 2 statements: assignment + expression
    assert_eq!(f.lambda.body.statements.len(), 2);
}

#[test]
fn realistic_lambda_in_expression() {
    let input = r#"main = ->
    x = (a -> a + 1) 5
    x
"#;
    validate(input);
}

#[test]
fn unparenthesized_lambda_after_infix_operator() {
    validate(
        r#"main = ->
    source <!> map_error >>= value -> consume value
"#,
    );
}

// ===========================================================================
// Test: Edge cases
// ===========================================================================

#[test]
fn empty_tuple() {
    // Unit value
    let input = "main = -> ()\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_ok());
}

#[test]
fn deeply_nested_expressions() {
    let input = "main = -> ((((1 + 2))))\n";
    validate(input);
}

#[test]
fn multiline_function_body() {
    let input = r#"main = ->
    a = 1
    b = 2
    c = 3
    d = a + b + c
    d
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 5);
}

#[test]
fn empty_array() {
    let input = "main = -> []\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_ok());
}

#[test]
fn programs_separated_by_blank_lines() {
    let input = r#"

foo = -> 1


bar = -> 2


baz = -> 3

"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.functions, 3);
}

#[test]
fn function_with_many_parameters() {
    let input = "f = a, b, c, d, e -> a\n";
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    assert_eq!(f.lambda.parameters.len(), 5);
}

// ===========================================================================
// Test: Top-level counting for a complex program
// ===========================================================================

#[test]
fn complex_program_top_level_counts() {
    let input = r#"struct Point
    x: Int
    y: Int

struct Color
    r: Int
    g: Int
    b: Int

enum Shape
    Circle Int
    Rect Int, Int

trait Drawable
    draw: () -> Int

impl Drawable for Point
    draw = -> @x

impl Point
    new = x, y -> Point x: x, y: y

main = ->
    p = Point::new 1, 2
    p.draw!
"#;
    validate(input);

    let p = parse(input);
    let c = TopLevelCounter::count(&p);
    assert_eq!(c.structs, 2, "Should have 2 structs");
    assert_eq!(c.enums, 1, "Should have 1 enum");
    assert_eq!(c.traits, 1, "Should have 1 trait");
    assert_eq!(c.impls, 2, "Should have 2 impls");
    assert_eq!(c.functions, 1, "Should have 1 top-level function");
}

// ===========================================================================
// Test: Parse errors (should fail)
// ===========================================================================

#[test]
fn parse_error_bad_indent() {
    let input = "main = ->\na\n  2\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_err());
}

#[test]
fn parse_error_missing_body() {
    let input = "main = ->\n";
    let config = Config::default();
    assert!(parse_string(input, &config).is_err());
}

// ===========================================================================
// Test: Unsafe blocks
// ===========================================================================

#[test]
fn unsafe_block_valid() {
    let input = r#"main = ->
    x = unsafe
        42
    x
"#;
    validate(input);
}

// ===========================================================================
// Test: Double-dot (method cascade / copy-with)
// ===========================================================================

#[test]
fn double_dot_expression() {
    let input = r#"main = ->
    chained = foo..bar
    chained
"#;
    validate(input);
}

// ===========================================================================
// Test: Error propagation (?)
// ===========================================================================

#[test]
fn error_propagation() {
    let input = r#"main = ->
    x = foo?
    x
"#;
    validate(input);

    let p = parse(input);
    let f = first_fn(&p);
    // First statement is assignment
    if let Statement::Assignment(a) = &f.lambda.body.statements[0] {
        if let Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe)) = &a.rhs {
            assert!(pe.secondaries.is_some());
            let secs = pe.secondaries.as_ref().unwrap();
            assert!(
                secs.iter()
                    .any(|s| matches!(s, SecondaryExpr::Interogation)),
                "Should have ? operator in secondaries",
            );
        } else {
            panic!("Expected primary expression for RHS");
        }
    } else {
        panic!("Expected assignment");
    }
}

// ===========================================================================
// Test: Infix operator declarations
// ===========================================================================

#[test]
fn infix_operator_declaration() {
    let input = r#"infix 5 |>

main = -> 1
"#;
    let config = Config::default();
    let result = parse_string(input, &config);
    assert!(
        result.is_ok(),
        "Infix decl should parse: {:?}",
        result.err()
    );

    let p = result.unwrap();
    let has_infix = p
        .module
        .top_levels
        .iter()
        .any(|tl| matches!(tl, TopLevel::InfixOperator(5, _)));
    assert!(has_infix, "Should have an infix operator declaration");
}

// ===========================================================================
// Test: Visitor walks the full AST without panicking
// ===========================================================================

/// A visitor that simply counts every node type it encounters.
struct NodeCounter {
    total: usize,
    functions: usize,
    expressions: usize,
    statements: usize,
    literals: usize,
    patterns: usize,
    identifiers: usize,
    blocks: usize,
}

impl NodeCounter {
    fn new() -> Self {
        Self {
            total: 0,
            functions: 0,
            expressions: 0,
            statements: 0,
            literals: 0,
            patterns: 0,
            identifiers: 0,
            blocks: 0,
        }
    }
}

impl<'ast> Visitor<'ast> for NodeCounter {
    fn visit_function_decl(&mut self, node: &'ast FunctionDecl) {
        self.total += 1;
        self.functions += 1;
        crate::ast::visit::walk_function_decl(self, node);
    }

    fn visit_expression(&mut self, node: &'ast Expression) {
        self.total += 1;
        self.expressions += 1;
        crate::ast::visit::walk_expression(self, node);
    }

    fn visit_statement(&mut self, node: &'ast Statement) {
        self.total += 1;
        self.statements += 1;
        crate::ast::visit::walk_statement(self, node);
    }

    fn visit_literal(&mut self, node: &'ast Literal) {
        self.total += 1;
        self.literals += 1;
        crate::ast::visit::walk_literal(self, node);
    }

    fn visit_pattern(&mut self, node: &'ast Pattern) {
        self.total += 1;
        self.patterns += 1;
        crate::ast::visit::walk_pattern(self, node);
    }

    fn visit_ident(&mut self, node: &'ast Ident) {
        self.total += 1;
        self.identifiers += 1;
        crate::ast::visit::walk_ident(self, node);
    }

    fn visit_block(&mut self, node: &'ast Block) {
        self.total += 1;
        self.blocks += 1;
        crate::ast::visit::walk_block(self, node);
    }
}

#[test]
fn visitor_counts_realistic_program() {
    let input = r#"struct Point
    x: Int
    y: Int

impl Point
    add = other ->
        Point
            x: @x + other.x
            y: @y + other.y

main = ->
    a = Point x: 1, y: 2
    b = Point x: 3, y: 4
    c = a.add b
    result = match c.x
        0 => "zero"
        n if n > 0 => "positive"
        _ => "negative"
    result
"#;
    let p = parse(input);
    let mut counter = NodeCounter::new();
    p.visit(&mut counter);

    assert!(counter.total > 0, "Visitor should have visited nodes");
    assert!(
        counter.functions >= 2,
        "Should have at least 2 function decls (add + main)"
    );
    assert!(counter.expressions > 0, "Should have expressions");
    assert!(counter.statements > 0, "Should have statements");
    assert!(counter.identifiers > 0, "Should have identifiers");
    assert!(counter.blocks > 0, "Should have blocks");
    assert!(
        counter.patterns > 0,
        "Should have patterns (in match + function params)"
    );
}

#[test]
fn visitor_walks_all_control_flow() {
    let input = r#"main = ->
    x = if true then 1 else 2
    y = match x
        1 => "one"
        _ => "other"
    z = 0
    for i in [1, 2, 3]
        z = z + i
    w = 0
    while w < 10
        w = w + 1
    result = loop
        break 42
    result
"#;
    let p = parse(input);
    let mut counter = NodeCounter::new();
    p.visit(&mut counter);

    // Sanity check that we traversed deeply
    assert!(
        counter.total > 20,
        "Should visit many nodes, got {}",
        counter.total
    );
    assert!(
        counter.blocks >= 5,
        "Should have multiple blocks (main body + if/match/for/while/loop), got {}",
        counter.blocks
    );
}

// ===========================================================================
// Test: Validate that all .rk test files parse correctly
// ===========================================================================

// ===========================================================================
// ===========================================================================
//
//   DEEP STRUCTURAL VALIDATION
//
//   The tests below don't just check "does it parse + pass invariants",
//   they crack open the AST and verify the *shape* is sensible: operator
//   chains, nesting depths, argument binding, expression-in-expression, etc.
//
// ===========================================================================
// ===========================================================================

// ---- helpers for digging into the AST ------------------------------------

/// Unwrap a statement that is a bare expression.
fn as_expr(stmt: &Statement) -> &Expression {
    match stmt {
        Statement::Expression(e) => e,
        other => panic!("expected Expression statement, got {:?}", other),
    }
}

/// Unwrap a statement that is an assignment.
fn as_assign(stmt: &Statement) -> &Assignment {
    match stmt {
        Statement::Assignment(a) => a,
        other => panic!("expected Assignment statement, got {:?}", other),
    }
}

/// Unwrap Expression::UnaryExpr -> UnaryExpr::PrimaryExpr -> PrimaryExpr.
fn as_primary(expr: &Expression) -> &PrimaryExpr {
    match expr {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(pe)) => pe,
        other => panic!("expected simple primary expression, got {:?}", other),
    }
}

/// Unwrap to Operand from a simple (no-secondary) expression.
fn as_operand(expr: &Expression) -> &Operand {
    &as_primary(expr).operand
}

/// Unwrap a BinopExpr; return (lhs unary, operator string, rhs expression).
fn as_binop(expr: &Expression) -> (&UnaryExpr, &str, &Expression) {
    match expr {
        Expression::BinopExpr(lhs, op, rhs) => (lhs, &op.value, rhs),
        other => panic!("expected BinopExpr, got {:?}", other),
    }
}

/// Get the identifier name from a simple ident operand.
fn ident_name(expr: &Expression) -> &str {
    match as_operand(expr) {
        Operand::Ident(path) => match &path.path[0] {
            IdentOrType::Ident(id) => &id.name,
            other => panic!("expected Ident in path, got {:?}", other),
        },
        other => panic!("expected Ident operand, got {:?}", other),
    }
}

/// Get the integer value from a literal number operand.
fn lit_number(expr: &Expression) -> u64 {
    match as_operand(expr) {
        Operand::Literal(lit) => match &lit.kind {
            LiteralKind::Number(n) => *n,
            other => panic!("expected Number literal, got {:?}", other),
        },
        other => panic!("expected Literal operand, got {:?}", other),
    }
}

/// Get the integer value from a UnaryExpr that wraps a simple literal.
fn unary_lit_number(u: &UnaryExpr) -> u64 {
    match u {
        UnaryExpr::PrimaryExpr(pe) => match &pe.operand {
            Operand::Literal(lit) => match &lit.kind {
                LiteralKind::Number(n) => *n,
                other => panic!("expected Number literal, got {:?}", other),
            },
            other => panic!("expected Literal operand, got {:?}", other),
        },
        other => panic!("expected PrimaryExpr, got {:?}", other),
    }
}

/// Get the ident name from a UnaryExpr that wraps a simple ident.
fn unary_ident_name(u: &UnaryExpr) -> &str {
    match u {
        UnaryExpr::PrimaryExpr(pe) => match &pe.operand {
            Operand::Ident(path) => match &path.path[0] {
                IdentOrType::Ident(id) => &id.name,
                other => panic!("expected Ident in path, got {:?}", other),
            },
            other => panic!("expected Ident operand, got {:?}", other),
        },
        other => panic!("expected PrimaryExpr, got {:?}", other),
    }
}

/// Get the secondaries list from a primary expression.
fn get_secondaries(expr: &Expression) -> &[SecondaryExpr] {
    let pe = as_primary(expr);
    pe.secondaries.as_deref().unwrap_or(&[])
}

// ===========================================================================
// Operator chain structure
// ===========================================================================

#[test]
fn binop_chain_is_right_associative() {
    // `1 + 2 + 3` should parse as BinopExpr(1, +, BinopExpr(2, +, 3))
    let p = parse("main = -> 1 + 2 + 3\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    assert_eq!(unary_lit_number(lhs), 1);

    // rhs should be another BinopExpr(2, +, 3)
    let (lhs2, op2, rhs2) = as_binop(rhs);
    assert_eq!(op2, "+");
    assert_eq!(unary_lit_number(lhs2), 2);
    assert_eq!(lit_number(rhs2), 3);
}

#[test]
fn mixed_operators_form_right_recursive_chain() {
    // `1 + 2 * 3 - 4` → BinopExpr(1, +, BinopExpr(2, *, BinopExpr(3, -, 4)))
    let p = parse("main = -> 1 + 2 * 3 - 4\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    assert_eq!(unary_lit_number(lhs), 1);

    let (lhs2, op2, rhs2) = as_binop(rhs);
    assert_eq!(op2, "*");
    assert_eq!(unary_lit_number(lhs2), 2);

    let (lhs3, op3, rhs3) = as_binop(rhs2);
    assert_eq!(op3, "-");
    assert_eq!(unary_lit_number(lhs3), 3);
    assert_eq!(lit_number(rhs3), 4);
}

#[test]
fn comparison_operators_in_chain() {
    // `a == b` should be a single BinopExpr
    let p = parse("main = -> a == b\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "==");
    assert_eq!(unary_ident_name(lhs), "a");
    assert_eq!(ident_name(rhs), "b");
}

#[test]
fn four_level_operator_chain() {
    // `a + b + c + d + e` → 4 levels of right-nesting
    let p = parse("main = -> a + b + c + d + e\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, _, rhs) = as_binop(expr);
    assert_eq!(unary_ident_name(lhs), "a");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "b");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "c");

    let (lhs, _, rhs) = as_binop(rhs);
    assert_eq!(unary_ident_name(lhs), "d");

    // terminal
    assert_eq!(ident_name(rhs), "e");
}

// ===========================================================================
// Unary operators
// ===========================================================================

#[test]
fn unary_operator_wraps_inner_expression() {
    // `-x` should be UnaryExpr(-, PrimaryExpr(x))
    let p = parse("main = -> -x\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    match expr {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, inner)) => {
            assert_eq!(op.value, "-");
            match inner.as_ref() {
                UnaryExpr::PrimaryExpr(pe) => {
                    assert!(matches!(&pe.operand, Operand::Ident(_)));
                }
                other => panic!("expected PrimaryExpr inside unary, got {:?}", other),
            }
        }
        other => panic!("expected unary expression, got {:?}", other),
    }
}

#[test]
fn double_unary_is_single_stuck_operator() {
    // `!!x` is lexed as a single stuck operator "!!" applied to x
    let p = parse("main = -> !!x\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    match expr {
        Expression::UnaryExpr(UnaryExpr::UnaryExpr(op, inner)) => {
            assert_eq!(op.value, "!!");
            assert!(
                matches!(inner.as_ref(), UnaryExpr::PrimaryExpr(_)),
                "inner should be PrimaryExpr(x)"
            );
        }
        other => panic!("expected unary expression, got {:?}", other),
    }
}

#[test]
fn unary_before_binop() {
    // `-a + b` should be BinopExpr(UnaryExpr(-, a), +, b)
    let p = parse("main = -> -a + b\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    let (lhs, op, rhs) = as_binop(expr);
    assert_eq!(op, "+");
    // LHS should be unary negation
    match lhs {
        UnaryExpr::UnaryExpr(unop, _inner) => assert_eq!(unop.value, "-"),
        other => panic!("expected unary LHS, got {:?}", other),
    }
    assert_eq!(ident_name(rhs), "b");
}

// ===========================================================================
// Function call argument binding
// ===========================================================================

#[test]
fn single_argument_binds_to_function() {
    // `foo 42` → foo with one Argument(42)
    let p = parse("main = -> foo 42\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    assert!(matches!(&pe.operand, Operand::Ident(_)));
    let secs = pe.secondaries.as_ref().expect("should have secondaries");
    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => {
            assert_eq!(args.len(), 1);
            assert_eq!(lit_number(&args[0].arg), 42);
        }
        other => panic!("expected Arguments, got {:?}", other),
    }
}

#[test]
fn multiple_arguments_bind_together() {
    // `foo 1, 2, 3` → foo with one Arguments([1, 2, 3])
    let p = parse("main = -> foo 1, 2, 3\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    let secs = pe.secondaries.as_ref().expect("should have secondaries");
    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => {
            assert_eq!(args.len(), 3);
            assert_eq!(lit_number(&args[0].arg), 1);
            assert_eq!(lit_number(&args[1].arg), 2);
            assert_eq!(lit_number(&args[2].arg), 3);
        }
        other => panic!("expected Arguments, got {:?}", other),
    }
}

#[test]
fn function_call_then_dot_structure() {
    // `foo.bar` → foo with Dot(bar)
    let p = parse("main = -> foo.bar\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Dot(IdentOrNumber::Ident(id)) => assert_eq!(id.name, "bar"),
        other => panic!("expected Dot(bar), got {:?}", other),
    }
}

#[test]
fn chained_dots_structure() {
    // `a.b.c.d` → one PrimaryExpr with 3 Dot secondaries
    let p = parse("main = -> a.b.c.d\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 3, "should have 3 dot secondaries");
    let names: Vec<&str> = secs
        .iter()
        .map(|s| match s {
            SecondaryExpr::Dot(IdentOrNumber::Ident(id)) => id.name.as_str(),
            other => panic!("expected Dot, got {:?}", other),
        })
        .collect();
    assert_eq!(names, vec!["b", "c", "d"]);
}

#[test]
fn index_expression_structure_deep() {
    // `arr[0]` → arr with Indice(0)
    let p = parse("main = -> arr[0]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Indice(inner) => {
            assert_eq!(lit_number(inner), 0);
        }
        other => panic!("expected Indice, got {:?}", other),
    }
}

#[test]
fn chained_index_expressions_structure() {
    // `arr[0][1]` → arr with 2 Indice secondaries
    let p = parse("main = -> arr[0][1]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 2);
    assert!(matches!(&secs[0], SecondaryExpr::Indice(_)));
    assert!(matches!(&secs[1], SecondaryExpr::Indice(_)));
}

#[test]
fn interleaved_dot_and_index() {
    // `a.b[0].c` → a with [Dot(b), Indice(0), Dot(c)]
    let p = parse("main = -> a.b[0].c\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 3, "secondaries: {:?}", secs);
    assert!(matches!(&secs[0], SecondaryExpr::Dot(IdentOrNumber::Ident(id)) if id.name == "b"));
    assert!(matches!(&secs[1], SecondaryExpr::Indice(_)));
    assert!(matches!(&secs[2], SecondaryExpr::Dot(IdentOrNumber::Ident(id)) if id.name == "c"));
}

#[test]
fn bang_call_has_no_arguments_structure() {
    // `foo!` → foo with Arguments([]) (bang = zero-arg call)
    let p = parse("main = -> foo!\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let secs = get_secondaries(expr);

    assert_eq!(secs.len(), 1);
    match &secs[0] {
        SecondaryExpr::Arguments(args) => assert!(args.is_empty(), "bang call should have 0 args"),
        other => panic!("expected Arguments, got {:?}", other),
    }
}

// ===========================================================================
// Expressions-as-values: if/match/loop used in assignment RHS
// ===========================================================================

#[test]
fn if_as_assignment_value_structure() {
    let p = parse("main = ->\n    x = if true then 1 else 2\n    x\n");
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // RHS should contain an If operand
    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::If(_)),
        "RHS should be an If, got {:?}",
        pe.operand
    );

    // Dig into the If
    if let Operand::If(if_expr) = &pe.operand {
        // Condition should be `true`
        let cond_expr = &if_expr.condition.expression;
        match as_operand(cond_expr) {
            Operand::Literal(lit) => assert_eq!(lit.kind, LiteralKind::Bool(true)),
            other => panic!("expected bool literal in condition, got {:?}", other),
        }

        // Then-block should have one statement yielding 1
        assert_eq!(if_expr.then.statements.len(), 1);
        assert_eq!(lit_number(as_expr(&if_expr.then.statements[0])), 1);

        // Else-block should yield 2
        match &if_expr.else_ {
            Some(Else::Block(block)) => {
                assert_eq!(block.statements.len(), 1);
                assert_eq!(lit_number(as_expr(&block.statements[0])), 2);
            }
            other => panic!("expected Else::Block, got {:?}", other),
        }
    }
}

#[test]
fn match_as_assignment_value_structure() {
    let input = r#"main = ->
    x = match 5
        0 => "zero"
        _ => "other"
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Match(_)),
        "RHS should be a Match"
    );

    if let Operand::Match(m) = &pe.operand {
        // Scrutinee should be 5
        assert_eq!(lit_number(&m.expr), 5);
        // Should have 2 arms
        assert_eq!(m.arms.len(), 2);
        // First arm pattern should be literal 0
        assert!(
            matches!(&m.arms[0].pattern.kind, PatternKind::Literal(lit) if lit.kind == LiteralKind::Number(0))
        );
        // Second arm should be wildcard
        assert!(matches!(&m.arms[1].pattern.kind, PatternKind::Wildcard));
    }
}

#[test]
fn loop_as_assignment_value_structure() {
    let input = r#"main = ->
    x = loop
        break 42
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Loop(_)),
        "RHS should be a Loop"
    );

    if let Operand::Loop(lp) = &pe.operand {
        match lp.as_ref() {
            Loop::Loop(block, _) => {
                assert_eq!(block.statements.len(), 1);
                assert!(matches!(&block.statements[0], Statement::Break(Some(_))));
            }
            other => panic!("expected Loop::Loop, got {:?}", other),
        }
    }
}

// ===========================================================================
// Nested control flow: expression within expression
// ===========================================================================

#[test]
fn match_nested_in_function_body_with_if() {
    // Test that match and if can coexist in the same function body
    let input = r#"main = ->
    x = match 1
        0 => "a"
        _ => "b"
    if true then x else "c"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let stmts = &f.lambda.body.statements;
    assert_eq!(stmts.len(), 2);

    // First statement: assignment with match on RHS
    let assign = as_assign(&stmts[0]);
    let pe = as_primary(&assign.rhs);
    assert!(
        matches!(&pe.operand, Operand::Match(m) if m.arms.len() == 2),
        "RHS should be a match with 2 arms, got {:?}",
        pe.operand
    );

    // Second statement: if expression that uses the match result
    let if_expr_stmt = as_expr(&stmts[1]);
    let if_pe = as_primary(if_expr_stmt);
    assert!(
        matches!(&if_pe.operand, Operand::If(_)),
        "second stmt should be an If"
    );
}

#[test]
fn if_inside_match_arm() {
    let input = r#"main = ->
    match 1
        1 => if true then "yes" else "no"
        _ => "default"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        // First arm body should contain an If
        let arm0_expr = as_expr(&m.arms[0].body.statements[0]);
        let arm0_pe = as_primary(arm0_expr);
        assert!(
            matches!(&arm0_pe.operand, Operand::If(_)),
            "arm 0 should be an If"
        );
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn for_loop_body_contains_if_with_break() {
    let input = r#"main = ->
    for i in [1, 2, 3]
        if i > 2 then break i
    0
"#;
    let p = parse(input);
    let f = first_fn(&p);
    // First statement should be a loop expression
    let loop_expr = as_expr(&f.lambda.body.statements[0]);
    let loop_pe = as_primary(loop_expr);

    if let Operand::Loop(lp) = &loop_pe.operand {
        match lp.as_ref() {
            Loop::For(pattern, iter_expr, body, _) => {
                // Pattern should be `i`
                assert!(matches!(&pattern.kind, PatternKind::Ident(ip) if ip.name.name == "i"));
                // Iterator should be an array literal
                assert!(
                    matches!(as_operand(iter_expr), Operand::Literal(lit) if matches!(&lit.kind, LiteralKind::Array(_)))
                );
                // Body should contain an if expression
                assert_eq!(body.statements.len(), 1);
                let body_pe = as_primary(as_expr(&body.statements[0]));
                assert!(matches!(&body_pe.operand, Operand::If(_)));
            }
            other => panic!("expected For loop, got {:?}", other),
        }
    } else {
        panic!("expected Loop operand");
    }
}

#[test]
fn while_condition_is_binop() {
    let input = r#"main = ->
    x = 0
    while x < 10
        x = x + 1
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    // Statement index 1 is the while loop
    let while_expr = as_expr(&f.lambda.body.statements[1]);
    let while_pe = as_primary(while_expr);

    if let Operand::Loop(lp) = &while_pe.operand {
        match lp.as_ref() {
            Loop::While(cond, body, _) => {
                // Condition should be a binop `x < 10`
                assert!(
                    matches!(&cond.expression, Expression::BinopExpr(_, _, _)),
                    "while condition should be a BinopExpr",
                );
                let (lhs, op, rhs) = as_binop(&cond.expression);
                assert_eq!(unary_ident_name(lhs), "x");
                assert_eq!(op, "<");
                assert_eq!(lit_number(rhs), 10);

                // Body should contain an assignment `x = x + 1`
                assert_eq!(body.statements.len(), 1);
                assert!(matches!(&body.statements[0], Statement::Assignment(_)));
            }
            other => panic!("expected While loop, got {:?}", other),
        }
    } else {
        panic!("expected Loop operand");
    }
}

// ===========================================================================
// Deeply nested constructs
// ===========================================================================

#[test]
fn nested_parenthesized_expressions_preserve_structure() {
    // `((1 + 2))` → Expression(Expression(BinopExpr(1, +, 2)))
    let p = parse("main = -> ((1 + 2))\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);

    // Outer should be a parenthesized expression
    let pe = as_primary(expr);
    match &pe.operand {
        Operand::Expression(inner) => {
            // Inner should also be parenthesized or directly the binop
            let inner_pe = as_primary(inner);
            match &inner_pe.operand {
                Operand::Expression(innermost) => {
                    // Innermost should be `1 + 2`
                    let (lhs, op, rhs) = as_binop(innermost);
                    assert_eq!(op, "+");
                    assert_eq!(unary_lit_number(lhs), 1);
                    assert_eq!(lit_number(rhs), 2);
                }
                other => panic!("expected nested Expression, got {:?}", other),
            }
        }
        other => panic!("expected Expression (parens), got {:?}", other),
    }
}

#[test]
fn nested_if_else_chain_structure() {
    let input = r#"main = x ->
    if x > 10 then 3
    else if x > 5 then 2
    else 1
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::If(outer_if) = &pe.operand {
        // Outer if: condition is `x > 10`, then yields 3
        let (lhs, op, _) = as_binop(&outer_if.condition.expression);
        assert_eq!(unary_ident_name(lhs), "x");
        assert_eq!(op, ">");

        // Else should be another If
        match &outer_if.else_ {
            Some(Else::If(inner_if)) => {
                // Inner if: condition is `x > 5`, then yields 2
                let (lhs2, _, _) = as_binop(&inner_if.condition.expression);
                assert_eq!(unary_ident_name(lhs2), "x");

                // Else should be a plain block yielding 1
                match &inner_if.else_ {
                    Some(Else::Block(block)) => {
                        assert_eq!(lit_number(as_expr(&block.statements[0])), 1);
                    }
                    other => panic!("expected Else::Block for innermost else, got {:?}", other),
                }
            }
            other => panic!("expected Else::If, got {:?}", other),
        }
    } else {
        panic!("expected If operand");
    }
}

#[test]
fn deeply_nested_match_with_complex_patterns() {
    let input = r#"main = ->
    match (1, (2, 3))
        (0, _) => "zero"
        (a, (b, c)) => match b
            2 => "found two"
            _ => "nope"
        _ => "default"
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 3);

        // Second arm: pattern is (a, (b, c)), body contains another match
        let arm1 = &m.arms[1];
        match &arm1.pattern.kind {
            PatternKind::Tuple(elems) => {
                assert_eq!(elems.len(), 2);
                // Second element should be a nested tuple pattern
                match &elems[1].kind {
                    PatternKind::Tuple(inner_elems) => assert_eq!(inner_elems.len(), 2),
                    other => panic!("expected nested tuple pattern, got {:?}", other),
                }
            }
            other => panic!("expected tuple pattern, got {:?}", other),
        }

        // Arm 1 body should be a nested match
        let arm1_expr = as_expr(&arm1.body.statements[0]);
        let arm1_pe = as_primary(arm1_expr);
        assert!(
            matches!(&arm1_pe.operand, Operand::Match(inner_m) if inner_m.arms.len() == 2),
            "inner match should have 2 arms"
        );
    } else {
        panic!("expected Match operand");
    }
}

// ===========================================================================
// Lambda structure
// ===========================================================================

#[test]
fn lambda_params_are_patterns() {
    let p = parse("f = (a, b), c -> a\n");
    let f = first_fn(&p);

    assert_eq!(f.lambda.parameters.len(), 2);
    // First param is a tuple pattern
    assert!(matches!(&f.lambda.parameters[0].kind, PatternKind::Tuple(elems) if elems.len() == 2));
    // Second param is a simple ident
    assert!(matches!(
        &f.lambda.parameters[1].kind,
        PatternKind::Ident(_)
    ));
}

#[test]
fn lambda_body_expression_vs_block() {
    // Inline body: `f = x -> x + 1`
    let p = parse("f = x -> x + 1\n");
    let f = first_fn(&p);
    assert_eq!(f.lambda.body.statements.len(), 1);
    assert!(matches!(
        &f.lambda.body.statements[0],
        Statement::Expression(Expression::BinopExpr(_, _, _))
    ));
}

#[test]
fn nested_lambda_in_assignment() {
    let input = r#"main = ->
    compose = f, g, x -> f (g x)
    compose
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // The RHS should be a lambda
    let pe = as_primary(&assign.rhs);
    if let Operand::LambdaDecl(lambda) = &pe.operand {
        assert_eq!(lambda.parameters.len(), 3);
        // Body should have one expression: `f (g x)`
        assert_eq!(lambda.body.statements.len(), 1);
        let body_expr = as_expr(&lambda.body.statements[0]);
        let body_pe = as_primary(body_expr);
        // `f` should have an argument which is `(g x)` — a parenthesized expression
        let secs = body_pe
            .secondaries
            .as_ref()
            .expect("f should have arguments");
        assert_eq!(secs.len(), 1);
        assert!(matches!(&secs[0], SecondaryExpr::Arguments(args) if args.len() == 1));
    } else {
        panic!("expected LambdaDecl, got {:?}", pe.operand);
    }
}

#[test]
fn inline_curried_lambda() {
    let input = r#"main = ->
    inc = x ~> x + 1
    inc
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    let pe = as_primary(&assign.rhs);
    if let Operand::LambdaDecl(lambda) = &pe.operand {
        assert_eq!(lambda.arrow_kind, LambdaArrowKind::Curried);
        assert_eq!(lambda.parameters.len(), 1);
    } else {
        panic!("expected LambdaDecl, got {:?}", pe.operand);
    }
}

// ===========================================================================
// Pattern depth in match
// ===========================================================================

#[test]
fn match_guard_is_proper_expression() {
    let input = r#"main = ->
    match x
        n if n > 0 && n < 100 => n
        _ => 0
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        // First arm should have a guard that is a BinopExpr (n > 0 && n < 100)
        let guard = m.arms[0]
            .condition
            .as_ref()
            .expect("arm 0 should have guard");
        // The guard is `n > 0 && n < 100` which is a chain of binops
        assert!(
            matches!(guard, Expression::BinopExpr(_, _, _)),
            "guard should be a BinopExpr"
        );
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn match_arm_with_binding_at_sign() {
    let input = r#"main = ->
    match (1, 2)
        foo @ (a, b) => foo
        _ => (0, 0)
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Match(m) = &pe.operand {
        let arm0 = &m.arms[0];
        // Should have a binding named "foo"
        assert!(arm0.pattern.binding.is_some());
        assert_eq!(arm0.pattern.binding.as_ref().unwrap().name, "foo");
        // Kind should be a tuple
        assert!(matches!(&arm0.pattern.kind, PatternKind::Tuple(elems) if elems.len() == 2));
    } else {
        panic!("expected Match operand");
    }
}

// ===========================================================================
// Array literal contents
// ===========================================================================

#[test]
fn array_literal_elements_are_correct() {
    let p = parse("main = -> [10, 20, 30]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Literal(lit) = &pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 3);
            assert_eq!(lit_number(&arr.elements[0]), 10);
            assert_eq!(lit_number(&arr.elements[1]), 20);
            assert_eq!(lit_number(&arr.elements[2]), 30);
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand");
    }
}

#[test]
fn array_of_expressions() {
    let p = parse("main = -> [1 + 2, 3 * 4]\n");
    let f = first_fn(&p);
    let expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(expr);

    if let Operand::Literal(lit) = &pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 2);
            // Each element should be a BinopExpr
            assert!(matches!(&arr.elements[0], Expression::BinopExpr(_, _, _)));
            assert!(matches!(&arr.elements[1], Expression::BinopExpr(_, _, _)));
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand");
    }
}

// ===========================================================================
// Struct instantiation field structure
// ===========================================================================

#[test]
fn struct_instantiation_fields_are_correct() {
    let input = r#"struct Point
    x: Int
    y: Int

main = ->
    p = Point x: 10, y: 20
    p
"#;
    let p = parse(input);
    let f = find_fn(&p, "main");
    let assign = as_assign(&f.lambda.body.statements[0]);
    let pe = as_primary(&assign.rhs);

    if let Operand::Instance(inst) = &pe.operand {
        assert_eq!(inst.fields.len(), 2);
        // Fields are in a BTreeMap so they come out alphabetically
        let x_expr = inst
            .fields
            .get(&Ident {
                name: "x".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have x");
        let y_expr = inst
            .fields
            .get(&Ident {
                name: "y".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have y");
        assert_eq!(lit_number(x_expr), 10);
        assert_eq!(lit_number(y_expr), 20);
    } else {
        panic!("expected Instance operand, got {:?}", pe.operand);
    }
}

#[test]
fn struct_field_with_expression_value() {
    let input = r#"struct Foo
    x: Int

main = ->
    p = Foo x: 1 + 2
    p
"#;
    let p = parse(input);
    let f = find_fn(&p, "main");
    let assign = as_assign(&f.lambda.body.statements[0]);
    let pe = as_primary(&assign.rhs);

    if let Operand::Instance(inst) = &pe.operand {
        let x_expr = inst
            .fields
            .get(&Ident {
                name: "x".to_string(),
                span: crate::lexer::Span::test(),
            })
            .expect("should have x");
        // x's value should be a BinopExpr: 1 + 2
        assert!(
            matches!(x_expr, Expression::BinopExpr(_, _, _)),
            "field value should be a BinopExpr"
        );
    } else {
        panic!("expected Instance operand");
    }
}

// ===========================================================================
// Self ident (@) structure
// ===========================================================================

#[test]
fn self_ident_desugars_correctly() {
    let input = r#"struct Foo
    x: Int

impl Foo
    get = -> @x
"#;
    let p = parse(input);
    let i = find_impl(&p, "Foo");
    let get = i
        .methods
        .get(&Ident {
            name: "get".to_string(),
            span: crate::lexer::Span::test(),
        })
        .expect("should have get method");
    let body_expr = as_expr(&get.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    assert!(
        matches!(&pe.operand, Operand::SelfIdent(ident) if ident.name == "x"),
        "expected SelfIdent(x), got {:?}",
        pe.operand
    );
}

// ===========================================================================
// Expression with type annotation
// ===========================================================================

#[test]
fn expression_with_type_annotation_structure() {
    let input = r#"main = ->
    x: Int = 42
    x
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let assign = as_assign(&f.lambda.body.statements[0]);

    // LHS should be a pattern with type annotation
    match &assign.lhs {
        AssignmentLHS::Pattern {
            pattern,
            type_annotation,
        } => {
            assert!(matches!(&pattern.kind, PatternKind::Ident(ip) if ip.name.name == "x"));
            assert!(type_annotation.is_some());
            let ty = type_annotation.as_ref().unwrap();
            match ty {
                ParseType::Type(inner) => assert_eq!(inner.name, "Int"),
                other => panic!("expected Type annotation, got {:?}", other),
            }
        }
        other => panic!("expected Pattern LHS, got {:?}", other),
    }
}

// ===========================================================================
// Complex real-world-ish programs: deep structural checks
// ===========================================================================

#[test]
fn realistic_recursive_style_program_structure() {
    let input = r#"map = f, list ->
    match list
        [] => []
        [head, ..tail] => [f head] + map f, tail
"#;
    let p = parse(input);
    let f = find_fn(&p, "map");

    // Should have 2 params
    assert_eq!(f.lambda.parameters.len(), 2);

    // Body is a match expression
    let body_expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 2);

        // First arm: pattern is empty array `[]`
        assert!(matches!(&m.arms[0].pattern.kind, PatternKind::Array(elems) if elems.is_empty()));

        // Second arm: pattern is `[head, ..tail]`
        match &m.arms[1].pattern.kind {
            PatternKind::Array(elems) => {
                assert_eq!(elems.len(), 2);
                assert!(matches!(&elems[0], ArrayPattern::Pattern(_)));
                assert!(matches!(&elems[1], ArrayPattern::Rest(_)));
            }
            other => panic!("expected array pattern, got {:?}", other),
        }

        // Second arm body should be a BinopExpr with `+`
        let arm1_expr = as_expr(&m.arms[1].body.statements[0]);
        let (_, op, _) = as_binop(arm1_expr);
        assert_eq!(op, "+", "second arm should use + to concat arrays");
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn enum_match_exhaustive_pattern_structure() {
    let input = r#"enum Direction
    North
    South
    East
    West

to_string = dir ->
    match dir
        North => "N"
        South => "S"
        East => "E"
        West => "W"
"#;
    let p = parse(input);
    let e = find_enum(&p, "Direction");
    assert_eq!(e.variants.len(), 4);

    let f = find_fn(&p, "to_string");
    let body_expr = as_expr(&f.lambda.body.statements[0]);
    let pe = as_primary(body_expr);

    if let Operand::Match(m) = &pe.operand {
        assert_eq!(m.arms.len(), 4, "should have one arm per variant");
        // Each arm pattern should be an ident matching a variant name
        let arm_names: Vec<&str> = m
            .arms
            .iter()
            .map(|arm| match &arm.pattern.kind {
                PatternKind::Ident(ip) => ip.name.name.as_str(),
                PatternKind::Instance(ip) => match &ip.name.path[0] {
                    IdentOrType::Ident(id) => id.name.as_str(),
                    IdentOrType::Type(ty) => match ty {
                        ParseType::Type(inner) => inner.name.as_str(),
                        _ => panic!("unexpected type in pattern"),
                    },
                },
                other => panic!("expected ident pattern, got {:?}", other),
            })
            .collect();
        assert_eq!(arm_names, vec!["North", "South", "East", "West"]);
    } else {
        panic!("expected Match operand");
    }
}

#[test]
fn multiple_statements_structure_preserved() {
    let input = r#"main = ->
    a = 1
    b = a + 2
    c = if b > 2 then b else 0
    [a, b, c]
"#;
    let p = parse(input);
    let f = first_fn(&p);
    let stmts = &f.lambda.body.statements;

    assert_eq!(stmts.len(), 4);
    // stmt 0: Assignment a = 1
    assert!(matches!(&stmts[0], Statement::Assignment(_)));
    // stmt 1: Assignment b = a + 2
    let assign_b = as_assign(&stmts[1]);
    assert!(matches!(&assign_b.rhs, Expression::BinopExpr(_, _, _)));
    // stmt 2: Assignment c = if ...
    let assign_c = as_assign(&stmts[2]);
    assert!(matches!(as_operand(&assign_c.rhs), Operand::If(_)));
    // stmt 3: Expression [a, b, c]
    let last_pe = as_primary(as_expr(&stmts[3]));
    if let Operand::Literal(lit) = &last_pe.operand {
        if let LiteralKind::Array(arr) = &lit.kind {
            assert_eq!(arr.elements.len(), 3);
        } else {
            panic!("expected Array literal");
        }
    } else {
        panic!("expected Literal operand for last statement");
    }
}
