use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::crate_system::CrateContext;
use crate::products::*;

fn products() -> CompilerProducts {
    let mut target = b"RT01".to_vec();
    for bytes in [b"x86_64-unknown-linux-gnu".as_slice(), b"e-p:64:64".as_slice()] {
        target.extend_from_slice(&(bytes.len() as u64).to_le_bytes()); target.extend_from_slice(bytes);
    }
    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("dictionary_root".into()),
        identity_table: ProductIdentityTable { local_crate: Some(ProductCrateId(0)), ..Default::default() },
        interface: Default::default(), bodies: Default::default(),
        link: ProductLinkData { native_return_origins: Default::default(), object_path: Some("dictionary.o".into()), runtime_support: BTreeSet::from([ProductRuntimeSupport::selected_dictionaries(target)]), records: Default::default() },
        dependencies: vec![], source_fingerprint: Default::default(), infix_precedence: Default::default(), proc_macros: vec![],
    }
}

#[test]
fn source_free_dictionary_root_roundtrips_and_imports_without_reusable_bodies() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-dictionary-wire-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&root).unwrap(); std::fs::write(root.join("dictionary.o"), []).unwrap();
    let products = products(); let artifact = root.join("dictionary.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    assert_eq!(CompilerProducts::read_artifact_from_path(&artifact).unwrap().link.runtime_support, products.link.runtime_support);
    let mut context = CrateContext::new(); context.load_product_artifact_from_path(artifact).unwrap();
    assert_eq!(context.dependency_link_inputs().runtime_support, products.link.runtime_support);
}

#[test]
fn wrong_missing_and_extra_dictionary_runtime_roots_reject_export_and_load() {
    for control in 0..3 {
        let mut products = products(); let mut support = products.link.runtime_support.pop_first().unwrap();
        let ProductRuntimeSupport::SelectedDictionariesV1 { roots, .. } = &mut support else { unreachable!() };
        match control { 0 => roots.clear(), 1 => *roots = BTreeSet::from([RuntimeRootSymbol::NominalDescriptorFactoryV1]),
            _ => { roots.insert(RuntimeRootSymbol::TupleDescriptorFactoryV1); } }
        products.link.runtime_support.insert(support);
        assert!(products.to_artifact_bytes().is_err()); assert!(super::validate_object_link_records(&products).is_err());
    }
}

#[test]
fn metadata_only_and_malformed_dictionary_target_roots_reject() {
    let mut products = products(); products.link.object_path = None;
    assert!(products.to_artifact_bytes().is_err());
    products.link.object_path = Some("dictionary.o".into());
    let mut support = products.link.runtime_support.pop_first().unwrap();
    let ProductRuntimeSupport::SelectedDictionariesV1 { target_key, .. } = &mut support else { unreachable!() };
    target_key.push(0); products.link.runtime_support.insert(support);
    assert!(products.to_artifact_bytes().is_err()); assert!(super::validate_object_link_records(&products).is_err());
}
