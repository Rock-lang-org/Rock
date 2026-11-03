use crate::ast::tree::*;
use crate::ast::visit::Visitor;
use crate::parser::parse_string;
use crate::Config;

// ---------------------------------------------------------------------------
// Helpers: AST invariant checker via the Visitor pattern
// ---------------------------------------------------------------------------

/// Collects every invariant violation found in the AST.
pub(super) struct AstValidator {
    pub(super) errors: Vec<String>,
    /// Stack tracking what context we are in for better error messages.
    context: Vec<String>,
}

impl AstValidator {
    pub(super) fn new() -> Self {
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
pub(super) fn validate(input: &str) {
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
pub(super) struct TopLevelCounter {
    pub(super) functions: usize,
    pub(super) structs: usize,
    pub(super) enums: usize,
    pub(super) traits: usize,
    pub(super) impls: usize,
    pub(super) externs: usize,
    pub(super) imports: usize,
    pub(super) exports: usize,
}

impl TopLevelCounter {
    pub(super) fn count(program: &Program) -> Self {
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
pub(super) fn parse(input: &str) -> Program {
    let config = Config::default();
    parse_string(input, &config).unwrap_or_else(|e| {
        panic!("parse_string failed: {:?}", e);
    })
}

/// Helper: extract the first function declaration from a program.
pub(super) fn first_fn(program: &Program) -> &FunctionDecl {
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
pub(super) fn find_fn<'a>(program: &'a Program, name: &str) -> &'a FunctionDecl {
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
pub(super) fn find_struct<'a>(program: &'a Program, name: &str) -> &'a StructDecl {
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
pub(super) fn find_enum<'a>(program: &'a Program, name: &str) -> &'a EnumDecl {
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
pub(super) fn find_trait<'a>(program: &'a Program, name: &str) -> &'a TraitDecl {
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
pub(super) fn find_impl<'a>(program: &'a Program, name: &str) -> &'a Impl {
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
