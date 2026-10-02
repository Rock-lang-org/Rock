use crate::ast;
use crate::hir::{HirEnum, HirStruct, HirTrait, HirTypeAlias};
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{TypeNormalizationEnv, TypeNormalizer};
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, NominalTypeKind, Type};

#[derive(Debug, Clone)]
pub(crate) enum ResolvedNominalType {
    Struct(HirStruct),
    Enum(HirEnum),
}

pub(crate) trait TypeLoweringContext {
    fn nominal_type_params(&self, _id: crate::ids::DefId) -> Vec<GenericParamDecl> {
        Vec::new()
    }
    fn sized_trait_id(&self) -> Option<crate::ids::DefId> {
        None
    }
    fn push_type_error(&mut self, message: String, span: crate::lexer::Span);
    fn record_type_reference(
        &mut self,
        _span: crate::lexer::Span,
        _target: crate::source_map::SourceSymbol,
    ) {
    }
    fn current_module_prefix(&self) -> Option<String>;
    fn current_trait_name(&self) -> Option<String>;
    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType>;
    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait>;
    fn existing_generic_type(&self, name: &str) -> Option<Type>;
    fn admit_object_type(
        &mut self,
        ty: Type,
        env: &TypeNormalizationEnv,
        span: crate::lexer::Span,
    ) -> Type;
    fn qualify_object_type(
        &mut self,
        object: crate::types::ObjectType,
        bindings: Vec<ObjectBindingSyntax>,
        binders: Vec<Vec<Kind>>,
        span: crate::lexer::Span,
    ) -> Type;
    fn resolve_type_alias(&self, name: &str) -> Option<HirTypeAlias>;
    fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type;
    fn populate_type_normalization_env(&self, env: &mut TypeNormalizationEnv);
}

pub(crate) struct ObjectBindingSyntax {
    pub owner: Option<crate::types::TraitBound>,
    pub member: ast::Ident,
    pub ty: Type,
}

pub(crate) fn resolve_object_bindings(
    mut object: crate::types::ObjectType,
    bindings: Vec<ObjectBindingSyntax>,
    traits: &std::collections::HashMap<crate::ids::DefId, HirTrait>,
    span: crate::lexer::Span,
) -> Result<Type, (String, crate::lexer::Span)> {
    // The shared evidence query assumes an acyclic declaration graph. Qualifier
    // resolution precedes admission, so establish that precondition here.
    fn check_graph(
        id: crate::ids::DefId,
        traits: &std::collections::HashMap<crate::ids::DefId, HirTrait>,
        path: &mut Vec<crate::ids::DefId>,
        done: &mut std::collections::HashSet<crate::ids::DefId>,
    ) -> Result<(), String> {
        if path.contains(&id) {
            return Err(format!("object supertrait cycle: {path:?} -> {id:?}"));
        }
        if done.contains(&id) {
            return Ok(());
        }
        if path.len() >= crate::type_services::normalize::DEFAULT_MAX_NORMALIZATION_DEPTH {
            return Err("object supertrait graph exceeds maximum depth".to_string());
        }
        let definition = traits
            .get(&id)
            .ok_or_else(|| format!("unknown object trait {id:?}"))?;
        let self_id = definition
            .target
            .as_ref()
            .map(|target| target.id)
            .unwrap_or(GenericParamId {
                owner: id,
                index: definition.generic_params.len() as u32,
            });
        path.push(id);
        for predicate in &definition.predicates {
            let crate::types::Predicate::Trait {
                subject, trait_id, ..
            } = predicate;
            if subject == &Type::Generic(self_id) {
                check_graph(*trait_id, traits, path, done)?;
            }
        }
        path.pop();
        done.insert(id);
        Ok(())
    }
    let roots = std::iter::once(object.principal.clone())
        .chain(object.guarantees.iter().cloned())
        .collect::<Vec<_>>();
    let mut done = std::collections::HashSet::new();
    for root in &roots {
        check_graph(root.trait_id, traits, &mut Vec::new(), &mut done)
            .map_err(|error| (error, span.clone()))?;
    }
    let closure = crate::traits::evidence::implied_trait_bounds(
        traits,
        &Type::ObjectSelf { depth: 0 },
        &roots,
    );
    for binding in bindings {
        let mut candidates = std::collections::BTreeSet::new();
        for bound in &closure {
            if binding.owner.as_ref().is_some_and(|owner| owner != bound) {
                continue;
            }
            if let Some(definition) = traits.get(&bound.trait_id) {
                for member in &definition.associated_types {
                    if member.name == binding.member.name {
                        candidates.insert(crate::types::ObjectAssociatedType {
                            trait_ref: bound.clone(),
                            member: member.id,
                        });
                    }
                }
            }
        }
        if candidates.len() != 1 {
            return Err((
                format!(
                    "object associated member '{}' is {}",
                    binding.member.name,
                    if candidates.is_empty() {
                        "unknown in the guaranteed trait closure"
                    } else {
                        "ambiguous; qualify its trait owner"
                    }
                ),
                binding.member.span,
            ));
        }
        let key = candidates.into_iter().next().unwrap();
        if object
            .bindings
            .iter()
            .any(|existing| existing.key == key && existing.ty != binding.ty)
        {
            return Err((
                format!(
                    "conflicting object associated binding '{}'",
                    binding.member.name
                ),
                binding.member.span,
            ));
        }
        object.bindings.insert(crate::types::ObjectBinding {
            key,
            ty: binding.ty,
        });
    }
    Ok(Type::Object(Box::new(object)))
}

pub(crate) struct TypeLowerer;

pub(crate) fn constructor_kind(params: &[crate::types::GenericParamDecl]) -> Kind {
    params.iter().rev().fold(Kind::Type, |output, param| {
        Kind::arrow(param.kind.clone(), output)
    })
}

pub(crate) fn constructor_kind_from_arity(arity: usize) -> Kind {
    (0..arity).fold(Kind::Type, |output, _| Kind::arrow(Kind::Type, output))
}

pub(crate) fn lower_generic_param_kind(kind: Option<&ast::TypeApplication>) -> Kind {
    let Some(kind) = kind else {
        return Kind::Type;
    };
    if !kind.args.is_empty() {
        return constructor_kind_from_arity(kind.args.len());
    }
    lower_explicit_kind_syntax(&kind.constructor).unwrap_or(Kind::Type)
}

fn lower_explicit_kind_syntax(kind: &ast::ParseType) -> Option<Kind> {
    match kind {
        ast::ParseType::Type(inner) if inner.name == "Type" && inner.generics.is_empty() => {
            Some(Kind::Type)
        }
        ast::ParseType::Function(types) if types.len() >= 2 => {
            let mut types = types.iter().rev();
            let mut output = lower_explicit_kind_syntax(types.next()?)?;
            for input in types {
                output = Kind::arrow(lower_explicit_kind_syntax(input)?, output);
            }
            Some(output)
        }
        _ => None,
    }
}

pub(crate) fn register_type_aliases<'a>(
    env: &mut TypeNormalizationEnv,
    aliases: impl IntoIterator<Item = &'a crate::hir::HirTypeAlias>,
) {
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    for alias in &aliases {
        env.register_alias(
            alias.id,
            constructor_kind(&alias.generic_params),
            alias.ty.clone(),
        );
        for param in &alias.generic_params {
            env.register_generic_kind(param.id, param.kind.clone());
        }
    }

    // Alias bodies may themselves be constructors. Iterate so alias chains can
    // acquire their body-derived kinds without declaration-order dependence.
    for _ in 0..aliases.len() {
        let derived = aliases
            .iter()
            .filter_map(|alias| {
                TypeNormalizer::new(env)
                    .kind_of(&alias.ty)
                    .ok()
                    .map(|kind| (alias.id, kind, alias.ty.clone()))
            })
            .collect::<Vec<_>>();
        for (id, kind, body) in derived {
            env.register_alias(id, kind, body);
        }
    }
}

pub(crate) fn lower_associated_type_kind(kind: Option<&ast::TypeApplication>) -> Kind {
    lower_generic_param_kind(kind)
}

pub(crate) fn lower_generic_param_decls(
    owner: crate::ids::DefId,
    params: &[ast::GenericParamDecl],
) -> Vec<GenericParamDecl> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let mut declaration = GenericParamDecl::new(
                GenericParamId {
                    owner,
                    index: index as u32,
                },
                param.name.name.clone(),
                lower_generic_param_kind(param.kind.as_ref()),
            );
            declaration.maybe_unsized = param.unsized_bound.is_some();
            declaration
        })
        .collect()
}

pub(crate) fn lower_generic_param_decls_checked<C: TypeLoweringContext + ?Sized>(
    context: &mut C,
    owner: crate::ids::DefId,
    params: &[ast::GenericParamDecl],
) -> Vec<GenericParamDecl> {
    for param in params {
        if let Some(bound) = &param.unsized_bound {
            let valid = match bound {
                ast::ParseType::Type(inner) if inner.generics.is_empty() => {
                    !params
                        .iter()
                        .any(|parameter| parameter.name.name == inner.name)
                        && context.existing_generic_type(&inner.name).is_none()
                        && context
                            .resolve_trait_type(&inner.name)
                            .is_some_and(|definition| {
                                Some(definition.id) == context.sized_trait_id()
                            })
                }
                _ => false,
            };
            if !valid {
                context.push_type_error(
                    "only the canonical Sized requirement may be relaxed".into(),
                    bound.span(),
                );
            }
        }
    }
    lower_generic_param_decls(owner, params)
}

pub(crate) fn lower_parse_generic_param_decls(
    owner: crate::ids::DefId,
    params: &[ast::ParseType],
) -> Vec<GenericParamDecl> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let (name, kind) = match param {
                ast::ParseType::Type(inner) => (inner.name.clone(), Kind::Type),
                ast::ParseType::Application(application) => (
                    application.constructor.type_name(),
                    constructor_kind_from_arity(application.args.len()),
                ),
                other => (other.type_name(), Kind::Type),
            };
            GenericParamDecl::new(
                GenericParamId {
                    owner,
                    index: index as u32,
                },
                name,
                kind,
            )
        })
        .collect()
}

impl TypeLowerer {
    fn normalize_component<C: TypeLoweringContext + ?Sized>(
        context: &C,
        env: &TypeNormalizationEnv,
        ty: &Type,
    ) -> Result<Type, crate::type_services::normalize::NormalizeError> {
        let mut env = env.clone();
        context.populate_type_normalization_env(&mut env);
        TypeNormalizer::new(&env).normalize(ty)
    }
    pub(crate) fn kind_of<C: TypeLoweringContext + ?Sized>(
        context: &C,
        ty: &Type,
    ) -> Result<Kind, String> {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        TypeNormalizer::new(&env)
            .kind_of(ty)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn lower_parse_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        Self::lower(context, parse_type, false)
    }

    pub(crate) fn lower_unsized_type<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        Self::lower(context, parse_type, true)
    }

    pub(crate) fn lower_parse_type_term<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let ty = Self::lower_raw(context, parse_type, false, &env, &mut Vec::new());
        if crate::type_services::visit::type_any(&ty, |ty| matches!(ty, Type::Object(_))) {
            context.admit_object_type(ty, &env, parse_type.span())
        } else {
            ty
        }
    }

    fn lower<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
        allow_bare_slice: bool,
    ) -> Type {
        let mut env = TypeNormalizationEnv::new();
        context.populate_type_normalization_env(&mut env);
        let raw = Self::lower_raw(context, parse_type, allow_bare_slice, &env, &mut Vec::new());
        Self::normalize(context, &env, raw, parse_type.span())
    }

    fn normalize<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        env: &TypeNormalizationEnv,
        ty: Type,
        span: crate::lexer::Span,
    ) -> Type {
        if matches!(ty, Type::Error) {
            return ty;
        }
        let mut refreshed = env.clone();
        context.populate_type_normalization_env(&mut refreshed);
        let env = &refreshed;
        match TypeNormalizer::new(env).normalize(&ty) {
            Ok(ty) => {
                let mut invalid = false;
                crate::type_services::visit::visit_type(&ty, &mut |nested: &Type| {
                    if let Type::Struct { id, args } | Type::Enum { id, args } = nested {
                        for (parameter, argument) in
                            context.nominal_type_params(*id).iter().zip(args)
                        {
                            if !parameter.maybe_unsized
                                && crate::type_services::layout::TypeLayout::sizedness(
                                    argument,
                                    &|_| crate::type_services::layout::Sizedness::Unknown,
                                ) == crate::type_services::layout::Sizedness::Unsized
                            {
                                invalid = true;
                            }
                        }
                    }
                });
                if invalid {
                    context.push_type_error("nominal type argument requires Sized; declare an explicitly relaxed parameter to accept unsized pointees".into(), span);
                    Type::Error
                } else if crate::type_services::visit::type_any(&ty, |ty| {
                    matches!(ty, Type::Object(_))
                }) {
                    context.admit_object_type(ty, env, span)
                } else {
                    ty
                }
            }
            Err(error) => {
                context.push_type_error(error.to_string(), span);
                Type::Error
            }
        }
    }

    fn lower_raw<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        parse_type: &ast::ParseType,
        allow_bare_slice: bool,
        env: &TypeNormalizationEnv,
        binders: &mut Vec<Vec<(String, Kind)>>,
    ) -> Type {
        match parse_type {
            ast::ParseType::Object(syntax) => {
                let raw = Self::lower_raw(context, &syntax.base, false, env, binders);
                let base = match Self::normalize_component(context, env, &raw) {
                    Ok(Type::Object(object)) => object,
                    Ok(_) => {
                        context.push_type_error(
                            "object qualifiers require a trait type".to_string(),
                            syntax.base.span(),
                        );
                        return Type::Error;
                    }
                    Err(error) => {
                        context.push_type_error(error.to_string(), syntax.base.span());
                        return Type::Error;
                    }
                };
                let mut object = *base;
                let mut bindings = Vec::new();
                for qualifier in &syntax.qualifiers {
                    match qualifier {
                        ast::ObjectQualifier::Trait(ty) => {
                            let raw = Self::lower_raw(context, ty, false, env, binders);
                            match Self::normalize_component(context, env, &raw) {
                                Ok(Type::Object(extra))
                                    if extra.bindings.is_empty() && extra.guarantees.is_empty() =>
                                {
                                    object.guarantees.insert(extra.principal);
                                }
                                _ => {
                                    context.push_type_error(
                                        "object guarantee must be a trait application".to_string(),
                                        ty.span(),
                                    );
                                    return Type::Error;
                                }
                            }
                        }
                        ast::ObjectQualifier::Binding { owner, member, ty } => {
                            let owner = if let Some(owner) = owner {
                                let raw = Self::lower_raw(context, owner, false, env, binders);
                                match Self::normalize_component(context, env, &raw) {
                                    Ok(Type::Object(owner))
                                        if owner.bindings.is_empty()
                                            && owner.guarantees.is_empty() =>
                                    {
                                        Some(owner.principal)
                                    }
                                    _ => {
                                        context.push_type_error(
                                            "associated owner must be a trait application"
                                                .to_string(),
                                            owner.span(),
                                        );
                                        return Type::Error;
                                    }
                                }
                            } else {
                                None
                            };
                            let raw = Self::lower_raw(context, ty, false, env, binders);
                            let ty = match Self::normalize_component(context, env, &raw) {
                                Ok(ty) => ty,
                                Err(error) => {
                                    context.push_type_error(error.to_string(), ty.span());
                                    return Type::Error;
                                }
                            };
                            bindings.push(ObjectBindingSyntax {
                                owner,
                                member: member.clone(),
                                ty,
                            });
                        }
                    }
                }
                context.qualify_object_type(
                    object,
                    bindings,
                    binders
                        .iter()
                        .map(|group| group.iter().map(|(_, kind)| kind.clone()).collect())
                        .collect(),
                    syntax.span.clone(),
                )
            }
            ast::ParseType::Unit(_) => Type::Unit,
            ast::ParseType::Type(inner) => {
                Self::lower_inner_raw(context, inner, allow_bare_slice, env, binders)
            }
            ast::ParseType::Application(application) => {
                let constructor = Self::lower_raw(
                    context,
                    &application.constructor,
                    allow_bare_slice,
                    env,
                    binders,
                );
                let mut hole_kinds = Vec::new();
                let args = application
                    .args
                    .iter()
                    .map(|arg| match arg {
                        ast::ParseType::Hole(_) => {
                            let index = hole_kinds.len() as u32;
                            hole_kinds.push(Kind::Type);
                            Type::BoundVar {
                                depth: 0,
                                index,
                                kind: Kind::Type,
                            }
                        }
                        arg => Self::lower_raw(context, arg, false, env, binders),
                    })
                    .collect();
                let application = Type::Apply {
                    constructor: Box::new(constructor),
                    args,
                };
                if hole_kinds.is_empty() {
                    application
                } else {
                    Type::Lambda {
                        params: hole_kinds,
                        body: Box::new(application),
                    }
                }
            }
            ast::ParseType::Lambda(lambda) => {
                if let Some(parameter) = lambda
                    .params
                    .iter()
                    .find(|parameter| parameter.unsized_bound.is_some())
                {
                    context.push_type_error("type-lambda binders describe kinds; put ?Sized on a value or nominal generic declaration".into(), parameter.span.clone());
                    return Type::Error;
                }
                let params = lambda
                    .params
                    .iter()
                    .map(|param| {
                        (
                            param.name.name.clone(),
                            lower_generic_param_kind(param.kind.as_ref()),
                        )
                    })
                    .collect::<Vec<_>>();
                let kinds = params.iter().map(|(_, kind)| kind.clone()).collect();
                binders.push(params);
                let body = Self::lower_raw(context, &lambda.body, false, env, binders);
                binders.pop();
                Type::Lambda {
                    params: kinds,
                    body: Box::new(body),
                }
            }
            ast::ParseType::Hole(_) => {
                context.push_type_error(
                    "type holes are not yet supported".to_string(),
                    parse_type.span(),
                );
                Type::Error
            }
            ast::ParseType::Associated { base, member } => {
                let base_ty = Self::lower_inner_raw(context, base, false, env, binders);
                let trait_name = if base.name == "Self" {
                    context
                        .current_trait_name()
                        .unwrap_or_else(|| base.name.clone())
                } else {
                    base.name.clone()
                };
                let Some(trait_def) = context.resolve_trait_type(&trait_name) else {
                    context.push_type_error(
                        format!("unknown trait '{}' in associated type", trait_name),
                        crate::lexer::Span::new(
                            base.span.file_path.clone(),
                            base.span.start,
                            member.span.end,
                        ),
                    );
                    return Type::Error;
                };
                let Some(assoc_type) = trait_def
                    .associated_types
                    .iter()
                    .find(|assoc| assoc.name == member.name)
                    .map(|assoc| AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: assoc.id,
                    })
                else {
                    context.push_type_error(
                        format!(
                            "trait '{}' has no associated type '{}'",
                            trait_name, member.name
                        ),
                        crate::lexer::Span::new(
                            base.span.file_path.clone(),
                            base.span.start,
                            member.span.end,
                        ),
                    );
                    return Type::Error;
                };
                context.record_type_reference(
                    member.span.clone(),
                    crate::source_map::SourceSymbol::AssociatedType {
                        owner: trait_def.id,
                        associated: assoc_type.assoc_type_id,
                    },
                );
                let trait_args = trait_def
                    .generic_params
                    .iter()
                    .enumerate()
                    .map(|(index, _)| {
                        Type::Generic(GenericParamId {
                            owner: trait_def.id,
                            index: index as u32,
                        })
                    })
                    .collect();

                Type::Projection {
                    ty: Box::new(base_ty),
                    trait_id: trait_def.id,
                    assoc_type,
                    trait_args,
                }
            }
            ast::ParseType::Function(types) => {
                if types.is_empty() {
                    return Type::Unit;
                }
                if types.len() == 1 {
                    return Self::lower_raw(context, &types[0], false, env, binders);
                }
                if types.len() == 2 && matches!(&types[0], ast::ParseType::Unit(_)) {
                    let ret = Self::lower_raw(context, &types[1], false, env, binders);
                    return Type::function(Vec::new(), ret);
                }
                let params: Vec<Type> = types[..types.len() - 1]
                    .iter()
                    .map(|ty| Self::lower_raw(context, ty, false, env, binders))
                    .collect();
                let ret = Self::lower_raw(context, &types[types.len() - 1], false, env, binders);
                Type::function(params, ret)
            }
            ast::ParseType::Slice(inner) => {
                if !allow_bare_slice {
                    context.push_type_error(
                        "bare slice type [T] must be written behind a reference, such as &[T] or &mut [T]".to_string(),
                        parse_type.span(),
                    );
                    return Type::Error;
                }
                Type::Slice(Box::new(Self::lower_raw(
                    context, inner, false, env, binders,
                )))
            }
            ast::ParseType::Array { inner, len } => {
                let inner_ty = Self::lower_raw(context, inner, false, env, binders);
                Type::Array(Box::new(inner_ty), *len)
            }
            ast::ParseType::Tuple(elems) => Type::Tuple(
                elems
                    .iter()
                    .map(|ty| Self::lower_raw(context, ty, false, env, binders))
                    .collect(),
            ),
            ast::ParseType::Reference { is_mut, pointee } => Type::Reference {
                mutable: *is_mut,
                inner: Box::new(Self::lower_raw(context, pointee, true, env, binders)),
            },
            ast::ParseType::Pointer(inner) => Type::Pointer(Box::new(Self::lower_raw(
                context, inner, true, env, binders,
            ))),
        }
    }

    fn lower_inner_raw<C: TypeLoweringContext + ?Sized>(
        context: &mut C,
        inner: &ast::ParseTypeInner,
        allow_bare_slice: bool,
        env: &TypeNormalizationEnv,
        binders: &mut Vec<Vec<(String, Kind)>>,
    ) -> Type {
        let generics: Vec<Type> = inner
            .generics
            .iter()
            .map(|generic| Self::lower_raw(context, generic, false, env, binders))
            .collect();

        let base = match inner.name.as_str() {
            "I8" | "i8" => Type::I8,
            "I16" | "i16" => Type::I16,
            "I32" | "i32" | "Int" => Type::I32,
            "I64" | "i64" => Type::I64,
            "U8" | "u8" => Type::U8,
            "U16" | "u16" => Type::U16,
            "U32" | "u32" => Type::U32,
            "U64" | "u64" => Type::U64,
            "F32" | "f32" => Type::F32,
            "F64" | "f64" | "Float" => Type::F64,
            "Bool" | "bool" => Type::Bool,
            "Char" | "char" => Type::Char,
            "Str" => {
                if !allow_bare_slice {
                    context.push_type_error(
                        "bare string slice type Str must be written behind a reference, such as &Str"
                            .to_string(),
                        inner.span.clone(),
                    );
                    return Type::Error;
                }
                Type::Str
            }
            name => {
                if let Some((depth, index, kind)) =
                    binders.iter().rev().enumerate().find_map(|(depth, scope)| {
                        scope
                            .iter()
                            .position(|(binder, _)| binder == name)
                            .map(|index| (depth as u32, index as u32, scope[index].1.clone()))
                    })
                {
                    return if generics.is_empty() {
                        Type::BoundVar { depth, index, kind }
                    } else {
                        Type::Apply {
                            constructor: Box::new(Type::BoundVar { depth, index, kind }),
                            args: generics,
                        }
                    };
                }
                let current_module_name = context
                    .current_module_prefix()
                    .map(|prefix| format!("{}::{}", prefix, name));

                if let Some(generic) = context.existing_generic_type(name) {
                    if generics.is_empty() {
                        return generic;
                    }
                    return Type::Apply {
                        constructor: Box::new(generic),
                        args: generics,
                    };
                }
                if let Some(alias) = current_module_name
                    .as_deref()
                    .and_then(|name| context.resolve_type_alias(name))
                    .or_else(|| context.resolve_type_alias(name))
                {
                    context.record_type_reference(
                        inner.span.clone(),
                        crate::source_map::SourceSymbol::Definition(alias.id),
                    );
                    Type::Constructor {
                        id: alias.id,
                        flavor: NominalTypeKind::Alias,
                    }
                } else if let Some(nominal) = current_module_name
                    .as_deref()
                    .and_then(|name| context.resolve_nominal_type(name))
                    .or_else(|| context.resolve_nominal_type(name))
                {
                    match nominal {
                        ResolvedNominalType::Struct(struct_def) => {
                            context.record_type_reference(
                                inner.span.clone(),
                                crate::source_map::SourceSymbol::Definition(struct_def.id),
                            );
                            Type::Constructor {
                                id: struct_def.id,
                                flavor: NominalTypeKind::Struct,
                            }
                        }
                        ResolvedNominalType::Enum(enum_def) => {
                            context.record_type_reference(
                                inner.span.clone(),
                                crate::source_map::SourceSymbol::Definition(enum_def.id),
                            );
                            Type::Constructor {
                                id: enum_def.id,
                                flavor: NominalTypeKind::Enum,
                            }
                        }
                    }
                } else if let Some(trait_def) = current_module_name
                    .as_deref()
                    .and_then(|name| context.resolve_trait_type(name))
                    .or_else(|| context.resolve_trait_type(name))
                {
                    context.record_type_reference(
                        inner.span.clone(),
                        crate::source_map::SourceSymbol::Definition(trait_def.id),
                    );
                    let params = trait_def
                        .generic_params
                        .iter()
                        .map(|param| param.kind.clone())
                        .collect::<Vec<_>>();
                    let object = Type::Object(Box::new(crate::types::ObjectType::new(
                        crate::types::TraitBound {
                            trait_id: trait_def.id,
                            type_args: params
                                .iter()
                                .enumerate()
                                .map(|(index, kind)| Type::BoundVar {
                                    depth: 0,
                                    index: index as u32,
                                    kind: kind.clone(),
                                })
                                .collect(),
                        },
                    )));
                    if params.is_empty() {
                        object
                    } else {
                        Type::Lambda {
                            params,
                            body: Box::new(object),
                        }
                    }
                } else {
                    let ty = context.generic_type_for_name(name, inner.span.clone());
                    if let Type::Generic(generic) = ty {
                        context.record_type_reference(
                            inner.span.clone(),
                            crate::source_map::SourceSymbol::Generic(generic),
                        );
                    }
                    ty
                }
            }
        };
        if generics.is_empty() {
            base
        } else {
            Type::Apply {
                constructor: Box::new(base),
                args: generics,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use crate::ast::{GenericParamDecl as AstGenericParamDecl, Ident, ParseType, ParseTypeInner};
    use crate::hir::{HirAssociatedTypeDecl, HirEnum, HirStruct, HirTrait};
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId};

    #[derive(Default)]
    struct TestTypeContext {
        errors: Vec<String>,
        structs: HashMap<String, HirStruct>,
        enums: HashMap<String, HirEnum>,
        traits: HashMap<String, HirTrait>,
        current_trait: Option<String>,
        generic_owner: Option<DefId>,
        generic_params: Vec<String>,
        generic_kinds: HashMap<String, Kind>,
    }

    impl TypeLoweringContext for TestTypeContext {
        fn push_type_error(&mut self, message: String, _span: crate::lexer::Span) {
            self.errors.push(message);
        }

        fn current_module_prefix(&self) -> Option<String> {
            None
        }

        fn current_trait_name(&self) -> Option<String> {
            self.current_trait.clone()
        }

        fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
            self.structs
                .get(name)
                .cloned()
                .map(ResolvedNominalType::Struct)
                .or_else(|| self.enums.get(name).cloned().map(ResolvedNominalType::Enum))
        }

        fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
            self.traits.get(name).cloned()
        }

        fn existing_generic_type(&self, name: &str) -> Option<Type> {
            let owner = self.generic_owner?;
            let index = self.generic_params.iter().position(|param| param == name)?;
            Some(Type::Generic(GenericParamId {
                owner,
                index: index as u32,
            }))
        }

        fn admit_object_type(
            &mut self,
            ty: Type,
            env: &TypeNormalizationEnv,
            span: crate::lexer::Span,
        ) -> Type {
            let traits: HashMap<_, _> = self
                .traits
                .values()
                .map(|definition| (definition.id, definition.clone()))
                .collect();
            match crate::traits::objects::admit_type_objects(&ty, &traits, env) {
                Ok(ty) => ty,
                Err(error) => {
                    self.push_type_error(error.to_string(), span);
                    Type::Error
                }
            }
        }

        fn resolve_type_alias(&self, _name: &str) -> Option<HirTypeAlias> {
            None
        }

        fn qualify_object_type(
            &mut self,
            object: crate::types::ObjectType,
            bindings: Vec<ObjectBindingSyntax>,
            _binders: Vec<Vec<Kind>>,
            span: crate::lexer::Span,
        ) -> Type {
            let traits = self
                .traits
                .values()
                .map(|definition| (definition.id, definition.clone()))
                .collect();
            match resolve_object_bindings(object, bindings, &traits, span) {
                Ok(ty) => ty,
                Err((message, span)) => {
                    self.push_type_error(message, span);
                    Type::Error
                }
            }
        }

        fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type {
            let Some(owner) = self.generic_owner else {
                self.push_type_error(format!("unknown type '{}'", name), span.clone());
                return Type::Error;
            };
            let Some(index) = self
                .generic_params
                .iter()
                .position(|param| param == name)
                .or_else(|| {
                    let mut chars = name.chars();
                    let is_implicit_generic = matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
                        && chars.next().is_none();
                    if is_implicit_generic {
                        self.generic_params.push(name.to_string());
                        Some(self.generic_params.len() - 1)
                    } else {
                        None
                    }
                })
            else {
                self.push_type_error(format!("unknown type '{}'", name), span);
                return Type::Error;
            };

            Type::Generic(GenericParamId {
                owner,
                index: index as u32,
            })
        }

        fn populate_type_normalization_env(&self, env: &mut TypeNormalizationEnv) {
            for structure in self.structs.values() {
                env.register_constructor(
                    structure.id,
                    NominalTypeKind::Struct,
                    constructor_kind(&structure.generic_params),
                );
            }
            for enumeration in self.enums.values() {
                env.register_constructor(
                    enumeration.id,
                    NominalTypeKind::Enum,
                    constructor_kind(&enumeration.generic_params),
                );
            }
            for trait_def in self.traits.values() {
                for associated_type in &trait_def.associated_types {
                    env.register_projection_kind(
                        AssociatedTypeKey {
                            owner: trait_def.id,
                            assoc_type_id: associated_type.id,
                        },
                        associated_type.kind.clone(),
                    );
                }
            }
            if let Some(owner) = self.generic_owner {
                for (index, name) in self.generic_params.iter().enumerate() {
                    env.register_generic_kind(
                        GenericParamId {
                            owner,
                            index: index as u32,
                        },
                        self.generic_kinds.get(name).cloned().unwrap_or(Kind::Type),
                    );
                }
            }
        }
    }

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: Span::test(),
        })
    }

    fn application(constructor: ParseType, args: Vec<ParseType>) -> ParseType {
        ParseType::Application(ast::TypeApplication {
            constructor: Box::new(constructor),
            args,
            span: Span::test(),
        })
    }

    #[test]
    fn constructor_generic_application_preserves_its_argument() {
        let owner = def_id(21);
        let mut context = TestTypeContext {
            generic_owner: Some(owner),
            generic_params: vec!["F".to_string(), "A".to_string()],
            generic_kinds: HashMap::from([("F".to_string(), Kind::arrow(Kind::Type, Kind::Type))]),
            ..TestTypeContext::default()
        };

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &application(named_type("F"), vec![named_type("A")]),
        );

        assert_eq!(
            ty,
            Type::Apply {
                constructor: Box::new(Type::Generic(GenericParamId { owner, index: 0 })),
                args: vec![Type::Generic(GenericParamId { owner, index: 1 })],
            }
        );
        assert!(context.errors.is_empty());
    }

    #[test]
    fn result_section_and_explicit_lambda_lower_identically() {
        let result_id = def_id(22);
        let owner = def_id(23);
        let mut context = TestTypeContext {
            generic_owner: Some(owner),
            generic_params: vec!["E".to_string()],
            ..TestTypeContext::default()
        };
        context.enums.insert(
            "Result".to_string(),
            HirEnum {
                id: result_id,
                name: "Result".to_string(),
                generic_params: GenericParamDecl::type_params(result_id, ["T", "E"]),
                variants: Vec::new(),
            },
        );
        let section = application(
            named_type("Result"),
            vec![
                ParseType::Hole(ast::TypeHole { span: Span::test() }),
                named_type("E"),
            ],
        );
        let explicit = ParseType::Lambda(ast::TypeLambda {
            params: vec![AstGenericParamDecl {
                unsized_bound: None,
                name: Ident {
                    name: "T".to_string(),
                    span: Span::test(),
                },
                kind: None,
                span: Span::test(),
            }],
            body: Box::new(application(
                named_type("Result"),
                vec![named_type("T"), named_type("E")],
            )),
            span: Span::test(),
        });

        let section_ty = TypeLowerer::lower_parse_type(&mut context, &section);
        let explicit_ty = TypeLowerer::lower_parse_type(&mut context, &explicit);

        assert_eq!(section_ty, explicit_ty);
        assert!(context.errors.is_empty());
    }

    #[test]
    fn type_lowerer_consumes_canonical_nominal_resolution_result() {
        let struct_id = def_id(30);
        let mut context = TestTypeContext::default();
        context.structs.insert(
            "Alias".to_string(),
            HirStruct {
                id: struct_id,
                name: "dep::Actual".to_string(),
                generic_params: Vec::new(),
                fields: Vec::new(),
            },
        );

        let ty = TypeLowerer::lower_parse_type(&mut context, &named_type("Alias"));

        assert_eq!(
            ty,
            Type::Struct {
                id: struct_id,
                args: Vec::new(),
            }
        );
        assert!(context.errors.is_empty());
    }

    #[test]
    fn kind_check_rejects_nominal_arity_overapplication() {
        let struct_id = def_id(31);
        let mut context = TestTypeContext::default();
        context.structs.insert(
            "Box".to_string(),
            HirStruct {
                id: struct_id,
                name: "Box".to_string(),
                generic_params: vec![GenericParamDecl::type_param(
                    GenericParamId {
                        owner: struct_id,
                        index: 0,
                    },
                    "T",
                )],
                fields: Vec::new(),
            },
        );

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &application(
                named_type("Box"),
                vec![named_type("I64"), named_type("Bool")],
            ),
        );

        assert_eq!(ty, Type::Error);
        assert!(context
            .errors
            .iter()
            .any(|message| message.contains("not applicable")));
    }

    #[test]
    fn kind_check_rejects_nominal_argument_kind_mismatch() {
        let struct_id = def_id(32);
        let mut context = TestTypeContext::default();
        context.structs.insert(
            "Higher".to_string(),
            HirStruct {
                id: struct_id,
                name: "Higher".to_string(),
                generic_params: vec![GenericParamDecl::new(
                    GenericParamId {
                        owner: struct_id,
                        index: 0,
                    },
                    "F",
                    Kind::arrow(Kind::Type, Kind::Type),
                )],
                fields: Vec::new(),
            },
        );

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &application(named_type("Higher"), vec![named_type("I64")]),
        );

        assert_eq!(ty, Type::Error);
        assert!(context
            .errors
            .iter()
            .any(|message| message.contains("kind mismatch")));
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn type_lowerer_reports_bare_slice_error() {
        let mut context = TestTypeContext::default();

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &ParseType::Slice(Box::new(named_type("I64"))),
        );

        assert_eq!(ty, Type::Error);
        assert!(context
            .errors
            .iter()
            .any(|message| message.contains("bare slice type [T]")));
    }

    #[test]
    fn type_lowerer_lowers_associated_projection_with_trait_identity() {
        let trait_id = def_id(10);
        let assoc_type_id = AssocTypeId(2);
        let mut context = TestTypeContext {
            current_trait: Some("Iterator".to_string()),
            generic_owner: Some(trait_id),
            generic_params: vec!["Self".to_string()],
            ..TestTypeContext::default()
        };
        context.traits.insert(
            "Iterator".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Iterator".to_string(),
                generic_params: GenericParamDecl::type_params(trait_id, ["Self"]),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id,
                    name: "Item".to_string(),
                    kind: Kind::Type,
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &ParseType::Associated {
                base: ParseTypeInner {
                    name: "Self".to_string(),
                    generics: vec![],
                    span: Span::test(),
                },
                member: Ident {
                    name: "Item".to_string(),
                    span: Span::test(),
                },
            },
        );

        assert_eq!(
            ty,
            Type::Projection {
                ty: Box::new(Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })),
                trait_id,
                assoc_type: AssociatedTypeKey {
                    owner: trait_id,
                    assoc_type_id,
                },
                trait_args: vec![Type::Generic(GenericParamId {
                    owner: trait_id,
                    index: 0,
                })],
            }
        );
        assert!(context.errors.is_empty());
    }

    #[test]
    fn type_lowerer_applies_constructor_valued_associated_projection() {
        let trait_id = def_id(11);
        let assoc_type_id = AssocTypeId(0);
        let mut context = TestTypeContext {
            current_trait: Some("Families".to_string()),
            generic_owner: Some(trait_id),
            generic_params: vec!["Self".to_string()],
            ..TestTypeContext::default()
        };
        context.traits.insert(
            "Families".to_string(),
            HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Families".to_string(),
                generic_params: GenericParamDecl::type_params(trait_id, ["Self"]),
                associated_types: vec![HirAssociatedTypeDecl {
                    id: assoc_type_id,
                    name: "Family".to_string(),
                    kind: Kind::arrow(Kind::Type, Kind::Type),
                }],
                methods: HashMap::new(),
                signatures: HashMap::new(),
            },
        );
        let projection = ParseType::Associated {
            base: ParseTypeInner {
                name: "Self".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            member: Ident {
                name: "Family".to_string(),
                span: Span::test(),
            },
        };

        let ty = TypeLowerer::lower_parse_type(
            &mut context,
            &application(projection, vec![named_type("I64")]),
        );

        assert!(matches!(
            ty,
            Type::Apply {
                constructor,
                args
            } if matches!(*constructor, Type::Projection { assoc_type, .. }
                if assoc_type == (AssociatedTypeKey { owner: trait_id, assoc_type_id }))
                && args == vec![Type::I64]
        ));
        assert!(context.errors.is_empty());
    }

    #[test]
    fn type_lowerer_creates_implicit_generic_in_context() {
        let owner = def_id(20);
        let mut context = TestTypeContext {
            generic_owner: Some(owner),
            ..TestTypeContext::default()
        };

        let ty = TypeLowerer::lower_parse_type(&mut context, &named_type("T"));

        assert_eq!(ty, Type::Generic(GenericParamId { owner, index: 0 }));
        assert_eq!(context.generic_params, vec!["T".to_string()]);
        assert!(context.errors.is_empty());
    }
}
