use crate::ids::TypeId;
use crate::type_context::Ty;
use crate::types::{CaptureKind, Type};

pub struct TypeFacts;

impl TypeFacts {
    pub fn is_integer(ty: &Type) -> bool {
        matches!(
            ty,
            Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
        )
    }

    pub fn is_signed_integer(ty: &Type) -> bool {
        matches!(ty, Type::I8 | Type::I16 | Type::I32 | Type::I64)
    }

    pub fn is_unsigned_integer(ty: &Type) -> bool {
        matches!(ty, Type::U8 | Type::U16 | Type::U32 | Type::U64)
    }

    pub fn is_float(ty: &Type) -> bool {
        matches!(ty, Type::F32 | Type::F64)
    }

    pub fn is_numeric(ty: &Type) -> bool {
        Self::is_integer(ty) || Self::is_float(ty)
    }

    pub fn is_type_var(ty: &Type) -> bool {
        matches!(ty, Type::TypeVar(_))
    }

    pub fn is_concrete(ty: &Type) -> bool {
        let mut concrete = true;
        crate::type_services::visit::visit_type(ty, &mut |nested: &Type| {
            concrete &= !matches!(
                nested,
                Type::TypeVar(_)
                    | Type::Generic(_)
                    | Type::Projection { .. }
                    | Type::Constructor { .. }
                    | Type::Apply { .. }
                    | Type::Lambda { .. }
                    | Type::BoundVar { .. }
            );
        });
        concrete
    }

    fn closed_constructor_value(ty: &Type) -> bool {
        match ty {
            Type::Constructor { .. } => true,
            Type::Apply { constructor, args } => {
                Self::closed_constructor_value(constructor)
                    && args.iter().all(Self::is_concrete_type_argument)
            }
            Type::Lambda { body, .. } => !crate::type_services::visit::type_any(body, |nested| {
                matches!(
                    nested,
                    Type::TypeVar(_) | Type::Generic(_) | Type::Projection { .. } | Type::Error
                )
            }),
            _ => false,
        }
    }

    /// Specialization arguments may be closed constructors rather than runtime values.
    pub fn is_concrete_type_argument(ty: &Type) -> bool {
        Self::is_codegen_concrete(ty) || Self::closed_constructor_value(ty)
    }

    pub fn is_codegen_concrete(ty: &Type) -> bool {
        fn runtime_type(ty: &Type) -> bool {
            match ty {
                Type::Slice(inner) | Type::Array(inner, _) | Type::Pointer(inner) => {
                    runtime_type(inner)
                }
                Type::Reference { inner, .. } => runtime_type(inner),
                Type::Tuple(elements) => elements.iter().all(runtime_type),
                Type::Function {
                    params,
                    ret,
                    captures,
                    ..
                } => {
                    params.iter().all(runtime_type)
                        && runtime_type(ret)
                        && captures.iter().all(|capture| runtime_type(&capture.ty))
                }
                Type::Struct { args, .. } | Type::Enum { args, .. } => {
                    args.iter().all(TypeFacts::is_concrete_type_argument)
                }
                Type::TypeVar(_)
                | Type::Generic(_)
                | Type::Projection { .. }
                | Type::Constructor { .. }
                | Type::Apply { .. }
                | Type::Lambda { .. }
                | Type::BoundVar { .. }
                | Type::Error => false,
                Type::I8
                | Type::I16
                | Type::I32
                | Type::I64
                | Type::U8
                | Type::U16
                | Type::U32
                | Type::U64
                | Type::F32
                | Type::F64
                | Type::Bool
                | Type::Str
                | Type::Char
                | Type::Unit
                | Type::Never => true,
            }
        }

        runtime_type(ty)
    }

    pub fn is_copy(ty: &Type) -> bool {
        match ty {
            Type::I8 | Type::I16 | Type::I32 | Type::I64 => true,
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => true,
            Type::F32 | Type::F64 => true,
            Type::Bool | Type::Char | Type::Unit => true,
            Type::Reference { mutable: false, .. } => true,
            Type::Reference { mutable: true, .. } => false,
            Type::Pointer(_) => true,
            Type::Function { captures, .. } => captures.iter().all(|capture| match capture.kind {
                CaptureKind::SharedBorrow => true,
                CaptureKind::MutableBorrow => false,
                CaptureKind::Move => Self::is_copy(&capture.ty),
            }),
            Type::Tuple(elems) => elems.iter().all(Self::is_copy),
            Type::Str => true,
            Type::Slice(_) => true,
            Type::Array(_, _) => false,
            Type::Projection { .. } => false,
            Type::Struct { .. } => false,
            Type::Enum { .. } => false,
            Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Constructor { .. }
            | Type::Apply { .. }
            | Type::Lambda { .. }
            | Type::BoundVar { .. }
            | Type::Error
            | Type::Never => false,
        }
    }

    pub fn is_copy_id<'a, F>(id: TypeId, ty: F) -> bool
    where
        F: Copy + Fn(TypeId) -> &'a Ty,
    {
        match ty(id) {
            Ty::I8
            | Ty::I16
            | Ty::I32
            | Ty::I64
            | Ty::U8
            | Ty::U16
            | Ty::U32
            | Ty::U64
            | Ty::F32
            | Ty::F64
            | Ty::Bool
            | Ty::Char
            | Ty::Unit
            | Ty::Str
            | Ty::Slice(_)
            | Ty::Pointer(_) => true,
            Ty::Function { captures, .. } => captures.iter().all(|capture| match capture.kind {
                CaptureKind::SharedBorrow => true,
                CaptureKind::MutableBorrow => false,
                CaptureKind::Move => Self::is_copy_id(capture.ty, ty),
            }),
            Ty::Reference { mutable: false, .. } => true,
            Ty::Tuple(elems) => elems.iter().all(|elem| Self::is_copy_id(*elem, ty)),
            Ty::Reference { mutable: true, .. }
            | Ty::Array { .. }
            | Ty::Projection { .. }
            | Ty::Struct { .. }
            | Ty::Enum { .. }
            | Ty::TypeVar(_)
            | Ty::Generic(_)
            | Ty::Constructor { .. }
            | Ty::Apply { .. }
            | Ty::Lambda { .. }
            | Ty::BoundVar { .. }
            | Ty::Error
            | Ty::Never => false,
        }
    }

    pub fn contains_reference(ty: &Type) -> bool {
        match ty {
            Type::Reference { .. } => true,
            Type::Array(inner, _) | Type::Slice(inner) => Self::contains_reference(inner),
            Type::Tuple(elems)
            | Type::Struct { args: elems, .. }
            | Type::Enum { args: elems, .. } => elems.iter().any(Self::contains_reference),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params.iter().any(Self::contains_reference)
                    || Self::contains_reference(ret)
                    || captures.iter().any(|capture| match capture.kind {
                        CaptureKind::SharedBorrow | CaptureKind::MutableBorrow => true,
                        CaptureKind::Move => Self::contains_reference(&capture.ty),
                    })
            }
            Type::Projection { ty, trait_args, .. } => {
                Self::contains_reference(ty) || trait_args.iter().any(Self::contains_reference)
            }
            Type::Apply { constructor, args } => {
                Self::contains_reference(constructor) || args.iter().any(Self::contains_reference)
            }
            Type::Lambda { body, .. } => Self::contains_reference(body),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
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
}
