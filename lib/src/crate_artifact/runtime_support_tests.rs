//! Link requirement protocol tests; synthetic object is never executed.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::crate_system::CrateContext;
use crate::products::{CompilerProducts, ProductCrateId, ProductCrateIdentity, ProductIdentityTable, ProductLinkData, ProductRuntimeSupport};

fn support() -> ProductRuntimeSupport {
    let mut key = b"RT01".to_vec();
    for bytes in [b"x86_64-unknown-linux-gnu".as_slice(), b"e-p:64:64".as_slice()] {
        key.extend_from_slice(&(bytes.len() as u64).to_le_bytes()); key.extend_from_slice(bytes);
    }
    ProductRuntimeSupport::tuple_descriptors(key)
}
fn products() -> CompilerProducts {
    CompilerProducts {
        crate_identity: ProductCrateIdentity::local("runtime_provider".into()),
        identity_table: ProductIdentityTable { local_crate: Some(ProductCrateId(0)), ..Default::default() },
        interface: Default::default(), bodies: Default::default(),
        link: ProductLinkData { native_return_origins: BTreeMap::new(), object_path: Some("provider.o".into()), runtime_support: BTreeSet::from([support()]), records: BTreeMap::new() },
        dependencies: vec![], source_fingerprint: Default::default(), infix_precedence: Default::default(), proc_macros: vec![],
    }
}

#[test]
fn persisted_support_survives_source_free_loading_without_any_body_or_callable() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join("opencode").join(format!("rock_support_wire_{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("provider.o"), []).unwrap();
    let products = products();
    let artifact = root.join("provider.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(decoded.link.runtime_support, products.link.runtime_support);
    let mut context = CrateContext::new();
    context.load_product_artifact_from_path(artifact).unwrap();
    let inputs = context.dependency_link_inputs();
    assert_eq!(inputs.runtime_support, products.link.runtime_support);
    assert_eq!(inputs.object_paths, vec![root.join("provider.o")]);
}

#[test]
fn malformed_root_or_target_support_is_rejected_by_export_and_loader() {
    for control in 0..3 {
        let mut products = products();
        let mut requirement = support();
        let ProductRuntimeSupport::TupleDescriptorsV1 { target_key, roots } = &mut requirement else { panic!("unexpected tuple runtime root kind"); };
        match control {
            0 => roots.clear(),
            1 => { target_key.push(0); },
            _ => { target_key[3] = b'9'; },
        }
        products.link.runtime_support = BTreeSet::from([requirement]);
        assert!(products.to_artifact_bytes().is_err());
        assert!(super::validate_object_link_records(&products).is_err());
    }
}

#[test]
fn metadata_only_products_cannot_claim_native_runtime_code_roots() {
    let mut products = products();
    products.link.object_path = None;
    assert!(products.to_artifact_bytes().is_err());
    assert!(super::validate_object_link_records(&products).is_err());
}
