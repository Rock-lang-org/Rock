use super::*;

pub(super) fn validate_index<P: HirPhase>(
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

pub(super) fn validate_index_mut<P: HirPhase>(
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

pub(super) fn validate_index_mut_pair(
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
