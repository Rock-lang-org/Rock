use super::*;

pub(super) fn validate_try<P: HirPhase>(
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
