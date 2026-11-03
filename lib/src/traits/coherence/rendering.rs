use super::*;

pub(super) fn coherence_display_context(
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

pub(super) fn render_receiver_pattern(
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

pub(super) fn render_target(
    target: &CanonicalTarget,
    display_type: &impl Fn(&Type) -> String,
) -> String {
    match target {
        CanonicalTarget::Exact(ty) | CanonicalTarget::Constructor(ty) => display_type(ty),
        CanonicalTarget::SliceFamily(element) => format!("[{}]", display_type(element)),
    }
}

pub(super) fn render_normalize_error(
    error: &NormalizeError,
    display_type: &impl Fn(&Type) -> String,
) -> String {
    match error {
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

pub(super) fn render_witness(
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

pub(super) fn coherence_error_sort_key(error: &CoherenceError) -> (u32, u32, u8, u32, u32) {
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
