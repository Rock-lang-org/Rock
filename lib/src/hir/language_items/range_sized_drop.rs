use super::*;

pub(super) fn validate_range<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &RangeLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Some(range_enum) = required_enum(program, items.enum_id, "range.enum", errors) else {
        return;
    };
    if !range_enum.generic_params.is_empty() {
        errors.push("range.enum must not declare generic parameters".to_string());
    }
    for (role, variant_id, arity) in [
        ("range.range_full", items.full_variant_id, 0usize),
        ("range.range_from", items.from_variant_id, 1),
        ("range.range_to", items.to_variant_id, 1),
        ("range.range_to_inclusive", items.to_inclusive_variant_id, 1),
        ("range.range_exclusive", items.exclusive_variant_id, 2),
        ("range.range_inclusive", items.inclusive_variant_id, 2),
    ] {
        let Some(variant) = range_enum
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)
        else {
            errors.push(format!(
                "{role} {variant_id:?} is not declared by enum {:?}",
                items.enum_id
            ));
            continue;
        };
        let fields = match &variant.fields {
            crate::hir::HirVariantFields::Unit => Vec::new(),
            crate::hir::HirVariantFields::Positional(fields) => fields.clone(),
            crate::hir::HirVariantFields::Named(fields) => {
                fields.iter().map(|field| field.ty.clone()).collect()
            }
        };
        if fields.len() != arity || fields.iter().any(|field| field != &Type::I64) {
            errors.push(format!("{role} must have {arity} I64 fields"));
        }
    }
}

pub(super) fn validate_sized<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &SizedLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Some(trait_def) = required_trait(program, items.trait_id, "sized.trait", errors) else {
        return;
    };

    require_trait_generics(trait_def, "sized.trait", 0, errors);
    if !trait_def.associated_types.is_empty() {
        errors.push("sized.trait must not declare associated types".to_string());
    }
    if !trait_def.methods.is_empty() || !trait_def.signatures.is_empty() {
        errors.push("sized.trait must not declare methods or signatures".to_string());
    }
}

pub(super) fn validate_drop<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &DropLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    require_same_crate(
        "drop",
        [
            ("drop.trait", items.trait_id),
            ("drop.method", items.method_id),
        ],
        errors,
    );
    let Some(trait_def) = required_trait(program, items.trait_id, "drop.trait", errors) else {
        return;
    };
    require_trait_generics(trait_def, "drop.trait", 0, errors);
    let Some(method) = required_trait_member(
        program,
        trait_def,
        items.trait_id,
        items.method_id,
        "drop.method",
        errors,
    ) else {
        return;
    };

    require_receiver(&method, "drop.method", Some(ReceiverMode::Move), errors);
    require_receiver_type(&method, "drop.method", self_ty(items.trait_id, 0), errors);
    require_explicit_params(&method, "drop.method", 0, errors);
    require_return(&method, "drop.method", &Type::Unit, "()", errors);
}
