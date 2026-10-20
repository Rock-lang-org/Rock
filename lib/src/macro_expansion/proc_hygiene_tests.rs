use std::collections::BTreeMap;

use crate::ast::{MacroInvoc, Program};
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::crate_system::{CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lexer::lossless::{ConstructionContext, ConstructionKind, JoinBoundary, SyntaxContext};
use crate::products::ProductCrateIdentity;
use crate::Config;

use super::binding_environment::BindingEnvironments;
use super::proc_macro::{encode_stream, ProcMacroResponse};
use super::token_tree::MACRO_LIMITS;
use super::{expand_macros_with_context, MacroExpansionContext, TokenStream};

fn parse(text: &str) -> Program { crate::parser::parse_string(text, &Config::default()).unwrap() }

fn fragment(invocation: &MacroInvoc, text: &str, kind: ConstructionKind) -> TokenStream {
    TokenStream::source_fragment(text.into(), ConstructionContext { invocation: invocation.id, kind }, MACRO_LIMITS).unwrap()
}

fn join(parts: &[TokenStream]) -> TokenStream {
    parts.iter().skip(1).fold(parts[0].clone(), |left, right| left.join(right, JoinBoundary::Separate, MACRO_LIMITS).unwrap())
}

fn expand(program: Program, output: TokenStream, dependencies: Vec<ProductCrateIdentity>) -> Result<(Program, BindingEnvironments), crate::diagnostic::Diagnostics> {
    let response = ProcMacroResponse::expanded(encode_stream(&output).unwrap());
    let (mut artifact, directory) = super::tests::proc_macro_artifact_with_response("make", response);
    artifact.definition_dependencies = dependencies.clone();
    let request_path = artifact.executable.with_extension("request");
    let config = Config::default();
    let context = MacroExpansionContext::new(&config).with_named_proc_macro_artifact("provider".into(), artifact);
    let result = expand_macros_with_context(program, &context);
    let request_bytes = std::fs::read(&request_path).unwrap_or_else(|error| panic!("worker request {request_path:?}: {error}; expansion: {result:?}"));
    let request = super::proc_macro::decode_request(&request_bytes).unwrap();
    assert_eq!(request.definition_dependencies, dependencies, "worker request in {directory:?}: {request:?}");
    let environments = context.binding_environments.borrow().clone();
    std::fs::remove_dir_all(directory).unwrap();
    result.map(|program| (program, environments))
}

#[test]
fn macro_a_14_native_worker_fresh_names_do_not_capture_caller_bindings() {
    let program = parse("helper = -> 42\n%provider::make\n");
    let invocation = super::tests::first_invocation(&program);
    let output = join(&[
        fragment(&invocation, "helper = -> true\n", ConstructionKind::Fresh(7)),
        fragment(&invocation, "main = -> ", ConstructionKind::CallSite),
        fragment(&invocation, "helper!\n", ConstructionKind::Fresh(7)),
    ]);
    let (program, environments) = expand(program, output, vec![]).unwrap();
    let crates = CrateContext::new();
    let declarations = crate::collect::collect_with_binding_environments(&program, None, &crates, false, None, &environments).unwrap();
    let partial = crate::lower::pipeline::LoweringPipeline::new(&program, &crates, None).lower_from_declarations(declarations).unwrap();
    let resolved = crate::infer::finalize(partial).unwrap();
    let main = resolved.program.functions.values().find(|function| function.name == "main").unwrap();
    assert_eq!(main.ret_type, crate::types::Type::Bool);
}

#[test]
fn macro_a_14_native_worker_forwarding_retains_nonroot_input_context() {
    let header = super::token_tree::source("%provider::make ");
    let argument = super::token_tree::source("value\n").map_contexts(|_| Ok(SyntaxContext::allocated(90))).unwrap();
    let input = header.join(&argument, JoinBoundary::Separate, MACRO_LIMITS).unwrap();
    let config = Config::default();
    let program = Program { module: crate::parser::parse_lossless_source(&input, &config, None).unwrap() };
    let invocation = super::tests::first_invocation(&program);
    let output = join(&[
        fragment(&invocation, "main = -> ", ConstructionKind::CallSite),
        invocation.args.clone(),
    ]);
    let (program, _) = expand(program, output, vec![]).unwrap();
    struct Names(Vec<crate::ast::Ident>);
    impl<'a> crate::ast::visit::Visitor<'a> for Names {
        fn visit_ident(&mut self, name: &'a crate::ast::Ident) { self.0.push(name.clone()); }
    }
    let mut names = Names(Vec::new());
    crate::ast::visit::walk_module(&mut names, &program.module);
    assert!(names.0.iter().any(|name| name.name == "value" && name.context == SyntaxContext::allocated(90)));
    assert!(names.0.iter().any(|name| name.name == "main" && name.context == SyntaxContext::ROOT));
}

#[test]
fn macro_a_14_native_worker_rejects_unowned_construction_and_raw_contexts() {
    for case in 0..4 {
        let program = parse("%provider::make value\n");
        let invocation = super::tests::first_invocation(&program);
        let output = match case {
            0 => invocation.args.map_contexts(|_| Ok(SyntaxContext::allocated(12345))).unwrap(),
            1 => fragment(&super::tests::first_invocation(&parse("%provider::make\n")), "main = -> 1\n", ConstructionKind::CallSite),
            2 => fragment(&invocation, "unknown::helper\n", ConstructionKind::Dependency(0)),
            _ => super::token_tree::source("main = -> 1\n"),
        };
        let error = expand(program, output, vec![]).unwrap_err();
        let message = format!("{error:?}");
        assert!(message.contains("unauthorized") || message.contains("another invocation") || message.contains("undeclared"), "{message}");
    }
}

fn dependency(alias: &str) -> (CrateContext, ProductCrateIdentity, DefId) {
    let identity = ProductCrateIdentity::local("runtime".into());
    let id = DefId::new(CrateId(7), LocalDefId(1));
    let mut interface = ArtifactCrateInterface::default();
    interface.root_export_ids.insert("helper".into(), ArtifactExport { source: "runtime::helper".into(), id });
    let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
    let mut crates = CrateContext::new();
    crates.product_crate_ids.insert(identity.clone(), CrateId(7));
    crates.add_extern_crate(ExternCrateRecord::new(CrateId(7), alias.into(), metadata, ExternCrateBodies::default(), ExternCrateLink::metadata_only(BTreeMap::new()))).unwrap();
    (crates, identity, id)
}

#[test]
fn macro_a_14_native_worker_dependency_paths_require_declared_supplied_public_authority() {
    for alias in ["runtime", "renamed"] {
        for member in ["helper", "private"] {
            let (crates, identity, id) = dependency(alias);
            let program = parse("%provider::make\n");
            let invocation = super::tests::first_invocation(&program);
            let output = join(&[
                fragment(&invocation, "main = -> ", ConstructionKind::CallSite),
                fragment(&invocation, &format!("arbitrary_spelling::{member}!\n"), ConstructionKind::Dependency(0)),
            ]);
            let (_, environments) = expand(program, output, vec![identity]).unwrap();
            assert!(environments.resolve(&CrateContext::new()).is_err());
            let mut resolver = ResolverTables::default();
            resolver.definition_environments = environments.resolve(&crates).unwrap();
            let context = *resolver.definition_environments.keys().next().unwrap();
            assert!(resolver.member_context_matches(id, context, SyntaxContext::ROOT));
            assert!(!resolver.member_context_matches(DefId::new(CrateId(0), LocalDefId(9)), context, SyntaxContext::ROOT));
            assert!(!resolver.member_context_matches(id, SyntaxContext::allocated(9876), SyntaxContext::ROOT));
            let path = vec![("arbitrary_spelling".into(), context), (member.into(), context)];
            assert_eq!(resolver.resolve_binding_path(None, &path), (member == "helper").then_some(id));
            let conflict = vec![("arbitrary_spelling".into(), context), (member.into(), SyntaxContext::allocated(9876))];
            assert_eq!(resolver.resolve_binding_path(None, &conflict), None);
        }
    }
}

#[test]
fn macro_a_14_construction_fresh_handles_are_invocation_local() {
    let config = Config::default();
    let context = MacroExpansionContext::new(&config);
    let mut minted = Vec::new();
    for _ in 0..2 {
        let invocation = super::tests::first_invocation(&parse("%provider::make\n"));
        let request = ConstructionContext { invocation: invocation.id, kind: ConstructionKind::Fresh(1) };
        let first = context.construction_context(request, &invocation, &[]).unwrap();
        assert_eq!(context.construction_context(request, &invocation, &[]).unwrap(), first);
        minted.push(first);
    }
    assert!(minted.iter().all(|context| *context != SyntaxContext::ROOT));
    assert_ne!(minted[0], minted[1]);
}
