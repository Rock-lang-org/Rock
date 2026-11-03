use std::collections::BTreeMap;

use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::crate_system::{
    ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{HirBlock, HirFunction};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lower::Lowerer;
use crate::types::Type;

use super::*;

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: Default::default(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn session_services_seed_resolvers_and_prelude_before_body_lowering() {
    let export_id = DefId::new(CrateId(7), LocalDefId(1));
    let mut interface = ArtifactCrateInterface::default();
    let qualified_name = "std::prelude::answer".to_string();
    let function = test_function(export_id, "answer");
    interface.insert_function(qualified_name.clone(), function.clone());

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("std::prelude::answer".to_string(), export_id);
    resolver
        .item_names_by_id
        .insert(export_id, "std::prelude::answer".to_string());

    let prelude_exports = BTreeMap::from([(
        "answer".to_string(),
        ArtifactExport {
            source: qualified_name.clone(),
            id: export_id,
        },
    )]);
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(7),
        "std".to_string(),
        ExternCrateMetadata::new(interface, resolver, prelude_exports),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let mut lowerer = Lowerer::with_options(true);
    lowerer.items.insert_function(function);
    LoweringSessionServices::new(&ctx, Some("demo"), None).prepare_lowerer(&mut lowerer);

    assert!(lowerer.dependency_resolvers.contains_key("std"));
    assert_eq!(
        lowerer.resolver.import_aliases.get("answer"),
        Some(&export_id)
    );
    assert!(lowerer.scope.binding_is_alias("answer"));
}

#[test]
fn session_services_preserve_non_std_prelude_provider_without_injection() {
    let export_id = DefId::new(CrateId(9), LocalDefId(1));
    let mut interface = ArtifactCrateInterface::default();
    let qualified_name = "tools::prelude::answer".to_string();
    let function = test_function(export_id, "answer");
    interface.insert_function(qualified_name.clone(), function.clone());

    let mut resolver = crate::collect::resolver::ResolverTables::default();
    resolver
        .item_paths
        .insert("tools::prelude::answer".to_string(), export_id);
    resolver
        .item_names_by_id
        .insert(export_id, "tools::prelude::answer".to_string());

    let prelude_exports = BTreeMap::from([(
        "answer".to_string(),
        ArtifactExport {
            source: qualified_name.clone(),
            id: export_id,
        },
    )]);
    let mut ctx = CrateContext::new();
    ctx.add_extern_crate(ExternCrateRecord::new(
        CrateId(9),
        "tools".to_string(),
        ExternCrateMetadata::new(interface, resolver, prelude_exports),
        ExternCrateBodies::default(),
        ExternCrateLink::metadata_only(BTreeMap::new()),
    ))
    .unwrap();

    let mut lowerer = Lowerer::with_options(true);
    lowerer.items.insert_function(function);
    LoweringSessionServices::new(&ctx, Some("demo"), None).prepare_lowerer(&mut lowerer);

    assert!(lowerer.dependency_resolvers.contains_key("tools"));
    assert!(lowerer.resolver.import_aliases.get("answer").is_none());
    assert!(!lowerer.scope.binding_is_alias("answer"));
}
