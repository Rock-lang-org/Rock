use std::collections::HashMap;

use crate::ids::DefId;
use crate::type_services::kind::Kind;
use crate::types::{AssociatedTypeKey, NominalTypeKind, Type};

pub const DEFAULT_MAX_NORMALIZATION_DEPTH: usize = 256;
pub const DEFAULT_MAX_NORMALIZATION_NODES: usize = 4_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizationLimits {
    pub max_depth: usize,
    pub max_nodes: usize,
}

impl Default for NormalizationLimits {
    fn default() -> Self {
        Self {
            max_depth: DEFAULT_MAX_NORMALIZATION_DEPTH,
            max_nodes: DEFAULT_MAX_NORMALIZATION_NODES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    AliasCycle(Vec<DefId>),
    DepthLimit { limit: usize },
    NodeLimit { limit: usize },
    UnknownConstructor(DefId),
    KindMismatch { expected: Kind, actual: Kind },
    NotApplicable { kind: Kind },
    NonCanonicalType { normalized: Type },
}

impl std::fmt::Display for NormalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AliasCycle(ids) => write!(f, "type alias cycle involving {ids:?}"),
            Self::DepthLimit { limit } => {
                write!(f, "type normalization depth limit exceeded ({limit})")
            }
            Self::NodeLimit { limit } => {
                write!(f, "type normalization node limit exceeded ({limit})")
            }
            Self::UnknownConstructor(id) => write!(f, "unknown type constructor {id:?}"),
            Self::KindMismatch {
                expected: Kind::Usize,
                actual,
            } => write!(
                f,
                "kind mismatch: expected a `usize` constant such as `3`, found a type of kind {actual}"
            ),
            Self::KindMismatch {
                expected,
                actual: Kind::Usize,
            } => write!(
                f,
                "kind mismatch: expected a type of kind {expected}, found a `usize` constant"
            ),
            Self::KindMismatch { expected, actual } => {
                write!(f, "kind mismatch: expected {expected}, found {actual}")
            }
            Self::NotApplicable { kind } => write!(f, "type of kind {kind} is not applicable"),
            Self::NonCanonicalType { normalized } => {
                write!(f, "type is not canonical; normalized form is {normalized}")
            }
        }
    }
}

impl std::error::Error for NormalizeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionArg {
    Type(Type),
    Hole(Kind),
}

#[derive(Debug, Clone)]
struct ConstructorInfo {
    flavor: NominalTypeKind,
    kind: Kind,
}

#[derive(Debug, Clone)]
struct AliasInfo {
    body: Type,
}

#[derive(Debug, Clone, Default)]
pub struct TypeNormalizationEnv {
    constructors: HashMap<DefId, ConstructorInfo>,
    aliases: HashMap<DefId, AliasInfo>,
    generic_kinds: HashMap<crate::types::GenericParamId, Kind>,
    inference_kinds: HashMap<crate::ids::TypeVarId, Kind>,
    projection_kinds: HashMap<AssociatedTypeKey, Kind>,
}

impl TypeNormalizationEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_nominal(&mut self, id: DefId, flavor: NominalTypeKind, arity: usize) {
        let mut kind = Kind::Type;
        for _ in 0..arity {
            kind = Kind::arrow(Kind::Type, kind);
        }
        self.register_constructor(id, flavor, kind);
    }

    pub fn register_constructor(&mut self, id: DefId, flavor: NominalTypeKind, kind: Kind) {
        self.constructors
            .insert(id, ConstructorInfo { flavor, kind });
    }

    pub fn register_alias(&mut self, id: DefId, kind: Kind, body: Type) {
        self.constructors.insert(
            id,
            ConstructorInfo {
                flavor: NominalTypeKind::Alias,
                kind: kind.clone(),
            },
        );
        self.aliases.insert(id, AliasInfo { body });
    }

    pub fn register_generic_kind(&mut self, id: crate::types::GenericParamId, kind: Kind) {
        self.generic_kinds.insert(id, kind);
    }

    pub fn register_inference_kind(&mut self, id: crate::ids::TypeVarId, kind: Kind) {
        self.inference_kinds.insert(id, kind);
    }

    pub fn register_projection_kind(&mut self, key: AssociatedTypeKey, kind: Kind) {
        self.projection_kinds.insert(key, kind);
    }

    fn constructor(&self, id: DefId) -> Option<&ConstructorInfo> {
        self.constructors.get(&id)
    }

    fn alias(&self, id: DefId) -> Option<&AliasInfo> {
        self.aliases.get(&id)
    }
}

pub struct TypeNormalizer<'a> {
    env: &'a TypeNormalizationEnv,
    limits: NormalizationLimits,
    nodes: usize,
    alias_stack: Vec<DefId>,
}

impl<'a> TypeNormalizer<'a> {
    pub fn new(env: &'a TypeNormalizationEnv) -> Self {
        Self::with_limits(env, NormalizationLimits::default())
    }

    pub fn with_limits(env: &'a TypeNormalizationEnv, limits: NormalizationLimits) -> Self {
        Self {
            env,
            limits,
            nodes: 0,
            alias_stack: Vec::new(),
        }
    }

    pub fn normalize(mut self, ty: &Type) -> Result<Type, NormalizeError> {
        self.normalize_inner(ty.clone(), 0)
    }

    pub fn validate_canonical(mut self, ty: &Type) -> Result<(), NormalizeError> {
        let normalized = self.normalize_inner(ty.clone(), 0)?;
        if normalized == *ty {
            Ok(())
        } else {
            Err(NormalizeError::NonCanonicalType { normalized })
        }
    }

    pub fn kind_of(&self, ty: &Type) -> Result<Kind, NormalizeError> {
        self.kind_of_inner(ty)
    }

    pub fn desugar_section(
        mut self,
        constructor: Type,
        args: Vec<SectionArg>,
    ) -> Result<Type, NormalizeError> {
        let mut params = Vec::new();
        let mut applied_args = Vec::with_capacity(args.len());
        for arg in args {
            match arg {
                SectionArg::Type(ty) => applied_args.push(ty),
                SectionArg::Hole(kind) => {
                    let index = params.len() as u32;
                    params.push(kind.clone());
                    applied_args.push(Type::BoundVar {
                        depth: 0,
                        index,
                        kind,
                    });
                }
            }
        }
        let body = Type::Apply {
            constructor: Box::new(constructor),
            args: applied_args,
        };
        self.normalize_inner(
            if params.is_empty() {
                body
            } else {
                Type::Lambda {
                    params,
                    body: Box::new(body),
                }
            },
            0,
        )
    }

    fn normalize_inner(&mut self, ty: Type, depth: usize) -> Result<Type, NormalizeError> {
        self.bump(depth)?;
        match ty {
            Type::Constructor { id, flavor } => self.normalize_constructor(id, flavor, depth),
            Type::Apply { constructor, args } => {
                self.normalize_application(*constructor, args, depth)
            }
            Type::Lambda { params, body } => {
                let body = self.normalize_inner(*body, depth + 1)?;
                let lambda = Type::Lambda {
                    params,
                    body: Box::new(body),
                };
                if let Some(reduced) = eta_reduce(lambda.clone()) {
                    self.normalize_inner(reduced, depth + 1)
                } else {
                    Ok(lambda)
                }
            }
            Type::Slice(inner) => Ok(Type::Slice(Box::new(
                self.normalize_inner(*inner, depth + 1)?,
            ))),
            Type::Array(inner, len) => Ok(Type::Array(
                Box::new(self.normalize_inner(*inner, depth + 1)?),
                len,
            )),
            Type::Tuple(elements) => Ok(Type::Tuple(
                elements
                    .into_iter()
                    .map(|ty| self.normalize_inner(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            )),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => Ok(Type::Function {
                params: params
                    .into_iter()
                    .map(|ty| self.normalize_inner(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
                ret: Box::new(self.normalize_inner(*ret, depth + 1)?),
                safety,
                callable_kind,
                captures: captures
                    .into_iter()
                    .map(|mut capture| {
                        capture.ty = self.normalize_inner(capture.ty, depth + 1)?;
                        Ok(capture)
                    })
                    .collect::<Result<_, NormalizeError>>()?,
            }),
            Type::Struct { id, args } => Ok(Type::Struct {
                id,
                args: args
                    .into_iter()
                    .map(|ty| self.normalize_inner(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            }),
            Type::Enum { id, args } => Ok(Type::Enum {
                id,
                args: args
                    .into_iter()
                    .map(|ty| self.normalize_inner(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            }),
            Type::Reference { mutable, inner } => Ok(Type::Reference {
                mutable,
                inner: Box::new(self.normalize_inner(*inner, depth + 1)?),
            }),
            Type::Pointer(inner) => Ok(Type::Pointer(Box::new(
                self.normalize_inner(*inner, depth + 1)?,
            ))),
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Ok(Type::Projection {
                ty: Box::new(self.normalize_inner(*ty, depth + 1)?),
                trait_id,
                assoc_type,
                trait_args: trait_args
                    .into_iter()
                    .map(|ty| self.normalize_inner(ty, depth + 1))
                    .collect::<Result<_, _>>()?,
            }),
            leaf => Ok(leaf),
        }
    }

    fn normalize_constructor(
        &mut self,
        id: DefId,
        flavor: NominalTypeKind,
        depth: usize,
    ) -> Result<Type, NormalizeError> {
        let info = self
            .env
            .constructor(id)
            .ok_or(NormalizeError::UnknownConstructor(id))?;
        if info.flavor != flavor {
            return Err(NormalizeError::UnknownConstructor(id));
        }
        let constructor_kind = info.kind.clone();
        let Some(alias) = self.env.alias(id) else {
            return match (&constructor_kind, flavor) {
                (Kind::Type, NominalTypeKind::Struct) => Ok(Type::Struct {
                    id,
                    args: Vec::new(),
                }),
                (Kind::Type, NominalTypeKind::Enum) => Ok(Type::Enum {
                    id,
                    args: Vec::new(),
                }),
                _ => Ok(Type::Constructor { id, flavor }),
            };
        };
        if let Some(start) = self.alias_stack.iter().position(|active| *active == id) {
            let mut cycle = self.alias_stack[start..].to_vec();
            cycle.push(id);
            return Err(NormalizeError::AliasCycle(cycle));
        }
        self.alias_stack.push(id);
        let body = alias.body.clone();
        let expected_kind = constructor_kind;
        let normalized = self.normalize_inner(body, depth + 1);
        self.alias_stack.pop();
        let normalized = normalized?;
        let actual_kind = self.kind_of_inner(&normalized)?;
        if actual_kind != expected_kind {
            return Err(NormalizeError::KindMismatch {
                expected: expected_kind,
                actual: actual_kind,
            });
        }
        Ok(normalized)
    }

    fn normalize_application(
        &mut self,
        constructor: Type,
        args: Vec<Type>,
        depth: usize,
    ) -> Result<Type, NormalizeError> {
        let mut constructor = self.normalize_inner(constructor, depth + 1)?;
        let mut args = args
            .into_iter()
            .map(|arg| self.normalize_inner(arg, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;

        if let Type::Apply {
            constructor: nested,
            args: mut nested_args,
        } = constructor
        {
            nested_args.append(&mut args);
            constructor = *nested;
            args = nested_args;
        }

        if let Type::Lambda { params, body } = constructor {
            return self.apply_lambda(params, *body, args, depth + 1);
        }

        let result_kind = self.apply_kinds(&constructor, &args)?;
        if matches!(result_kind, Kind::Type) {
            if let Type::Constructor { id, flavor } = constructor {
                return match flavor {
                    NominalTypeKind::Struct => Ok(Type::Struct { id, args }),
                    NominalTypeKind::Enum => Ok(Type::Enum { id, args }),
                    NominalTypeKind::Alias => Err(NormalizeError::UnknownConstructor(id)),
                };
            }
        }

        Ok(Type::Apply {
            constructor: Box::new(constructor),
            args,
        })
    }

    fn apply_lambda(
        &mut self,
        params: Vec<Kind>,
        body: Type,
        args: Vec<Type>,
        depth: usize,
    ) -> Result<Type, NormalizeError> {
        let consumed = params.len().min(args.len());
        for (expected, arg) in params.iter().zip(args.iter()).take(consumed) {
            let actual = self.kind_of_inner(arg)?;
            if actual != *expected {
                return Err(NormalizeError::KindMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
        }

        let removes_group = consumed == params.len();
        let body = substitute_lambda_group(&body, &args[..consumed], consumed, removes_group, 0);
        let mut reduced = if removes_group {
            body
        } else {
            Type::Lambda {
                params: params[consumed..].to_vec(),
                body: Box::new(body),
            }
        };
        reduced = self.normalize_inner(reduced, depth + 1)?;

        if args.len() > consumed {
            self.normalize_application(reduced, args[consumed..].to_vec(), depth + 1)
        } else {
            Ok(reduced)
        }
    }

    fn apply_kinds(&self, constructor: &Type, args: &[Type]) -> Result<Kind, NormalizeError> {
        let mut kind = self.kind_of_inner(constructor)?;
        for arg in args {
            let Kind::Arrow(expected, output) = kind else {
                return Err(NormalizeError::NotApplicable { kind });
            };
            let actual = self.kind_of_inner(arg)?;
            if actual != *expected {
                return Err(NormalizeError::KindMismatch {
                    expected: *expected,
                    actual,
                });
            }
            kind = *output;
        }
        Ok(kind)
    }

    fn kind_of_inner(&self, ty: &Type) -> Result<Kind, NormalizeError> {
        match ty {
            Type::Constructor { id, flavor } => self
                .env
                .constructor(*id)
                .filter(|info| info.flavor == *flavor || info.flavor == NominalTypeKind::Alias)
                .map(|info| info.kind.clone())
                .ok_or(NormalizeError::UnknownConstructor(*id)),
            Type::Apply { constructor, args } => self.apply_kinds(constructor, args),
            Type::Lambda { params, body } => {
                let mut kind = self.kind_of_inner(body)?;
                for param in params.iter().rev() {
                    kind = Kind::arrow(param.clone(), kind);
                }
                Ok(kind)
            }
            Type::BoundVar { kind, .. } => Ok(kind.clone()),
            Type::ConstValue(_) => Ok(Kind::Usize),
            Type::Generic(id) => Ok(self
                .env
                .generic_kinds
                .get(id)
                .cloned()
                .unwrap_or(Kind::Type)),
            Type::TypeVar(id) => Ok(self
                .env
                .inference_kinds
                .get(id)
                .cloned()
                .unwrap_or(Kind::Type)),
            Type::Projection { assoc_type, .. } => Ok(self
                .env
                .projection_kinds
                .get(assoc_type)
                .cloned()
                .unwrap_or(Kind::Type)),
            _ => Ok(Kind::Type),
        }
    }

    fn bump(&mut self, depth: usize) -> Result<(), NormalizeError> {
        if depth > self.limits.max_depth {
            return Err(NormalizeError::DepthLimit {
                limit: self.limits.max_depth,
            });
        }
        self.nodes = self.nodes.saturating_add(1);
        if self.nodes > self.limits.max_nodes {
            return Err(NormalizeError::NodeLimit {
                limit: self.limits.max_nodes,
            });
        }
        Ok(())
    }
}

fn substitute_lambda_group(
    ty: &Type,
    args: &[Type],
    consumed: usize,
    removes_group: bool,
    nested_depth: u32,
) -> Type {
    match ty {
        Type::BoundVar { depth, index, kind } if *depth == nested_depth => {
            if let Some(replacement) = args.get(*index as usize) {
                shift_bound_depths(replacement, nested_depth as i32, 0)
            } else {
                Type::BoundVar {
                    depth: *depth,
                    index: index.saturating_sub(consumed as u32),
                    kind: kind.clone(),
                }
            }
        }
        Type::BoundVar { depth, index, kind } if removes_group && *depth > nested_depth => {
            Type::BoundVar {
                depth: depth - 1,
                index: *index,
                kind: kind.clone(),
            }
        }
        Type::Lambda { params, body } => Type::Lambda {
            params: params.clone(),
            body: Box::new(substitute_lambda_group(
                body,
                args,
                consumed,
                removes_group,
                nested_depth + 1,
            )),
        },
        other => map_type_children(other, &mut |child| {
            substitute_lambda_group(child, args, consumed, removes_group, nested_depth)
        }),
    }
}

pub(crate) fn apply_type_lambda(ty: &Type, args: &[Type]) -> Option<Type> {
    let Type::Lambda { params, body } = ty else {
        return None;
    };
    if params.len() != args.len() {
        return None;
    }
    Some(substitute_lambda_group(body, args, args.len(), true, 0))
}

fn shift_bound_depths(ty: &Type, amount: i32, cutoff: u32) -> Type {
    match ty {
        Type::BoundVar { depth, index, kind } if *depth >= cutoff => Type::BoundVar {
            depth: ((*depth as i64) + amount as i64) as u32,
            index: *index,
            kind: kind.clone(),
        },
        Type::Lambda { params, body } => Type::Lambda {
            params: params.clone(),
            body: Box::new(shift_bound_depths(body, amount, cutoff + 1)),
        },
        other => map_type_children(other, &mut |child| {
            shift_bound_depths(child, amount, cutoff)
        }),
    }
}

fn eta_reduce(ty: Type) -> Option<Type> {
    let Type::Lambda { params, body } = ty else {
        return None;
    };
    let param_count = params.len();
    if param_count == 0 {
        return Some(*body);
    }

    match *body {
        Type::Apply {
            constructor,
            mut args,
        } if args.len() >= param_count => {
            let trailing = &args[args.len() - param_count..];
            if !matches_own_binders(trailing, &params)
                || contains_bound_group(&constructor, 0)
                || args[..args.len() - param_count]
                    .iter()
                    .any(|arg| contains_bound_group(arg, 0))
            {
                return None;
            }
            args.truncate(args.len() - param_count);
            let head = remove_bound_group(&constructor, 0);
            if args.is_empty() {
                Some(head)
            } else {
                Some(Type::Apply {
                    constructor: Box::new(head),
                    args: args.iter().map(|arg| remove_bound_group(arg, 0)).collect(),
                })
            }
        }
        Type::Struct { id, args }
            if args.len() == param_count && matches_own_binders(&args, &params) =>
        {
            Some(Type::Constructor {
                id,
                flavor: NominalTypeKind::Struct,
            })
        }
        Type::Enum { id, args }
            if args.len() == param_count && matches_own_binders(&args, &params) =>
        {
            Some(Type::Constructor {
                id,
                flavor: NominalTypeKind::Enum,
            })
        }
        _ => None,
    }
}

fn matches_own_binders(args: &[Type], params: &[Kind]) -> bool {
    args.iter()
        .zip(params)
        .enumerate()
        .all(|(index, (arg, kind))| {
            matches!(
                arg,
                Type::BoundVar {
                    depth: 0,
                    index: arg_index,
                    kind: arg_kind,
                } if *arg_index == index as u32 && arg_kind == kind
            )
        })
}

fn contains_bound_group(ty: &Type, nested_depth: u32) -> bool {
    match ty {
        Type::BoundVar { depth, .. } => *depth == nested_depth,
        Type::Lambda { body, .. } => contains_bound_group(body, nested_depth + 1),
        other => type_children(other)
            .into_iter()
            .any(|child| contains_bound_group(child, nested_depth)),
    }
}

fn remove_bound_group(ty: &Type, nested_depth: u32) -> Type {
    match ty {
        Type::BoundVar { depth, index, kind } if *depth > nested_depth => Type::BoundVar {
            depth: depth - 1,
            index: *index,
            kind: kind.clone(),
        },
        Type::Lambda { params, body } => Type::Lambda {
            params: params.clone(),
            body: Box::new(remove_bound_group(body, nested_depth + 1)),
        },
        other => map_type_children(other, &mut |child| remove_bound_group(child, nested_depth)),
    }
}

fn type_children(ty: &Type) -> Vec<&Type> {
    match ty {
        Type::Slice(inner)
        | Type::Array(inner, _)
        | Type::Reference { inner, .. }
        | Type::Pointer(inner) => vec![inner],
        Type::Tuple(elements) => elements.iter().collect(),
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => params
            .iter()
            .chain(std::iter::once(ret.as_ref()))
            .chain(captures.iter().map(|capture| &capture.ty))
            .collect(),
        Type::Struct { args, .. } | Type::Enum { args, .. } => args.iter().collect(),
        Type::Projection { ty, trait_args, .. } => std::iter::once(ty.as_ref())
            .chain(trait_args.iter())
            .collect(),
        Type::Apply { constructor, args } => std::iter::once(constructor.as_ref())
            .chain(args.iter())
            .collect(),
        Type::Lambda { body, .. } => vec![body],
        _ => Vec::new(),
    }
}

fn map_type_children(ty: &Type, map: &mut impl FnMut(&Type) -> Type) -> Type {
    match ty {
        Type::Slice(inner) => Type::Slice(Box::new(map(inner))),
        Type::Array(inner, len) => Type::Array(Box::new(map(inner)), *len),
        Type::Tuple(elements) => Type::Tuple(elements.iter().map(&mut *map).collect()),
        Type::Function {
            params,
            ret,
            safety,
            callable_kind,
            captures,
        } => Type::Function {
            params: params.iter().map(&mut *map).collect(),
            ret: Box::new(map(ret)),
            safety: *safety,
            callable_kind: *callable_kind,
            captures: captures
                .iter()
                .cloned()
                .map(|mut capture| {
                    capture.ty = map(&capture.ty);
                    capture
                })
                .collect(),
        },
        Type::Struct { id, args } => Type::Struct {
            id: *id,
            args: args.iter().map(&mut *map).collect(),
        },
        Type::Enum { id, args } => Type::Enum {
            id: *id,
            args: args.iter().map(&mut *map).collect(),
        },
        Type::Reference { mutable, inner } => Type::Reference {
            mutable: *mutable,
            inner: Box::new(map(inner)),
        },
        Type::Pointer(inner) => Type::Pointer(Box::new(map(inner))),
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => Type::Projection {
            ty: Box::new(map(ty)),
            trait_id: *trait_id,
            assoc_type: *assoc_type,
            trait_args: trait_args.iter().map(&mut *map).collect(),
        },
        Type::Apply { constructor, args } => Type::Apply {
            constructor: Box::new(map(constructor)),
            args: args.iter().map(&mut *map).collect(),
        },
        Type::Lambda { params, body } => Type::Lambda {
            params: params.clone(),
            body: Box::new(map(body)),
        },
        leaf => leaf.clone(),
    }
}

#[cfg(test)]
mod tests;
