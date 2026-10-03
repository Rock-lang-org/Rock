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

fn resolved(source: &str) -> ResolvedHirProgram {
    resolved_with_sources(source, Vec::new())
}

fn resolved_with_sources(
    source: &str,
    additional_sources: Vec<crate::SourceProvider>,
) -> ResolvedHirProgram {
    let path = PathBuf::from("/tmp/opencode/rock-package-alias-unit/contracts.rk");
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
        &PathBuf::from("/tmp/opencode/rock-package-alias-unit/contracts.rkca"),
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
            PathBuf::from("/tmp/opencode/rock-package-alias-unit/values.rk"),
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

#[test]
fn source_alias_canonical_indexes_survive_acceptance_type_ids_and_rebuild() {
    let nested_source = format!("{CONTRACTS}\nmod nested\n> nested::touch\n");
    for source in [CONTRACTS, nested_source.as_str()] {
        let additional_sources = if source == nested_source.as_str() {
            vec![crate::SourceProvider::Virtual {
                path: PathBuf::from("/tmp/opencode/rock-package-alias-unit/nested.rk"),
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
        &PathBuf::from("/tmp/opencode/rock-package-alias-unit/contracts.rkca"),
        &remap,
        &context,
    )
    .unwrap_err();
    assert_eq!(
        error,
        "Product artifact root export 'Pair' has no canonical DefId"
    );
}
