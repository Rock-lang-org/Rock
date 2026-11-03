use std::collections::{HashMap, HashSet};

use crate::ids::{DefId, IdGen, Idx, TypeId, TypeVarId};
use crate::type_services::kind::Kind;
use crate::types::{
    AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, FunctionSafety, GenericParamId,
    NominalTypeKind, Type,
};

mod view;

pub use view::TypeView;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    Str,
    Char,
    Unit,
    Never,
    Slice(TypeId),
    Array {
        inner: TypeId,
        len: usize,
    },
    Tuple(Vec<TypeId>),
    Function {
        params: Vec<TypeId>,
        ret: TypeId,
        safety: FunctionSafety,
        callable_kind: CallableKind,
        captures: Vec<TyFunctionCapture>,
    },
    Struct {
        id: DefId,
        args: Vec<TypeId>,
    },
    Enum {
        id: DefId,
        args: Vec<TypeId>,
    },
    Reference {
        mutable: bool,
        inner: TypeId,
    },
    Pointer(TypeId),
    TypeVar(TypeVarId),
    Generic(GenericParamId),
    Projection {
        ty: TypeId,
        trait_id: DefId,
        assoc_type: AssociatedTypeKey,
        trait_args: Vec<TypeId>,
    },
    Constructor {
        id: DefId,
        flavor: NominalTypeKind,
    },
    Apply {
        constructor: TypeId,
        args: Vec<TypeId>,
    },
    Lambda {
        params: Vec<Kind>,
        body: TypeId,
    },
    BoundVar {
        depth: u32,
        index: u32,
        kind: Kind,
    },
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TyFunctionCapture {
    pub kind: CaptureKind,
    pub ty: TypeId,
}

#[derive(Debug, Default)]
pub struct TypeContext {
    tys: Vec<Ty>,
    kinds: Vec<Kind>,
    interned: HashMap<Ty, TypeId>,
    ids: IdGen<TypeId>,
    accepting_normalized_hkt: bool,
    normalization_env: crate::type_services::normalize::TypeNormalizationEnv,
}

impl Clone for TypeContext {
    fn clone(&self) -> Self {
        let mut ids = IdGen::new();
        for _ in 0..self.tys.len() {
            ids.fresh();
        }

        Self {
            tys: self.tys.clone(),
            kinds: self.kinds.clone(),
            interned: self.interned.clone(),
            ids,
            accepting_normalized_hkt: false,
            normalization_env: self.normalization_env.clone(),
        }
    }
}

impl TypeContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn with_normalization_env(
        normalization_env: crate::type_services::normalize::TypeNormalizationEnv,
    ) -> Self {
        Self {
            normalization_env,
            ..Self::default()
        }
    }

    pub fn intern_ty(&mut self, ty: Ty) -> TypeId {
        assert!(
            self.accepting_normalized_hkt
                || !matches!(
                    ty,
                    Ty::Constructor { .. }
                        | Ty::Apply { .. }
                        | Ty::Lambda { .. }
                        | Ty::BoundVar { .. }
                ),
            "higher-kinded terms must enter TypeContext through intern_normalized_type"
        );
        let kind = self.kind_for_ty(&ty);
        if let Some(id) = self.interned.get(&ty) {
            debug_assert_eq!(self.kind(*id), &kind);
            return *id;
        }

        let id = self.ids.fresh();
        debug_assert_eq!(id.index(), self.tys.len());
        self.tys.push(ty.clone());
        self.kinds.push(kind);
        self.interned.insert(ty, id);
        id
    }

    fn kind_for_ty(&self, ty: &Ty) -> Kind {
        match ty {
            Ty::Constructor { id, flavor } => {
                crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env)
                    .kind_of(&Type::Constructor {
                        id: *id,
                        flavor: *flavor,
                    })
                    .unwrap_or(Kind::Type)
            }
            Ty::Apply { constructor, args } => {
                let mut kind = self.kind(*constructor).clone();
                for arg in args {
                    let Kind::Arrow(expected, output) = kind else {
                        return Kind::Type;
                    };
                    if self.kind(*arg) != expected.as_ref() {
                        return Kind::Type;
                    }
                    kind = *output;
                }
                kind
            }
            Ty::Lambda { params, body } => params
                .iter()
                .rev()
                .fold(self.kind(*body).clone(), |output, input| {
                    Kind::arrow(input.clone(), output)
                }),
            Ty::BoundVar { kind, .. } => kind.clone(),
            Ty::Generic(param) => {
                crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env)
                    .kind_of(&Type::Generic(*param))
                    .unwrap_or(Kind::Type)
            }
            Ty::TypeVar(id) => {
                crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env)
                    .kind_of(&Type::TypeVar(*id))
                    .unwrap_or(Kind::Type)
            }
            Ty::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env)
                .kind_of(&Type::Projection {
                    ty: Box::new(self.type_for(*ty)),
                    trait_id: *trait_id,
                    assoc_type: *assoc_type,
                    trait_args: trait_args.iter().map(|arg| self.type_for(*arg)).collect(),
                })
                .unwrap_or(Kind::Type),
            _ => Kind::Type,
        }
    }

    pub fn ty(&self, id: TypeId) -> &Ty {
        &self.tys[id.index()]
    }

    pub fn try_ty(&self, id: TypeId) -> Option<&Ty> {
        self.tys.get(id.index())
    }

    pub fn kind(&self, id: TypeId) -> &Kind {
        &self.kinds[id.index()]
    }

    pub fn try_kind(&self, id: TypeId) -> Option<&Kind> {
        self.kinds.get(id.index())
    }

    pub fn normalize_type(
        &self,
        ty: &Type,
    ) -> Result<Type, crate::type_services::normalize::NormalizeError> {
        crate::type_services::normalize::TypeNormalizer::new(&self.normalization_env).normalize(ty)
    }

    pub fn contains_type_id(&self, id: TypeId) -> bool {
        self.try_ty(id).is_some()
    }

    pub fn type_id_tree_is_valid(&self, id: TypeId) -> bool {
        fn visit(
            context: &TypeContext,
            id: TypeId,
            visiting: &mut HashSet<TypeId>,
            valid: &mut HashSet<TypeId>,
        ) -> bool {
            if valid.contains(&id) {
                return true;
            }
            if !visiting.insert(id) {
                return false;
            }
            let Some(ty) = context.try_ty(id) else {
                visiting.remove(&id);
                return false;
            };
            let children_are_valid = match ty {
                Ty::Slice(inner) | Ty::Reference { inner, .. } | Ty::Pointer(inner) => {
                    visit(context, *inner, visiting, valid)
                }
                Ty::Array { inner, .. } => visit(context, *inner, visiting, valid),
                Ty::Tuple(elements) => elements
                    .iter()
                    .all(|element| visit(context, *element, visiting, valid)),
                Ty::Function {
                    params,
                    ret,
                    captures,
                    ..
                } => {
                    params
                        .iter()
                        .all(|param| visit(context, *param, visiting, valid))
                        && visit(context, *ret, visiting, valid)
                        && captures
                            .iter()
                            .all(|capture| visit(context, capture.ty, visiting, valid))
                }
                Ty::Struct { args, .. } | Ty::Enum { args, .. } => {
                    args.iter().all(|arg| visit(context, *arg, visiting, valid))
                }
                Ty::Projection { ty, trait_args, .. } => {
                    visit(context, *ty, visiting, valid)
                        && trait_args
                            .iter()
                            .all(|arg| visit(context, *arg, visiting, valid))
                }
                Ty::Apply { constructor, args } => {
                    visit(context, *constructor, visiting, valid)
                        && args.iter().all(|arg| visit(context, *arg, visiting, valid))
                }
                Ty::Lambda { body, .. } => visit(context, *body, visiting, valid),
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
                | Ty::Str
                | Ty::Char
                | Ty::Unit
                | Ty::Never
                | Ty::TypeVar(_)
                | Ty::Generic(_)
                | Ty::Constructor { .. }
                | Ty::BoundVar { .. }
                | Ty::Error => true,
            };
            visiting.remove(&id);
            if children_are_valid {
                valid.insert(id);
            }
            children_are_valid
        }

        visit(self, id, &mut HashSet::new(), &mut HashSet::new())
    }

    pub(crate) fn len(&self) -> usize {
        self.tys.len()
    }

    pub fn intern_type(&mut self, ty: &Type) -> TypeId {
        assert!(
            self.accepting_normalized_hkt
                || !crate::type_services::visit::type_any(ty, |nested| matches!(
                    nested,
                    Type::Constructor { .. }
                        | Type::Apply { .. }
                        | Type::Lambda { .. }
                        | Type::BoundVar { .. }
                )),
            "higher-kinded terms must enter TypeContext through intern_normalized_type"
        );
        match ty {
            Type::I8 => self.intern_ty(Ty::I8),
            Type::I16 => self.intern_ty(Ty::I16),
            Type::I32 => self.intern_ty(Ty::I32),
            Type::I64 => self.intern_ty(Ty::I64),
            Type::U8 => self.intern_ty(Ty::U8),
            Type::U16 => self.intern_ty(Ty::U16),
            Type::U32 => self.intern_ty(Ty::U32),
            Type::U64 => self.intern_ty(Ty::U64),
            Type::F32 => self.intern_ty(Ty::F32),
            Type::F64 => self.intern_ty(Ty::F64),
            Type::Bool => self.intern_ty(Ty::Bool),
            Type::Str => self.intern_ty(Ty::Str),
            Type::Char => self.intern_ty(Ty::Char),
            Type::Unit => self.intern_ty(Ty::Unit),
            Type::Never => self.intern_ty(Ty::Never),
            Type::Slice(inner) => {
                let inner = self.intern_type(inner);
                self.intern_ty(Ty::Slice(inner))
            }
            Type::Array(inner, len) => {
                let inner = self.intern_type(inner);
                self.intern_ty(Ty::Array { inner, len: *len })
            }
            Type::Tuple(elems) => {
                let elems = elems.iter().map(|elem| self.intern_type(elem)).collect();
                self.intern_ty(Ty::Tuple(elems))
            }
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => {
                let params = params.iter().map(|arg| self.intern_type(arg)).collect();
                let ret = self.intern_type(ret);
                let captures = captures
                    .iter()
                    .map(|capture| TyFunctionCapture {
                        kind: capture.kind,
                        ty: self.intern_type(&capture.ty),
                    })
                    .collect();
                self.intern_ty(Ty::Function {
                    params,
                    ret,
                    safety: *safety,
                    callable_kind: *callable_kind,
                    captures,
                })
            }
            Type::Struct { id, args } => {
                let args = args.iter().map(|arg| self.intern_type(arg)).collect();
                self.intern_ty(Ty::Struct { id: *id, args })
            }
            Type::Enum { id, args } => {
                let args = args.iter().map(|arg| self.intern_type(arg)).collect();
                self.intern_ty(Ty::Enum { id: *id, args })
            }
            Type::Reference { mutable, inner } => {
                let inner = self.intern_type(inner);
                self.intern_ty(Ty::Reference {
                    mutable: *mutable,
                    inner,
                })
            }
            Type::Pointer(inner) => {
                let inner = self.intern_type(inner);
                self.intern_ty(Ty::Pointer(inner))
            }
            Type::TypeVar(id) => self.intern_ty(Ty::TypeVar(*id)),
            Type::Generic(param) => self.intern_ty(Ty::Generic(*param)),
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                let ty = self.intern_type(ty);
                let trait_args = trait_args.iter().map(|arg| self.intern_type(arg)).collect();
                self.intern_ty(Ty::Projection {
                    ty,
                    trait_id: *trait_id,
                    assoc_type: *assoc_type,
                    trait_args,
                })
            }
            Type::Constructor { id, flavor } => self.intern_ty(Ty::Constructor {
                id: *id,
                flavor: *flavor,
            }),
            Type::Apply { constructor, args } => {
                let constructor = self.intern_type(constructor);
                let args = args.iter().map(|arg| self.intern_type(arg)).collect();
                self.intern_ty(Ty::Apply { constructor, args })
            }
            Type::Lambda { params, body } => {
                let body = self.intern_type(body);
                self.intern_ty(Ty::Lambda {
                    params: params.clone(),
                    body,
                })
            }
            Type::BoundVar { depth, index, kind } => self.intern_ty(Ty::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            }),
            Type::Error => self.intern_ty(Ty::Error),
        }
    }

    pub fn type_for(&self, id: TypeId) -> Type {
        match self.ty(id) {
            Ty::I8 => Type::I8,
            Ty::I16 => Type::I16,
            Ty::I32 => Type::I32,
            Ty::I64 => Type::I64,
            Ty::U8 => Type::U8,
            Ty::U16 => Type::U16,
            Ty::U32 => Type::U32,
            Ty::U64 => Type::U64,
            Ty::F32 => Type::F32,
            Ty::F64 => Type::F64,
            Ty::Bool => Type::Bool,
            Ty::Str => Type::Str,
            Ty::Char => Type::Char,
            Ty::Unit => Type::Unit,
            Ty::Never => Type::Never,
            Ty::Slice(inner) => Type::Slice(Box::new(self.type_for(*inner))),
            Ty::Array { inner, len } => Type::Array(Box::new(self.type_for(*inner)), *len),
            Ty::Tuple(elems) => {
                Type::Tuple(elems.iter().map(|elem| self.type_for(*elem)).collect())
            }
            Ty::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Type::Function {
                params: params.iter().map(|param| self.type_for(*param)).collect(),
                ret: Box::new(self.type_for(*ret)),
                safety: *safety,
                callable_kind: *callable_kind,
                captures: captures
                    .iter()
                    .map(|capture| FunctionCapture {
                        kind: capture.kind,
                        ty: self.type_for(capture.ty),
                    })
                    .collect(),
            },
            Ty::Struct { id, args } => Type::Struct {
                id: *id,
                args: args.iter().map(|arg| self.type_for(*arg)).collect(),
            },
            Ty::Enum { id, args } => Type::Enum {
                id: *id,
                args: args.iter().map(|arg| self.type_for(*arg)).collect(),
            },
            Ty::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.type_for(*inner)),
            },
            Ty::Pointer(inner) => Type::Pointer(Box::new(self.type_for(*inner))),
            Ty::TypeVar(id) => Type::TypeVar(*id),
            Ty::Generic(param) => Type::Generic(*param),
            Ty::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Type::Projection {
                ty: Box::new(self.type_for(*ty)),
                trait_id: *trait_id,
                assoc_type: *assoc_type,
                trait_args: trait_args.iter().map(|arg| self.type_for(*arg)).collect(),
            },
            Ty::Constructor { id, flavor } => Type::Constructor {
                id: *id,
                flavor: *flavor,
            },
            Ty::Apply { constructor, args } => Type::Apply {
                constructor: Box::new(self.type_for(*constructor)),
                args: args.iter().map(|arg| self.type_for(*arg)).collect(),
            },
            Ty::Lambda { params, body } => Type::Lambda {
                params: params.clone(),
                body: Box::new(self.type_for(*body)),
            },
            Ty::BoundVar { depth, index, kind } => Type::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
            Ty::Error => Type::Error,
        }
    }

    pub fn id_for_type(&self, ty: &Type) -> Option<TypeId> {
        let ty = self.ty_for_existing_type(ty)?;
        self.interned.get(&ty).copied()
    }

    fn ty_for_existing_type(&self, ty: &Type) -> Option<Ty> {
        Some(match ty {
            Type::I8 => Ty::I8,
            Type::I16 => Ty::I16,
            Type::I32 => Ty::I32,
            Type::I64 => Ty::I64,
            Type::U8 => Ty::U8,
            Type::U16 => Ty::U16,
            Type::U32 => Ty::U32,
            Type::U64 => Ty::U64,
            Type::F32 => Ty::F32,
            Type::F64 => Ty::F64,
            Type::Bool => Ty::Bool,
            Type::Str => Ty::Str,
            Type::Char => Ty::Char,
            Type::Unit => Ty::Unit,
            Type::Never => Ty::Never,
            Type::Slice(inner) => Ty::Slice(self.id_for_type(inner)?),
            Type::Array(inner, len) => Ty::Array {
                inner: self.id_for_type(inner)?,
                len: *len,
            },
            Type::Tuple(elems) => Ty::Tuple(
                elems
                    .iter()
                    .map(|elem| self.id_for_type(elem))
                    .collect::<Option<Vec<_>>>()?,
            ),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Ty::Function {
                params: params
                    .iter()
                    .map(|arg| self.id_for_type(arg))
                    .collect::<Option<Vec<_>>>()?,
                ret: self.id_for_type(ret)?,
                safety: *safety,
                callable_kind: *callable_kind,
                captures: captures
                    .iter()
                    .map(|capture| {
                        Some(TyFunctionCapture {
                            kind: capture.kind,
                            ty: self.id_for_type(&capture.ty)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            },
            Type::Struct { id, args } => Ty::Struct {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.id_for_type(arg))
                    .collect::<Option<Vec<_>>>()?,
            },
            Type::Enum { id, args } => Ty::Enum {
                id: *id,
                args: args
                    .iter()
                    .map(|arg| self.id_for_type(arg))
                    .collect::<Option<Vec<_>>>()?,
            },
            Type::Reference { mutable, inner } => Ty::Reference {
                mutable: *mutable,
                inner: self.id_for_type(inner)?,
            },
            Type::Pointer(inner) => Ty::Pointer(self.id_for_type(inner)?),
            Type::TypeVar(id) => Ty::TypeVar(*id),
            Type::Generic(param) => Ty::Generic(*param),
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Ty::Projection {
                ty: self.id_for_type(ty)?,
                trait_id: *trait_id,
                assoc_type: *assoc_type,
                trait_args: trait_args
                    .iter()
                    .map(|arg| self.id_for_type(arg))
                    .collect::<Option<Vec<_>>>()?,
            },
            Type::Constructor { id, flavor } => Ty::Constructor {
                id: *id,
                flavor: *flavor,
            },
            Type::Apply { constructor, args } => Ty::Apply {
                constructor: self.id_for_type(constructor)?,
                args: args
                    .iter()
                    .map(|arg| self.id_for_type(arg))
                    .collect::<Option<Vec<_>>>()?,
            },
            Type::Lambda { params, body } => Ty::Lambda {
                params: params.clone(),
                body: self.id_for_type(body)?,
            },
            Type::BoundVar { depth, index, kind } => Ty::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
            Type::Error => Ty::Error,
        })
    }

    pub fn intern_normalized_type(
        &mut self,
        ty: &Type,
        env: &crate::type_services::normalize::TypeNormalizationEnv,
    ) -> Result<TypeId, crate::type_services::normalize::NormalizeError> {
        let normalized = crate::type_services::normalize::TypeNormalizer::new(env).normalize(ty)?;
        self.normalization_env = env.clone();
        let previous = std::mem::replace(&mut self.accepting_normalized_hkt, true);
        let id = self.intern_type(&normalized);
        self.accepting_normalized_hkt = previous;
        Ok(id)
    }

    pub(crate) fn intern_canonical_type(&mut self, ty: &Type) -> TypeId {
        let previous = std::mem::replace(&mut self.accepting_normalized_hkt, true);
        let id = self.intern_type(ty);
        self.accepting_normalized_hkt = previous;
        id
    }

    pub fn validate_canonical_types(
        &self,
        env: &crate::type_services::normalize::TypeNormalizationEnv,
    ) -> Result<(), crate::type_services::normalize::NormalizeError> {
        for ty in &self.tys {
            let Some(id) = self.interned.get(ty).copied() else {
                continue;
            };
            crate::type_services::normalize::TypeNormalizer::new(env)
                .validate_canonical(&self.type_for(id))?;
        }
        Ok(())
    }

    pub fn substitute_generics(
        &mut self,
        id: TypeId,
        subst: &HashMap<GenericParamId, TypeId>,
    ) -> TypeId {
        struct TypeIdSubstituter<'a> {
            context: &'a TypeContext,
            subst: &'a HashMap<GenericParamId, TypeId>,
            visiting: Vec<GenericParamId>,
        }

        impl crate::type_services::visit::TypeFolder for TypeIdSubstituter<'_> {
            fn fold_type(&mut self, ty: Type) -> Type {
                let Type::Generic(param) = ty else {
                    return crate::type_services::visit::fold_type_children(ty, self);
                };

                if self.visiting.contains(&param) {
                    return Type::Generic(param);
                }
                let Some(replacement) = self.subst.get(&param).copied() else {
                    return Type::Generic(param);
                };

                self.visiting.push(param);
                let replacement = self.context.type_for(replacement);
                let substituted = self.fold_type(replacement);
                self.visiting.pop();
                substituted
            }
        }

        let substituted = {
            let ty = self.type_for(id);
            crate::type_services::visit::fold_type(
                ty,
                &mut TypeIdSubstituter {
                    context: self,
                    subst,
                    visiting: Vec::new(),
                },
            )
        };
        let env = self.normalization_env.clone();
        self.intern_normalized_type(&substituted, &env)
            .expect("TypeId substitution must preserve canonical kind-correct types")
    }
}

#[cfg(test)]
mod tests;
