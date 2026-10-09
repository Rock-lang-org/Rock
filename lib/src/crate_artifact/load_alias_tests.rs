use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::crate_system::CrateContext;
use crate::infer::ResolvedHirProgram;
use crate::products::{
    CompilerProducts, ProductCrateIdentity, ProductDefId, ProductLinkData, ProductSourceFingerprint,
};
use crate::types::Type;

use super::{extern_crate_from_products, ProductIdentityRemap};

const CONTRACTS: &str = r#"
< trait Accept
    @accept: &Self -> I64
type Pair = exists T where T: Accept in (&T, &T)
< Pair
type Scalar = I64
< Scalar
type Identity T = T
< Identity
"#;

fn alias_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures/rock-package-alias-unit").join(file)
}

fn resolved(source: &str) -> ResolvedHirProgram {
    resolved_with_sources(source, Vec::new())
}

fn resolved_with_sources(
    source: &str,
    additional_sources: Vec<crate::SourceProvider>,
) -> ResolvedHirProgram {
    let path = alias_path("contracts.rk");
    let mut source_providers = vec![crate::SourceProvider::Virtual {
        path: path.clone(),
        text: source.into(),
    }];
    source_providers.extend(additional_sources);
    crate::analyze(&crate::Config {
        entry_file: path.clone(),
        current_crate_name: Some("contracts".into()),
        no_std: true,
        no_prelude: true,
        source_providers,
        ..Default::default()
    })
    .unwrap()
    .hir
}

fn products(hir: &ResolvedHirProgram) -> CompilerProducts {
    CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("contracts".into()),
        hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap()
}

fn load(products: &CompilerProducts) -> (CrateContext, ProductIdentityRemap) {
    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let mut context = CrateContext::new();
    // Force consumer identities to differ; no producer sources or native
    // objects are available to this metadata-only, in-memory loader path.
    context
        .consumer_crate_id_for_product_identity(&ProductCrateIdentity::local("unrelated".into()));
    let remap = ProductIdentityRemap::from_products(&mut context, &decoded).unwrap();
    let record = extern_crate_from_products(
        &decoded,
        &alias_path("contracts.rkca"),
        &remap,
        &context,
    )
    .unwrap();
    context.add_extern_crate(record).unwrap();
    assert_eq!(products.to_artifact_bytes().unwrap(), bytes);
    (context, remap)
}

fn consumer(source: &str, context: &CrateContext) -> ResolvedHirProgram {
    let config = crate::Config {
        no_std: true,
        no_prelude: true,
        ..Default::default()
    };
    let ast = crate::ast::Program {
        module: crate::parser::parse_source(
            alias_path("values.rk"),
            source,
            &config,
        )
        .unwrap(),
    };
    let declarations = crate::collect::collect(&ast, context, false, Some("values")).unwrap();
    let partial =
        crate::lower::program::lower_from_declarations(&ast, declarations, context, Some("values"))
            .unwrap();
    crate::infer::finalize(partial).unwrap()
}

fn visible_field_program(source: &str) -> crate::ast::Program {
    crate::ast::Program {
        module: crate::parser::parse_source(
            alias_path("values.rk"),
            source,
            &crate::Config { no_std: true, no_prelude: true, ..Default::default() },
        ).unwrap(),
    }
}

fn visible_field_lowerer(program: &crate::ast::Program, context: &CrateContext) -> crate::lower::Lowerer {
    let declarations = crate::collect::collect(program, context, false, Some("values")).unwrap();
    let mut lowerer = crate::lower::Lowerer::from_declarations(declarations).unwrap();
    crate::lower::session::LoweringSessionServices::new(
        context, Some("values"), program.module.filepath.as_deref(),
    ).prepare_lowerer(&mut lowerer);
    lowerer
}

fn visible_field_partial(source: &str, context: &CrateContext) -> crate::infer::PartialHir {
    let program = visible_field_program(source);
    let declarations = crate::collect::collect(&program, context, false, Some("values")).unwrap();
    crate::lower::program::lower_from_declarations(&program, declarations, context, Some("values")).unwrap()
}

fn visible_field_projection<P: crate::hir::HirPhase>(
    body: &crate::hir::HirBlockFor<P>,
) -> &crate::hir::HirExprFor<P> {
    fn find<P: crate::hir::HirPhase>(
        body: &crate::hir::HirBlockFor<P>, depth: usize,
    ) -> Option<&crate::hir::HirExprFor<P>> {
        assert!(depth <= 4 && body.stmts.len() <= 3, "bounded field control body");
        for statement in &body.stmts {
            let crate::hir::HirStmtFor::Expr(expression) = statement else { continue; };
            match &expression.kind {
                crate::hir::HirExprKindFor::FieldAccess(..) => return Some(expression),
                crate::hir::HirExprKindFor::Assign(place, _) => {
                    if matches!(place.kind, crate::hir::HirExprKindFor::FieldAccess(..)) { return Some(place); }
                }
                crate::hir::HirExprKindFor::Block(inner) | crate::hir::HirExprKindFor::UnsafeBlock(inner) => {
                    if let Some(field) = find(inner, depth + 1) { return Some(field); }
                }
                _ => {}
            }
        }
        None
    }
    find(body, 0).expect("actual control field projection")
}

fn assert_visible_field_deferred(partial: &crate::infer::PartialHir, name: &str) {
    use crate::hir::HirExprKind;
    use crate::infer::constraints::Constraint;

    let function = partial.functions.values().find(|function| function.name == name).unwrap();
    let field = visible_field_projection(&function.body);
    let HirExprKind::FieldAccess(receiver, _, location) = &field.kind else { unreachable!() };
    let HirExprKind::Deref(input) = &receiver.kind else { panic!("actual deferred raw dereference"); };
    eprintln!("VISIBLE_FIELD_PREACCEPT function={:?} location={location:?} rawInput={:?} inputRepresentative={:?} rawOutput={:?} outputRepresentative={:?}",
        function.id, input.ty, partial.engine.resolve(&input.ty), receiver.ty, partial.engine.resolve(&receiver.ty));
    let relations = partial.constraint_store.iter().filter_map(|(_, constraint)| match constraint {
        Constraint::Dereference { input, output, span, unsafe_allowed } if *span == receiver.span => Some((input, output, unsafe_allowed)),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(relations.len(), 1);
    assert_ne!(relations[0].0, relations[0].1, "input and result have independent slots");
    assert!(*relations[0].2);
    assert!(location.is_none(), "visible foreign collision must not install a local field owner");
    assert!(matches!(partial.engine.resolve(&receiver.ty), Type::TypeVar(_)), "no guessed nominal dereference result");
}

fn assert_visible_field_accepted(hir: &ResolvedHirProgram, name: &str, owner: crate::ids::DefId) {
    use crate::hir::{HirExprKindFor, HirVarTarget};

    let function = hir.program.functions.values().find(|function| function.name == name).unwrap();
    let nominal = Type::Struct { id: owner, args: Vec::new() };
    assert_eq!(function.params[0].ty, Type::Pointer(Box::new(nominal.clone())));
    let field = visible_field_projection(&function.body);
    assert_eq!(field.ty, Type::Pointer(Box::new(Type::U8)));
    let HirExprKindFor::FieldAccess(receiver, _, Some(location)) = &field.kind else { panic!("canonical accepted field target"); };
    assert_eq!(location.owner, owner);
    let definition = &hir.program.structs[&owner];
    assert_eq!(location.field_id, definition.fields.iter().find(|field| field.name == "base").unwrap().id);
    assert_eq!(receiver.ty, nominal);
    let HirExprKindFor::Deref(input) = &receiver.kind else { panic!("actual pointer dereference"); };
    assert_eq!(input.ty, function.params[0].ty);
    assert!(matches!(&input.kind, HirExprKindFor::ResolvedVar(reference)
        if reference.target == HirVarTarget::Local(function.params[0].local_id)));
}

#[test]
fn sourcefree_visible_field_explicit_import_defers_duplicate_until_pointer_evidence() {
    let product = products(&resolved("< struct Audit\n    < base: *U8\n"));
    let original = product.to_artifact_bytes().unwrap();
    let (context, remap) = load(&product);
    let owner = remap.def_id(product.identity_table.export_names["Audit"]).unwrap();
    let source = "> contracts::Audit\nstruct Resource\n    < base: *U8\nwrite = audit !-> unsafe\n    (*audit).base = (0 as *U8)\nmain = !->\n    mut audit = Audit\n        base: 0 as *U8\n    write ((&mut audit) as *Audit)\n";
    let lowerer = visible_field_lowerer(&visible_field_program(source), &context);
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.visible_imported_struct_ids(), std::collections::BTreeSet::from([owner]));
    assert_eq!(resolution.resolve_owner_id("Audit"), Some(owner));
    let partial = visible_field_partial(source, &context);
    // Intentionally RED until the later secondary.rs ambiguity veto is installed.
    assert_visible_field_deferred(&partial, "write");
    let accepted = crate::infer::finalize(partial).unwrap();
    assert_visible_field_accepted(&accepted, "write", owner);
    assert_eq!(product.to_artifact_bytes().unwrap(), original);
}

#[test]
fn sourcefree_visible_field_private_import_never_grants_access() {
    let product = products(&resolved("< struct Audit\n    base: *U8\n< pointer = -> 0 as *Audit\n"));
    let (context, remap) = load(&product);
    let owner = remap.def_id(product.identity_table.export_names["Audit"]).unwrap();
    let interface = context.extern_crate("contracts").unwrap().metadata().interface();
    let definition = &interface.structs[&owner];
    assert!(!definition.fields.iter().find(|field| field.name == "base").unwrap().public);
    let source = "> contracts::Audit\n> contracts::pointer\nread = audit -> unsafe (*audit).base\nmain = !-> read pointer!\n";
    let lowerer = visible_field_lowerer(&visible_field_program(source), &context);
    assert!(crate::lower::resolution::LowerResolutionContext::new(&lowerer).visible_imported_struct_ids().contains(&owner),
        "owner visibility is not field permission");
    let partial = visible_field_partial(source, &context);
    assert_visible_field_deferred(&partial, "read");
    let function = partial.functions.values().find(|function| function.name == "read").unwrap();
    let expected_span = visible_field_projection(&function.body).span.clone();
    let errors = crate::infer::finalize(partial).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, crate::diagnostic::DiagnosticCode::Resolve);
    assert_eq!(errors[0].message, format!("field 'base' of struct '{}' is private", definition.name));
    assert_eq!(errors[0].span(), Some(expected_span));

    // A known pointer retains the existing lowering privacy diagnostic; no
    // visibility helper is allowed to turn this into an authorized location.
    let known = "> contracts::Audit\nread: *Audit -> *U8\nread = audit -> unsafe (*audit).base\nmain = !-> ()\n";
    let program = visible_field_program(known);
    let declarations = crate::collect::collect(&program, &context, false, Some("values")).unwrap();
    let errors = match crate::lower::program::lower_from_declarations(&program, declarations, &context, Some("values")) {
        Err(errors) => errors,
        Ok(_) => panic!("known private field must remain rejected"),
    };
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, crate::diagnostic::DiagnosticCode::Resolve);
    assert_eq!(errors[0].message, "Field 'base' of struct 'Audit' is private");
    let start = known.find("(*audit).base").unwrap() + "(*audit).".len();
    let span = errors[0].span().unwrap();
    assert_eq!(span.file_path, program.module.filepath.clone().unwrap());
    assert_eq!((span.start, span.end), (start, start + "base".len()));
    assert_eq!(&known[span.start..span.end], "base");
}

#[test]
fn sourcefree_visible_field_unexported_metadata_does_not_veto_local_uniqueness() {
    let producer = resolved("struct Support\n    < base: *U8\n< support = -> Support\n    base: 0 as *U8\n");
    let source_support = producer.resolver.resolve_item_or_alias("Support").unwrap();
    let mut product = products(&producer);
    let support = product.identity_table.export_names["Support"];
    assert_eq!(support, ProductDefId::from(source_support));
    let schema = bincode::serialize(&product.interface.structs[&support]).unwrap();
    let display_names = product.identity_table.display_names.clone();
    // Controlled export-table fixture, NOT proof of source privacy syntax:
    // record_name currently exports even bare nominal declarations. Keep the
    // source-analyzed schema/ID and remove only this owner's public export rows.
    let removed = product.identity_table.export_names.iter()
        .filter(|(_, id)| **id == support).map(|(name, _)| name.clone()).collect::<Vec<_>>();
    assert_eq!(removed, vec!["Support".to_string()]);
    product.identity_table.export_names.retain(|_, id| *id != support);
    assert!(!product.identity_table.export_names.contains_key("Support"));
    assert_eq!(product.identity_table.display_names, display_names);
    assert_eq!(bincode::serialize(&product.interface.structs[&support]).unwrap(), schema);
    let fixture_bytes = product.to_artifact_bytes().unwrap();
    let (context, remap) = load(&product);
    let support = remap.def_id(support).unwrap();
    let dependency = context.extern_crate("contracts").unwrap();
    assert!(dependency.root_exports().values().all(|export| export.id != support));
    assert!(dependency.metadata().resolver().export_aliases.values().all(|id| *id != support));
    assert_eq!(dependency.metadata().interface().structs[&support].id, support);
    let source = "struct Resource\n    < base: *U8\nread = resource -> resource.base\nwrite = mut resource ->\n    resource.base = (0 as *U8)\n    resource\nmain = !-> ()\n";
    let lowerer = visible_field_lowerer(&visible_field_program(source), &context);
    assert!(lowerer.items.structure(support).unwrap().fields[0].public,
        "actual supporting schema is registered despite not being exported");
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.resolve_owner_id("contracts::Support"), Some(support));
    assert!(!resolution.visible_imported_struct_ids().contains(&support));
    let accepted = consumer(source, &context);
    let resource = accepted.resolver.resolve_item_or_alias("Resource").unwrap();
    for name in ["read", "write"] {
        let function = accepted.program.functions.values().find(|function| function.name == name).unwrap();
        assert_eq!(function.params[0].ty, Type::Struct { id: resource, args: Vec::new() });
        let field = visible_field_projection(&function.body);
        let crate::hir::HirExprKindFor::FieldAccess(_, _, Some(location)) = &field.kind else { panic!("unique local field location"); };
        assert_eq!(location.owner, resource);
        assert_eq!(location.field_id, accepted.program.structs[&resource].fields[0].id);
        assert_eq!(field.ty, Type::Pointer(Box::new(Type::U8)));
    }
    assert_eq!(product.to_artifact_bytes().unwrap(), fixture_bytes);
}

#[test]
fn sourcefree_visible_field_alias_ids_deduplicate_and_module_aliases_stay_scoped() {
    let product = products(&resolved("< struct Audit\n    < base: *U8\n"));
    let (context, remap) = load(&product);
    let owner = remap.def_id(product.identity_table.export_names["Audit"]).unwrap();
    let root = visible_field_program("> contracts::Audit\nmain = !-> ()\n");
    let lowerer = visible_field_lowerer(&root, &context);
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.resolve_owner_id("Audit"), Some(owner));
    assert_eq!(resolution.resolve_owner_id("contracts::Audit"), Some(owner));
    assert_eq!(resolution.visible_imported_struct_ids(), std::collections::BTreeSet::from([owner]));

    let mut program = visible_field_program("main = !-> ()\n");
    let mut inner = visible_field_program("> contracts::Audit\n").module;
    inner.name = Some(crate::ast::Ident { name: "inner".into(), span: crate::lexer::Span::test() });
    inner.is_inline = true;
    program.module.top_levels.push(crate::ast::TopLevel::Module(crate::ast::ModuleDecl(inner)));
    let mut lowerer = visible_field_lowerer(&program, &context);
    assert!(!lowerer.resolver.import_aliases.contains_key("Audit"));
    assert_eq!(crate::lower::resolution::LowerResolutionContext::new(&lowerer).resolve_owner_id("Audit"), None);
    lowerer.modules.set_current_qualified_module_prefix(Some("inner".into()));
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.resolve_owner_id("Audit"), Some(owner));
    assert_eq!(resolution.visible_imported_struct_ids(), std::collections::BTreeSet::from([owner]));
    lowerer.modules.set_current_qualified_module_prefix(Some("other".into()));
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.resolve_owner_id("Audit"), None);
    // The public qualified path remains accessible independently of the local
    // alias; isolating the alias must not erase valid export-path authority.
    assert_eq!(resolution.resolve_owner_id("contracts::Audit"), Some(owner));
    assert_eq!(resolution.visible_imported_struct_ids(), std::collections::BTreeSet::from([owner]));
}

#[test]
fn sourcefree_visible_field_qualified_export_defers_duplicate_without_short_import() {
    let product = products(&resolved("< struct Audit\n    < base: *U8\n< pointer = -> 0 as *Audit\n"));
    let (context, remap) = load(&product);
    let owner = remap.def_id(product.identity_table.export_names["Audit"]).unwrap();
    let source = "struct Resource\n    < base: *U8\nwrite = audit !-> unsafe\n    (*audit).base = (0 as *U8)\nmain = !-> write (contracts::pointer!)\n";
    let lowerer = visible_field_lowerer(&visible_field_program(source), &context);
    assert!(!lowerer.resolver.import_aliases.contains_key("Audit"));
    assert!(!lowerer.resolver.import_aliases.contains_key("pointer"));
    let resolution = crate::lower::resolution::LowerResolutionContext::new(&lowerer);
    assert_eq!(resolution.resolve_owner_id("contracts::Audit"), Some(owner));
    assert!(resolution.visible_imported_struct_ids().contains(&owner));
    let partial = visible_field_partial(source, &context);
    assert_visible_field_deferred(&partial, "write");
    assert_visible_field_accepted(&crate::infer::finalize(partial).unwrap(), "write", owner);
}

#[test]
fn sourcefree_visible_field_sole_imported_owner_comes_from_actual_caller() {
    let product = products(&resolved("< struct Audit\n    < base: *U8\n< pointer = -> 0 as *Audit\n"));
    let (context, remap) = load(&product);
    let owner = remap.def_id(product.identity_table.export_names["Audit"]).unwrap();
    let source = "> contracts::Audit\n> contracts::pointer\nread = audit -> unsafe (*audit).base\nmain = !-> read pointer!\n";
    let lowerer = visible_field_lowerer(&visible_field_program(source), &context);
    assert_eq!(crate::lower::resolution::LowerResolutionContext::new(&lowerer).visible_imported_struct_ids(),
        std::collections::BTreeSet::from([owner]));
    let partial = visible_field_partial(source, &context);
    assert_visible_field_deferred(&partial, "read");
    assert_visible_field_accepted(&crate::infer::finalize(partial).unwrap(), "read", owner);
}

#[test]
fn source_alias_canonical_indexes_survive_acceptance_type_ids_and_rebuild() {
    let nested_source = format!("{CONTRACTS}\nmod nested\n> nested::touch\n");
    for source in [CONTRACTS, nested_source.as_str()] {
        let additional_sources = if source == nested_source.as_str() {
            vec![crate::SourceProvider::Virtual {
                path: alias_path("nested.rk"),
                text: "type Pair = exists Hidden where Hidden: Accept in (&Hidden, &Hidden)\n< Pair\n< touch = !-> ()\n".into(),
            }]
        } else {
            Vec::new()
        };
        let hir = resolved_with_sources(source, additional_sources);
        let canonical_names = &hir.resolver.item_names_by_id;
        assert_eq!(
            hir.program.indexes.type_aliases_by_id.len(),
            hir.program.type_aliases.len()
        );
        for (id, alias) in &hir.program.type_aliases {
            let canonical_name = canonical_names.get(id).unwrap();
            assert_eq!(
                hir.program.indexes.type_aliases_by_id.get(id),
                Some(canonical_name)
            );
            assert_eq!(
                hir.program.names.type_aliases_by_name.get(canonical_name),
                Some(id)
            );
            let location = crate::hir::HirTypeLocation::TypeAliasBody { alias: *id };
            assert_eq!(hir.type_at(hir.type_id_at(&location).unwrap()), alias.ty);
        }
        if source == nested_source.as_str() {
            let nested_id = hir
                .resolver
                .resolve_item_or_alias("contracts::nested::Pair")
                .unwrap();
            let root_id = hir.resolver.resolve_item_or_alias("Pair").unwrap();
            assert_ne!(root_id, nested_id);
            assert_eq!(hir.program.type_aliases[&nested_id].name, "Pair");
            assert_eq!(
                hir.program.indexes.type_aliases_by_id[&nested_id],
                "contracts::nested::Pair"
            );
        }

        let expected = hir.program.indexes.type_aliases_by_id.clone();
        let mut program = hir.program.program().clone();
        program.rebuild_indexes_with_canonical_names(canonical_names);
        assert_eq!(program.indexes.type_aliases_by_id, expected);
        let accepted = crate::hir::AcceptedHirProgram::revalidate_for_test(program).unwrap();
        assert_eq!(accepted.indexes.type_aliases_by_id, expected);
        let mut context = crate::type_context::TypeContext::new();
        let type_ids = crate::hir::collect_hir_type_ids(accepted.program(), &mut context);
        assert_eq!(accepted.indexes.type_aliases_by_id, expected);
        for (id, alias) in &accepted.type_aliases {
            let location = crate::hir::HirTypeLocation::TypeAliasBody { alias: *id };
            assert_eq!(context.type_for(type_ids.get(&location).unwrap()), alias.ty);
        }
    }
}

#[test]
fn package_alias_root_import_preserves_canonical_ids_and_closed_type_without_sources() {
    let hir = resolved(CONTRACTS);
    let products = products(&hir);
    let original_bytes = products.to_artifact_bytes().unwrap();
    let pair_id = products.identity_table.export_names["Pair"];
    let accept_id = products.identity_table.export_names["Accept"];
    let Type::Exists(original) = &products.interface.type_aliases[&pair_id].ty else {
        panic!("expected producer package alias");
    };
    let (context, remap) = load(&products);
    let dependency = context.extern_crate("contracts").unwrap();
    let pair_export = &dependency.root_exports()["Pair"];
    let imported_id = remap.def_id(pair_id).unwrap();
    assert_ne!(imported_id.crate_id.0, pair_id.crate_id.0);
    assert_eq!(pair_export.id, imported_id);
    assert_eq!(pair_export.source, "contracts::Pair");
    let interface = dependency.metadata().interface();
    assert_eq!(
        interface.canonical_name(imported_id),
        Some("contracts::Pair")
    );
    let alias = &interface.type_aliases[&imported_id];
    let Type::Exists(imported) = &alias.ty else {
        panic!("expected imported package alias");
    };
    assert_eq!(imported.binder, original.binder);
    assert_eq!(imported.payload, original.payload);
    assert_eq!(
        imported.bounds.principal.trait_id,
        remap.def_id(accept_id).unwrap()
    );
    assert_eq!(
        dependency
            .metadata()
            .resolver()
            .resolve_item_or_alias("contracts::Pair"),
        Some(imported_id),
    );

    for imports in ["> contracts::Accept\n> contracts::Pair", "> contracts::*"] {
        let source = format!(
            r#"
{imports}
relay: Pair -> Pair
relay = package ->
    open package as Hidden, pair
        pack (Pair) (pair)
main = !-> ()
"#
        );
        let downstream = consumer(&source, &context);
        assert_eq!(
            downstream.resolver.resolve_item_or_alias("Pair"),
            Some(imported_id)
        );
        assert_eq!(
            downstream.resolver.canonical_name(imported_id),
            Some("contracts::Pair")
        );
        let relay = downstream
            .program
            .functions
            .values()
            .find(|function| function.name == "relay")
            .unwrap();
        assert_eq!(relay.params[0].ty, alias.ty);
        assert_eq!(relay.ret_type, alias.ty);
    }
    assert_eq!(products.to_artifact_bytes().unwrap(), original_bytes);
}

#[test]
fn ordinary_and_generic_alias_root_imports_reuse_the_same_canonical_authority() {
    let products = products(&resolved(CONTRACTS));
    let identity_id = products.identity_table.export_names["Identity"];
    let (context, remap) = load(&products);
    let dependency = context.extern_crate("contracts").unwrap();
    let imported_id = remap.def_id(identity_id).unwrap();
    let alias = &dependency.metadata().interface().type_aliases[&imported_id];
    assert_eq!(alias.generic_params[0].id.owner, imported_id);
    let downstream = consumer(
        r#"
> contracts::Scalar
> contracts::Identity
identity: Identity Scalar -> Scalar
identity = number -> number
main = !-> ()
"#,
        &context,
    );
    assert_eq!(
        downstream.resolver.resolve_item_or_alias("Identity"),
        Some(imported_id)
    );
    let identity = downstream
        .program
        .functions
        .values()
        .find(|function| function.name == "identity")
        .unwrap();
    assert_eq!(identity.params[0].ty, Type::I64);
    assert_eq!(identity.ret_type, Type::I64);
}

#[test]
fn alias_products_publish_canonical_paths_not_short_display_names() {
    let mut hir = resolved(CONTRACTS);
    let alias_id = hir.resolver.resolve_item_or_alias("Pair").unwrap();
    let canonical_name = "nested::Pair";
    // The accepted declaration keeps its short display name; the established
    // canonical index is the exporter authority, as for other declarations.
    hir.program
        .program_mut_for_test()
        .indexes
        .type_aliases_by_id
        .insert(alias_id, canonical_name.into());
    hir.resolver.insert_export_alias_with_name(
        "PublicPair".into(),
        canonical_name.into(),
        alias_id,
    );
    let products = products(&hir);
    let product_id = ProductDefId::from(alias_id);
    assert_eq!(
        products.identity_table.display_names[&product_id],
        canonical_name
    );
    assert_eq!(
        products.identity_table.export_names["PublicPair"],
        product_id
    );
    let (context, remap) = load(&products);
    let imported_id = remap.def_id(product_id).unwrap();
    let dependency = context.extern_crate("contracts").unwrap();
    assert_eq!(
        dependency.root_exports()["PublicPair"].source,
        "contracts::nested::Pair"
    );
    assert_eq!(
        dependency
            .metadata()
            .interface()
            .canonical_name(imported_id),
        Some("contracts::nested::Pair")
    );
    let downstream = consumer(
        r#"
> contracts::PublicPair
relay: PublicPair -> PublicPair
relay = package ->
    open package as Hidden, pair
        pack (PublicPair) (pair)
main = !-> ()
"#,
        &context,
    );
    assert_eq!(
        downstream.resolver.resolve_item_or_alias("PublicPair"),
        Some(imported_id)
    );
    assert_eq!(
        downstream.resolver.canonical_name(imported_id),
        Some("contracts::nested::Pair")
    );
}

#[test]
fn alias_exporter_rejects_a_missing_canonical_index_without_name_recovery() {
    let mut hir = resolved(CONTRACTS);
    let alias_id = hir.resolver.resolve_item_or_alias("Pair").unwrap();
    hir.program
        .program_mut_for_test()
        .indexes
        .type_aliases_by_id
        .remove(&alias_id);
    let error = CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("contracts".into()),
        &hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        format!("type alias {alias_id:?} has no canonical declaration name")
    );
}

#[test]
fn package_alias_root_export_rejects_a_missing_canonical_declaration_row() {
    let mut products = products(&resolved(CONTRACTS));
    let alias_id = products.identity_table.export_names["Pair"];
    products.interface.type_aliases.remove(&alias_id);
    let mut context = CrateContext::new();
    let remap = ProductIdentityRemap::from_products(&mut context, &products).unwrap();
    let error = extern_crate_from_products(
        &products,
        &alias_path("contracts.rkca"),
        &remap,
        &context,
    )
    .unwrap_err();
    assert_eq!(
        error,
        "Product artifact root export 'Pair' has no canonical DefId"
    );
}
