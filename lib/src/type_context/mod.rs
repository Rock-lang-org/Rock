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
    Object(Box<crate::types::ObjectType<TypeId>>),
    ObjectSelf {
        depth: u32,
    },
    Witness(crate::types::WitnessId),
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
                Ty::Object(object) => object
                    .children()
                    .all(|child| visit(context, *child, visiting, valid)),
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
                | Ty::ObjectSelf { .. }
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
            Type::ObjectSelf { depth } => self.intern_ty(Ty::ObjectSelf { depth: *depth }),
            Type::Witness(witness) => self.intern_ty(Ty::Witness(*witness)),
            Type::Object(object) => {
                let object = object.map_types(|ty| self.intern_type(ty));
                self.intern_ty(Ty::Object(Box::new(object)))
            }
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
            Ty::ObjectSelf { depth } => Type::ObjectSelf { depth: *depth },
            Ty::Witness(witness) => Type::Witness(*witness),
            Ty::Object(object) => Type::Object(Box::new(object.map_types(|id| self.type_for(*id)))),
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
            Type::ObjectSelf { depth } => Ty::ObjectSelf { depth: *depth },
            Type::Witness(witness) => Ty::Witness(*witness),
            Type::Object(object) => Ty::Object(Box::new(
                object
                    .try_map_types(|ty| self.id_for_type(ty).ok_or(()))
                    .ok()?,
            )),
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
mod tests {
    use super::*;
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
    use crate::type_context::TypeView;
    use crate::type_services::kind::Kind;
    use crate::type_services::normalize::{
        NormalizationLimits, NormalizeError, TypeNormalizationEnv, TypeNormalizer,
    };
    use crate::types::{
        AssociatedTypeKey, CallableKind, CaptureKind, FunctionCapture, GenericParamId,
        NominalTypeKind, Type,
    };

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn generic(owner: DefId, index: u32) -> GenericParamId {
        GenericParamId { owner, index }
    }

    fn assoc_type(owner: DefId, index: u32) -> AssociatedTypeKey {
        AssociatedTypeKey {
            owner,
            assoc_type_id: AssocTypeId(index),
        }
    }

    #[test]
    fn normalized_interning_uses_one_canonical_type_id() {
        let option = def_id(40);
        let mut env = TypeNormalizationEnv::new();
        env.register_nominal(option, NominalTypeKind::Enum, 1);
        let constructor = Type::Constructor {
            id: option,
            flavor: NominalTypeKind::Enum,
        };
        let eta_equivalent = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Apply {
                constructor: Box::new(constructor.clone()),
                args: vec![Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                }],
            }),
        };
        let mut context = TypeContext::new();

        let constructor_id = context.intern_normalized_type(&constructor, &env).unwrap();
        let eta_id = context
            .intern_normalized_type(&eta_equivalent, &env)
            .unwrap();

        assert_eq!(constructor_id, eta_id);

        let generic = generic(def_id(43), 0);
        env.register_generic_kind(generic, Kind::arrow(Kind::Type, Kind::Type));
        let abstract_application = Type::Apply {
            constructor: Box::new(Type::Generic(generic)),
            args: vec![Type::I64],
        };
        let application_id = context
            .intern_normalized_type(&abstract_application, &env)
            .unwrap();
        assert_eq!(context.type_for(application_id), abstract_application);

        let lambda = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Tuple(vec![
                Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                },
                Type::I64,
            ])),
        };
        let lambda_id = context.intern_normalized_type(&lambda, &env).unwrap();
        assert_eq!(context.type_for(lambda_id), lambda);
        context.validate_canonical_types(&env).unwrap();
    }

    #[test]
    fn constructor_type_ids_retain_kinds_without_becoming_runtime_layouts() {
        let option = def_id(44);
        let unary = Kind::arrow(Kind::Type, Kind::Type);
        let mut env = TypeNormalizationEnv::new();
        env.register_constructor(option, NominalTypeKind::Enum, unary.clone());
        let constructor = Type::Constructor {
            id: option,
            flavor: NominalTypeKind::Enum,
        };
        let mut context = TypeContext::new();

        let constructor_id = context.intern_normalized_type(&constructor, &env).unwrap();
        let applied_id = context
            .intern_normalized_type(
                &Type::Apply {
                    constructor: Box::new(constructor.clone()),
                    args: vec![Type::I64],
                },
                &env,
            )
            .unwrap();

        assert_eq!(context.kind(constructor_id), &unary);
        assert_eq!(context.kind(applied_id), &Kind::Type);
        assert_eq!(
            crate::type_services::layout::TypeLayout::validate_runtime_type(
                &context.type_for(constructor_id)
            ),
            Err(crate::type_services::layout::RuntimeTypeError::UnsaturatedConstructor)
        );
        assert_eq!(
            context.type_for(applied_id),
            Type::Enum {
                id: option,
                args: vec![Type::I64],
            }
        );
    }

    #[test]
    #[should_panic(expected = "higher-kinded terms must enter TypeContext")]
    fn raw_higher_kinded_terms_cannot_bypass_normalization() {
        let mut context = TypeContext::new();
        context.intern_type(&Type::Constructor {
            id: def_id(42),
            flavor: NominalTypeKind::Struct,
        });
    }

    #[test]
    fn normalization_failure_does_not_partially_insert_types() {
        let alias = def_id(41);
        let mut env = TypeNormalizationEnv::new();
        env.register_alias(
            alias,
            Kind::Type,
            Type::Constructor {
                id: alias,
                flavor: NominalTypeKind::Alias,
            },
        );
        let mut context = TypeContext::new();
        let before = context.len();
        let result = context.intern_normalized_type(
            &Type::Constructor {
                id: alias,
                flavor: NominalTypeKind::Alias,
            },
            &env,
        );

        assert!(matches!(result, Err(NormalizeError::AliasCycle(_))));
        assert_eq!(context.len(), before);

        let limited = NormalizationLimits {
            max_depth: 0,
            max_nodes: 1,
        };
        assert!(matches!(
            TypeNormalizer::with_limits(&TypeNormalizationEnv::new(), limited)
                .normalize(&Type::Slice(Box::new(Type::I64))),
            Err(NormalizeError::DepthLimit { .. }) | Err(NormalizeError::NodeLimit { .. })
        ));
        assert_eq!(context.len(), before);
    }

    #[test]
    fn type_view_reports_shape_without_structural_type_storage() {
        let mut context = TypeContext::new();
        let elem = context.intern_type(&Type::U8);
        let slice = context.intern_ty(Ty::Slice(elem));
        let shared_ref = context.intern_ty(Ty::Reference {
            mutable: false,
            inner: slice,
        });
        let view = TypeView::new(&context);

        assert!(view.is_slice_shape(slice));
        assert!(view.is_fat_pointer_shape(shared_ref));
        assert!(view.is_copy(shared_ref));
    }

    #[test]
    fn type_view_delegates_semantic_facts_to_type_facts() {
        let source = include_str!("view.rs");
        let impl_start = source.find("impl<'a> TypeView<'a>").unwrap();
        let impl_body = &source[impl_start..];

        assert!(impl_body.contains("TypeFacts::is_copy_id"));
        assert!(!impl_body.contains("Ty::I8"));
        assert!(!impl_body.contains("Ty::Array { inner"));
    }

    #[test]
    fn type_view_exposes_callable_metadata() {
        let mut context = TypeContext::new();
        let capture_ty = context.intern_type(&Type::I64);
        let function = context.intern_ty(Ty::Function {
            params: Vec::new(),
            ret: capture_ty,
            safety: FunctionSafety::Safe,
            callable_kind: CallableKind::FnMut,
            captures: vec![TyFunctionCapture {
                kind: CaptureKind::MutableBorrow,
                ty: capture_ty,
            }],
        });
        let view = TypeView::new(&context);

        assert_eq!(view.callable_kind(function), Some(CallableKind::FnMut));
        assert_eq!(view.captures(function).unwrap()[0].ty, capture_ty);
    }

    #[test]
    fn type_context_finds_previously_interned_structural_type_without_mutation() {
        let mut context = TypeContext::new();
        let ty = Type::function(vec![Type::I64], Type::Bool);
        let id = context.intern_type(&ty);

        assert_eq!(context.id_for_type(&ty), Some(id));
        assert_eq!(context.type_for(id), ty);
    }

    #[test]
    fn type_context_preserves_function_safety() {
        let mut context = TypeContext::new();
        let safe = Type::function(vec![Type::I64], Type::Bool);
        let unsafe_fn = Type::unsafe_function(vec![Type::I64], Type::Bool);

        let safe_id = context.intern_type(&safe);
        let unsafe_id = context.intern_type(&unsafe_fn);

        assert_ne!(safe_id, unsafe_id);
        assert_eq!(context.type_for(safe_id), safe);
        assert_eq!(context.type_for(unsafe_id), unsafe_fn);
        assert_eq!(context.id_for_type(&safe), Some(safe_id));
        assert_eq!(context.id_for_type(&unsafe_fn), Some(unsafe_id));
    }

    #[test]
    fn type_context_preserves_callable_metadata_and_capture_types() {
        let mut context = TypeContext::new();
        let ty = Type::function_with_metadata(
            vec![Type::I64],
            Type::Bool,
            FunctionSafety::Safe,
            CallableKind::FnMut,
            vec![FunctionCapture::new(CaptureKind::MutableBorrow, Type::Str)],
        );
        let id = context.intern_type(&ty);

        assert_eq!(context.type_for(id), ty);
        assert_eq!(context.id_for_type(&ty), Some(id));
    }

    #[test]
    fn type_interner_substitutes_generics_to_type_ids() {
        let owner = def_id(70);
        let generic_param = GenericParamId { owner, index: 0 };
        let mut context = TypeContext::new();
        let generic = context.intern_ty(Ty::Generic(generic_param));
        let replacement = context.intern_type(&Type::U64);
        let tuple = context.intern_ty(Ty::Tuple(vec![generic]));
        let mut subst = HashMap::new();
        subst.insert(generic_param, replacement);

        let substituted = context.substitute_generics(tuple, &subst);

        assert_eq!(context.type_for(substituted), Type::Tuple(vec![Type::U64]));
    }

    #[test]
    fn type_interner_substitutes_chained_generics_to_type_ids() {
        let owner = def_id(71);
        let first_param = GenericParamId { owner, index: 0 };
        let second_param = GenericParamId { owner, index: 1 };
        let mut context = TypeContext::new();
        let first = context.intern_ty(Ty::Generic(first_param));
        let second = context.intern_ty(Ty::Generic(second_param));
        let replacement = context.intern_type(&Type::U64);
        let mut subst = HashMap::new();
        subst.insert(first_param, second);
        subst.insert(second_param, replacement);

        let substituted = context.substitute_generics(first, &subst);

        assert_eq!(context.type_for(substituted), Type::U64);
    }

    #[test]
    fn type_interner_substitute_generics_breaks_cycles() {
        let owner = def_id(72);
        let first_param = GenericParamId { owner, index: 0 };
        let second_param = GenericParamId { owner, index: 1 };
        let mut context = TypeContext::new();
        let first = context.intern_ty(Ty::Generic(first_param));
        let second = context.intern_ty(Ty::Generic(second_param));
        let mut subst = HashMap::new();
        subst.insert(first_param, second);
        subst.insert(second_param, first);

        let substituted = context.substitute_generics(first, &subst);

        assert_eq!(context.type_for(substituted), Type::Generic(first_param));
    }

    #[test]
    fn type_interner_does_not_resolve_unused_substitution_ids() {
        let owner = def_id(73);
        let unused_param = GenericParamId { owner, index: 0 };
        let mut context = TypeContext::new();
        let concrete = context.intern_type(&Type::I64);
        let mut subst = HashMap::new();
        subst.insert(unused_param, TypeId(u32::MAX));

        let substituted = context.substitute_generics(concrete, &subst);

        assert_eq!(substituted, concrete);
    }

    #[test]
    fn type_context_interns_same_primitive_once() {
        let mut context = TypeContext::new();

        let first = context.intern_ty(Ty::I64);
        let second = context.intern_ty(Ty::I64);
        let different = context.intern_ty(Ty::Bool);

        assert_eq!(first, second);
        assert_ne!(first, different);
        assert_eq!(context.ty(first), &Ty::I64);
    }

    #[test]
    fn type_context_roundtrips_structural_types() {
        let mut context = TypeContext::new();
        let owner = def_id(30);
        let generic_param = generic(owner, 0);
        let projection_trait = def_id(31);

        let cases = vec![
            Type::I8,
            Type::I16,
            Type::I32,
            Type::I64,
            Type::U8,
            Type::U16,
            Type::U32,
            Type::U64,
            Type::F32,
            Type::F64,
            Type::Bool,
            Type::Str,
            Type::Char,
            Type::Unit,
            Type::Never,
            Type::Error,
            Type::Slice(Box::new(Type::I64)),
            Type::Array(Box::new(Type::Bool), 4),
            Type::Tuple(vec![Type::I64, Type::Bool]),
            Type::function(vec![Type::I64, Type::Bool], Type::Unit),
            Type::Struct {
                id: def_id(32),
                args: vec![Type::Generic(generic_param)],
            },
            Type::Enum {
                id: def_id(33),
                args: vec![Type::I64],
            },
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            },
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
            Type::Pointer(Box::new(Type::I64)),
            Type::TypeVar(TypeVarId(7)),
            Type::Generic(generic_param),
            Type::Projection {
                ty: Box::new(Type::Struct {
                    id: def_id(34),
                    args: vec![Type::Generic(generic_param)],
                }),
                trait_id: projection_trait,
                assoc_type: assoc_type(projection_trait, 0),
                trait_args: vec![Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Generic(generic_param)),
                }],
            },
        ];

        for ty in cases {
            let id = context.intern_type(&ty);
            let roundtrip = context.type_for(id);

            assert_eq!(roundtrip, ty);
            assert_eq!(context.intern_type(&roundtrip), id);
        }
    }

    #[test]
    fn type_context_intern_type_reuses_equal_structural_types() {
        let mut context = TypeContext::new();
        let first = Type::function(vec![Type::I64], Type::Bool);
        let second = Type::function(vec![Type::I64], Type::Bool);
        let different = Type::function(vec![Type::Bool], Type::Bool);

        let first_id = context.intern_type(&first);
        let second_id = context.intern_type(&second);
        let different_id = context.intern_type(&different);

        assert_eq!(first_id, second_id);
        assert_ne!(first_id, different_id);
    }

    #[test]
    fn type_context_keeps_type_var_ids_distinct_from_type_ids() {
        let mut context = TypeContext::new();

        let first = context.intern_type(&Type::TypeVar(TypeVarId(0)));
        let same = context.intern_type(&Type::TypeVar(TypeVarId(0)));
        let different = context.intern_type(&Type::TypeVar(TypeVarId(1)));

        assert_eq!(first, same);
        assert_ne!(first, different);
        assert_eq!(context.type_for(first), Type::TypeVar(TypeVarId(0)));
    }

    #[test]
    fn type_context_interns_equal_composite_tys() {
        let mut context = TypeContext::new();
        let i64_ty = context.intern_ty(Ty::I64);
        let bool_ty = context.intern_ty(Ty::Bool);

        assert_eq!(
            context.intern_ty(Ty::Slice(i64_ty)),
            context.intern_ty(Ty::Slice(i64_ty))
        );
        assert_eq!(
            context.intern_ty(Ty::Array {
                inner: i64_ty,
                len: 4,
            }),
            context.intern_ty(Ty::Array {
                inner: i64_ty,
                len: 4,
            })
        );
        assert_ne!(
            context.intern_ty(Ty::Array {
                inner: i64_ty,
                len: 4,
            }),
            context.intern_ty(Ty::Array {
                inner: i64_ty,
                len: 8,
            })
        );
        assert_eq!(
            context.intern_ty(Ty::Tuple(vec![i64_ty, bool_ty])),
            context.intern_ty(Ty::Tuple(vec![i64_ty, bool_ty]))
        );
        assert_eq!(
            context.intern_ty(Ty::Function {
                params: vec![i64_ty],
                ret: bool_ty,
                safety: FunctionSafety::Safe,
                callable_kind: CallableKind::Fn,
                captures: Vec::new(),
            }),
            context.intern_ty(Ty::Function {
                params: vec![i64_ty],
                ret: bool_ty,
                safety: FunctionSafety::Safe,
                callable_kind: CallableKind::Fn,
                captures: Vec::new(),
            })
        );
        assert_eq!(
            context.intern_ty(Ty::Reference {
                mutable: false,
                inner: i64_ty,
            }),
            context.intern_ty(Ty::Reference {
                mutable: false,
                inner: i64_ty,
            })
        );
        assert_ne!(
            context.intern_ty(Ty::Reference {
                mutable: false,
                inner: i64_ty,
            }),
            context.intern_ty(Ty::Reference {
                mutable: true,
                inner: i64_ty,
            })
        );
        assert_eq!(
            context.intern_ty(Ty::Pointer(i64_ty)),
            context.intern_ty(Ty::Pointer(i64_ty))
        );
    }

    #[test]
    fn type_context_uses_identities_for_nominal_generic_projection_and_type_vars() {
        let mut context = TypeContext::new();
        let owner = def_id(1);
        let other_owner = def_id(2);

        let first_generic = context.intern_ty(Ty::Generic(generic(owner, 0)));
        let same_generic = context.intern_ty(Ty::Generic(generic(owner, 0)));
        let different_owner_generic = context.intern_ty(Ty::Generic(generic(other_owner, 0)));
        let different_index_generic = context.intern_ty(Ty::Generic(generic(owner, 1)));

        assert_eq!(first_generic, same_generic);
        assert_ne!(first_generic, different_owner_generic);
        assert_ne!(first_generic, different_index_generic);

        let type_var = context.intern_ty(Ty::TypeVar(TypeVarId(7)));
        let same_type_var = context.intern_ty(Ty::TypeVar(TypeVarId(7)));
        let different_type_var = context.intern_ty(Ty::TypeVar(TypeVarId(8)));

        assert_eq!(type_var, same_type_var);
        assert_ne!(type_var, different_type_var);

        assert_ne!(
            context.intern_ty(Ty::Struct {
                id: def_id(10),
                args: vec![first_generic],
            }),
            context.intern_ty(Ty::Struct {
                id: def_id(11),
                args: vec![first_generic],
            })
        );
        assert_ne!(
            context.intern_ty(Ty::Enum {
                id: def_id(12),
                args: vec![first_generic],
            }),
            context.intern_ty(Ty::Enum {
                id: def_id(13),
                args: vec![first_generic],
            })
        );

        let trait_id = def_id(20);
        let projection = context.intern_ty(Ty::Projection {
            ty: first_generic,
            trait_id,
            assoc_type: assoc_type(trait_id, 0),
            trait_args: vec![first_generic],
        });

        assert_eq!(
            projection,
            context.intern_ty(Ty::Projection {
                ty: first_generic,
                trait_id,
                assoc_type: assoc_type(trait_id, 0),
                trait_args: vec![first_generic],
            })
        );
        assert_ne!(
            projection,
            context.intern_ty(Ty::Projection {
                ty: different_owner_generic,
                trait_id,
                assoc_type: assoc_type(trait_id, 0),
                trait_args: vec![first_generic],
            })
        );
        assert_ne!(
            projection,
            context.intern_ty(Ty::Projection {
                ty: first_generic,
                trait_id: def_id(21),
                assoc_type: assoc_type(def_id(21), 0),
                trait_args: vec![first_generic],
            })
        );
        assert_ne!(
            projection,
            context.intern_ty(Ty::Projection {
                ty: first_generic,
                trait_id,
                assoc_type: assoc_type(trait_id, 1),
                trait_args: vec![first_generic],
            })
        );
        assert_ne!(
            projection,
            context.intern_ty(Ty::Projection {
                ty: first_generic,
                trait_id,
                assoc_type: assoc_type(trait_id, 0),
                trait_args: vec![different_index_generic],
            })
        );
    }
}
