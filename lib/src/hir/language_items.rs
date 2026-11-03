use crate::hir::{HirFunctionFor, HirFunctionSig, HirPhase, HirProgramFor, HirTraitFor};
use crate::ids::{AssocTypeId, DefId, VariantId};
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, RangeLanguageItems, SizedLanguageItems,
    TryLanguageItems,
};
use crate::types::ReceiverMode;
use crate::types::{GenericParamId, Type};

pub(crate) fn validate_language_items<P: HirPhase>(program: &HirProgramFor<P>) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(items) = &program.language_items.fold {
        require_same_crate(
            "fold",
            [
                ("fold.trait", items.trait_id),
                ("fold.method", items.method_id),
            ],
            &mut errors,
        );
        if let Some(trait_def) = required_trait(program, items.trait_id, "fold.trait", &mut errors)
        {
            require_trait_generics(trait_def, "fold.trait", 1, &mut errors);
            if let Some(method) = required_trait_member(
                program,
                trait_def,
                items.trait_id,
                items.method_id,
                "fold.method",
                &mut errors,
            ) {
                require_receiver(
                    &method,
                    "fold.method",
                    Some(ReceiverMode::Move),
                    &mut errors,
                );
                require_receiver_type(
                    &method,
                    "fold.method",
                    self_ty(items.trait_id, 1),
                    &mut errors,
                );
                require_explicit_params(&method, "fold.method", 2, &mut errors);
                let params = method.params();
                match method.ret() {
                    Type::Enum { args, .. } if args.len() == 2 && params.get(2).is_some_and(|state| *state == &args[1]) => {},
                    _ => errors.push("fold.method must return a two-parameter control-flow enum preserving its accumulator type".to_string()),
                }
            }
        }
    }

    if let Some(items) = &program.language_items.sized {
        validate_sized(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.drop {
        validate_drop(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.index {
        validate_index(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.index_mut {
        validate_index_mut(program, items, &mut errors);
        validate_index_mut_pair(program.language_items.index.as_ref(), items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_once {
        validate_fn_once(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_mut {
        validate_fn_mut(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_trait {
        validate_fn(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.send {
        validate_marker_trait(program, items.trait_id, "send.trait", &mut errors);
    }
    if let Some(items) = &program.language_items.sync {
        validate_marker_trait(program, items.trait_id, "sync.trait", &mut errors);
    }
    if let Some(items) = &program.language_items.try_protocol {
        validate_try(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.range {
        validate_range(program, items, &mut errors);
    }

    errors
}

fn validate_range<P: HirPhase>(
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

fn validate_sized<P: HirPhase>(
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

fn validate_drop<P: HirPhase>(
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

fn validate_index<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &IndexLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    require_same_crate(
        "index",
        [
            ("index.trait", items.trait_id),
            ("index.method", items.method_id),
        ],
        errors,
    );
    let Some(trait_def) = required_trait(program, items.trait_id, "index.trait", errors) else {
        return;
    };
    require_trait_generics(trait_def, "index.trait", 1, errors);
    let output_present = required_associated_type(
        program,
        trait_def,
        items.trait_id,
        items.output_id,
        "index.output",
        errors,
    );
    let Some(method) = required_trait_member(
        program,
        trait_def,
        items.trait_id,
        items.method_id,
        "index.method",
        errors,
    ) else {
        return;
    };

    require_receiver(&method, "index.method", Some(ReceiverMode::Shared), errors);
    require_receiver_type(
        &method,
        "index.method",
        Type::Reference {
            mutable: false,
            inner: Box::new(self_ty(items.trait_id, 1)),
        },
        errors,
    );
    require_explicit_params(&method, "index.method", 1, errors);
    require_explicit_param(
        &method,
        "index.method",
        0,
        &Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        }),
        errors,
    );
    if output_present {
        validate_index_return(&method, items, errors);
    }
}

fn validate_index_mut<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &IndexMutLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    require_same_crate(
        "index_mut",
        [
            ("index_mut.trait", items.trait_id),
            ("index_mut.method", items.method_id),
        ],
        errors,
    );
    let Some(trait_def) = required_trait(program, items.trait_id, "index_mut.trait", errors) else {
        return;
    };
    require_trait_generics(trait_def, "index_mut.trait", 1, errors);
    let output_present = required_associated_type(
        program,
        trait_def,
        items.trait_id,
        items.output_id,
        "index_mut.output",
        errors,
    );
    let Some(method) = required_trait_member(
        program,
        trait_def,
        items.trait_id,
        items.method_id,
        "index_mut.method",
        errors,
    ) else {
        return;
    };

    require_receiver(&method, "index_mut.method", Some(ReceiverMode::Mut), errors);
    require_receiver_type(
        &method,
        "index_mut.method",
        Type::Reference {
            mutable: true,
            inner: Box::new(self_ty(items.trait_id, 1)),
        },
        errors,
    );
    require_explicit_params(&method, "index_mut.method", 1, errors);
    require_explicit_param(
        &method,
        "index_mut.method",
        0,
        &Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        }),
        errors,
    );
    if output_present {
        validate_index_mut_return(&method, items, errors);
    }
}

fn validate_fn_once<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &FnOnceLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    validate_fn_shape(
        program,
        items.trait_id,
        items.output_id,
        items.method_id,
        "fn_once",
        ReceiverMode::Move,
        errors,
    );
}

fn validate_fn_mut<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &FnMutLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    validate_fn_shape(
        program,
        items.trait_id,
        items.output_id,
        items.method_id,
        "fn_mut",
        ReceiverMode::Mut,
        errors,
    );
}

fn validate_fn<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &FnLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    validate_fn_shape(
        program,
        items.trait_id,
        items.output_id,
        items.method_id,
        "fn",
        ReceiverMode::Shared,
        errors,
    );
}

fn validate_fn_shape<P: HirPhase>(
    program: &HirProgramFor<P>,
    trait_id: DefId,
    output_id: AssocTypeId,
    method_id: DefId,
    protocol: &str,
    receiver: ReceiverMode,
    errors: &mut Vec<String>,
) {
    let trait_role = format!("{protocol}.trait");
    let method_role = format!("{protocol}.method");
    require_same_crate(
        protocol,
        [(&trait_role, trait_id), (&method_role, method_id)],
        errors,
    );
    let Some(trait_def) = required_trait(program, trait_id, &trait_role, errors) else {
        return;
    };
    require_trait_generics(trait_def, &trait_role, 2, errors);
    let output_present = required_associated_type(
        program,
        trait_def,
        trait_id,
        output_id,
        &format!("{protocol}.output"),
        errors,
    );
    let Some(method) = required_trait_member(
        program,
        trait_def,
        trait_id,
        method_id,
        &method_role,
        errors,
    ) else {
        return;
    };

    require_receiver(&method, &method_role, Some(receiver), errors);
    let expected_receiver = match receiver {
        ReceiverMode::Move => self_ty(trait_id, 2),
        ReceiverMode::Shared => Type::Reference {
            mutable: false,
            inner: Box::new(self_ty(trait_id, 2)),
        },
        ReceiverMode::Mut => Type::Reference {
            mutable: true,
            inner: Box::new(self_ty(trait_id, 2)),
        },
    };
    require_receiver_type(&method, &method_role, expected_receiver, errors);
    require_explicit_params(&method, &method_role, 1, errors);
    require_explicit_param(
        &method,
        &method_role,
        0,
        &Type::Generic(GenericParamId {
            owner: trait_id,
            index: 0,
        }),
        errors,
    );
    if output_present {
        validate_fn_return(&method, trait_id, output_id, protocol, errors);
    }
}

fn validate_marker_trait<P: HirPhase>(
    program: &HirProgramFor<P>,
    trait_id: DefId,
    role: &str,
    errors: &mut Vec<String>,
) {
    let Some(trait_def) = required_trait(program, trait_id, role, errors) else {
        return;
    };
    require_trait_generics(trait_def, role, 0, errors);
    if !trait_def.associated_types.is_empty() {
        errors.push(format!("{role} must not declare associated types"));
    }
    if !trait_def.methods.is_empty() || !trait_def.signatures.is_empty() {
        errors.push(format!("{role} must not declare methods or signatures"));
    }
}

fn validate_index_mut_pair(
    index: Option<&IndexLanguageItems<DefId>>,
    index_mut: &IndexMutLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Some(index) = index else {
        errors.push("index_mut requires index".to_string());
        return;
    };
    if index.trait_id.crate_id != index_mut.trait_id.crate_id {
        errors.push("index_mut.trait must use the same crate as index.trait".to_string());
    }
}

fn validate_try<P: HirPhase>(
    program: &HirProgramFor<P>,
    items: &TryLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    require_same_crate(
        "try",
        [
            ("try.trait", items.try_trait_id),
            ("try.branch", items.branch_method_id),
            ("from_residual.trait", items.from_residual_trait_id),
            ("from_residual.method", items.from_residual_method_id),
            ("control_flow.enum", items.control_flow_enum_id),
        ],
        errors,
    );
    let Some(try_trait) = required_trait(program, items.try_trait_id, "try.trait", errors) else {
        return;
    };
    require_trait_generics(try_trait, "try.trait", 0, errors);
    let output_present = required_associated_type(
        program,
        try_trait,
        items.try_trait_id,
        items.output_id,
        "try.output",
        errors,
    );
    let residual_present = required_associated_type(
        program,
        try_trait,
        items.try_trait_id,
        items.residual_id,
        "try.residual",
        errors,
    );
    if let Some(branch) = required_trait_member(
        program,
        try_trait,
        items.try_trait_id,
        items.branch_method_id,
        "try.branch",
        errors,
    ) {
        require_receiver(&branch, "try.branch", Some(ReceiverMode::Move), errors);
        require_receiver_type(
            &branch,
            "try.branch",
            self_ty(items.try_trait_id, 0),
            errors,
        );
        require_explicit_params(&branch, "try.branch", 0, errors);
        if output_present && residual_present {
            validate_try_branch_return(&branch, items, errors);
        }
    }

    if let Some(from_residual_trait) = required_trait(
        program,
        items.from_residual_trait_id,
        "from_residual.trait",
        errors,
    ) {
        require_trait_generics(from_residual_trait, "from_residual.trait", 1, errors);
        if let Some(method) = required_trait_member(
            program,
            from_residual_trait,
            items.from_residual_trait_id,
            items.from_residual_method_id,
            "from_residual.method",
            errors,
        ) {
            require_receiver(&method, "from_residual.method", None, errors);
            require_explicit_params(&method, "from_residual.method", 1, errors);
            require_explicit_param(
                &method,
                "from_residual.method",
                0,
                &Type::Generic(GenericParamId {
                    owner: items.from_residual_trait_id,
                    index: 0,
                }),
                errors,
            );
            require_return(
                &method,
                "from_residual.method",
                &self_ty(items.from_residual_trait_id, 1),
                "Self",
                errors,
            );
        }
    }

    let Some(control_flow) = required_enum(
        program,
        items.control_flow_enum_id,
        "control_flow.enum",
        errors,
    ) else {
        return;
    };
    if control_flow.generic_params.len() != 2 {
        errors.push(format!(
            "control_flow.enum must declare exactly 2 generic parameters; found {}",
            control_flow.generic_params.len()
        ));
    }
    if control_flow.variants.len() != 2 {
        errors.push(format!(
            "control_flow.enum must declare exactly 2 variants; found {}",
            control_flow.variants.len()
        ));
    }
    if items.break_variant_id == items.continue_variant_id {
        errors.push(
            "control_flow.break and control_flow.continue must identify distinct variants"
                .to_string(),
        );
    }
    require_variant_payload(
        program,
        control_flow,
        items.control_flow_enum_id,
        items.break_variant_id,
        "control_flow.break",
        0,
        errors,
    );
    require_variant_payload(
        program,
        control_flow,
        items.control_flow_enum_id,
        items.continue_variant_id,
        "control_flow.continue",
        1,
        errors,
    );
}

fn required_trait<'a, P: HirPhase>(
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

fn required_enum<'a, P: HirPhase>(
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

fn required_associated_type<P: HirPhase>(
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

fn required_trait_member<'a, P: HirPhase>(
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

fn require_variant_payload<P: HirPhase>(
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

fn require_same_crate<const N: usize>(
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

fn require_trait_generics<P: HirPhase>(
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

enum TraitMember<'a, P: HirPhase> {
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

    fn params(&self) -> Vec<&Type> {
        match self {
            Self::Default(method) => method.params.iter().map(|param| &param.ty).collect(),
            Self::Signature(signature) => signature.params.iter().collect(),
        }
    }

    fn ret(&self) -> &Type {
        match self {
            Self::Default(method) => &method.ret_type,
            Self::Signature(signature) => &signature.ret,
        }
    }
}

fn require_receiver<P: HirPhase>(
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

fn require_receiver_type<P: HirPhase>(
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

fn require_explicit_params<P: HirPhase>(
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

fn require_explicit_param<P: HirPhase>(
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

fn require_return<P: HirPhase>(
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

fn validate_index_return<P: HirPhase>(
    method: &TraitMember<'_, P>,
    items: &IndexLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Type::Reference {
        mutable: false,
        inner,
    } = method.ret()
    else {
        errors.push(
            "index.method must return a shared reference to index.output on Self".to_string(),
        );
        return;
    };
    let Type::Projection {
        ty,
        trait_id,
        assoc_type,
        trait_args,
    } = inner.as_ref()
    else {
        errors.push(
            "index.method must return a shared reference to index.output on Self".to_string(),
        );
        return;
    };
    if *trait_id != items.trait_id {
        errors.push("index.method return projection must use index.trait".to_string());
    }
    if assoc_type.owner != items.trait_id || assoc_type.assoc_type_id != items.output_id {
        errors.push("index.method return projection must use index.output".to_string());
    }
    if ty.as_ref() != &self_ty(items.trait_id, 1) {
        errors.push("index.method return projection must project from index Self".to_string());
    }
    if trait_args
        != &[Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        })]
    {
        errors.push("index.method return projection must use index generic 0".to_string());
    }
}

fn validate_index_mut_return<P: HirPhase>(
    method: &TraitMember<'_, P>,
    items: &IndexMutLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Type::Reference {
        mutable: true,
        inner,
    } = method.ret()
    else {
        errors.push(
            "index_mut.method must return a mutable reference to index_mut.output on Self"
                .to_string(),
        );
        return;
    };
    let Type::Projection {
        ty,
        trait_id,
        assoc_type,
        trait_args,
    } = inner.as_ref()
    else {
        errors.push(
            "index_mut.method must return a mutable reference to index_mut.output on Self"
                .to_string(),
        );
        return;
    };
    if *trait_id != items.trait_id {
        errors.push("index_mut.method return projection must use index_mut.trait".to_string());
    }
    if assoc_type.owner != items.trait_id || assoc_type.assoc_type_id != items.output_id {
        errors.push("index_mut.method return projection must use index_mut.output".to_string());
    }
    if ty.as_ref() != &self_ty(items.trait_id, 1) {
        errors.push(
            "index_mut.method return projection must project from index_mut Self".to_string(),
        );
    }
    if trait_args
        != &[Type::Generic(GenericParamId {
            owner: items.trait_id,
            index: 0,
        })]
    {
        errors.push("index_mut.method return projection must use index_mut generic 0".to_string());
    }
}

fn validate_fn_return<P: HirPhase>(
    method: &TraitMember<'_, P>,
    trait_id: DefId,
    output_id: AssocTypeId,
    protocol: &str,
    errors: &mut Vec<String>,
) {
    let expected = Type::Generic(GenericParamId {
        owner: trait_id,
        index: 1,
    });
    if method.ret() != &expected {
        errors.push(format!(
            "{protocol}.method must return {protocol} generic 1"
        ));
    }

    let _ = output_id;
}

fn validate_try_branch_return<P: HirPhase>(
    method: &TraitMember<'_, P>,
    items: &TryLanguageItems<DefId>,
    errors: &mut Vec<String>,
) {
    let Type::Enum { id, args } = method.ret() else {
        errors.push("try.branch must return control_flow<Residual, Output>".to_string());
        return;
    };
    if *id != items.control_flow_enum_id {
        errors.push("try.branch must return the marked control_flow enum".to_string());
    }
    if args.len() != 2 {
        errors.push("try.branch must return control_flow<Residual, Output>".to_string());
        return;
    }
    validate_try_projection(
        &args[0],
        items,
        items.residual_id,
        "try.branch residual projection",
        errors,
    );
    validate_try_projection(
        &args[1],
        items,
        items.output_id,
        "try.branch output projection",
        errors,
    );
}

fn validate_try_projection(
    ty: &Type,
    items: &TryLanguageItems<DefId>,
    assoc_id: AssocTypeId,
    role: &str,
    errors: &mut Vec<String>,
) {
    let Type::Projection {
        ty: base,
        trait_id,
        assoc_type,
        trait_args,
    } = ty
    else {
        errors.push(format!("{role} must be a projection"));
        return;
    };
    if *trait_id != items.try_trait_id {
        errors.push(format!("{role} must use try.trait"));
    }
    if assoc_type.owner != items.try_trait_id || assoc_type.assoc_type_id != assoc_id {
        errors.push(format!("{role} must use its marked associated type"));
    }
    if base.as_ref() != &self_ty(items.try_trait_id, 0) {
        errors.push(format!("{role} must project from try Self"));
    }
    if !trait_args.is_empty() {
        errors.push(format!("{role} must not have trait arguments"));
    }
}

fn self_ty(trait_id: DefId, generic_count: u32) -> Type {
    Type::Generic(GenericParamId {
        owner: trait_id,
        index: generic_count,
    })
}

#[cfg(test)]
mod tests;
