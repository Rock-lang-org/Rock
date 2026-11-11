use crate::type_services::kind::Kind;
use crate::types::{ArrayLen, FunctionCapture, GenericParamId, Type};

/// Recursively observes every node in a [`Type`] tree.
///
/// Binding forms must gain explicit scope callbacks before they are added to
/// [`Type`]; binder declarations must not be treated as ordinary child types.
pub trait TypeVisitor {
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
            Type::Array(inner, len) => {
                recurse(inner, predicate) || recurse(&len.to_const_type(), predicate)
            }
            Type::Slice(inner) | Type::Reference { inner, .. } | Type::Pointer(inner) => {
                recurse(inner, predicate)
            }
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
            | Type::ConstValue(_)
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
        Type::Array(inner, len) => {
            visitor.visit_type(inner);
            // The length is visited as a const-kinded type so every visitor
            // sees `Param` / `Var` lengths as `Generic` / `TypeVar` leaves.
            visitor.visit_type(&len.to_const_type());
        }
        Type::Slice(inner) | Type::Reference { inner, .. } | Type::Pointer(inner) => {
            visitor.visit_type(inner)
        }
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
        | Type::ConstValue(_)
        | Type::Error => {}
    }
}

/// Recursively rewrites a [`Type`] tree.
///
/// Binding forms must override traversal with scope-aware behavior rather than
/// exposing binder declarations through [`fold_type_children`].
pub trait TypeFolder {
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
        Type::Slice(inner) => Type::Slice(Box::new(folder.fold_type(*inner))),
        Type::Array(inner, len) => {
            let inner = folder.fold_type(*inner);
            // A length is a const-kinded type argument: folding it through `fold_type`
            // keeps every folder's generic/type-variable handling authoritative for lengths.
            let len = folder.fold_type(len.to_const_type());
            Type::Array(Box::new(inner), ArrayLen::from_const_type(&len))
        }
        Type::Tuple(elements) => Type::Tuple(
            elements
                .into_iter()
                .map(|element| folder.fold_type(element))
                .collect(),
        ),
        Type::Function { params, ret, safety, callable_kind, captures, captures_known } => {
            let params = params.into_iter().map(|param| folder.fold_type(param)).collect();
            let ret = Box::new(folder.fold_type(*ret));
            let captures = captures.into_iter().map(|capture| FunctionCapture {
                kind: capture.kind,
                effect: capture.effect,
                ty: folder.fold_type(capture.ty),
            }).collect::<Vec<_>>();
            let callable_kind = if captures.is_empty() { callable_kind } else {
                crate::types::CallableKind::from_captures(&captures)
            };
            Type::Function { params, ret, safety, callable_kind, captures, captures_known }
        }
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
        Type::Slice(inner) => Type::Slice(Box::new(folder.try_fold_type(*inner)?)),
        Type::Array(inner, len) => {
            let inner = folder.try_fold_type(*inner)?;
            let len = folder.try_fold_type(len.to_const_type())?;
            Type::Array(Box::new(inner), ArrayLen::from_const_type(&len))
        }
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
            captures_known,
        } => Type::Function {
            params: params
                .into_iter()
                .map(|param| folder.try_fold_type(param))
                .collect::<Result<Vec<_>, _>>()?,
            ret: Box::new(folder.try_fold_type(*ret)?),
            safety,
            callable_kind,
            captures_known,
            captures: captures
                .into_iter()
                .map(|capture| {
                    Ok(FunctionCapture {
                        kind: capture.kind,
                        effect: capture.effect,
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
mod tests;
