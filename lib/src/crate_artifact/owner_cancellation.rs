//! Allocation/cancellation registry transport, without native construction bodies.
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::crate_system::CrateContext;
use crate::products::{
    CompilerProducts, ProductCrateId, ProductCrateIdentity, ProductDefId, ProductLinkData,
    ProductLocalDefId, ProductSourceFingerprint,
};
use crate::{Config, SourceProvider};

use super::language_items::{language_items_from_products, validate_product_language_items};
use super::load::ProductIdentityRemap;

fn products() -> CompilerProducts {
    let path = PathBuf::from("/virtual/cancellation_registry/provider.rk");
    let analysis = crate::analyze(&Config {
        entry_file: path.clone(),
        current_crate_name: Some("reservation_provider".into()),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: r#"
lang sized
< trait LayoutKnown
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?LayoutKnown
    lang owner_from_parts
    unsafe rebuild: *T -> State -> F T where T: ?LayoutKnown
lang object_owner_allocate
< trait Reservation State for F _
    lang owner_allocate
    unsafe reserve: I64 -> I64 -> (*U8, State)
    lang owner_cancel
    unsafe abandon: *U8 -> State -> ()
lang object_owner_unique
< trait Extraction State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
main = !-> ()
"#
            .into(),
        }],
        ..Config::default()
    })
    .unwrap();
    CompilerProducts::from_resolved_hir(
        ProductCrateIdentity::local("reservation_provider".into()),
        &analysis.hir,
        Vec::new(),
        BTreeMap::new(),
        ProductSourceFingerprint::default(),
        ProductLinkData::default(),
    )
    .unwrap()
}

#[test]
fn allocation_cancellation_registry_roundtrips_format59_and_remaps_every_member() {
    let products = products();
    assert_eq!(crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION, 59);
    assert_eq!(rock_shared::sysroot::PRODUCT_ARTIFACT_FORMAT_VERSION, 59);
    let bytes = products.to_artifact_bytes().unwrap();
    let decoded = CompilerProducts::from_artifact_bytes(&bytes).unwrap();
    let original = products
        .interface
        .language_items
        .object_owner_allocate
        .as_ref()
        .unwrap();
    assert_eq!(
        decoded
            .interface
            .language_items
            .object_owner_allocate
            .as_ref(),
        Some(original)
    );
    let mut context = CrateContext::new();
    let remap = ProductIdentityRemap::from_products(&mut context, &decoded).unwrap();
    let imported = language_items_from_products(&decoded, &remap).unwrap();
    let allocation = imported.object_owner_allocate.unwrap();
    assert_eq!(
        allocation.trait_id,
        remap.def_id(original.trait_id).unwrap()
    );
    assert_eq!(
        allocation.method_id,
        remap.def_id(original.method_id).unwrap()
    );
    assert_eq!(
        allocation.cancel_id,
        remap.def_id(original.cancel_id).unwrap()
    );
    assert_ne!(allocation.trait_id.crate_id.0, original.trait_id.crate_id.0);
    assert_ne!(
        allocation.cancel_id,
        imported.object_owner_unique.unwrap().method_id
    );
}

#[test]
fn allocation_cancellation_registry_rejects_forged_member_parent_or_state() {
    for mutation in 0..6 {
        let mut products = products();
        let original = products
            .interface
            .language_items
            .object_owner_allocate
            .clone()
            .unwrap();
        match mutation {
            0 => {
                products
                    .interface
                    .language_items
                    .object_owner_allocate
                    .as_mut()
                    .unwrap()
                    .cancel_id = original.method_id
            }
            1 => {
                products
                    .interface
                    .language_items
                    .object_owner_allocate
                    .as_mut()
                    .unwrap()
                    .cancel_id = products
                    .interface
                    .language_items
                    .object_owner_unique
                    .as_ref()
                    .unwrap()
                    .method_id
            }
            2 => {
                products
                    .interface
                    .language_items
                    .object_owner_allocate
                    .as_mut()
                    .unwrap()
                    .cancel_id = ProductDefId {
                    crate_id: ProductCrateId(99),
                    local_id: original.cancel_id.local_id,
                }
            }
            3 => {
                products
                    .interface
                    .language_items
                    .object_owner_allocate
                    .as_mut()
                    .unwrap()
                    .cancel_id = ProductDefId {
                    crate_id: original.cancel_id.crate_id,
                    local_id: ProductLocalDefId(u32::MAX),
                }
            }
            4 => {
                products
                    .interface
                    .traits
                    .get_mut(&original.trait_id)
                    .unwrap()
                    .signatures
                    .get_mut("abandon")
                    .unwrap()
                    .params[1] = crate::types::Type::I64
            }
            _ => {
                products
                    .interface
                    .traits
                    .get_mut(&original.trait_id)
                    .unwrap()
                    .signatures
                    .get_mut("abandon")
                    .unwrap()
                    .is_unsafe = false
            }
        }
        let error = validate_product_language_items(&products).unwrap_err();
        assert!(
            error.contains("owner_cancel") || error.contains("object_owner_allocate.cancel"),
            "case {mutation}: {error}"
        );
        let mut context = CrateContext::new();
        let remap = ProductIdentityRemap::from_products(&mut context, &products).unwrap();
        assert!(language_items_from_products(&products, &remap).is_err());
    }
}
