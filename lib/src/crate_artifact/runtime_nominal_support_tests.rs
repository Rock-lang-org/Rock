use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::crate_system::CrateContext;
use crate::products::*;

fn support() -> ProductRuntimeSupport {
    let mut bytes = b"RT01".to_vec();
    for component in [b"x86_64-unknown-linux-gnu".as_slice(), b"e-p:64:64".as_slice()] {
        bytes.extend_from_slice(&(component.len() as u64).to_le_bytes()); bytes.extend_from_slice(component);
    }
    ProductRuntimeSupport::nominal_descriptors(bytes)
}
fn products() -> CompilerProducts {
    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("nominal_root".into()),
        identity_table: ProductIdentityTable { local_crate: Some(ProductCrateId(0)), ..Default::default() },
        interface: Default::default(), bodies: Default::default(),
        link: ProductLinkData { native_return_origins: Default::default(), object_path: Some("nominal.o".into()), runtime_support: BTreeSet::from([support()]), records: Default::default() },
        dependencies: vec![], source_fingerprint: Default::default(), infix_precedence: Default::default(), proc_macros: vec![],
    }
}
#[test]
fn source_free_nominal_roots_roundtrip_and_import_without_any_reusable_body() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-nominal-wire-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&root).unwrap(); std::fs::write(root.join("nominal.o"), []).unwrap();
    let products = products(); let path = root.join("nominal.rkca");
    products.write_artifact_to_path(&path).unwrap();
    assert_eq!(CompilerProducts::read_artifact_from_path(&path).unwrap().link.runtime_support, products.link.runtime_support);
    let mut context = CrateContext::new(); context.load_product_artifact_from_path(path).unwrap();
    assert_eq!(context.dependency_link_inputs().runtime_support, products.link.runtime_support);
}
#[test]
fn missing_wrong_kind_and_extra_nominal_root_sets_reject() {
    for control in 0..3 {
        let mut products = products(); let mut requirement = support();
        let ProductRuntimeSupport::NominalDescriptorsV1 { roots, .. } = &mut requirement else { panic!("nominal support"); };
        match control {
            0 => roots.clear(),
            1 => *roots = BTreeSet::from([RuntimeRootSymbol::TupleDescriptorFactoryV1]),
            _ => { roots.insert(RuntimeRootSymbol::TupleDescriptorFactoryV1); },
        }
        products.link.runtime_support = BTreeSet::from([requirement]);
        assert!(products.to_artifact_bytes().is_err()); assert!(super::validate_object_link_records(&products).is_err());
    }
}
#[test]
fn nominal_metadata_only_or_malformed_target_claims_reject() {
    let mut products = products(); products.link.object_path = None;
    assert!(products.to_artifact_bytes().is_err());
    products.link.object_path = Some("nominal.o".into());
    let mut requirement = support();
    let ProductRuntimeSupport::NominalDescriptorsV1 { target_key, .. } = &mut requirement else { panic!("nominal support"); };
    target_key.push(0); products.link.runtime_support = BTreeSet::from([requirement]);
    assert!(products.to_artifact_bytes().is_err());
}
