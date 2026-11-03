use super::*;

pub(super) fn collect_function_signature<P: HirPhase>(
    function: &HirFunctionFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    record_type(
        context,
        ids,
        HirTypeLocation::FunctionReturn {
            function: function.id,
        },
        &function.ret_type,
    );
    for (index, param) in function.params.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::FunctionParam {
                function: function.id,
                index,
            },
            &param.ty,
        );
    }
    collect_generic_bound_args(
        &function.generic_bounds,
        context,
        ids,
        |param, bound_index, arg_index| HirTypeLocation::FunctionBoundArg {
            function: function.id,
            param,
            bound_index,
            arg_index,
        },
    );
    collect_predicate_types(
        &function.generic_bounds.predicates,
        function.id,
        context,
        ids,
    );
}

pub(super) fn collect_extern_signature(
    ext: &HirExtern,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    record_type(
        context,
        ids,
        HirTypeLocation::ExternReturn { extern_id: ext.id },
        &ext.ret,
    );
    for (index, param) in ext.params.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::ExternParam {
                extern_id: ext.id,
                index,
            },
            param,
        );
    }
}

pub(super) fn collect_struct_fields(
    strukt: &HirStruct,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    for field in &strukt.fields {
        record_type(
            context,
            ids,
            HirTypeLocation::StructField {
                owner: strukt.id,
                field: field.id,
            },
            &field.ty,
        );
    }
}

pub(super) fn collect_enum_fields(enm: &HirEnum, context: &mut TypeContext, ids: &mut HirTypeIds) {
    for variant in &enm.variants {
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                for field in fields {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::EnumVariantNamedField {
                            owner: enm.id,
                            variant: variant.id,
                            field: field.id,
                        },
                        &field.ty,
                    );
                }
            }
            HirVariantFields::Positional(types) => {
                for (index, ty) in types.iter().enumerate() {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::EnumVariantPositionalField {
                            owner: enm.id,
                            variant: variant.id,
                            index,
                        },
                        ty,
                    );
                }
            }
            HirVariantFields::Unit => {}
        }
    }
}

pub(super) fn collect_trait_signatures<P: HirPhase>(
    trait_def: &HirTraitFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    collect_predicate_types(&trait_def.predicates, trait_def.id, context, ids);
    let mut signatures: Vec<_> = trait_def.signatures.values().collect();
    signatures.sort_by_key(|signature| signature.id);
    for signature in signatures {
        record_type(
            context,
            ids,
            HirTypeLocation::TraitSignatureReturn {
                trait_id: trait_def.id,
                signature: signature.id,
            },
            &signature.ret,
        );
        for (index, param) in signature.params.iter().enumerate() {
            record_type(
                context,
                ids,
                HirTypeLocation::TraitSignatureParam {
                    trait_id: trait_def.id,
                    signature: signature.id,
                    index,
                },
                param,
            );
        }
        collect_generic_bound_args(
            &signature.generic_bounds,
            context,
            ids,
            |param, bound_index, arg_index| HirTypeLocation::TraitSignatureBoundArg {
                trait_id: trait_def.id,
                signature: signature.id,
                param,
                bound_index,
                arg_index,
            },
        );
        collect_predicate_types(
            &signature.generic_bounds.predicates,
            signature.id,
            context,
            ids,
        );
    }
}

pub(super) fn collect_impl_types<P: HirPhase>(
    imp: &HirImplFor<P>,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    let mut receiver_index = 0;
    P::visit_impl_receiver_types(&imp.receiver_pattern, &mut |ty| {
        record_type(
            context,
            ids,
            HirTypeLocation::ImplReceiverArg {
                impl_id: imp.id,
                index: receiver_index,
            },
            ty,
        );
        receiver_index += 1;
    });

    for (index, ty) in imp.trait_arg_types.iter().enumerate() {
        record_type(
            context,
            ids,
            HirTypeLocation::ImplTraitArg {
                impl_id: imp.id,
                index,
            },
            ty,
        );
    }

    for assoc in &imp.associated_types {
        record_type(
            context,
            ids,
            HirTypeLocation::AssociatedTypeDef {
                impl_id: imp.id,
                assoc_type: assoc.id,
            },
            &assoc.ty,
        );
    }

    let mut bounds: Vec<_> = imp.bounds.iter().collect();
    bounds.sort_by_key(|(param, _)| (param.owner, param.index));
    for (param, trait_bounds) in bounds {
        for (bound_index, bound) in trait_bounds.iter().enumerate() {
            collect_bound_type_args(bound, context, ids, |arg_index| {
                HirTypeLocation::ImplBoundArg {
                    impl_id: imp.id,
                    param: *param,
                    bound_index,
                    arg_index,
                }
            });
        }
    }
    collect_predicate_types(&imp.bounds.predicates, imp.id, context, ids);
}

fn collect_predicate_types(
    predicates: &[crate::types::Predicate],
    owner: DefId,
    context: &mut TypeContext,
    ids: &mut HirTypeIds,
) {
    for (predicate_index, predicate) in predicates.iter().enumerate() {
        match predicate {
            crate::types::Predicate::Trait { subject, args, .. } => {
                record_type(
                    context,
                    ids,
                    HirTypeLocation::PredicateSubject {
                        owner,
                        index: predicate_index,
                    },
                    subject,
                );
                for (arg_index, arg) in args.iter().enumerate() {
                    record_type(
                        context,
                        ids,
                        HirTypeLocation::PredicateArg {
                            owner,
                            predicate_index,
                            arg_index,
                        },
                        arg,
                    );
                }
            }
        }
    }
}
