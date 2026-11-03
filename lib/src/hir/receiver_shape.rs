use super::*;

pub(crate) fn validate_effective_trait_method_receiver_shape(
    trait_def: &HirTrait,
    imp: &HirImpl,
    member_id: DefId,
    method_id: DefId,
) -> Result<(), String> {
    let mut member_receiver_shape = None;
    for shape in trait_def
        .methods
        .values()
        .filter(|method| method.id == member_id)
        .map(function_has_receiver)
        .chain(
            trait_def
                .signatures
                .values()
                .filter(|signature| signature.id == member_id)
                .map(|signature| signature.self_receiver.is_some()),
        )
    {
        if let Some(existing) = member_receiver_shape {
            if existing != shape {
                return Err(format!(
                    "effective trait member {member_id:?} has conflicting receiver/static representations"
                ));
            }
        } else {
            member_receiver_shape = Some(shape);
        }
    }
    let Some(member_has_receiver) = member_receiver_shape else {
        return Err(format!(
            "effective trait member {member_id:?} is not declared by trait {:?}",
            trait_def.id
        ));
    };
    let Some(method) = imp.methods.values().find(|method| method.id == method_id) else {
        return Err(format!(
            "effective trait member {member_id:?} maps to method {method_id:?} outside impl {:?}",
            imp.id
        ));
    };
    validate_effective_trait_member_receiver_shape(
        member_id,
        method_id,
        member_has_receiver,
        function_has_receiver(method),
    )
}

pub(crate) fn validate_effective_trait_member_receiver_shape(
    member_id: DefId,
    method_id: DefId,
    member_has_receiver: bool,
    method_has_receiver: bool,
) -> Result<(), String> {
    if member_has_receiver != method_has_receiver {
        let member_shape = if member_has_receiver {
            "receiver"
        } else {
            "static"
        };
        let method_shape = if method_has_receiver {
            "receiver"
        } else {
            "static"
        };
        return Err(format!(
            "effective trait member {member_id:?} maps to method {method_id:?} with receiver/static mismatch: trait member is {member_shape}, impl method is {method_shape}"
        ));
    }

    Ok(())
}

pub(crate) fn function_has_receiver(function: &HirFunction) -> bool {
    function.is_method || function.self_receiver.is_some()
}

pub(crate) fn validate_impl_receiver_pattern_bindings<P: HirPhase>(
    imp: &HirImplFor<P>,
    pattern: &HirImplReceiverPattern,
) -> Result<(), String> {
    let mut actual = HashSet::new();
    match pattern {
        HirImplReceiverPattern::Exact(ty) | HirImplReceiverPattern::Constructor(ty) => {
            ty.collect_generic_params(&mut actual)
        }
        HirImplReceiverPattern::SliceFamily { element } => {
            element.collect_generic_params(&mut actual)
        }
    }
    for trait_arg in &imp.trait_arg_types {
        trait_arg.collect_generic_params(&mut actual);
    }
    let expected = (0..imp.type_generics.len())
        .map(|index| GenericParamId {
            owner: imp.id,
            index: index as u32,
        })
        .collect::<HashSet<_>>();
    let invalid_expected = expected
        .iter()
        .any(|param| param.owner != imp.id || param.index as usize >= imp.type_generics.len());
    let invalid_actual = actual
        .iter()
        .any(|param| param.owner != imp.id || param.index as usize >= imp.type_generics.len());

    if !invalid_expected && !invalid_actual && expected == actual {
        Ok(())
    } else {
        Err(format!(
            "implementation {:?} typed receiver/trait argument generic bindings are incomplete or invalid for {:?}: required {expected:?}, found {actual:?}",
            imp.id, imp.type_generics
        ))
    }
}
