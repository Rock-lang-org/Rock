use super::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::ids::{CrateId, DefId, FieldId, InstanceId, LocalDefId, VariantId};

fn hash_of<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn mir_function_identity_is_canonical_not_display_name() {
    let id = DefId::new(CrateId(0), LocalDefId(1));
    let left = MirFunctionId::Function(id);
    let right = MirFunctionId::Function(id);

    assert_eq!(left, right);
    assert_eq!(left.display_fallback("alias"), "def#0:1");
}

#[test]
fn callable_and_aggregate_identities_carry_existing_compiler_ids() {
    let function = DefId::new(CrateId(0), LocalDefId(2));
    let structure = DefId::new(CrateId(0), LocalDefId(3));
    let enumeration = DefId::new(CrateId(0), LocalDefId(4));
    let variant = VariantId(5);

    assert_eq!(
        MirCallable::Resolved(MirCallableKey::Function(function)).function_id(),
        Some(MirFunctionId::Function(function))
    );
    assert_eq!(
        MirCallable::Resolved(MirCallableKey::Instance(InstanceId(7))).function_id(),
        Some(MirFunctionId::Instance(InstanceId(7)))
    );
    assert_eq!(
        MirAggregateIdentity::Struct {
            id: structure,
            display_name: "Point".to_string(),
        }
        .def_id(),
        structure
    );
    assert_eq!(
        MirAggregateIdentity::EnumVariant {
            enum_id: enumeration,
            variant_id: variant,
            enum_name: "Option".to_string(),
            variant_name: "Some".to_string(),
        }
        .def_id(),
        enumeration
    );
    assert_eq!(
        MirFieldIdentity::new(structure, FieldId(1)).field_id,
        FieldId(1)
    );
}

#[test]
fn callable_intrinsic_identity_is_typed() {
    fn intrinsic_callable(id: MirIntrinsicId) -> MirCallable {
        MirCallable::Resolved(MirCallableKey::Intrinsic(id))
    }

    assert_eq!(
        intrinsic_callable(MirIntrinsicId::PtrFromParts),
        MirCallable::Resolved(MirCallableKey::Intrinsic(MirIntrinsicId::PtrFromParts))
    );
}

#[test]
fn runtime_helper_has_explicit_heap_allocation_identity() {
    assert_ne!(MirRuntimeHelper::HeapAlloc, MirRuntimeHelper::DropGlue);
}

#[test]
fn intrinsic_from_name_rejects_unknown_source_name() {
    assert!(MirIntrinsicId::from_name("NotARealIntrinsic").is_none());
}

#[test]
fn removed_make_arr_has_no_intrinsic_identity() {
    for name in ["MakeArr", "BorrowSlice", "BorrowSliceMut", "BorrowStr"] {
        assert!(MirIntrinsicId::from_name(name).is_none());
        assert!(!crate::lower::intrinsics::is_intrinsic_name(name));
    }
}

#[test]
fn struct_identity_ignores_display_name() {
    let id = DefId::new(CrateId(0), LocalDefId(20));
    let left = MirAggregateIdentity::Struct {
        id,
        display_name: "Point".to_string(),
    };
    let right = MirAggregateIdentity::Struct {
        id,
        display_name: "Alias".to_string(),
    };

    assert_eq!(left, right);
    assert_eq!(hash_of(&left), hash_of(&right));
}

#[test]
fn enum_variant_identity_ignores_display_names() {
    let enum_id = DefId::new(CrateId(0), LocalDefId(30));
    let variant_id = VariantId(31);
    let left = MirAggregateIdentity::EnumVariant {
        enum_id,
        variant_id,
        enum_name: "Option".to_string(),
        variant_name: "Some".to_string(),
    };
    let right = MirAggregateIdentity::EnumVariant {
        enum_id,
        variant_id,
        enum_name: "Maybe".to_string(),
        variant_name: "Present".to_string(),
    };

    assert_eq!(left, right);
    assert_eq!(hash_of(&left), hash_of(&right));
}
