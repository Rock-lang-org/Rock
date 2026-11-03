use super::*;

pub(super) fn required_trait<'a, P: HirPhase>(
    program: &'a HirProgramFor<P>,
    id: DefId,
    role: &str,
    errors: &mut Vec<String>,
) -> Option<&'a HirTraitFor<P>> {
    if let Some(trait_def) = program.traits.get(&id) {
        return Some(trait_def);
    }
    errors.push(format!(
        "{role} must identify a trait; {}",
        declaration_kind(program, id)
    ));
    None
}

pub(super) fn required_enum<'a, P: HirPhase>(
    program: &'a HirProgramFor<P>,
    id: DefId,
    role: &str,
    errors: &mut Vec<String>,
) -> Option<&'a crate::hir::HirEnum> {
    if let Some(enum_def) = program.enums.get(&id) {
        return Some(enum_def);
    }
    errors.push(format!(
        "{role} must identify an enum; {}",
        declaration_kind(program, id)
    ));
    None
}

fn declaration_kind<P: HirPhase>(program: &HirProgramFor<P>, id: DefId) -> String {
    let kind = if program.functions.contains_key(&id) {
        "a function"
    } else if program.structs.contains_key(&id) {
        "a struct"
    } else if program.enums.contains_key(&id) {
        "an enum"
    } else if program.traits.contains_key(&id) {
        "a trait"
    } else if program.impls.contains_key(&id) {
        "an impl"
    } else if program.externs.contains_key(&id) {
        "an extern"
    } else {
        "no declaration"
    };
    format!("{id:?} is {kind}")
}

pub(super) fn required_associated_type<P: HirPhase>(
    program: &HirProgramFor<P>,
    trait_def: &HirTraitFor<P>,
    trait_id: DefId,
    assoc_id: AssocTypeId,
    role: &str,
    errors: &mut Vec<String>,
) -> bool {
    if trait_def
        .associated_types
        .iter()
        .any(|assoc| assoc.id == assoc_id)
    {
        return true;
    }
    let owner = program
        .traits
        .iter()
        .filter_map(|(id, candidate)| {
            candidate
                .associated_types
                .iter()
                .any(|assoc| assoc.id == assoc_id)
                .then_some(*id)
        })
        .min();
    match owner {
        Some(owner) => errors.push(format!(
            "{role} {assoc_id:?} is declared by trait {owner:?}, not {trait_id:?}"
        )),
        None => errors.push(format!(
            "{role} {assoc_id:?} is not declared by trait {trait_id:?}"
        )),
    }
    false
}

pub(super) fn required_trait_member<'a, P: HirPhase>(
    program: &'a HirProgramFor<P>,
    trait_def: &'a HirTraitFor<P>,
    trait_id: DefId,
    method_id: DefId,
    role: &str,
    errors: &mut Vec<String>,
) -> Option<TraitMember<'a, P>> {
    if let Some(method) = trait_def
        .methods
        .values()
        .find(|method| method.id == method_id)
    {
        return Some(TraitMember::Default(method));
    }
    if let Some(signature) = trait_def
        .signatures
        .values()
        .find(|signature| signature.id == method_id)
    {
        return Some(TraitMember::Signature(signature));
    }
    let owner = program
        .traits
        .iter()
        .filter_map(|(owner, candidate)| {
            (candidate
                .methods
                .values()
                .any(|method| method.id == method_id)
                || candidate
                    .signatures
                    .values()
                    .any(|signature| signature.id == method_id))
            .then_some(*owner)
        })
        .min();
    match owner {
        Some(owner) => errors.push(format!(
            "{role} {method_id:?} is owned by trait {owner:?}, not {trait_id:?}"
        )),
        None => errors.push(format!("{role} {method_id:?} is not declared by any trait")),
    }
    None
}

pub(super) fn require_variant_payload<P: HirPhase>(
    program: &HirProgramFor<P>,
    enum_def: &crate::hir::HirEnum,
    enum_id: DefId,
    variant_id: VariantId,
    role: &str,
    generic_index: u32,
    errors: &mut Vec<String>,
) {
    let Some(variant) = enum_def
        .variants
        .iter()
        .find(|variant| variant.id == variant_id)
    else {
        let owner = program
            .enums
            .iter()
            .filter_map(|(id, candidate)| {
                candidate
                    .variants
                    .iter()
                    .any(|variant| variant.id == variant_id)
                    .then_some(*id)
            })
            .min();
        match owner {
            Some(owner) => errors.push(format!(
                "{role} {variant_id:?} is declared by enum {owner:?}, not {enum_id:?}"
            )),
            None => errors.push(format!(
                "{role} {variant_id:?} is not declared by enum {enum_id:?}"
            )),
        }
        return;
    };
    let fields = match &variant.fields {
        crate::hir::HirVariantFields::Positional(fields) => fields,
        crate::hir::HirVariantFields::Named(_) => {
            errors.push(format!("{role} must use a positional payload"));
            return;
        }
        crate::hir::HirVariantFields::Unit => {
            errors.push(format!("{role} must not be unit"));
            return;
        }
    };
    if fields.len() != 1 {
        errors.push(format!(
            "{role} must carry exactly one positional payload; found {}",
            fields.len()
        ));
        return;
    }
    let expected = Type::Generic(GenericParamId {
        owner: enum_id,
        index: generic_index,
    });
    if fields[0] != expected {
        errors.push(format!("{role} must carry generic {generic_index}"));
    }
}

pub(super) fn require_same_crate<const N: usize>(
    bundle: &str,
    fields: [(&str, DefId); N],
    errors: &mut Vec<String>,
) {
    let crate_id = fields[0].1.crate_id;
    for (role, id) in fields.into_iter().skip(1) {
        if id.crate_id != crate_id {
            errors.push(format!(
                "{bundle} bundle field {role} must use crate {crate_id:?}; found {:?}",
                id.crate_id
            ));
        }
    }
}

pub(super) fn require_trait_generics<P: HirPhase>(
    trait_def: &HirTraitFor<P>,
    role: &str,
    expected: usize,
    errors: &mut Vec<String>,
) {
    if trait_def.generic_params.len() != expected {
        errors.push(format!(
            "{role} must declare exactly {expected} generic parameters; found {}",
            trait_def.generic_params.len()
        ));
    }
}

pub(super) enum TraitMember<'a, P: HirPhase> {
    Default(&'a HirFunctionFor<P>),
    Signature(&'a HirFunctionSig),
}

impl<P: HirPhase> TraitMember<'_, P> {
    fn self_receiver(&self) -> Option<ReceiverMode> {
        match self {
            Self::Default(method) => method.self_receiver,
            Self::Signature(signature) => signature.self_receiver,
        }
    }

    pub(super) fn params(&self) -> Vec<&Type> {
        match self {
            Self::Default(method) => method.params.iter().map(|param| &param.ty).collect(),
            Self::Signature(signature) => signature.params.iter().collect(),
        }
    }

    pub(super) fn ret(&self) -> &Type {
        match self {
            Self::Default(method) => &method.ret_type,
            Self::Signature(signature) => &signature.ret,
        }
    }
}

pub(super) fn require_receiver<P: HirPhase>(
    method: &TraitMember<'_, P>,
    role: &str,
    expected: Option<ReceiverMode>,
    errors: &mut Vec<String>,
) {
    if method.self_receiver() != expected {
        let expectation = match expected {
            Some(ReceiverMode::Move) => "a move receiver",
            Some(ReceiverMode::Shared) => "a shared receiver",
            Some(ReceiverMode::Mut) => "a mutable receiver",
            None => "a static method",
        };
        errors.push(format!("{role} must use {expectation}"));
    }
}

pub(super) fn require_receiver_type<P: HirPhase>(
    method: &TraitMember<'_, P>,
    role: &str,
    expected: Type,
    errors: &mut Vec<String>,
) {
    let Some(receiver) = method.params().first().copied() else {
        errors.push(format!("{role} must declare a receiver parameter"));
        return;
    };
    if receiver != &expected {
        errors.push(format!("{role} must use the trait Self receiver type"));
    }
}

pub(super) fn require_explicit_params<P: HirPhase>(
    method: &TraitMember<'_, P>,
    role: &str,
    expected: usize,
    errors: &mut Vec<String>,
) {
    let receiver_count = usize::from(method.self_receiver().is_some());
    let actual = method.params().len().saturating_sub(receiver_count);
    if actual != expected {
        errors.push(format!(
            "{role} must declare exactly {expected} explicit parameters; found {actual}"
        ));
    }
}

pub(super) fn require_explicit_param<P: HirPhase>(
    method: &TraitMember<'_, P>,
    role: &str,
    index: usize,
    expected: &Type,
    errors: &mut Vec<String>,
) {
    let receiver_count = usize::from(method.self_receiver().is_some());
    let actual = method.params().get(receiver_count + index).copied();
    if actual != Some(expected) {
        errors.push(format!(
            "{role} explicit parameter {index} has the wrong type"
        ));
    }
}

pub(super) fn require_return<P: HirPhase>(
    method: &TraitMember<'_, P>,
    role: &str,
    expected: &Type,
    expectation: &str,
    errors: &mut Vec<String>,
) {
    if method.ret() != expected {
        errors.push(format!("{role} must return {expectation}"));
    }
}

pub(super) fn self_ty(trait_id: DefId, generic_count: u32) -> Type {
    Type::Generic(GenericParamId {
        owner: trait_id,
        index: generic_count,
    })
}
