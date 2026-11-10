use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{AssocTypeId, DefId, TypeVarId};
use crate::type_services::facts::TypeFacts;
use crate::type_services::kind::Kind;
use crate::type_services::visit::{
    fold_type, fold_type_children, fold_type_in_place, visit_type, visit_type_children, TypeFolder,
    TypeVisitor,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReceiverMode {
    Shared,
    Mut,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GenericParamId {
    pub owner: DefId,
    pub index: u32,
}

impl GenericParamId {
    pub fn remap_def_ids<F>(&mut self, remap: &mut F)
    where
        F: FnMut(DefId) -> DefId,
    {
        self.owner = remap(self.owner);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GenericParamDecl {
    pub id: GenericParamId,
    pub name: String,
    pub kind: Kind,
    pub default: Option<Type>,
}

impl GenericParamDecl {
    pub fn new(id: GenericParamId, name: impl Into<String>, kind: Kind) -> Self {
        Self {
            id,
            name: name.into(),
            kind,
            default: None,
        }
    }

    pub fn type_param(id: GenericParamId, name: impl Into<String>) -> Self {
        Self::new(id, name, Kind::Type)
    }

    pub fn type_params(
        owner: DefId,
        names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Vec<Self> {
        names
            .into_iter()
            .enumerate()
            .map(|(index, name)| {
                Self::type_param(
                    GenericParamId {
                        owner,
                        index: index as u32,
                    },
                    name,
                )
            })
            .collect()
    }

    pub fn ids(params: &[Self]) -> impl Iterator<Item = GenericParamId> + '_ {
        params.iter().map(|param| param.id)
    }

    pub fn names(params: &[Self]) -> impl Iterator<Item = &str> {
        params.iter().map(|param| param.name.as_str())
    }

    pub fn remap_def_ids<F>(&mut self, remap: &mut F)
    where
        F: FnMut(DefId) -> DefId,
    {
        self.id.remap_def_ids(remap);
        if let Some(default) = &mut self.default {
            default.remap_def_ids(remap);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssociatedTypeKey {
    pub owner: DefId,
    pub assoc_type_id: AssocTypeId,
}

impl AssociatedTypeKey {
    pub fn remap_def_ids<F>(&mut self, remap: &mut F)
    where
        F: FnMut(DefId) -> DefId,
    {
        self.owner = remap(self.owner);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FunctionSafety {
    Safe,
    Unsafe,
}

impl FunctionSafety {
    pub fn from_is_unsafe(is_unsafe: bool) -> FunctionSafety {
        if is_unsafe {
            FunctionSafety::Unsafe
        } else {
            FunctionSafety::Safe
        }
    }

    pub fn permits_call_without_unsafe(self) -> bool {
        matches!(self, FunctionSafety::Safe)
    }

    pub fn join(self, other: FunctionSafety) -> FunctionSafety {
        match (self, other) {
            (FunctionSafety::Unsafe, _) | (_, FunctionSafety::Unsafe) => FunctionSafety::Unsafe,
            (FunctionSafety::Safe, FunctionSafety::Safe) => FunctionSafety::Safe,
        }
    }
}

/// The callable capabilities supported by a function value.
///
/// The declaration order is intentional: later kinds require stronger receiver
/// access. A callable of an earlier kind satisfies the later trait requirements.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum CallableKind {
    #[default]
    Fn,
    FnMut,
    FnOnce,
}

impl CallableKind {
    pub fn from_captures(captures: &[FunctionCapture]) -> Self {
        captures
            .iter()
            .map(|capture| match capture.kind {
                CaptureKind::SharedBorrow => Self::Fn,
                CaptureKind::MutableBorrow => Self::FnMut,
                CaptureKind::Move if TypeFacts::is_copy(&capture.ty) => Self::Fn,
                CaptureKind::Move => Self::FnOnce,
            })
            .max()
            .unwrap_or(Self::Fn)
    }

    pub fn is_default(self, captures: &[FunctionCapture]) -> bool {
        self == Self::Fn && captures.is_empty()
    }
}

/// How a closure stores a captured value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptureKind {
    SharedBorrow,
    MutableBorrow,
    Move,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FunctionCapture {
    pub kind: CaptureKind,
    pub ty: Type,
}

impl FunctionCapture {
    pub fn new(kind: CaptureKind, ty: Type) -> Self {
        Self { kind, ty }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NominalTypeKind {
    Struct,
    Enum,
    Alias,
}

/// The length of a fixed-size array type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArrayLen {
    /// A concrete length.
    Value(u64),
    /// A const generic parameter (e.g. `N` in `[T; N]`).
    Param(GenericParamId),
    /// An inference variable standing for a not-yet-known length.
    Var(TypeVarId),
    /// Error placeholder for error recovery.
    Error,
}

impl ArrayLen {
    /// The length a const-kinded type argument stands for.
    pub fn from_const_type(ty: &Type) -> ArrayLen {
        match ty {
            Type::ConstValue(n) => ArrayLen::Value(*n),
            Type::Generic(param) => ArrayLen::Param(*param),
            Type::TypeVar(var) => ArrayLen::Var(*var),
            _ => ArrayLen::Error,
        }
    }

    /// This length as a const-kinded type argument.
    pub fn to_const_type(&self) -> Type {
        match self {
            ArrayLen::Value(n) => Type::ConstValue(*n),
            ArrayLen::Param(param) => Type::Generic(*param),
            ArrayLen::Var(var) => Type::TypeVar(*var),
            ArrayLen::Error => Type::Error,
        }
    }

    /// The concrete length, if this length is fully known.
    pub fn value(&self) -> Option<u64> {
        match self {
            ArrayLen::Value(n) => Some(*n),
            _ => None,
        }
    }

    /// The concrete length. Codegen-facing: a non-concrete length after
    /// monomorphization is a compiler bug.
    pub fn as_concrete(&self) -> u64 {
        match self {
            ArrayLen::Value(n) => *n,
            other => panic!("internal error: non-concrete array length {other:?} after monomorphization"),
        }
    }

    pub fn is_concrete(&self) -> bool {
        matches!(self, ArrayLen::Value(_))
    }
}

impl From<u64> for ArrayLen {
    fn from(n: u64) -> Self {
        ArrayLen::Value(n)
    }
}

impl From<usize> for ArrayLen {
    fn from(n: usize) -> Self {
        ArrayLen::Value(n as u64)
    }
}

/// The core type representation used throughout the compiler after parsing.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Type {
    /// Primitive integer types
    I8,
    I16,
    I32,
    I64,
    /// Unsigned integer types
    U8,
    U16,
    U32,
    U64,
    /// Floating point types
    F32,
    F64,
    /// Boolean
    Bool,
    /// String slice (pointer to static string in binary)
    Str,
    /// Character
    Char,
    /// Unit type (void)
    Unit,
    /// Never type (for diverging expressions)
    Never,
    /// Slice type: [T]
    Slice(Box<Type>),
    /// Fixed-size array: [T; N]
    Array(Box<Type>, ArrayLen),
    /// A const value of kind `usize` used as a type argument.
    ConstValue(u64),
    /// Tuple of types
    Tuple(Vec<Type>),
    /// Function type: args -> ret
    Function {
        params: Vec<Type>,
        ret: Box<Type>,
        safety: FunctionSafety,
        callable_kind: CallableKind,
        captures: Vec<FunctionCapture>,
    },
    /// Named struct type
    Struct {
        id: DefId,
        args: Vec<Type>,
    },
    /// Named enum type
    Enum {
        id: DefId,
        args: Vec<Type>,
    },
    /// Reference: &T or &mut T
    Reference {
        mutable: bool,
        inner: Box<Type>,
    },
    /// Raw pointer: *T
    Pointer(Box<Type>),
    /// A type variable (for inference)
    TypeVar(TypeVarId),
    /// A generic type parameter (e.g. T in fn foo<T>)
    Generic(GenericParamId),
    /// Associated type projection, e.g. <Self as Deref>::Target
    Projection {
        ty: Box<Type>,
        trait_id: DefId,
        assoc_type: AssociatedTypeKey,
        trait_args: Vec<Type>,
    },
    /// A named type constructor that has not yet been saturated.
    Constructor {
        id: DefId,
        flavor: NominalTypeKind,
    },
    /// Type-level application that remains abstract after normalization.
    Apply {
        constructor: Box<Type>,
        args: Vec<Type>,
    },
    /// A type lambda. Bound variables use lexical de Bruijn identity.
    Lambda {
        params: Vec<Kind>,
        body: Box<Type>,
    },
    /// A variable bound by a type lambda.
    BoundVar {
        depth: u32,
        index: u32,
        kind: Kind,
    },
    /// An error type placeholder (for error recovery)
    Error,
}

struct DefIdRemapper<'a, F> {
    remap: &'a mut F,
}

impl<F> TypeFolder for DefIdRemapper<'_, F>
where
    F: FnMut(DefId) -> DefId,
{
    fn fold_type(&mut self, mut ty: Type) -> Type {
        match &mut ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } | Type::Constructor { id, .. } => {
                *id = (self.remap)(*id)
            }
            Type::Projection {
                trait_id,
                assoc_type,
                ..
            } => {
                *trait_id = (self.remap)(*trait_id);
                assoc_type.remap_def_ids(self.remap);
            }
            Type::Generic(param) => param.remap_def_ids(self.remap),
            _ => {}
        }
        fold_type_children(ty, self)
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        match len {
            ArrayLen::Param(mut param) => {
                param.remap_def_ids(self.remap);
                ArrayLen::Param(param)
            }
            other => other,
        }
    }
}

struct TypeVarSubstituter<'a> {
    subst: &'a HashMap<TypeVarId, Type>,
}

impl TypeFolder for TypeVarSubstituter<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => self
                .subst
                .get(&id)
                .cloned()
                .map(|replacement| self.fold_type(replacement))
                .unwrap_or(Type::TypeVar(id)),
            other => fold_type_children(other, self),
        }
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        match len {
            ArrayLen::Var(id) => match self.subst.get(&id) {
                Some(replacement) => {
                    let replacement = self.fold_type(replacement.clone());
                    ArrayLen::from_const_type(&replacement)
                }
                None => len,
            },
            other => other,
        }
    }
}

struct GenericSubstituter<'a> {
    subst: &'a HashMap<GenericParamId, Type>,
    visiting: Vec<GenericParamId>,
}

impl TypeFolder for GenericSubstituter<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        let Type::Generic(param) = ty else {
            return fold_type_children(ty, self);
        };

        if self.visiting.contains(&param) {
            return Type::Generic(param);
        }
        let Some(replacement) = self.subst.get(&param).cloned() else {
            return Type::Generic(param);
        };

        self.visiting.push(param);
        let substituted = self.fold_type(replacement);
        self.visiting.pop();
        substituted
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        let ArrayLen::Param(param) = len else {
            return len;
        };
        if self.visiting.contains(&param) {
            return len;
        }
        let Some(replacement) = self.subst.get(&param).cloned() else {
            return len;
        };

        self.visiting.push(param);
        let substituted = self.fold_type(replacement);
        self.visiting.pop();
        ArrayLen::from_const_type(&substituted)
    }
}

struct ArrayLenParamCollector<'a> {
    out: &'a mut std::collections::HashSet<GenericParamId>,
}

impl TypeFolder for ArrayLenParamCollector<'_> {
    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        if let ArrayLen::Param(param) = len {
            self.out.insert(param);
        }
        len
    }
}

struct GenericParamCollector<'a> {
    out: &'a mut std::collections::HashSet<GenericParamId>,
}

impl TypeVisitor for GenericParamCollector<'_> {
    fn visit_type(&mut self, ty: &Type) {
        if let Type::Generic(param) = ty {
            self.out.insert(*param);
        }
        visit_type_children(ty, self);
    }
}

struct TypeVarToGenericFolder<'a> {
    mapping: &'a HashMap<TypeVarId, GenericParamId>,
    composite_types: &'a HashMap<TypeVarId, Type>,
}

impl TypeFolder for TypeVarToGenericFolder<'_> {
    fn fold_type(&mut self, ty: Type) -> Type {
        let Type::TypeVar(id) = ty else {
            return fold_type_children(ty, self);
        };

        if let Some(composite) = self.composite_types.get(&id).cloned() {
            self.fold_type(composite)
        } else if let Some(param) = self.mapping.get(&id) {
            Type::Generic(*param)
        } else {
            Type::TypeVar(id)
        }
    }

    fn fold_array_len(&mut self, len: ArrayLen) -> ArrayLen {
        let ArrayLen::Var(id) = len else {
            return len;
        };
        if let Some(composite) = self.composite_types.get(&id).cloned() {
            let folded = self.fold_type(composite);
            ArrayLen::from_const_type(&folded)
        } else if let Some(param) = self.mapping.get(&id) {
            ArrayLen::Param(*param)
        } else {
            len
        }
    }
}

impl Type {
    pub fn function(params: Vec<Type>, ret: Type) -> Type {
        Type::function_with_safety(params, ret, FunctionSafety::Safe)
    }

    pub fn unsafe_function(params: Vec<Type>, ret: Type) -> Type {
        Type::function_with_safety(params, ret, FunctionSafety::Unsafe)
    }

    pub fn function_with_safety(params: Vec<Type>, ret: Type, safety: FunctionSafety) -> Type {
        Self::function_with_metadata(params, ret, safety, CallableKind::Fn, Vec::new())
    }

    pub fn function_with_metadata(
        params: Vec<Type>,
        ret: Type,
        safety: FunctionSafety,
        callable_kind: CallableKind,
        captures: Vec<FunctionCapture>,
    ) -> Type {
        Type::Function {
            params,
            ret: Box::new(ret),
            safety,
            callable_kind,
            captures,
        }
    }

    pub fn function_parts(&self) -> Option<(&[Type], &Type, FunctionSafety)> {
        match self {
            Type::Function {
                params,
                ret,
                safety,
                ..
            } => Some((params, ret.as_ref(), *safety)),
            _ => None,
        }
    }

    pub fn remap_def_ids<F>(&mut self, remap: &mut F)
    where
        F: FnMut(DefId) -> DefId,
    {
        fold_type_in_place(self, &mut DefIdRemapper { remap });
    }

    pub fn is_integer(&self) -> bool {
        TypeFacts::is_integer(self)
    }

    pub fn is_signed_integer(&self) -> bool {
        TypeFacts::is_signed_integer(self)
    }

    pub fn is_unsigned_integer(&self) -> bool {
        TypeFacts::is_unsigned_integer(self)
    }

    pub fn is_float(&self) -> bool {
        TypeFacts::is_float(self)
    }

    pub fn is_numeric(&self) -> bool {
        TypeFacts::is_numeric(self)
    }

    pub fn is_type_var(&self) -> bool {
        TypeFacts::is_type_var(self)
    }

    pub fn is_concrete(&self) -> bool {
        TypeFacts::is_concrete(self)
    }

    /// Check if this type implements Copy (can be implicitly copied)
    pub fn is_copy(&self) -> bool {
        TypeFacts::is_copy(self)
    }

    pub fn contains_reference(&self) -> bool {
        TypeFacts::contains_reference(self)
    }

    /// Substitute type variables with concrete types using a substitution map
    pub fn substitute(&self, subst: &HashMap<TypeVarId, Type>) -> Type {
        fold_type(self.clone(), &mut TypeVarSubstituter { subst })
    }

    /// Substitute generic parameters with concrete types.
    pub fn substitute_generics(&self, subst: &HashMap<GenericParamId, Type>) -> Type {
        fold_type(
            self.clone(),
            &mut GenericSubstituter {
                subst,
                visiting: Vec::new(),
            },
        )
    }

    /// Collect the generic parameters used as array lengths (const generics).
    pub fn collect_array_len_params(&self, out: &mut std::collections::HashSet<GenericParamId>) {
        fold_type(self.clone(), &mut ArrayLenParamCollector { out });
    }

    /// Collect all distinct Generic parameter identities appearing in this type.
    pub fn collect_generic_params(&self, out: &mut std::collections::HashSet<GenericParamId>) {
        visit_type(self, &mut GenericParamCollector { out });
    }

    /// Replace TypeVar nodes with Generic nodes during generalization.
    ///
    /// `mapping` maps TypeVar IDs to generic parameter identities.
    /// `composite_types` maps TypeVar IDs to resolved composite types (e.g., 5 → Function([TypeVar(3)], TypeVar(4))).
    /// When a TypeVar ID is found in `composite_types`, the composite is recursively substituted
    /// instead of mapping directly to a Generic.
    pub fn replace_type_vars_to_generics(
        &self,
        mapping: &HashMap<TypeVarId, GenericParamId>,
        composite_types: &HashMap<TypeVarId, Type>,
    ) -> Type {
        fold_type(
            self.clone(),
            &mut TypeVarToGenericFolder {
                mapping,
                composite_types,
            },
        )
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        crate::type_services::display::write_type(self, f)
    }
}

/// Struct field information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructInfo {
    pub name: String,
    pub generic_params: Vec<GenericParamDecl>,
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub public: bool,
}

/// Enum variant information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumInfo {
    pub name: String,
    pub generic_params: Vec<GenericParamDecl>,
    pub variants: Vec<VariantInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantInfo {
    pub name: String,
    pub fields: VariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VariantFields {
    Named(Vec<FieldInfo>),
    Positional(Vec<Type>),
    Unit,
}

/// Trait information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitInfo {
    pub name: String,
    pub generic_params: Vec<GenericParamDecl>,
    pub methods: HashMap<String, FunctionSig>,
}

/// Function signature
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionSig {
    pub name: String,
    pub generic_params: Vec<GenericParamDecl>,
    pub params: Vec<Type>,
    pub ret: Type,
    pub self_receiver: Option<ReceiverMode>,
}

/// Trait bound for type variables (e.g., Num T)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TraitBound {
    pub trait_id: DefId,
    pub type_args: Vec<Type>,
}

/// A semantic where-clause predicate with an arbitrary typed subject.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Predicate {
    Trait {
        subject: Type,
        trait_id: DefId,
        args: Vec<Type>,
    },
}

impl Predicate {
    pub fn substitute_generics(&self, subst: &HashMap<GenericParamId, Type>) -> Self {
        match self {
            Self::Trait {
                subject,
                trait_id,
                args,
            } => Self::Trait {
                subject: subject.substitute_generics(subst),
                trait_id: *trait_id,
                args: args
                    .iter()
                    .map(|arg| arg.substitute_generics(subst))
                    .collect(),
            },
        }
    }

    pub fn remap_def_ids<F>(&mut self, remap: &mut F)
    where
        F: FnMut(DefId) -> DefId,
    {
        match self {
            Self::Trait {
                subject,
                trait_id,
                args,
            } => {
                subject.remap_def_ids(remap);
                *trait_id = remap(*trait_id);
                for arg in args {
                    arg.remap_def_ids(remap);
                }
            }
        }
    }
}

pub fn validate_supertrait_graph<'a>(
    traits: impl IntoIterator<Item = (DefId, &'a str, &'a [Predicate])>,
) -> Vec<String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum VisitState {
        Visiting,
        Complete,
    }

    fn visit(
        id: DefId,
        graph: &std::collections::BTreeMap<DefId, (String, Vec<DefId>)>,
        states: &mut std::collections::HashMap<DefId, VisitState>,
        stack: &mut Vec<DefId>,
        errors: &mut Vec<String>,
        reported: &mut std::collections::HashSet<Vec<DefId>>,
    ) {
        if states.get(&id) == Some(&VisitState::Complete) {
            return;
        }
        states.insert(id, VisitState::Visiting);
        stack.push(id);

        if let Some((_, edges)) = graph.get(&id) {
            for edge in edges {
                match states.get(edge) {
                    Some(VisitState::Visiting) => {
                        let start = stack
                            .iter()
                            .position(|candidate| candidate == edge)
                            .unwrap();
                        let mut cycle = stack[start..].to_vec();
                        cycle.push(*edge);
                        let mut identity = cycle[..cycle.len() - 1].to_vec();
                        if let Some((offset, _)) =
                            identity.iter().enumerate().min_by_key(|(_, id)| *id)
                        {
                            identity.rotate_left(offset);
                        }
                        if reported.insert(identity) {
                            let path = cycle
                                .iter()
                                .map(|cycle_id| {
                                    let name = graph
                                        .get(cycle_id)
                                        .map(|(name, _)| name.as_str())
                                        .unwrap_or("<unknown>");
                                    name.to_string()
                                })
                                .collect::<Vec<_>>()
                                .join(" -> ");
                            errors.push(format!("supertrait cycle detected: {path}"));
                        }
                    }
                    Some(VisitState::Complete) => {}
                    None => visit(*edge, graph, states, stack, errors, reported),
                }
            }
        }

        stack.pop();
        states.insert(id, VisitState::Complete);
    }

    let mut graph = std::collections::BTreeMap::new();
    for (id, name, predicates) in traits {
        let mut edges = predicates
            .iter()
            .map(|predicate| match predicate {
                Predicate::Trait { trait_id, .. } => *trait_id,
            })
            .collect::<Vec<_>>();
        edges.sort();
        edges.dedup();
        graph.insert(id, (name.to_string(), edges));
    }

    let mut errors = Vec::new();
    for (_id, (name, edges)) in &graph {
        for edge in edges {
            if !graph.contains_key(edge) {
                errors.push(format!("trait '{}' references an unknown supertrait", name));
            }
        }
    }
    let mut states = std::collections::HashMap::new();
    let mut stack = Vec::new();
    let mut reported = std::collections::HashSet::new();
    for id in graph.keys().copied() {
        if !states.contains_key(&id) {
            visit(
                id,
                &graph,
                &mut states,
                &mut stack,
                &mut errors,
                &mut reported,
            );
        }
    }
    errors.sort();
    errors
}

impl fmt::Display for TraitBound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "trait#{}::{}",
            self.trait_id.crate_id.0, self.trait_id.local.0
        )?;
        if !self.type_args.is_empty() {
            for arg in &self.type_args {
                write!(f, " {}", arg)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
