use crate::type_context::Ty;

use super::*;

#[test]
fn type_facts_classify_numeric_and_concrete_types() {
    assert!(TypeFacts::is_integer(&Type::I64));
    assert!(TypeFacts::is_unsigned_integer(&Type::U8));
    assert!(TypeFacts::is_signed_integer(&Type::I32));
    assert!(TypeFacts::is_float(&Type::F64));
    assert!(TypeFacts::is_numeric(&Type::F32));
    assert!(!TypeFacts::is_numeric(&Type::Bool));
    assert!(TypeFacts::is_concrete(&Type::Tuple(vec![
        Type::I64,
        Type::Bool
    ])));
    assert!(!TypeFacts::is_concrete(&Type::Generic(
        crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(1)),
            index: 0,
        }
    )));
    assert!(!TypeFacts::is_concrete(&Type::Constructor {
        id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(2)),
        flavor: crate::types::NominalTypeKind::Struct,
    }));
    assert!(!TypeFacts::is_concrete(&Type::Lambda {
        params: vec![crate::type_services::kind::Kind::Type],
        body: Box::new(Type::BoundVar {
            depth: 0,
            index: 0,
            kind: crate::type_services::kind::Kind::Type,
        }),
    }));
    assert!(!TypeFacts::is_codegen_concrete(&Type::Apply {
        constructor: Box::new(Type::Generic(crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(3),),
            index: 0,
        })),
        args: vec![Type::I64],
    }));
    let constructor = Type::Constructor {
        id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(4)),
        flavor: crate::types::NominalTypeKind::Enum,
    };
    assert!(!TypeFacts::is_codegen_concrete(&constructor));
    assert!(TypeFacts::is_concrete_type_argument(&constructor));
    assert!(!TypeFacts::is_concrete_type_argument(&Type::Generic(
        crate::types::GenericParamId {
            owner: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(4)),
            index: 0,
        }
    )));
    assert!(TypeFacts::is_codegen_concrete(&Type::Struct {
        id: crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(5)),
        args: vec![constructor, Type::I64],
    }));
}

#[test]
fn type_facts_match_copy_and_reference_behavior() {
    assert!(TypeFacts::is_copy(&Type::Tuple(vec![
        Type::I64,
        Type::Bool
    ])));
    assert!(!TypeFacts::is_copy(&Type::Array(Box::new(Type::I64), 4)));
    assert!(TypeFacts::contains_reference(&Type::Tuple(vec![
        Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }
    ])));
    assert!(!TypeFacts::contains_reference(&Type::Tuple(vec![
        Type::I64,
        Type::Bool
    ])));
}

#[test]
fn type_facts_report_id_backed_copy() {
    let mut context = crate::type_context::TypeContext::new();
    let elem = context.intern_type(&Type::U8);
    let bool_ty = context.intern_type(&Type::Bool);
    let tuple = context.intern_ty(Ty::Tuple(vec![elem, bool_ty]));
    let mutable_ref = context.intern_ty(Ty::Reference {
        mutable: true,
        inner: elem,
    });

    assert!(TypeFacts::is_copy_id(tuple, |id| context.ty(id)));
    assert!(!TypeFacts::is_copy_id(mutable_ref, |id| context.ty(id)));
}

#[test]
fn type_wrappers_match_direct_type_facts() {
    let composite = Type::function(
        vec![Type::Reference {
            mutable: false,
            inner: Box::new(Type::I64),
        }],
        Type::Bool,
    );

    assert_eq!(
        composite.contains_reference(),
        TypeFacts::contains_reference(&composite)
    );
    assert_eq!(composite.is_copy(), TypeFacts::is_copy(&composite));
    assert_eq!(Type::I64.is_integer(), TypeFacts::is_integer(&Type::I64));
    assert_eq!(Type::F64.is_float(), TypeFacts::is_float(&Type::F64));
    assert_eq!(
        Type::Bool.is_concrete(),
        TypeFacts::is_concrete(&Type::Bool)
    );
}

#[test]
fn type_facts_include_function_capture_types_and_modes() {
    let move_non_copy = Type::function_with_metadata(
        Vec::new(),
        Type::Unit,
        crate::types::FunctionSafety::Safe,
        crate::types::CallableKind::FnOnce,
        vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::Move,
            Type::TypeVar(crate::ids::TypeVarId(0)),
        )],
    );
    let mutable_borrow = Type::function_with_metadata(
        Vec::new(),
        Type::Unit,
        crate::types::FunctionSafety::Safe,
        crate::types::CallableKind::FnMut,
        vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::MutableBorrow,
            Type::I64,
        )],
    );

    assert!(!TypeFacts::is_concrete(&move_non_copy));
    assert!(!TypeFacts::is_copy(&move_non_copy));
    assert!(!TypeFacts::is_copy(&mutable_borrow));
    assert!(TypeFacts::contains_reference(&mutable_borrow));
}
