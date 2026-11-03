use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};

use super::{
    merge_language_item_providers, merge_language_item_providers_all, DropLanguageItems,
    FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems, IndexLanguageItems,
    IndexMutLanguageItems, LanguageItemProviderConflict, LanguageItems, SendLanguageItems,
    SizedLanguageItems, SyncLanguageItems,
};

fn def_id(crate_id: u32, local_id: u32) -> DefId {
    DefId::new(CrateId(crate_id), LocalDefId(local_id))
}

#[test]
fn merge_language_items_accepts_disjoint_providers() {
    let left = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: def_id(1, 1),
        }),
        ..LanguageItems::default()
    };
    let right = LanguageItems {
        drop: Some(DropLanguageItems {
            trait_id: def_id(2, 1),
            method_id: def_id(2, 2),
        }),
        ..LanguageItems::default()
    };

    let merged = merge_language_item_providers([("left", &left), ("right", &right)])
        .expect("disjoint providers should merge");

    assert_eq!(merged.sized, left.sized);
    assert_eq!(merged.drop, right.drop);
    assert_eq!(merged.index, None);
    assert_eq!(merged.try_protocol, None);
}

#[test]
fn merge_language_items_rejects_duplicate_provider_deterministically() {
    let first = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: def_id(1, 1),
        }),
        ..LanguageItems::default()
    };
    let second = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: def_id(2, 1),
        }),
        ..LanguageItems::default()
    };

    let error = merge_language_item_providers([("zeta", &second), ("alpha", &first)])
        .expect_err("duplicate sized providers should be rejected");

    let LanguageItemProviderConflict::Multiple {
        protocol,
        providers,
    } = error
    else {
        panic!("expected a multiple-provider conflict");
    };
    assert_eq!(protocol, "sized");
    assert_eq!(
        providers.as_slice(),
        ["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn merge_language_items_rejects_duplicate_provider_with_same_sized_id() {
    let shared_sized_id = def_id(1, 1);
    let alpha = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: shared_sized_id,
        }),
        ..LanguageItems::default()
    };
    let zeta = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: shared_sized_id,
        }),
        ..LanguageItems::default()
    };

    let error = merge_language_item_providers([("zeta", &zeta), ("alpha", &alpha)])
        .expect_err("provider provenance must conflict even when sized IDs match");

    let LanguageItemProviderConflict::Multiple {
        protocol,
        providers,
    } = error
    else {
        panic!("expected a multiple-provider conflict");
    };
    assert_eq!(protocol, "sized");
    assert_eq!(
        providers.as_slice(),
        ["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn merge_language_items_requires_index_for_index_mut() {
    let mutable = LanguageItems {
        index_mut: Some(IndexMutLanguageItems {
            trait_id: def_id(1, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 2),
        }),
        ..LanguageItems::default()
    };

    let error = merge_language_item_providers([("mutable", &mutable)])
        .expect_err("an IndexMut provider must also provide Index");

    assert_eq!(
            error.to_string(),
            "language-item provider 'mutable' claims protocol 'index_mut' without required protocol 'index'",
        );
}

#[test]
fn merge_language_items_rejects_split_index_provider() {
    let read = LanguageItems {
        index: Some(IndexLanguageItems {
            trait_id: def_id(1, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 2),
        }),
        ..LanguageItems::default()
    };
    let write = LanguageItems {
        index_mut: Some(IndexMutLanguageItems {
            trait_id: def_id(2, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(2, 2),
        }),
        ..LanguageItems::default()
    };

    let error = merge_language_item_providers([("read", &read), ("write", &write)])
        .expect_err("Index and IndexMut must share a provider");

    assert_eq!(
            error.to_string(),
            "language-item protocols 'index' and 'index_mut' must use one provider; found index=read, index_mut=write",
        );
}

#[test]
fn merge_language_items_reports_all_protocol_conflicts_in_stable_order() {
    let alpha = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: def_id(1, 1),
        }),
        drop: Some(DropLanguageItems {
            trait_id: def_id(1, 2),
            method_id: def_id(1, 3),
        }),
        ..LanguageItems::default()
    };
    let zeta = LanguageItems {
        sized: Some(SizedLanguageItems {
            trait_id: def_id(2, 1),
        }),
        drop: Some(DropLanguageItems {
            trait_id: def_id(2, 2),
            method_id: def_id(2, 3),
        }),
        ..LanguageItems::default()
    };

    let errors = merge_language_item_providers_all([("zeta", &zeta), ("alpha", &alpha)])
        .expect_err("independent protocol collisions should all be reported");

    assert_eq!(errors.len(), 2);
    let LanguageItemProviderConflict::Multiple {
        protocol,
        providers,
    } = &errors[0]
    else {
        panic!("expected a multiple-provider conflict");
    };
    assert_eq!(*protocol, "sized");
    assert_eq!(
        providers.as_slice(),
        ["alpha".to_string(), "zeta".to_string()]
    );
    let LanguageItemProviderConflict::Multiple {
        protocol,
        providers,
    } = &errors[1]
    else {
        panic!("expected a multiple-provider conflict");
    };
    assert_eq!(*protocol, "drop");
    assert_eq!(
        providers.as_slice(),
        ["alpha".to_string(), "zeta".to_string()]
    );
}

#[test]
fn merge_language_items_preserves_callable_and_marker_bundles() {
    let provider = LanguageItems {
        fn_once: Some(FnOnceLanguageItems {
            trait_id: def_id(1, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 2),
        }),
        fn_mut: Some(FnMutLanguageItems {
            trait_id: def_id(1, 3),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 4),
        }),
        fn_trait: Some(FnLanguageItems {
            trait_id: def_id(1, 5),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 6),
        }),
        send: Some(SendLanguageItems {
            trait_id: def_id(1, 7),
        }),
        sync: Some(SyncLanguageItems {
            trait_id: def_id(1, 8),
        }),
        ..LanguageItems::default()
    };

    let merged = merge_language_item_providers([("std", &provider)])
        .expect("callable and marker bundles should merge");

    assert_eq!(merged.fn_once, provider.fn_once);
    assert_eq!(merged.fn_mut, provider.fn_mut);
    assert_eq!(merged.fn_trait, provider.fn_trait);
    assert_eq!(merged.send, provider.send);
    assert_eq!(merged.sync, provider.sync);
}

#[test]
fn merge_language_items_rejects_duplicate_callable_provider() {
    let first = LanguageItems {
        fn_once: Some(FnOnceLanguageItems {
            trait_id: def_id(1, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(1, 2),
        }),
        ..LanguageItems::default()
    };
    let second = LanguageItems {
        fn_once: Some(FnOnceLanguageItems {
            trait_id: def_id(2, 1),
            output_id: AssocTypeId(0),
            method_id: def_id(2, 2),
        }),
        ..LanguageItems::default()
    };

    let error = merge_language_item_providers([("zeta", &second), ("alpha", &first)])
        .expect_err("duplicate FnOnce providers should be rejected");

    assert_eq!(
        error.to_string(),
        "multiple language-item providers claim protocol 'fn_once': alpha, zeta"
    );
}
