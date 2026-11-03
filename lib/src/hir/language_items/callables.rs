use super::*;

pub(super) fn validate_fn_once<P: HirPhase>(
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

pub(super) fn validate_fn_mut<P: HirPhase>(
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

pub(super) fn validate_fn<P: HirPhase>(
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

pub(super) fn validate_marker_trait<P: HirPhase>(
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
