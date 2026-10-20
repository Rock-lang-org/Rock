use std::collections::HashMap;

use crate::ast::{InvocationId, MacroInvoc, NodeId};
use crate::lexer::lossless::{ConstructionContext, ConstructionKind, SyntaxContext};
use crate::products::ProductCrateIdentity;

use super::{ExpansionId, MacroExpansionContext};

#[derive(Debug)]
pub(super) struct Hygiene {
    next: u32,
    templates: HashMap<(ExpansionId, SyntaxContext), SyntaxContext>,
    requests: HashMap<(InvocationId, ConstructionKind), SyntaxContext>,
    definitions: HashMap<NodeId, Vec<(String, SyntaxContext)>>,
}

impl Default for Hygiene {
    fn default() -> Self {
        Self { next: 1, templates: HashMap::new(), requests: HashMap::new(), definitions: HashMap::new() }
    }
}

impl Hygiene {
    fn allocate(&mut self) -> Result<SyntaxContext, String> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| "macro syntax context space exhausted".to_string())?;
        Ok(SyntaxContext::allocated(id))
    }
}

impl MacroExpansionContext<'_> {
    pub(crate) fn fresh_private_context(&self) -> Result<SyntaxContext, String> {
        self.hygiene.borrow_mut().allocate()
    }

    pub(crate) fn fresh_source_context(&self, path: Vec<(String, SyntaxContext)>, original: SyntaxContext) -> Result<SyntaxContext, String> {
        let context = self.fresh_private_context()?;
        self.binding_environments.borrow_mut().record_source_scope(context, path, original)?;
        Ok(context)
    }

    pub(crate) fn fresh_dependency_context(&self, identity: &ProductCrateIdentity) -> Result<SyntaxContext, String> {
        let context = self.fresh_private_context()?;
        self.binding_environments.borrow_mut().record_dependency(context, identity, std::slice::from_ref(identity))?;
        Ok(context)
    }

    pub(super) fn record_definition_scope(&self, id: NodeId, path: Vec<(String, SyntaxContext)>) -> Result<(), String> {
        let mut hygiene = self.hygiene.borrow_mut();
        if hygiene.definitions.get(&id).is_some_and(|previous| *previous != path) {
            return Err("macro definition identity has conflicting source environments".into());
        }
        hygiene.definitions.insert(id, path);
        Ok(())
    }

    pub(super) fn template_context(&self, definition: NodeId, expansion: ExpansionId, original: SyntaxContext) -> Result<SyntaxContext, String> {
        let mut hygiene = self.hygiene.borrow_mut();
        if let Some(context) = hygiene.templates.get(&(expansion, original)) { return Ok(*context); }
        let path = hygiene.definitions.get(&definition).cloned()
            .ok_or_else(|| "macro definition has no recorded source environment".to_string())?;
        let context = hygiene.allocate()?;
        self.binding_environments.borrow_mut().record_source_scope(context, path, original)?;
        hygiene.templates.insert((expansion, original), context);
        Ok(context)
    }

    pub(super) fn construction_context(
        &self,
        request: ConstructionContext,
        invocation: &MacroInvoc,
        dependencies: &[ProductCrateIdentity],
    ) -> Result<SyntaxContext, String> {
        if request.invocation != invocation.id { return Err("construction belongs to another invocation".into()); }
        if request.kind == ConstructionKind::CallSite {
            return crate::collect::resolver::binding_path(&invocation.path.path)
                .and_then(|path| path.first().map(|(_, context)| *context))
                .ok_or_else(|| "macro invocation has no binding context".into());
        }
        let mut hygiene = self.hygiene.borrow_mut();
        if let Some(context) = hygiene.requests.get(&(invocation.id, request.kind)) { return Ok(*context); }
        let dependency = match request.kind {
            ConstructionKind::Dependency(index) => Some(dependencies.get(index as usize)
                .ok_or_else(|| "undeclared definition-side dependency handle".to_string())?),
            ConstructionKind::Fresh(_) => None,
            ConstructionKind::CallSite => unreachable!("call-site requests return above"),
        };
        let context = hygiene.allocate()?;
        if let Some(identity) = dependency {
            self.binding_environments.borrow_mut().record_dependency(context, identity, dependencies)?;
        }
        hygiene.requests.insert((invocation.id, request.kind), context);
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Program, TopLevel};

    fn resolve(text: &str) -> crate::infer::ResolvedHirProgram {
        let config = crate::Config::default();
        let context = MacroExpansionContext::new(&config);
        let program = crate::parser::parse_string(text, &config).unwrap();
        let program = super::super::expand_macros_with_context(program, &context).unwrap();
        let crates = crate::crate_system::CrateContext::new();
        let declarations = crate::collect::collect_with_binding_environments(&program, None, &crates, false, None, &context.binding_environments.borrow()).unwrap();
        let partial = crate::lower::pipeline::LoweringPipeline::new(&program, &crates, None).lower_from_declarations(declarations).unwrap();
        crate::infer::finalize(partial).unwrap()
    }

    #[test]
    fn macro_a_14_templates_cannot_capture_same_spelled_caller_locals() {
        let resolved = resolve("macro keep\n    $value:expr =>\n        if true\n            value = true\n            $value\n        else\n            $value\nmain = ->\n    value = 42\n    %keep value\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::I64);
    }

    #[test]
    fn macro_a_14_template_free_names_use_definition_import_environment() {
        let resolved = resolve("mod support\n    < helper = -> true\nmod tools\n    > support::helper\n    macro pick\n        =>\n            helper!\n    < pick\nhelper = -> 1\nmain = -> %tools::pick!\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::Bool);
    }

    #[test]
    fn macro_a_14_template_item_names_are_private_unless_captured() {
        let config = crate::Config::default();
        let context = MacroExpansionContext::new(&config);
        let Program { module } = super::super::expand_macros_with_context(crate::parser::parse_string(
            "macro make\n    $name:ident =>\n        $name = -> 1\n        helper = -> 2\n%make exposed\n%make other\n", &config).unwrap(), &context).unwrap();
        let names = module.top_levels.iter().filter_map(|item| match item { TopLevel::FunctionDecl(item) => Some(&item.name), _ => None }).collect::<Vec<_>>();
        assert_eq!(names[0].context, SyntaxContext::ROOT);
        assert_ne!(names[1].context, SyntaxContext::ROOT);
        assert_eq!(names[2].context, SyntaxContext::ROOT);
        assert_ne!(names[3].context, names[1].context);
    }

    #[test]
    fn macro_a_14_nested_templates_keep_inherited_private_macro_bindings() {
        let resolved = resolve("macro install\n    $name:ident =>\n        macro $name\n            =>\n                %helper!\n        macro helper\n            =>\n                true\n%install first\n%install second\nmacro helper\n    =>\n        1\nmain = -> (%first!, %second!)\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::Tuple(vec![crate::types::Type::Bool, crate::types::Type::Bool]));
    }

    #[test]
    fn macro_a_14_generated_same_spelling_modules_keep_distinct_macro_scopes() {
        let resolved = resolve("macro install\n    $name:ident =>\n        mod internal\n            < value\n            macro value\n                =>\n                    true\n        macro $name\n            =>\n                %internal::value!\n%install first\n%install second\nmain = -> (%first!, %second!)\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::Tuple(vec![crate::types::Type::Bool, crate::types::Type::Bool]));
    }

    #[test]
    fn macro_a_14_template_macro_imports_use_definition_scope_without_exposing_aliases() {
        let resolved = resolve("mod tools\n    < number\n    macro number\n        =>\n            true\nmacro build\n    $entry:ident =>\n        > tools::number\n        $entry = -> %number!\n%build main\nmacro number\n    =>\n        1\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::Bool);
    }

    #[test]
    fn macro_a_14_public_surfaces_require_captured_names() {
        let config = crate::Config::default();
        let context = MacroExpansionContext::new(&config);
        let program = super::super::expand_macros_with_context(crate::parser::parse_string(
            "macro make\n    $name:ident =>\n        < $name = -> 1\n        < hidden = -> 2\n%make exposed\n", &config).unwrap(), &context).unwrap();
        let exports = crate::collect::collect_exports(&program.module);
        assert!(exports.contains_key("exposed"));
        assert!(!exports.contains_key("hidden"));
    }

    #[test]
    fn macro_a_14_template_macro_imports_preserve_selected_module_aliases() {
        let resolved = resolve("mod support\n    < tools\n    mod tools\n        < number\n        macro number\n            =>\n                true\nmacro build\n    $entry:ident =>\n        > support::tools\n        > tools::number\n        $entry = -> %number!\n%build main\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::Bool);
    }

    #[test]
    fn macro_a_14_template_variant_construction_and_patterns_use_definition_members() {
        let resolved = resolve("enum Choice\n    Value I64\n    Empty\nmacro choose\n    =>\n        match Choice::Value 42\n            Choice::Value value => value\n            Choice::Empty => 0\nmain = -> %choose!\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::I64);
    }

    #[test]
    fn macro_a_14_template_static_calls_use_definition_member_contexts() {
        let resolved = resolve("struct Maker\n    < value: I64\nimpl Maker\n    make = -> Maker value: 42\nmacro make\n    =>\n        Maker::make!\nmain = -> (%make!).value\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::I64);
    }

    #[test]
    fn macro_a_14_unqualified_template_variants_use_definition_module_before_root() {
        let resolved = resolve("enum Other\n    Yes\nmod tools\n    < enum Choice\n        Yes\n    < choose\n    macro choose\n        =>\n            Yes\nmain = ->\n    match (%tools::choose!)\n        tools::Choice::Yes => 42\n");
        let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
        assert_eq!(main.ret_type, crate::types::Type::I64);
    }

    #[test]
    fn macro_a_14_template_associated_type_names_retain_definition_members() {
        resolve("macro define\n    $name:ty, $member:ty =>\n        trait $name\n            type $member\n            pass: Self::Item -> Self::Item\n%define Other, Item\nmain = -> 1\n");
    }
}
