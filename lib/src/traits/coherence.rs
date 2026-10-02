use std::collections::{HashMap, HashSet};

use crate::hir::{HirEnum, HirImpl, HirImplReceiverPattern, HirStruct, HirTrait, HirTypeAlias};
use crate::ids::DefId;
use crate::lower::Lowerer;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::{NormalizeError, TypeNormalizationEnv, TypeNormalizer};
use crate::types::{AssociatedTypeKey, GenericParamId, NominalTypeKind, Type};

pub struct CoherencePhase;

impl CoherencePhase {
    pub(crate) fn run(lowerer: &mut Lowerer) {
        let errors = validate_coherence_with_ids(
            lowerer.items.trait_defs().map(|(_, trait_def)| trait_def),
            lowerer.items.impl_defs().map(|(_, imp)| imp),
            lowerer.items.structures().map(|(_, structure)| structure),
            lowerer
                .items
                .enumerations()
                .map(|(_, enumeration)| enumeration),
            lowerer.items.type_aliases().map(|(_, alias)| alias),
            lowerer
                .language_items
                .sized
                .as_ref()
                .map(|items| items.trait_id),
        );
        for error in errors {
            let Some(primary_impl_id) = error.current_impl_id(lowerer.root_crate_id) else {
                lowerer
                    .diagnostics
                    .push_toolchain_once(error.render_toolchain());
                continue;
            };
            let span = lowerer
                .source_map
                .definition_span(primary_impl_id)
                .cloned()
                .expect("current-crate coherence error requires its impl source span");
            let message = error.render_with(
                |ty| lowerer.display_type(ty),
                |id| {
                    lowerer
                        .canonical_name_for_def_id(id)
                        .unwrap_or("<unknown item>")
                        .to_string()
                },
            );
            let labels = error
                .overlapping_other_impl_id(primary_impl_id)
                .filter(|id| id.crate_id == lowerer.root_crate_id)
                .and_then(|id| lowerer.source_map.definition_span(id).cloned())
                .into_iter()
                .map(|span| {
                    crate::diagnostic::DiagnosticLabel::new("overlapping implementation", span)
                })
                .collect();
            lowerer
                .diagnostics
                .push_with_span_and_labels(message, span, labels);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceError {
    pub impl_id: DefId,
    pub kind: CoherenceErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoherenceErrorKind {
    InvalidTarget {
        trait_id: DefId,
        target: HirImplReceiverPattern,
        error: NormalizeError,
    },
    TargetKindMismatch {
        trait_id: DefId,
        expected: Kind,
        actual: Kind,
    },
    TargetCategoryMismatch {
        trait_id: DefId,
        expected: Kind,
    },
    InvalidConstructorTarget {
        trait_id: DefId,
        target: Type,
        reason: String,
    },
    Orphan {
        trait_id: DefId,
        target: CanonicalTarget,
    },
    InvalidTraitArguments {
        trait_id: DefId,
        error: NormalizeError,
    },
    Overlapping {
        left_impl_id: DefId,
        right_impl_id: DefId,
        trait_id: DefId,
        left_target: CanonicalTarget,
        right_target: CanonicalTarget,
        witness: Vec<CoherenceWitnessBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoherenceWitnessBinding {
    pub parameter: GenericParamId,
    pub value: Type,
}

impl CoherenceError {
    fn render_with(
        &self,
        display_type: impl Fn(&Type) -> String,
        display_name: impl Fn(DefId) -> String,
    ) -> String {
        let trait_name = |id| display_name(id);
        match &self.kind {
            CoherenceErrorKind::InvalidTarget {
                trait_id,
                target,
                error,
            } => format!(
                "invalid impl target for trait `{}`: `{}` cannot be normalized ({})",
                trait_name(*trait_id),
                render_receiver_pattern(target, &display_type),
                render_normalize_error(error, &display_type),
            ),
            CoherenceErrorKind::TargetKindMismatch {
                trait_id,
                expected,
                actual,
            } => format!(
                "constructor impl target for trait `{}` has kind {actual}, but the trait requires {expected}",
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::TargetCategoryMismatch {
                trait_id,
                expected,
            } => format!(
                "impl target for trait `{}` does not match the required target kind `{expected}`",
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::InvalidConstructorTarget {
                trait_id,
                target,
                reason,
            } => format!(
                "invalid constructor impl target `{}` for trait `{}`: {reason}",
                display_type(target),
                trait_name(*trait_id),
            ),
            CoherenceErrorKind::Orphan { trait_id, target } => format!(
                "orphan impl for trait `{}` is illegal: trait and normalized target `{}` are both foreign",
                trait_name(*trait_id),
                render_target(target, &display_type),
            ),
            CoherenceErrorKind::InvalidTraitArguments { trait_id, error } => format!(
                "invalid trait arguments for trait `{}`: {}",
                trait_name(*trait_id),
                render_normalize_error(error, &display_type),
            ),
            CoherenceErrorKind::Overlapping {
                trait_id,
                left_target,
                right_target,
                witness,
                ..
            } => format!(
                "overlapping impls for trait `{}`: normalized targets `{}` and `{}`; witness {}",
                trait_name(*trait_id),
                render_target(left_target, &display_type),
                render_target(right_target, &display_type),
                render_witness(witness, &display_type),
            ),
        }
    }

    fn render_toolchain(&self) -> String {
        format!(
            "invalid coherence metadata in an external implementation: {}",
            self.neutral_reason()
        )
    }

    fn neutral_reason(&self) -> &'static str {
        match &self.kind {
            CoherenceErrorKind::InvalidTarget { .. } => "invalid target",
            CoherenceErrorKind::TargetKindMismatch { .. } => "target kind mismatch",
            CoherenceErrorKind::TargetCategoryMismatch { .. } => "target category mismatch",
            CoherenceErrorKind::InvalidConstructorTarget { .. } => "invalid constructor target",
            CoherenceErrorKind::Orphan { .. } => "orphan implementation",
            CoherenceErrorKind::InvalidTraitArguments { .. } => "invalid trait arguments",
            CoherenceErrorKind::Overlapping { .. } => "overlapping implementations",
        }
    }

    fn current_impl_id(&self, root_crate_id: crate::ids::CrateId) -> Option<DefId> {
        if self.impl_id.crate_id == root_crate_id {
            return Some(self.impl_id);
        }
        match &self.kind {
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } => [*left_impl_id, *right_impl_id]
                .into_iter()
                .find(|id| id.crate_id == root_crate_id),
            _ => None,
        }
    }

    fn overlapping_other_impl_id(&self, primary_impl_id: DefId) -> Option<DefId> {
        match &self.kind {
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } if *left_impl_id == primary_impl_id => Some(*right_impl_id),
            CoherenceErrorKind::Overlapping {
                left_impl_id,
                right_impl_id,
                ..
            } if *right_impl_id == primary_impl_id => Some(*left_impl_id),
            _ => None,
        }
    }
}

pub fn validate_coherence<'a>(
    traits: impl IntoIterator<Item = &'a HirTrait>,
    impls: impl IntoIterator<Item = &'a HirImpl>,
    structs: impl IntoIterator<Item = &'a HirStruct>,
    enums: impl IntoIterator<Item = &'a HirEnum>,
    aliases: impl IntoIterator<Item = &'a HirTypeAlias>,
    sized_trait_id: Option<DefId>,
) -> Vec<String> {
    let traits = traits.into_iter().collect::<Vec<_>>();
    let impls = impls.into_iter().collect::<Vec<_>>();
    let structs = structs.into_iter().collect::<Vec<_>>();
    let enums = enums.into_iter().collect::<Vec<_>>();
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    let display_context = coherence_display_context(&traits, &impls, &structs, &enums, &aliases);
    validate_coherence_with_ids(
        traits.iter().copied(),
        impls.iter().copied(),
        structs.iter().copied(),
        enums.iter().copied(),
        aliases.iter().copied(),
        sized_trait_id,
    )
    .into_iter()
    .map(|error| {
        error.render_with(
            |ty| {
                crate::type_services::display::display_type_with_context(ty, &display_context)
                    .to_string()
            },
            |id| {
                display_context
                    .definition_name(id)
                    .unwrap_or("<unknown item>")
                    .to_string()
            },
        )
    })
    .collect()
}

fn validate_coherence_with_ids<'a>(
    traits: impl IntoIterator<Item = &'a HirTrait>,
    impls: impl IntoIterator<Item = &'a HirImpl>,
    structs: impl IntoIterator<Item = &'a HirStruct>,
    enums: impl IntoIterator<Item = &'a HirEnum>,
    aliases: impl IntoIterator<Item = &'a HirTypeAlias>,
    sized_trait_id: Option<DefId>,
) -> Vec<CoherenceError> {
    let traits = traits.into_iter().collect::<Vec<_>>();
    let mut impls = impls.into_iter().collect::<Vec<_>>();
    let structs = structs.into_iter().collect::<Vec<_>>();
    let enums = enums.into_iter().collect::<Vec<_>>();
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    impls.sort_by_key(|imp| def_id_key(imp.id));

    let trait_by_id = traits
        .iter()
        .map(|trait_def| (trait_def.id, *trait_def))
        .collect::<HashMap<_, _>>();
    let env = normalization_env(&traits, &impls, &structs, &enums, &aliases);
    let mut diagnostics = Vec::new();
    let mut targets = HashMap::new();
    let mut trait_args = HashMap::new();

    for imp in &impls {
        let Some(trait_id) = imp.trait_id else {
            continue;
        };
        let Some(trait_def) = trait_by_id.get(&trait_id).copied() else {
            continue;
        };

        let target = match normalize_target(&env, &imp.receiver_pattern) {
            Ok(target) => target,
            Err(error) => {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidTarget {
                        trait_id,
                        target: imp.receiver_pattern.clone(),
                        error,
                    },
                });
                continue;
            }
        };
        let expected_kind = trait_def
            .target
            .as_ref()
            .map(|target| target.kind.clone())
            .unwrap_or(Kind::Type);
        let actual_kind = match target.kind(&env) {
            Ok(kind) => kind,
            Err(error) => {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidTarget {
                        trait_id,
                        target: imp.receiver_pattern.clone(),
                        error,
                    },
                });
                continue;
            }
        };
        if actual_kind != expected_kind {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::TargetKindMismatch {
                    trait_id,
                    expected: expected_kind,
                    actual: actual_kind,
                },
            });
            continue;
        }
        if target.is_constructor() != !matches!(expected_kind, Kind::Type) {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::TargetCategoryMismatch {
                    trait_id,
                    expected: expected_kind,
                },
            });
            continue;
        }
        if let CanonicalTarget::Constructor(ty) = &target {
            if let Err(error) = validate_constructor_header(ty, &imp_generic_ids(imp)) {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidConstructorTarget {
                        trait_id,
                        target: ty.clone(),
                        reason: error,
                    },
                });
                continue;
            }
        }

        let outer = target.outer_nominal();
        if trait_id.crate_id != imp.id.crate_id
            && outer.is_none_or(|outer_id| outer_id.crate_id != imp.id.crate_id)
        {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::Orphan {
                    trait_id,
                    target: target.clone(),
                },
            });
        }

        let normalized_args = imp
            .trait_arg_types
            .iter()
            .map(|arg| TypeNormalizer::new(&env).normalize(arg))
            .collect::<Result<Vec<_>, _>>();
        match normalized_args {
            Ok(args) => {
                targets.insert(imp.id, target);
                trait_args.insert(imp.id, args);
            }
            Err(error) => diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::InvalidTraitArguments { trait_id, error },
            }),
        }
    }

    for (index, left) in impls.iter().enumerate() {
        let Some(trait_id) = left.trait_id else {
            continue;
        };
        let Some(left_target) = targets.get(&left.id) else {
            continue;
        };
        for right in impls.iter().skip(index + 1) {
            if right.trait_id != Some(trait_id) {
                continue;
            }
            let Some(right_target) = targets.get(&right.id) else {
                continue;
            };
            let left_args = &trait_args[&left.id];
            let right_args = &trait_args[&right.id];
            if left_args.len() != right_args.len() {
                continue;
            }

            let mut unifier = FirstOrderUnifier::new(left, right, sized_trait_id, &env);
            if !left_args
                .iter()
                .zip(right_args)
                .all(|(left, right)| unifier.unify(left, right))
                || !unifier.unify_targets(left_target, right_target)
            {
                continue;
            }
            diagnostics.push(CoherenceError {
                impl_id: left.id,
                kind: CoherenceErrorKind::Overlapping {
                    left_impl_id: left.id,
                    right_impl_id: right.id,
                    trait_id,
                    left_target: left_target.clone(),
                    right_target: right_target.clone(),
                    witness: unifier.witness(),
                },
            });
        }
    }

    diagnostics.sort_by_key(coherence_error_sort_key);
    diagnostics.dedup();
    diagnostics
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CanonicalTarget {
    Exact(Type),
    SliceFamily(Type),
    Constructor(Type),
}

impl CanonicalTarget {
    fn is_constructor(&self) -> bool {
        matches!(self, Self::Constructor(_))
    }

    fn kind(&self, env: &TypeNormalizationEnv) -> Result<Kind, NormalizeError> {
        match self {
            Self::SliceFamily(_) => Ok(Kind::Type),
            Self::Exact(ty) | Self::Constructor(ty) => TypeNormalizer::new(env).kind_of(ty),
        }
    }

    fn outer_nominal(&self) -> Option<DefId> {
        match self {
            Self::Exact(ty) | Self::Constructor(ty) => outer_nominal(ty),
            Self::SliceFamily(_) => None,
        }
    }
}

fn normalize_target(
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

fn coherence_display_context(
    traits: &[&HirTrait],
    impls: &[&HirImpl],
    structs: &[&HirStruct],
    enums: &[&HirEnum],
    aliases: &[&HirTypeAlias],
) -> crate::type_services::display::TypeDisplayContext {
    let mut context = crate::type_services::display::TypeDisplayContext::default();
    for structure in structs {
        context.insert_definition_name(structure.id, structure.name.clone());
        for generic in &structure.generic_params {
            context.insert_generic_name(generic.id, generic.name.clone());
        }
    }
    for enumeration in enums {
        context.insert_definition_name(enumeration.id, enumeration.name.clone());
        for generic in &enumeration.generic_params {
            context.insert_generic_name(generic.id, generic.name.clone());
        }
    }
    for alias in aliases {
        context.insert_definition_name(alias.id, alias.name.clone());
        for generic in &alias.generic_params {
            context.insert_generic_name(generic.id, generic.name.clone());
        }
    }
    for trait_def in traits {
        context.insert_definition_name(trait_def.id, trait_def.name.clone());
        for generic in &trait_def.generic_params {
            context.insert_generic_name(generic.id, generic.name.clone());
        }
        if let Some(target) = &trait_def.target {
            context.insert_generic_name(target.id, target.name.clone());
        }
        for associated in &trait_def.associated_types {
            context.insert_associated_name(
                AssociatedTypeKey {
                    owner: trait_def.id,
                    assoc_type_id: associated.id,
                },
                associated.name.clone(),
            );
        }
    }
    for imp in impls {
        for generic in imp.type_generics.iter().chain(&imp.trait_generics) {
            context.insert_generic_name(generic.id, generic.name.clone());
        }
        for associated in &imp.associated_types {
            context.insert_associated_name(
                AssociatedTypeKey {
                    owner: imp.id,
                    assoc_type_id: associated.id,
                },
                associated.name.clone(),
            );
        }
    }
    context
}

fn render_receiver_pattern(
    target: &HirImplReceiverPattern,
    display_type: &impl Fn(&Type) -> String,
) -> String {
    match target {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
            display_type(ty)
        }
        HirImplReceiverPattern::SliceFamily { element } => format!("[{}]", display_type(element)),
    }
}

fn render_target(target: &CanonicalTarget, display_type: &impl Fn(&Type) -> String) -> String {
    match target {
        CanonicalTarget::Exact(ty) | CanonicalTarget::Constructor(ty) => display_type(ty),
        CanonicalTarget::SliceFamily(element) => format!("[{}]", display_type(element)),
    }
}

fn render_normalize_error(
    error: &NormalizeError,
    display_type: &impl Fn(&Type) -> String,
) -> String {
    match error {
        NormalizeError::Substitution(error) => format!("type substitution failed: {error}"),
        NormalizeError::ConflictingObjectBinding(_) => {
            "conflicting associated-type bindings in trait object".to_string()
        }
        NormalizeError::AliasCycle(_) => "type alias cycle".to_string(),
        NormalizeError::DepthLimit { .. } => "type normalization depth limit exceeded".to_string(),
        NormalizeError::NodeLimit { .. } => "type normalization node limit exceeded".to_string(),
        NormalizeError::UnknownConstructor(_) => "unknown type constructor".to_string(),
        NormalizeError::KindMismatch { expected, actual } => {
            format!("kind mismatch: expected {expected}, found {actual}")
        }
        NormalizeError::NotApplicable { kind } => {
            format!("type of kind {kind} is not applicable")
        }
        NormalizeError::NonCanonicalType { normalized } => format!(
            "type is not canonical; normalized form is `{}`",
            display_type(normalized)
        ),
    }
}

fn render_witness(
    witness: &[CoherenceWitnessBinding],
    display_type: &impl Fn(&Type) -> String,
) -> String {
    if witness.is_empty() {
        return "<identity>".to_string();
    }
    witness
        .iter()
        .map(|binding| {
            format!(
                "{} = {}",
                display_type(&Type::Generic(binding.parameter)),
                display_type(&binding.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn coherence_error_sort_key(error: &CoherenceError) -> (u32, u32, u8, u32, u32) {
    let (kind, other) = match &error.kind {
        CoherenceErrorKind::InvalidTarget { .. } => (0, (0, 0)),
        CoherenceErrorKind::TargetKindMismatch { .. } => (1, (0, 0)),
        CoherenceErrorKind::TargetCategoryMismatch { .. } => (2, (0, 0)),
        CoherenceErrorKind::InvalidConstructorTarget { .. } => (3, (0, 0)),
        CoherenceErrorKind::Orphan { .. } => (4, (0, 0)),
        CoherenceErrorKind::InvalidTraitArguments { .. } => (5, (0, 0)),
        CoherenceErrorKind::Overlapping { right_impl_id, .. } => (6, def_id_key(*right_impl_id)),
    };
    (
        error.impl_id.crate_id.0,
        error.impl_id.local.0,
        kind,
        other.0,
        other.1,
    )
}

fn normalization_env(
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

fn validate_constructor_header(
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

fn imp_generic_ids(imp: &HirImpl) -> HashSet<GenericParamId> {
    imp.type_generics.iter().map(|param| param.id).collect()
}

struct FirstOrderUnifier<'a> {
    flexible: HashSet<GenericParamId>,
    requires_sized: HashSet<GenericParamId>,
    generic_kinds: HashMap<GenericParamId, Kind>,
    bindings: HashMap<GenericParamId, Type>,
    env: &'a TypeNormalizationEnv,
}

impl<'a> FirstOrderUnifier<'a> {
    fn new(
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

    fn unify_targets(&mut self, left: &CanonicalTarget, right: &CanonicalTarget) -> bool {
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

    fn unify(&mut self, left: &Type, right: &Type) -> bool {
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
                left_len == right_len && self.unify(left, right)
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
                },
                Type::Function {
                    params: right_params,
                    ret: right_ret,
                    safety: right_safety,
                    callable_kind: right_kind,
                    captures: right_captures,
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

    fn witness(&self) -> Vec<CoherenceWitnessBinding> {
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

fn def_id_key(id: DefId) -> (u32, u32) {
    (id.crate_id.0, id.local.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::hir::{HirGenericBounds, HirImplOwner};
    use crate::ids::{CrateId, LocalDefId};
    use crate::types::GenericParamDecl;

    fn id(crate_id: u32, local: u32) -> DefId {
        DefId::new(CrateId(crate_id), LocalDefId(local))
    }

    fn unary_kind() -> Kind {
        Kind::arrow(Kind::Type, Kind::Type)
    }

    fn trait_def(trait_id: DefId, target_kind: Kind) -> HirTrait {
        HirTrait {
            id: trait_id,
            name: format!("Trait{}", trait_id.local.0),
            generic_params: Vec::new(),
            target: Some(GenericParamDecl::new(
                GenericParamId {
                    owner: trait_id,
                    index: 0,
                },
                "F",
                target_kind,
            )),
            predicates: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    fn structure(struct_id: DefId, arity: usize) -> HirStruct {
        HirStruct {
            id: struct_id,
            name: format!("Type{}", struct_id.local.0),
            generic_params: (0..arity)
                .map(|index| {
                    GenericParamDecl::type_param(
                        GenericParamId {
                            owner: struct_id,
                            index: index as u32,
                        },
                        format!("T{index}"),
                    )
                })
                .collect(),
            fields: Vec::new(),
        }
    }

    fn trait_impl(
        impl_id: DefId,
        trait_id: DefId,
        target: HirImplReceiverPattern,
        generics: Vec<GenericParamDecl>,
    ) -> HirImpl {
        HirImpl {
            id: impl_id,
            owner: HirImplOwner::Named("target".to_string()),
            type_name: "target".to_string(),
            type_generics: generics,
            receiver_pattern: target,
            trait_name: Some("Trait".to_string()),
            trait_id: Some(trait_id),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HirGenericBounds::new(),
            methods: HashMap::new(),
        }
    }

    fn check(
        traits: &[HirTrait],
        impls: &[HirImpl],
        structs: &[HirStruct],
        aliases: &[HirTypeAlias],
    ) -> Vec<String> {
        check_with_sized(traits, impls, structs, aliases, None)
    }

    fn check_with_sized(
        traits: &[HirTrait],
        impls: &[HirImpl],
        structs: &[HirStruct],
        aliases: &[HirTypeAlias],
        sized_trait_id: Option<DefId>,
    ) -> Vec<String> {
        validate_coherence(
            traits.iter(),
            impls.iter(),
            structs.iter(),
            std::iter::empty(),
            aliases.iter(),
            sized_trait_id,
        )
    }

    #[test]
    fn constructor_impl_target_kind_must_match_trait_target_kind() {
        let trait_id = id(0, 1);
        let imp = trait_impl(
            id(0, 2),
            trait_id,
            HirImplReceiverPattern::Constructor(Type::I64),
            Vec::new(),
        );

        let errors = check(&[trait_def(trait_id, unary_kind())], &[imp], &[], &[]);

        assert!(errors.iter().any(|error| error.contains("has kind Type")));
    }

    #[test]
    fn orphan_rules_accept_one_local_side_and_reject_two_foreign_sides() {
        let foreign_constructor = id(2, 1);
        let local_constructor = id(0, 2);
        let local_trait = id(0, 3);
        let foreign_trait = id(1, 4);
        let target = |constructor| {
            HirImplReceiverPattern::Exact(Type::Struct {
                id: constructor,
                args: Vec::new(),
            })
        };
        let impls = vec![
            trait_impl(
                id(0, 10),
                local_trait,
                target(foreign_constructor),
                Vec::new(),
            ),
            trait_impl(
                id(0, 11),
                foreign_trait,
                target(local_constructor),
                Vec::new(),
            ),
            trait_impl(
                id(0, 12),
                foreign_trait,
                target(foreign_constructor),
                Vec::new(),
            ),
        ];

        let errors = check(
            &[
                trait_def(local_trait, Kind::Type),
                trait_def(foreign_trait, Kind::Type),
            ],
            &impls,
            &[
                structure(foreign_constructor, 0),
                structure(local_constructor, 0),
            ],
            &[],
        );

        assert_eq!(
            errors
                .iter()
                .filter(|error| error.contains("orphan impl"))
                .count(),
            1
        );
        assert!(errors
            .iter()
            .any(|error| error.contains("Trait4") && error.contains("Type1")));
    }

    #[test]
    fn orphan_locality_uses_alias_expanded_outer_constructor() {
        let trait_id = id(1, 1);
        let alias_id = id(0, 2);
        let foreign = id(2, 3);
        let alias = HirTypeAlias {
            id: alias_id,
            name: "LocalAlias".to_string(),
            generic_params: Vec::new(),
            ty: Type::Struct {
                id: foreign,
                args: Vec::new(),
            },
        };
        let imp = trait_impl(
            id(0, 4),
            trait_id,
            HirImplReceiverPattern::Exact(Type::Constructor {
                id: alias_id,
                flavor: NominalTypeKind::Alias,
            }),
            Vec::new(),
        );

        let errors = check(
            &[trait_def(trait_id, Kind::Type)],
            &[imp],
            &[structure(foreign, 0)],
            &[alias],
        );

        assert!(errors.iter().any(|error| error.contains("orphan impl")));
    }

    #[test]
    fn alpha_renamed_constructor_targets_overlap() {
        let trait_id = id(0, 1);
        let option = id(0, 2);
        let lambda = || Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Struct {
                id: option,
                args: vec![Type::BoundVar {
                    depth: 0,
                    index: 0,
                    kind: Kind::Type,
                }],
            }),
        };
        let impls = vec![
            trait_impl(
                id(0, 3),
                trait_id,
                HirImplReceiverPattern::Constructor(lambda()),
                Vec::new(),
            ),
            trait_impl(
                id(0, 4),
                trait_id,
                HirImplReceiverPattern::Constructor(lambda()),
                Vec::new(),
            ),
        ];

        let errors = check(
            &[trait_def(trait_id, unary_kind())],
            &impls,
            &[structure(option, 1)],
            &[],
        );

        assert!(errors
            .iter()
            .any(|error| error.contains("overlapping impls")));
    }

    #[test]
    fn result_section_overlaps_explicit_lambda_with_first_order_witness() {
        let trait_id = id(0, 1);
        let result = id(0, 2);
        let left_id = id(0, 3);
        let generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: left_id,
                index: 0,
            },
            "E",
        );
        let body = |error: Type| Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(Type::Struct {
                id: result,
                args: vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: Kind::Type,
                    },
                    error,
                ],
            }),
        };
        let impls = vec![
            trait_impl(
                left_id,
                trait_id,
                HirImplReceiverPattern::Constructor(body(Type::Generic(generic.id))),
                vec![generic],
            ),
            trait_impl(
                id(0, 4),
                trait_id,
                HirImplReceiverPattern::Constructor(body(Type::I64)),
                Vec::new(),
            ),
        ];

        let errors = check(
            &[trait_def(trait_id, unary_kind())],
            &impls,
            &[structure(result, 2)],
            &[],
        );

        assert!(errors.iter().any(|error| {
            error.contains("Trait1") && error.contains("Type2") && error.contains("E = I64")
        }));
    }

    #[test]
    fn overlap_errors_retain_semantic_ids_targets_and_witnesses() {
        let trait_id = id(0, 1);
        let target_id = id(0, 2);
        let left_id = id(0, 3);
        let right_id = id(0, 4);
        let generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: left_id,
                index: 0,
            },
            "T",
        );
        let target = |ty| {
            HirImplReceiverPattern::Exact(Type::Struct {
                id: target_id,
                args: vec![ty],
            })
        };
        let impls = vec![
            trait_impl(
                left_id,
                trait_id,
                target(Type::Generic(generic.id)),
                vec![generic.clone()],
            ),
            trait_impl(right_id, trait_id, target(Type::I64), Vec::new()),
        ];

        let errors = validate_coherence_with_ids(
            [&trait_def(trait_id, Kind::Type)].into_iter(),
            impls.iter(),
            [&structure(target_id, 1)].into_iter(),
            std::iter::empty(),
            std::iter::empty(),
            None,
        );
        let error = errors
            .iter()
            .find(|error| matches!(&error.kind, CoherenceErrorKind::Overlapping { .. }))
            .expect("overlap error");

        assert_eq!(error.impl_id, left_id);
        let CoherenceErrorKind::Overlapping {
            left_impl_id,
            right_impl_id,
            trait_id: found_trait_id,
            left_target,
            right_target,
            witness,
        } = &error.kind
        else {
            unreachable!();
        };
        assert_eq!(*left_impl_id, left_id);
        assert_eq!(*right_impl_id, right_id);
        assert_eq!(*found_trait_id, trait_id);
        assert!(matches!(
            left_target,
            CanonicalTarget::Exact(Type::Struct { id, args })
                if *id == target_id && args == &vec![Type::Generic(generic.id)]
        ));
        assert!(matches!(
            right_target,
            CanonicalTarget::Exact(Type::Struct { id, args })
                if *id == target_id && args == &vec![Type::I64]
        ));
        assert_eq!(
            witness,
            &vec![CoherenceWitnessBinding {
                parameter: generic.id,
                value: Type::I64,
            }]
        );
    }

    #[test]
    fn coherence_compares_local_and_dependency_impls() {
        let trait_id = id(0, 1);
        let target_id = id(0, 2);
        let target = || {
            HirImplReceiverPattern::Exact(Type::Struct {
                id: target_id,
                args: Vec::new(),
            })
        };
        let impls = vec![
            trait_impl(id(0, 3), trait_id, target(), Vec::new()),
            trait_impl(id(1, 4), trait_id, target(), Vec::new()),
        ];

        let errors = check(
            &[trait_def(trait_id, Kind::Type)],
            &impls,
            &[structure(target_id, 0)],
            &[],
        );

        assert!(errors.iter().any(|error| {
            error.contains("Trait1") && error.contains("Type2") && !error.contains("impl#")
        }));
    }

    #[test]
    fn constructor_variable_blanket_target_is_rejected() {
        let trait_id = id(0, 1);
        let impl_id = id(0, 2);
        let generic = GenericParamDecl::new(
            GenericParamId {
                owner: impl_id,
                index: 0,
            },
            "F",
            unary_kind(),
        );
        let imp = trait_impl(
            impl_id,
            trait_id,
            HirImplReceiverPattern::Constructor(Type::Generic(generic.id)),
            vec![generic],
        );

        let errors = check(&[trait_def(trait_id, unary_kind())], &[imp], &[], &[]);

        assert!(errors
            .iter()
            .any(|error| error.contains("concrete nominal outer constructor")));
    }

    #[test]
    fn coherence_diagnostics_are_independent_of_impl_insertion_order() {
        let trait_id = id(0, 1);
        let target_id = id(0, 2);
        let target = || {
            HirImplReceiverPattern::Exact(Type::Struct {
                id: target_id,
                args: Vec::new(),
            })
        };
        let left = trait_impl(id(0, 3), trait_id, target(), Vec::new());
        let right = trait_impl(id(0, 4), trait_id, target(), Vec::new());
        let traits = [trait_def(trait_id, Kind::Type)];
        let structs = [structure(target_id, 0)];

        assert_eq!(
            check(&traits, &[left.clone(), right.clone()], &structs, &[]),
            check(&traits, &[right, left], &structs, &[]),
        );
    }

    #[test]
    fn sized_blanket_cannot_overlap_an_unsized_slice_target() {
        let index_trait = id(0, 1);
        let sized_trait = id(0, 2);
        let blanket_id = id(0, 3);
        let slice_id = id(0, 4);
        let blanket_generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: blanket_id,
                index: 0,
            },
            "T",
        );
        let slice_generic = GenericParamDecl::type_param(
            GenericParamId {
                owner: slice_id,
                index: 0,
            },
            "U",
        );
        let mut blanket = trait_impl(
            blanket_id,
            index_trait,
            HirImplReceiverPattern::Exact(Type::Pointer(Box::new(Type::Generic(
                blanket_generic.id,
            )))),
            vec![blanket_generic.clone()],
        );
        blanket.bounds.insert(
            blanket_generic.id,
            vec![crate::types::TraitBound {
                trait_id: sized_trait,
                type_args: Vec::new(),
            }],
        );
        let slice = trait_impl(
            slice_id,
            index_trait,
            HirImplReceiverPattern::Exact(Type::Pointer(Box::new(Type::Slice(Box::new(
                Type::Generic(slice_generic.id),
            ))))),
            vec![slice_generic],
        );
        let traits = [
            trait_def(index_trait, Kind::Type),
            trait_def(sized_trait, Kind::Type),
        ];

        assert!(check_with_sized(
            &traits,
            &[blanket.clone(), slice.clone()],
            &[],
            &[],
            Some(sized_trait),
        )
        .iter()
        .all(|error| !error.contains("overlapping impls")));
        assert!(check(&traits, &[blanket, slice], &[], &[])
            .iter()
            .any(|error| error.contains("overlapping impls")));
    }
}
