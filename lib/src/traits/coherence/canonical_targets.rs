use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CanonicalTarget {
    Exact(Type),
    SliceFamily(Type),
    Constructor(Type),
}

impl CanonicalTarget {
    pub(super) fn is_constructor(&self) -> bool {
        matches!(self, Self::Constructor(_))
    }

    pub(super) fn kind(&self, env: &TypeNormalizationEnv) -> Result<Kind, NormalizeError> {
        match self {
            Self::SliceFamily(_) => Ok(Kind::Type),
            Self::Exact(ty) | Self::Constructor(ty) => TypeNormalizer::new(env).kind_of(ty),
        }
    }

    pub(super) fn outer_nominal(&self) -> Option<DefId> {
        match self {
            Self::Exact(ty) | Self::Constructor(ty) => outer_nominal(ty),
            Self::SliceFamily(_) => None,
        }
    }
}

pub(super) fn normalize_target(
    env: &TypeNormalizationEnv,
    target: &HirImplReceiverPattern,
) -> Result<CanonicalTarget, NormalizeError> {
    match target {
        HirImplReceiverPattern::Exact(ty) => TypeNormalizer::new(env)
            .normalize(ty)
            .map(CanonicalTarget::Exact),
        HirImplReceiverPattern::SliceFamily { element } => TypeNormalizer::new(env)
            .normalize(element)
            .map(CanonicalTarget::SliceFamily),
        HirImplReceiverPattern::Constructor(ty) => TypeNormalizer::new(env)
            .normalize(ty)
            .map(CanonicalTarget::Constructor),
    }
}

pub(super) fn normalization_env(
    traits: &[&HirTrait],
    impls: &[&HirImpl],
    structs: &[&HirStruct],
    enums: &[&HirEnum],
    aliases: &[&HirTypeAlias],
) -> TypeNormalizationEnv {
    let mut env = TypeNormalizationEnv::new();
    for structure in structs {
        env.register_constructor(
            structure.id,
            NominalTypeKind::Struct,
            crate::type_lowering::constructor_kind(&structure.generic_params),
        );
        for param in &structure.generic_params {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }
    for enumeration in enums {
        env.register_constructor(
            enumeration.id,
            NominalTypeKind::Enum,
            crate::type_lowering::constructor_kind(&enumeration.generic_params),
        );
        for param in &enumeration.generic_params {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }
    crate::type_lowering::register_type_aliases(&mut env, aliases.iter().copied());
    for trait_def in traits {
        for param in trait_def
            .generic_params
            .iter()
            .chain(trait_def.target.iter())
        {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }
    for imp in impls {
        for param in &imp.type_generics {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }
    env
}

pub(super) fn validate_constructor_header(
    target: &Type,
    impl_generics: &HashSet<GenericParamId>,
) -> Result<(), String> {
    let body = match target {
        Type::Lambda { body, .. } => body.as_ref(),
        other => other,
    };
    if outer_nominal(body).is_none() {
        return Err("a concrete nominal outer constructor is required".to_string());
    }
    validate_constructor_term(body, impl_generics, false)
}

fn validate_constructor_term(
    ty: &Type,
    impl_generics: &HashSet<GenericParamId>,
    application_head: bool,
) -> Result<(), String> {
    match ty {
        Type::TypeVar(_) => Err("unresolved inference variables are forbidden".to_string()),
        Type::Projection { .. } => Err("associated projections are forbidden".to_string()),
        Type::Error => Err("error types are forbidden".to_string()),
        Type::Generic(id) if application_head && impl_generics.contains(id) => {
            Err("an impl-generic constructor may not be an application head".to_string())
        }
        Type::Generic(_) | Type::BoundVar { .. } | Type::Constructor { .. } => Ok(()),
        Type::Apply { constructor, args } => {
            validate_constructor_term(constructor, impl_generics, true)?;
            for arg in args {
                validate_constructor_term(arg, impl_generics, false)?;
            }
            Ok(())
        }
        Type::Lambda { .. } => Err("only one outer type lambda is permitted".to_string()),
        Type::Slice(inner) | Type::Pointer(inner) => {
            validate_constructor_term(inner, impl_generics, false)
        }
        Type::Array(inner, _) | Type::Reference { inner, .. } => {
            validate_constructor_term(inner, impl_generics, false)
        }
        Type::Tuple(items) | Type::Struct { args: items, .. } | Type::Enum { args: items, .. } => {
            for item in items {
                validate_constructor_term(item, impl_generics, false)?;
            }
            Ok(())
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for ty in params
                .iter()
                .chain(std::iter::once(ret.as_ref()))
                .chain(captures.iter().map(|capture| &capture.ty))
            {
                validate_constructor_term(ty, impl_generics, false)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn outer_nominal(ty: &Type) -> Option<DefId> {
    match ty {
        Type::Lambda { body, .. } => outer_nominal(body),
        Type::Struct { id, .. } | Type::Enum { id, .. } => Some(*id),
        Type::Constructor { id, flavor } if !matches!(flavor, NominalTypeKind::Alias) => Some(*id),
        Type::Apply { constructor, .. } => outer_nominal(constructor),
        _ => None,
    }
}
