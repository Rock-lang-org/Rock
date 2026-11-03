use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::ast::Module;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactCrateInterface;
use crate::crate_system::{
    CrateConfig, CrateContext, CrateManifest, CrateTarget, ExternCrateBodies, ExternCrateLink,
    ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{HirBlock, HirFunction, HirImpl, HirImplOwner};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lower::crates::registration::LowerCrateRegistration;
use crate::lower::Lowerer;
use crate::types::Type;

#[test]
fn source_backed_crate_does_not_register_module_path() {
    let mut crate_ctx = CrateContext::new();
    crate_ctx.register_crate(
        CrateManifest {
            crate_: CrateConfig {
                name: "dep".to_string(),
                version: "0.1.0".to_string(),
                no_std: false,
            },
            targets: vec![CrateTarget::Lib {
                path: "lib.rk".to_string(),
            }],
            dependencies: None,
        },
        PathBuf::from("/dep"),
        Module {
            name: None,
            top_levels: vec![],
            is_inline: false,
            filepath: None,
        },
    );

    let mut lowerer = Lowerer::new_for_test();

    LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

    assert!(lowerer.modules.source_module_paths().is_empty());
    assert_eq!(lowerer.errors().len(), 1);
    assert!(lowerer.errors()[0]
        .message
        .contains("source-backed external dependency 'dep' is not supported during lowering"));
}

#[test]
fn extern_artifact_crate_does_not_register_module_path() {
    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .add_extern_crate(ExternCrateRecord::new(
            CrateId(1),
            "dep".to_string(),
            ExternCrateMetadata::new(
                ArtifactCrateInterface::default(),
                ResolverTables::default(),
                BTreeMap::new(),
            ),
            ExternCrateBodies::default(),
            ExternCrateLink::metadata_only(BTreeMap::new()),
        ))
        .unwrap();
    let mut lowerer = Lowerer::new_for_test();

    LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

    assert!(lowerer.errors().is_empty());
    assert!(lowerer.modules.source_module_paths().is_empty());
}

#[test]
fn lower_registration_preserves_object_backed_impl_method_source_name() {
    let mut interface = ArtifactCrateInterface::default();
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            name: "show".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        },
    );
    interface.insert_impl(HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(1)),
        owner: HirImplOwner::Named("DepThing".to_string()),
        type_name: "DepThing".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods,
    });

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .add_extern_crate(ExternCrateRecord::new(
            CrateId(1),
            "dep".to_string(),
            ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new()),
            ExternCrateBodies::default(),
            ExternCrateLink::object(
                PathBuf::from("/dep/dep.o"),
                BTreeMap::from([(
                    DefId::new(CrateId(0), LocalDefId(2)),
                    "DepThing_show".to_string(),
                )]),
            ),
        ))
        .unwrap();

    let mut lowerer = Lowerer::new_for_test();
    LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

    let method = lowerer
        .items
        .impl_def(DefId::new(CrateId(0), LocalDefId(1)))
        .unwrap()
        .methods
        .get("show")
        .expect("object-backed method should remain attached to its impl");
    assert_eq!(method.name, "show");
}

#[test]
fn lower_registration_keeps_trait_impl_methods_attached_to_impl() {
    let mut interface = ArtifactCrateInterface::default();
    let mut methods = HashMap::new();
    methods.insert(
        "show".to_string(),
        HirFunction {
            id: DefId::new(CrateId(0), LocalDefId(3)),
            name: "show".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type: Type::I64,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::I64,
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        },
    );
    interface.insert_impl(HirImpl {
        id: DefId::new(CrateId(0), LocalDefId(2)),
        owner: HirImplOwner::Named("DepThing".to_string()),
        type_name: "DepThing".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: Vec::new().into(),
        trait_name: Some("Show".to_string()),
        trait_id: Some(DefId::new(CrateId(0), LocalDefId(1))),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: std::collections::HashMap::new().into(),
        methods,
    });

    let mut crate_ctx = CrateContext::new();
    crate_ctx
        .add_extern_crate(ExternCrateRecord::new(
            CrateId(1),
            "dep".to_string(),
            ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new()),
            ExternCrateBodies::default(),
            ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
        ))
        .unwrap();

    let mut lowerer = Lowerer::new_for_test();
    LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

    assert!(lowerer
        .items
        .impl_def(DefId::new(CrateId(0), LocalDefId(2)))
        .unwrap()
        .methods
        .contains_key("show"));
}
