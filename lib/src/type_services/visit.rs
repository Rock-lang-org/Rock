use crate::type_services::kind::Kind;
use crate::types::{FunctionCapture, GenericParamId, Type};

/// Recursively observes every node in a [`Type`] tree.
///
/// Binding forms must gain explicit scope callbacks before they are added to
/// [`Type`]; binder declarations must not be treated as ordinary child types.
pub trait TypeVisitor {
    fn enter_object(&mut self) {}

    fn exit_object(&mut self) {}

    fn enter_binders(&mut self, _params: &[Kind]) {}

    fn exit_binders(&mut self) {}

    fn visit_type(&mut self, ty: &Type) {
        visit_type_children(ty, self);
    }
}

impl<F> TypeVisitor for F
where
    F: FnMut(&Type),
{
    fn visit_type(&mut self, ty: &Type) {
        self(ty);
        visit_type_children(ty, self);
    }
}

pub fn visit_type<V>(ty: &Type, visitor: &mut V)
where
    V: TypeVisitor + ?Sized,
{
    visitor.visit_type(ty);
}

pub fn type_any<F>(ty: &Type, mut predicate: F) -> bool
where
    F: FnMut(&Type) -> bool,
{
    fn recurse<F>(ty: &Type, predicate: &mut F) -> bool
    where
        F: FnMut(&Type) -> bool,
    {
        if predicate(ty) {
            return true;
        }

        match ty {
            Type::Object(object) => object.children().any(|ty| recurse(ty, predicate)),
            Type::Slice(inner)
            | Type::Array(inner, _)
            | Type::Reference { inner, .. }
            | Type::Pointer(inner) => recurse(inner, predicate),
            Type::Tuple(elements) => elements.iter().any(|ty| recurse(ty, predicate)),
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                params.iter().any(|ty| recurse(ty, predicate))
                    || recurse(ret, predicate)
                    || captures
                        .iter()
                        .any(|capture| recurse(&capture.ty, predicate))
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                args.iter().any(|ty| recurse(ty, predicate))
            }
            Type::Projection { ty, trait_args, .. } => {
                recurse(ty, predicate) || trait_args.iter().any(|ty| recurse(ty, predicate))
            }
            Type::Apply { constructor, args } => {
                recurse(constructor, predicate) || args.iter().any(|ty| recurse(ty, predicate))
            }
            Type::Lambda { body, .. } => recurse(body, predicate),
            Type::I8
            | Type::ObjectSelf { .. }
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
            | Type::Never
            | Type::TypeVar(_)
            | Type::Generic(_)
            | Type::Constructor { .. }
            | Type::BoundVar { .. }
            | Type::Error => false,
        }
    }

    recurse(ty, &mut predicate)
}

/// Visits the immediate children of `ty` in stable source order.
pub fn visit_type_children<V>(ty: &Type, visitor: &mut V)
where
    V: TypeVisitor + ?Sized,
{
    match ty {
        Type::Object(object) => {
            visitor.enter_object();
            for child in object.children() {
                visitor.visit_type(child);
            }
            visitor.exit_object();
        }
        Type::Slice(inner)
        | Type::Array(inner, _)
        | Type::Reference { inner, .. }
        | Type::Pointer(inner) => visitor.visit_type(inner),
        Type::Tuple(elements) => {
            for element in elements {
                visitor.visit_type(element);
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for param in params {
                visitor.visit_type(param);
            }
            visitor.visit_type(ret);
            for capture in captures {
                visitor.visit_type(&capture.ty);
            }
        }
        Type::Struct { args, .. } | Type::Enum { args, .. } => {
            for arg in args {
                visitor.visit_type(arg);
            }
        }
        Type::Projection { ty, trait_args, .. } => {
            visitor.visit_type(ty);
            for arg in trait_args {
                visitor.visit_type(arg);
            }
        }
        Type::Apply { constructor, args } => {
            visitor.visit_type(constructor);
            for arg in args {
                visitor.visit_type(arg);
            }
        }
        Type::Lambda { params, body } => {
            visitor.enter_binders(params);
            visitor.visit_type(body);
            visitor.exit_binders();
        }
        Type::I8
        | Type::ObjectSelf { .. }
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
        | Type::Never
        | Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Constructor { .. }
        | Type::BoundVar { .. }
        | Type::Error => {}
    }
}

/// Recursively rewrites a [`Type`] tree.
///
/// Binding forms must override traversal with scope-aware behavior rather than
/// exposing binder declarations through [`fold_type_children`].
pub trait TypeFolder {
    fn enter_object(&mut self) {}

    fn exit_object(&mut self) {}

    fn enter_binders(&mut self, _params: &[Kind]) {}

    fn exit_binders(&mut self) {}

    fn fold_type(&mut self, ty: Type) -> Type {
        fold_type_children(ty, self)
    }
}

pub fn fold_type<F>(ty: Type, folder: &mut F) -> Type
where
    F: TypeFolder + ?Sized,
{
    folder.fold_type(ty)
}

pub fn fold_type_in_place<F>(ty: &mut Type, folder: &mut F)
where
    F: TypeFolder + ?Sized,
{
    *ty = folder.fold_type(ty.clone());
}

pub fn remap_generic_params_in_place<F>(ty: &mut Type, remap: &mut F)
where
    F: FnMut(GenericParamId) -> GenericParamId,
{
    struct GenericParamRemapper<'a, F> {
        remap: &'a mut F,
    }

    impl<F> TypeFolder for GenericParamRemapper<'_, F>
    where
        F: FnMut(GenericParamId) -> GenericParamId,
    {
        fn fold_type(&mut self, ty: Type) -> Type {
            match ty {
                Type::Generic(param) => Type::Generic((self.remap)(param)),
                other => fold_type_children(other, self),
            }
        }
    }

    fold_type_in_place(ty, &mut GenericParamRemapper { remap });
}

/// Folds the immediate children of `ty`, preserving all node metadata.
pub fn fold_type_children<F>(ty: Type, folder: &mut F) -> Type
where
    F: TypeFolder + ?Sized,
{
    match ty {
        Type::Object(object) => {
            folder.enter_object();
            let object = object.map_types(|ty| folder.fold_type(ty.clone()));
            folder.exit_object();
            Type::Object(Box::new(object))
        }
        Type::Slice(inner) => Type::Slice(Box::new(folder.fold_type(*inner))),
        Type::Array(inner, len) => Type::Array(Box::new(folder.fold_type(*inner)), len),
        Type::Tuple(elements) => Type::Tuple(
            elements
                .into_iter()
                .map(|element| folder.fold_type(element))
                .collect(),
        ),
        Type::Function {
            params,
            ret,
            safety,
            callable_kind,
            captures,
        } => Type::Function {
            params: params
                .into_iter()
                .map(|param| folder.fold_type(param))
                .collect(),
            ret: Box::new(folder.fold_type(*ret)),
            safety,
            callable_kind,
            captures: captures
                .into_iter()
                .map(|capture| FunctionCapture {
                    kind: capture.kind,
                    ty: folder.fold_type(capture.ty),
                })
                .collect(),
        },
        Type::Struct { id, args } => Type::Struct {
            id,
            args: args.into_iter().map(|arg| folder.fold_type(arg)).collect(),
        },
        Type::Enum { id, args } => Type::Enum {
            id,
            args: args.into_iter().map(|arg| folder.fold_type(arg)).collect(),
        },
        Type::Reference { mutable, inner } => Type::Reference {
            mutable,
            inner: Box::new(folder.fold_type(*inner)),
        },
        Type::Pointer(inner) => Type::Pointer(Box::new(folder.fold_type(*inner))),
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => Type::Projection {
            ty: Box::new(folder.fold_type(*ty)),
            trait_id,
            assoc_type,
            trait_args: trait_args
                .into_iter()
                .map(|arg| folder.fold_type(arg))
                .collect(),
        },
        Type::Apply { constructor, args } => Type::Apply {
            constructor: Box::new(folder.fold_type(*constructor)),
            args: args.into_iter().map(|arg| folder.fold_type(arg)).collect(),
        },
        Type::Lambda { params, body } => {
            folder.enter_binders(&params);
            let body = folder.fold_type(*body);
            folder.exit_binders();
            Type::Lambda {
                params,
                body: Box::new(body),
            }
        }
        leaf => leaf,
    }
}

/// A fallible counterpart to [`TypeFolder`].
pub trait TryTypeFolder {
    type Error;

    fn try_enter_object(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn try_exit_object(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn try_enter_binders(&mut self, _params: &[Kind]) -> Result<(), Self::Error> {
        Ok(())
    }

    fn try_exit_binders(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
        try_fold_type_children(ty, self)
    }
}

pub fn try_fold_type<F>(ty: Type, folder: &mut F) -> Result<Type, F::Error>
where
    F: TryTypeFolder + ?Sized,
{
    folder.try_fold_type(ty)
}

/// Fallibly folds the immediate children of `ty`, preserving node metadata.
pub fn try_fold_type_children<F>(ty: Type, folder: &mut F) -> Result<Type, F::Error>
where
    F: TryTypeFolder + ?Sized,
{
    Ok(match ty {
        Type::Object(object) => {
            folder.try_enter_object()?;
            let object = object.try_map_types(|ty| folder.try_fold_type(ty.clone()))?;
            folder.try_exit_object()?;
            Type::Object(Box::new(object))
        }
        Type::Slice(inner) => Type::Slice(Box::new(folder.try_fold_type(*inner)?)),
        Type::Array(inner, len) => Type::Array(Box::new(folder.try_fold_type(*inner)?), len),
        Type::Tuple(elements) => Type::Tuple(
            elements
                .into_iter()
                .map(|element| folder.try_fold_type(element))
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Type::Function {
            params,
            ret,
            safety,
            callable_kind,
            captures,
        } => Type::Function {
            params: params
                .into_iter()
                .map(|param| folder.try_fold_type(param))
                .collect::<Result<Vec<_>, _>>()?,
            ret: Box::new(folder.try_fold_type(*ret)?),
            safety,
            callable_kind,
            captures: captures
                .into_iter()
                .map(|capture| {
                    Ok(FunctionCapture {
                        kind: capture.kind,
                        ty: folder.try_fold_type(capture.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        },
        Type::Struct { id, args } => Type::Struct {
            id,
            args: args
                .into_iter()
                .map(|arg| folder.try_fold_type(arg))
                .collect::<Result<Vec<_>, _>>()?,
        },
        Type::Enum { id, args } => Type::Enum {
            id,
            args: args
                .into_iter()
                .map(|arg| folder.try_fold_type(arg))
                .collect::<Result<Vec<_>, _>>()?,
        },
        Type::Reference { mutable, inner } => Type::Reference {
            mutable,
            inner: Box::new(folder.try_fold_type(*inner)?),
        },
        Type::Pointer(inner) => Type::Pointer(Box::new(folder.try_fold_type(*inner)?)),
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => Type::Projection {
            ty: Box::new(folder.try_fold_type(*ty)?),
            trait_id,
            assoc_type,
            trait_args: trait_args
                .into_iter()
                .map(|arg| folder.try_fold_type(arg))
                .collect::<Result<Vec<_>, _>>()?,
        },
        Type::Apply { constructor, args } => Type::Apply {
            constructor: Box::new(folder.try_fold_type(*constructor)?),
            args: args
                .into_iter()
                .map(|arg| folder.try_fold_type(arg))
                .collect::<Result<Vec<_>, _>>()?,
        },
        Type::Lambda { params, body } => {
            folder.try_enter_binders(&params)?;
            let body = folder.try_fold_type(*body)?;
            folder.try_exit_binders()?;
            Type::Lambda {
                params,
                body: Box::new(body),
            }
        }
        leaf => leaf,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
    use crate::types::{AssociatedTypeKey, CaptureKind, FunctionCapture, GenericParamId, Type};

    use super::{
        fold_type, fold_type_children, fold_type_in_place, try_fold_type, try_fold_type_children,
        visit_type, visit_type_children, TryTypeFolder, TypeFolder, TypeVisitor,
    };

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn generic(owner: u32, index: u32) -> GenericParamId {
        GenericParamId {
            owner: def_id(owner),
            index,
        }
    }

    fn nested_type() -> Type {
        Type::Projection {
            ty: Box::new(Type::Reference {
                mutable: false,
                inner: Box::new(Type::Struct {
                    id: def_id(1),
                    args: vec![Type::Function {
                        params: vec![
                            Type::Slice(Box::new(Type::TypeVar(TypeVarId(1)))),
                            Type::Tuple(vec![
                                Type::Generic(generic(10, 0)),
                                Type::Pointer(Box::new(Type::TypeVar(TypeVarId(2)))),
                            ]),
                        ],
                        ret: Box::new(Type::Array(
                            Box::new(Type::Enum {
                                id: def_id(2),
                                args: vec![Type::Generic(generic(11, 0))],
                            }),
                            4,
                        )),
                        safety: crate::types::FunctionSafety::Safe,
                        callable_kind: crate::types::CallableKind::FnOnce,
                        captures: vec![FunctionCapture::new(
                            CaptureKind::Move,
                            Type::Reference {
                                mutable: true,
                                inner: Box::new(Type::TypeVar(TypeVarId(3))),
                            },
                        )],
                    }],
                }),
            }),
            trait_id: def_id(3),
            assoc_type: AssociatedTypeKey {
                owner: def_id(4),
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![
                Type::Enum {
                    id: def_id(5),
                    args: vec![Type::TypeVar(TypeVarId(4))],
                },
                Type::Generic(generic(12, 0)),
            ],
        }
    }

    #[derive(Default)]
    struct Recorder {
        nodes: usize,
        type_vars: Vec<TypeVarId>,
        generics: Vec<GenericParamId>,
    }

    impl TypeVisitor for Recorder {
        fn visit_type(&mut self, ty: &Type) {
            self.nodes += 1;
            match ty {
                Type::TypeVar(id) => self.type_vars.push(*id),
                Type::Generic(id) => self.generics.push(*id),
                _ => {}
            }
            visit_type_children(ty, self);
        }
    }

    #[test]
    fn immutable_visitor_reaches_every_nested_type_once() {
        let mut recorder = Recorder::default();
        visit_type(&nested_type(), &mut recorder);

        assert_eq!(recorder.nodes, 18);
        assert_eq!(
            recorder.type_vars,
            vec![TypeVarId(1), TypeVarId(2), TypeVarId(3), TypeVarId(4)]
        );
        assert_eq!(
            recorder.generics,
            vec![generic(10, 0), generic(11, 0), generic(12, 0)]
        );
    }

    struct ReplaceTypeVars;

    impl TypeFolder for ReplaceTypeVars {
        fn fold_type(&mut self, ty: Type) -> Type {
            match ty {
                Type::TypeVar(id) => Type::Generic(generic(20, id.0)),
                other => fold_type_children(other, self),
            }
        }
    }

    #[test]
    fn mutable_folder_rewrites_every_nested_type_location() {
        let folded = fold_type(nested_type(), &mut ReplaceTypeVars);
        let mut recorder = Recorder::default();
        visit_type(&folded, &mut recorder);

        assert!(recorder.type_vars.is_empty());
        assert_eq!(
            recorder.generics,
            vec![
                generic(20, 1),
                generic(10, 0),
                generic(20, 2),
                generic(11, 0),
                generic(20, 3),
                generic(20, 4),
                generic(12, 0),
            ]
        );
    }

    struct RejectTypeVar(TypeVarId);

    impl TryTypeFolder for RejectTypeVar {
        type Error = TypeVarId;

        fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
            match ty {
                Type::TypeVar(id) if id == self.0 => Err(id),
                other => try_fold_type_children(other, self),
            }
        }
    }

    #[test]
    fn fallible_folder_propagates_nested_errors() {
        assert_eq!(
            try_fold_type(nested_type(), &mut RejectTypeVar(TypeVarId(3))),
            Err(TypeVarId(3))
        );
    }

    struct PanicFolder;

    impl TypeFolder for PanicFolder {
        fn fold_type(&mut self, ty: Type) -> Type {
            if matches!(ty, Type::TypeVar(TypeVarId(3))) {
                panic!("intentional folder panic");
            }
            fold_type_children(ty, self)
        }
    }

    #[test]
    fn in_place_folder_preserves_original_type_when_folder_panics() {
        let mut ty = nested_type();
        let original = ty.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fold_type_in_place(&mut ty, &mut PanicFolder);
        }));

        assert!(result.is_err());
        assert_eq!(ty, original);
    }

    struct CountDefIds<'a>(&'a mut HashMap<DefId, usize>);

    impl TypeFolder for CountDefIds<'_> {
        fn fold_type(&mut self, mut ty: Type) -> Type {
            let mut record = |id: DefId| {
                *self.0.entry(id).or_default() += 1;
            };
            match &mut ty {
                Type::Struct { id, .. } | Type::Enum { id, .. } => record(*id),
                Type::Projection {
                    trait_id,
                    assoc_type,
                    ..
                } => {
                    record(*trait_id);
                    record(assoc_type.owner);
                }
                Type::Generic(param) => record(param.owner),
                _ => {}
            }
            fold_type_children(ty, self)
        }
    }

    #[test]
    fn def_id_folder_reaches_every_nested_identity_once() {
        let mut counts = HashMap::new();
        fold_type(nested_type(), &mut CountDefIds(&mut counts));

        assert_eq!(counts.len(), 8);
        assert!(counts.values().all(|count| *count == 1));
    }

    #[test]
    fn type_def_id_remap_preserves_original_type_when_callback_panics() {
        let mut ty = nested_type();
        let original = ty.clone();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            ty.remap_def_ids(&mut |id| {
                if id == def_id(2) {
                    panic!("intentional remap panic");
                }
                def_id(id.local.0 + 100)
            });
        }));

        assert!(result.is_err());
        assert_eq!(ty, original);
    }
}
