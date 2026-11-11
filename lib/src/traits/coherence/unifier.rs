use super::*;

pub(super) fn imp_generic_ids(imp: &HirImpl) -> HashSet<GenericParamId> {
    imp.type_generics.iter().map(|param| param.id).collect()
}

pub(super) struct FirstOrderUnifier<'a> {
    flexible: HashSet<GenericParamId>,
    requires_sized: HashSet<GenericParamId>,
    generic_kinds: HashMap<GenericParamId, Kind>,
    bindings: HashMap<GenericParamId, Type>,
    env: &'a TypeNormalizationEnv,
}

impl<'a> FirstOrderUnifier<'a> {
    pub(super) fn new(
        left: &HirImpl,
        right: &HirImpl,
        sized_trait_id: Option<DefId>,
        env: &'a TypeNormalizationEnv,
    ) -> Self {
        let mut flexible = HashSet::new();
        let mut requires_sized = HashSet::new();
        let mut generic_kinds = HashMap::new();
        for imp in [left, right] {
            for param in &imp.type_generics {
                flexible.insert(param.id);
                generic_kinds.insert(param.id, param.kind.clone());
            }
            if let Some(sized_trait_id) = sized_trait_id {
                for (param, bounds) in &imp.bounds {
                    if bounds.iter().any(|bound| bound.trait_id == sized_trait_id) {
                        requires_sized.insert(*param);
                    }
                }
            }
        }
        Self {
            flexible,
            requires_sized,
            generic_kinds,
            bindings: HashMap::new(),
            env,
        }
    }

    pub(super) fn unify_targets(
        &mut self,
        left: &CanonicalTarget,
        right: &CanonicalTarget,
    ) -> bool {
        match (left, right) {
            (CanonicalTarget::Constructor(left), CanonicalTarget::Constructor(right))
            | (CanonicalTarget::Exact(left), CanonicalTarget::Exact(right)) => {
                self.unify(left, right)
            }
            (CanonicalTarget::SliceFamily(left), CanonicalTarget::SliceFamily(right)) => {
                self.unify(left, right)
            }
            (CanonicalTarget::SliceFamily(element), CanonicalTarget::Exact(actual))
            | (CanonicalTarget::Exact(actual), CanonicalTarget::SliceFamily(element)) => {
                match actual {
                    Type::Slice(actual) | Type::Array(actual, _) => self.unify(element, actual),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    pub(super) fn unify(&mut self, left: &Type, right: &Type) -> bool {
        let left = self.resolve_head(left);
        let right = self.resolve_head(right);
        if left == right {
            return true;
        }
        if let Type::Generic(id) = left {
            if self.flexible.contains(&id) {
                return self.bind(id, &right);
            }
        }
        if let Type::Generic(id) = right {
            if self.flexible.contains(&id) {
                return self.bind(id, &left);
            }
        }

        match (&left, &right) {
            (Type::Slice(left), Type::Slice(right))
            | (Type::Pointer(left), Type::Pointer(right)) => self.unify(left, right),
            (Type::Array(left, left_len), Type::Array(right, right_len)) => {
                self.unify_lengths(*left_len, *right_len) && self.unify(left, right)
            }
            (Type::Tuple(left), Type::Tuple(right)) => self.unify_lists(left, right),
            (
                Type::Struct {
                    id: left_id,
                    args: left,
                },
                Type::Struct {
                    id: right_id,
                    args: right,
                },
            )
            | (
                Type::Enum {
                    id: left_id,
                    args: left,
                },
                Type::Enum {
                    id: right_id,
                    args: right,
                },
            ) => left_id == right_id && self.unify_lists(left, right),
            (
                Type::Reference {
                    mutable: left_mut,
                    inner: left,
                },
                Type::Reference {
                    mutable: right_mut,
                    inner: right,
                },
            ) => left_mut == right_mut && self.unify(left, right),
            (
                Type::Function {
                    params: left_params,
                    ret: left_ret,
                    safety: left_safety,
                    callable_kind: left_kind,
                    captures: left_captures,
                    ..
                },
                Type::Function {
                    params: right_params,
                    ret: right_ret,
                    safety: right_safety,
                    callable_kind: right_kind,
                    captures: right_captures,
                    ..
                },
            ) => {
                left_safety == right_safety
                    && left_kind == right_kind
                    && self.unify_lists(left_params, right_params)
                    && self.unify(left_ret, right_ret)
                    && left_captures.len() == right_captures.len()
                    && left_captures
                        .iter()
                        .zip(right_captures)
                        .all(|(left, right)| {
                            left.kind == right.kind && self.unify(&left.ty, &right.ty)
                        })
            }
            (
                Type::Projection {
                    ty: left_ty,
                    trait_id: left_trait,
                    assoc_type: left_assoc,
                    trait_args: left_args,
                },
                Type::Projection {
                    ty: right_ty,
                    trait_id: right_trait,
                    assoc_type: right_assoc,
                    trait_args: right_args,
                },
            ) => {
                left_trait == right_trait
                    && left_assoc == right_assoc
                    && self.unify(left_ty, right_ty)
                    && self.unify_lists(left_args, right_args)
            }
            (
                Type::Constructor {
                    id: left_id,
                    flavor: left_flavor,
                },
                Type::Constructor {
                    id: right_id,
                    flavor: right_flavor,
                },
            ) => left_id == right_id && left_flavor == right_flavor,
            (
                Type::Apply {
                    constructor: left_constructor,
                    args: left_args,
                },
                Type::Apply {
                    constructor: right_constructor,
                    args: right_args,
                },
            ) => {
                self.unify(left_constructor, right_constructor)
                    && self.unify_lists(left_args, right_args)
            }
            (
                Type::Lambda {
                    params: left_params,
                    body: left_body,
                },
                Type::Lambda {
                    params: right_params,
                    body: right_body,
                },
            ) => left_params == right_params && self.unify(left_body, right_body),
            _ => false,
        }
    }

    /// Array lengths unify like const-kinded generic arguments: a flexible
    /// const parameter binds to a concrete length or to another parameter.
    fn unify_lengths(&mut self, left: ArrayLen, right: ArrayLen) -> bool {
        if left == right {
            return true;
        }
        if matches!(left, ArrayLen::Error) || matches!(right, ArrayLen::Error) {
            return false;
        }
        self.unify(&left.to_const_type(), &right.to_const_type())
    }

    fn unify_lists(&mut self, left: &[Type], right: &[Type]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(left, right)| self.unify(left, right))
    }

    fn resolve_head(&self, ty: &Type) -> Type {
        let mut current = ty.clone();
        let mut seen = HashSet::new();
        while let Type::Generic(id) = current {
            if !seen.insert(id) {
                break;
            }
            let Some(bound) = self.bindings.get(&id) else {
                break;
            };
            current = bound.clone();
        }
        current
    }

    fn bind(&mut self, id: GenericParamId, ty: &Type) -> bool {
        let ty = self.resolve_head(ty);
        if ty == Type::Generic(id) {
            return true;
        }
        let mut generics = HashSet::new();
        ty.collect_generic_params(&mut generics);
        if generics.contains(&id) {
            return false;
        }
        let Some(expected_kind) = self.generic_kinds.get(&id) else {
            return false;
        };
        if TypeNormalizer::new(self.env).kind_of(&ty).ok().as_ref() != Some(expected_kind) {
            return false;
        }
        if self.requires_sized.contains(&id) && type_is_definitely_unsized(&ty) {
            return false;
        }
        if let Type::Generic(other) = ty {
            if self.requires_sized.contains(&id) {
                self.requires_sized.insert(other);
            }
            if self.requires_sized.contains(&other) {
                self.requires_sized.insert(id);
            }
            self.bindings.insert(id, Type::Generic(other));
            return true;
        }
        self.bindings.insert(id, ty);
        true
    }

    pub(super) fn witness(&self) -> Vec<CoherenceWitnessBinding> {
        let mut bindings = self
            .bindings
            .iter()
            .map(|(id, ty)| {
                let mut resolved = ty.clone();
                for _ in 0..self.bindings.len() {
                    let next = resolved.substitute_generics(&self.bindings);
                    if next == resolved {
                        break;
                    }
                    resolved = next;
                }
                CoherenceWitnessBinding {
                    parameter: *id,
                    value: resolved,
                }
            })
            .collect::<Vec<_>>();
        bindings
            .sort_by_key(|binding| (def_id_key(binding.parameter.owner), binding.parameter.index));
        bindings
    }
}

fn type_is_definitely_unsized(ty: &Type) -> bool {
    matches!(ty, Type::Slice(_) | Type::Str)
}

pub(super) fn def_id_key(id: DefId) -> (u32, u32) {
    (id.crate_id.0, id.local.0)
}
