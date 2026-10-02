//! Canonical unique-owner evidence for consuming virtual dispatch.
use std::collections::HashMap;

use super::*;
use crate::selection::{ReceiverAdjustment, ReceiverCandidate, SelectedMethod, SelectionService};
use crate::types::{FunctionSafety, NominalTypeKind, ObjectType};

pub(crate) fn object(ty: &Type) -> Option<&ObjectType> {
    match ty {
        Type::Struct { args, .. } => match args.as_slice() {
            [Type::Object(object)] => Some(object),
            _ => None,
        },
        _ => None,
    }
}

pub(crate) fn select(
    owner: &HirExpr,
    name: &str,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
    opened: Option<&HirOpenBinding>,
) -> Result<Option<(SelectedMethod, HirOwnedObjectCall)>, String> {
    let Some(object) = object(&owner.ty) else {
        if let Some(binding) = opened {
            return select_opened(owner, name, traits, items, service, binding);
        }
        return Ok(None);
    };
    select_with_witness(owner, name, traits, items, service, object, None)
}

pub(crate) fn select_opened(
    owner: &HirExpr,
    name: &str,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
    binding: &HirOpenBinding,
) -> Result<Option<(SelectedMethod, HirOwnedObjectCall)>, String> {
    if !matches!(&owner.ty, Type::Struct { args, .. } if args.as_slice() == [Type::Witness(binding.witness)])
    {
        return Ok(None);
    }
    select_with_witness(
        owner,
        name,
        traits,
        items,
        service,
        &binding.object,
        Some(binding.witness),
    )
}

fn select_with_witness(
    owner: &HirExpr,
    name: &str,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
    object: &ObjectType,
    witness: Option<crate::types::WitnessId>,
) -> Result<Option<(SelectedMethod, HirOwnedObjectCall)>, String> {
    let payload = witness
        .map(Type::Witness)
        .unwrap_or_else(|| Type::Object(Box::new(object.clone())));
    let opened = if let Some(witness) = witness {
        object
            .try_map_types(|ty| {
                crate::type_services::substitution::instantiate_object_self(
                    ty,
                    &Type::Witness(witness),
                )
            })
            .map_err(|error| error.to_string())?
    } else {
        object.clone()
    };
    let roots = std::iter::once(opened.principal.clone())
        .chain(opened.guarantees.iter().cloned())
        .collect::<Vec<_>>();
    // An operation of the owner itself retains ordinary lookup precedence.
    // This probe grants no access; normal selection checks receiver mutability.
    let direct = ReceiverCandidate {
        expr: owner.clone(),
        adjustment: ReceiverAdjustment::None,
        can_autoref_mut: true,
    };
    if !service
        .select_concrete_method_candidates(&[direct], name, Clone::clone)
        .is_empty()
    {
        return Ok(None);
    }
    // This is only a signature probe. Never publish this synthetic payload as
    // a runtime receiver: the resulting HIR retains the original owner operand.
    let candidate = ReceiverCandidate {
        expr: HirExpr {
            ty: payload.clone(),
            ..owner.clone()
        },
        adjustment: ReceiverAdjustment::None,
        can_autoref_mut: false,
    };
    let mut selected = match service.select_bound_method(&[candidate], &roots, name, payload) {
        Ok(selected) => selected,
        Err(crate::selection::SelectionDiagnostic::NoImplementation { .. }) => return Ok(None),
        Err(error) => return Err(error.message()),
    };
    if selected
        .function
        .as_ref()
        .and_then(|function| function.self_receiver)
        != Some(ReceiverMode::Move)
    {
        return Ok(None);
    }
    if let Some(witness) = witness {
        super::object_methods::adapt_opened_owned(&mut selected, traits, object, witness)?;
    } else {
        super::object_methods::adapt_owned(&mut selected, traits, object)?;
    }
    selected.receiver = owner.clone();
    selected.receiver_adjustment = ReceiverAdjustment::None;
    let call = prove_with_witness(
        &owner.ty,
        selected.target.clone(),
        traits,
        items,
        service,
        object,
        witness,
    )?;
    Ok(Some((selected, call)))
}

pub(crate) fn prove(
    owner: &Type,
    method: HirMethodCallTarget,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
    opened: Option<&HirOpenBinding>,
) -> Result<HirOwnedObjectCall, String> {
    if let Some(binding) = opened.filter(|binding| binding.matches_owner(owner)) {
        return prove_with_witness(
            owner,
            method,
            traits,
            items,
            service,
            &binding.object,
            Some(binding.witness),
        );
    }
    let object = object(owner).ok_or(
        "consuming object dispatch requires an owning value, not a reference or raw pointer",
    )?;
    prove_with_witness(owner, method, traits, items, service, object, None)
}

fn prove_with_witness(
    owner: &Type,
    method: HirMethodCallTarget,
    traits: &HashMap<DefId, HirTrait>,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
    object: &ObjectType,
    witness: Option<crate::types::WitnessId>,
) -> Result<HirOwnedObjectCall, String> {
    let Type::Struct { id, .. } = owner else {
        return Err("consuming object dispatch requires a nominal owner value".into());
    };
    let transport = items
        .object_owner
        .as_ref()
        .ok_or("consuming object dispatch requires object_owner")?;
    let unique = items
        .object_owner_unique
        .as_ref()
        .ok_or("consuming object dispatch requires a unique owner capability")?;
    let HirSelectedMethodTarget::TraitMethod {
        trait_id,
        trait_args,
        dispatch,
        ..
    } = &method.target
    else {
        return Err("owned virtual call requires canonical Object method authority".into());
    };
    if *dispatch
        != witness
            .map(HirTraitDispatchKind::Opened)
            .unwrap_or(HirTraitDispatchKind::Object)
    {
        return Err("owned virtual call has inconsistent witness authority".into());
    }
    let opened = if let Some(witness) = witness {
        if !matches!(owner, Type::Struct { args, .. } if args.as_slice() == [Type::Witness(witness)])
        {
            return Err("opened consuming call has a different payload witness".into());
        }
        object
            .try_map_types(|ty| {
                crate::type_services::substitution::instantiate_object_self(
                    ty,
                    &Type::Witness(witness),
                )
            })
            .map_err(|error| error.to_string())?
    } else {
        object.clone()
    };
    let roots = std::iter::once(opened.principal.clone())
        .chain(opened.guarantees.iter().cloned())
        .collect::<Vec<_>>();
    let closure = crate::traits::evidence::implied_trait_bounds(
        traits,
        &witness
            .map(Type::Witness)
            .unwrap_or(Type::ObjectSelf { depth: 0 }),
        &roots,
    );
    if !closure.contains(&TraitBound {
        trait_id: *trait_id,
        type_args: trait_args.clone(),
    }) {
        return Err("consuming member is outside the owner's object guarantees".into());
    }
    if let Some(witness) = witness {
        super::object_methods::opened_consuming_signature(traits, &method, object, witness)?;
    } else {
        super::object_methods::consuming_signature(traits, &method, object)?;
    }
    let constructor = Type::Constructor {
        id: *id,
        flavor: NominalTypeKind::Struct,
    };
    let state_args = service
        .infer_trait_arguments(&constructor, transport.trait_id)
        .ok_or("owner transport State is not uniquely determined")?;
    let [state] = state_args.as_slice() else {
        return Err("invalid owner transport State arity".into());
    };
    if crate::type_services::visit::type_any(state, |ty| {
        matches!(
            ty,
            Type::Generic(_) | Type::TypeVar(_) | Type::ObjectSelf { .. } | Type::Witness(_)
        )
    }) || crate::type_services::layout::TypeLayout::sizedness(state, &|_| {
        crate::type_services::layout::Sizedness::Unknown
    }) != crate::type_services::layout::Sizedness::Sized
    {
        return Err("unique owner State must be sized and independent of the payload".into());
    }
    let signature = Type::Object(Box::new(object.clone()));
    let split_ty = Type::function_with_safety(
        vec![owner.clone()],
        Type::Tuple(vec![
            Type::Pointer(Box::new(
                witness
                    .map(Type::Witness)
                    .unwrap_or_else(|| signature.clone()),
            )),
            state.clone(),
        ]),
        FunctionSafety::Unsafe,
    );
    let release_ty = Type::function_with_safety(
        vec![Type::Pointer(Box::new(Type::U8)), state.clone()],
        Type::Unit,
        FunctionSafety::Unsafe,
    );
    let (_, into_parts) = crate::infer::owner_coercion::select_operation(
        service,
        &constructor,
        transport.trait_id,
        &state_args,
        transport.into_parts_id,
        &split_ty,
    )?;
    let (_, release) = crate::infer::owner_coercion::select_operation(service, &constructor, unique.trait_id, &state_args, unique.method_id, &release_ty)
        .map_err(|error| format!("consuming object call requires unique release for the same owner constructor and State: {error}"))?;
    Ok(HirOwnedObjectCall {
        method,
        object: signature,
        state: state.clone(),
        into_parts,
        release,
    })
}

fn same_bindings(left: &[HirTypeBinding], right: &[HirTypeBinding]) -> bool {
    let bindings: HashMap<_, _> = left
        .iter()
        .map(|binding| (binding.param, &binding.ty))
        .collect();
    bindings.len() == left.len()
        && left.len() == right.len()
        && right
            .iter()
            .all(|binding| bindings.get(&binding.param).copied() == Some(&binding.ty))
}

fn same_operation(left: &HirStaticMethodTarget, right: &HirStaticMethodTarget) -> bool {
    left.owner_ty == right.owner_ty
        && left.method.target == right.method.target
        && same_bindings(
            &left.method.owner_substitution,
            &right.method.owner_substitution,
        )
        && same_bindings(
            &left.method.method_substitution,
            &right.method.method_substitution,
        )
}

pub(crate) fn validate<P: HirPhase>(
    program: &HirProgram,
    owner: &HirExprFor<P>,
    args: &[HirExprFor<P>],
    result: &Type,
    call: &HirOwnedObjectCall,
) -> Result<(), String> {
    let mut assumptions = HirGenericBounds::new();
    crate::type_services::visit::visit_type(&owner.ty, &mut |ty: &Type| {
        if let Type::Generic(parameter) = ty {
            let declared = program
                .functions
                .get(&parameter.owner)
                .map(|function| &function.generic_bounds)
                .or_else(|| {
                    program
                        .impls
                        .get(&parameter.owner)
                        .map(|implementation| &implementation.bounds)
                });
            if let Some(bounds) = declared.and_then(|bounds| bounds.get(parameter)) {
                assumptions.insert(*parameter, bounds.clone());
            }
        }
    });
    let service = SelectionService::new(
        &program.traits,
        &program.impls,
        program
            .language_items
            .sized
            .as_ref()
            .map(|item| item.trait_id),
        None,
        &assumptions,
    )
    .with_effective_trait_methods(&program.indexes.effective_trait_methods);
    let Type::Object(object) = &call.object else {
        return Err("owned call requires an object signature".into());
    };
    let witness = match &call.method.target {
        HirSelectedMethodTarget::TraitMethod {
            dispatch: HirTraitDispatchKind::Opened(witness),
            ..
        } => Some(*witness),
        _ => None,
    };
    let expected = if witness.is_some() {
        prove_with_witness(
            &owner.ty,
            call.method.clone(),
            &program.traits,
            &program.language_items,
            &service,
            object,
            witness,
        )?
    } else {
        prove(
            &owner.ty,
            call.method.clone(),
            &program.traits,
            &program.language_items,
            &service,
            None,
        )?
    };
    if call.object != expected.object
        || call.state != expected.state
        || !same_operation(&call.into_parts, &expected.into_parts)
        || !same_operation(&call.release, &expected.release)
    {
        return Err("owned object call has invalid transport or unique-release evidence".into());
    }
    let Type::Object(object) = &call.object else {
        return Err("owned call requires an object signature".into());
    };
    let (params, ret) = if let Some(witness) = witness {
        super::object_methods::opened_consuming_signature(
            &program.traits,
            &call.method,
            object,
            witness,
        )?
    } else {
        super::object_methods::consuming_signature(&program.traits, &call.method, object)?
    };
    if &ret != result
        || params.len() != args.len()
        || params
            .iter()
            .zip(args)
            .any(|(expected, arg)| expected != &arg.ty && arg.ty != Type::Never)
    {
        return Err("owned virtual call disagrees with the canonical consuming signature".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> HirProgramFor<AcceptedHir> {
        let path = std::env::temp_dir().join("rock_owned_hir/main.rk");
        let source = r#"
lang sized
< trait Layout
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?Layout
    lang owner_from_parts
    unsafe build: *T -> State -> F T where T: ?Layout
lang object_owner_unique
< trait Unique State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
struct Own (T: ?Layout)
    < raw: *T
impl Transport I64 for Own
    unsafe split = owner ->
        result = (owner.raw, 0)
        ~Forget owner
        result
    unsafe build = pointer, _ -> Own
        raw: pointer
impl Unique I64 for Own
    unsafe release = _, _ !-> ()
trait Take
    ~@take: I64
consume: Own Take -> I64
consume = owner -> owner.take!
consume_opened: Own Take -> I64
consume_opened = owner ->
    open owner as Hidden, value
        value.take!
main = !-> ()
"#;
        crate::analyze(&crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            ..Default::default()
        })
        .unwrap()
        .hir
        .program
        .into_program()
    }

    #[test]
    fn opened_owner_consumption_keeps_witness_and_unique_protocol_authority() {
        let program = program();
        let consume = program
            .functions
            .values()
            .find(|function| function.name == "consume_opened")
            .unwrap();
        let HirStmtFor::Expr(expression) = &consume.body.stmts[0] else {
            panic!("expected opening");
        };
        let HirExprKindFor::Open { binding, body, .. } = &expression.kind else {
            panic!("expected opening");
        };
        let HirStmtFor::Expr(expression) = &body.stmts[0] else {
            panic!("expected consuming call");
        };
        let HirExprKindFor::OwnedObjectCall { owner, call, .. } = &expression.kind else {
            panic!("expected owning operand");
        };
        assert!(binding.matches_owner(&owner.ty));
        assert!(
            matches!(call.method.target, HirSelectedMethodTarget::TraitMethod { dispatch: HirTraitDispatchKind::Opened(witness), .. } if witness == binding.witness)
        );
        assert_eq!(call.object, Type::Object(Box::new(binding.object.clone())));
        assert!(call
            .into_parts
            .method
            .method_substitution
            .iter()
            .any(|binding_type| binding_type.ty == Type::Witness(binding.witness)));
        assert_eq!(call.state, Type::I64);
    }

    #[test]
    fn accepted_owned_call_rejects_forged_state_release_and_borrowed_owner() {
        let valid = program();
        for mutation in 0..4 {
            let mut forged = valid.clone();
            let function = forged
                .functions
                .values_mut()
                .find(|function| function.name == "consume")
                .unwrap();
            let Some(HirStmtFor::Expr(expression)) = function.body.stmts.last_mut() else {
                panic!("owned call tail")
            };
            let HirExprKindFor::OwnedObjectCall { owner, call, .. } = &mut expression.kind else {
                panic!("owned call")
            };
            match mutation {
                0 => call.state = Type::Bool,
                1 => call.release = call.into_parts.clone(),
                2 => {
                    owner.ty = Type::Reference {
                        mutable: true,
                        inner: Box::new(owner.ty.clone()),
                    }
                }
                _ => {
                    if let HirSelectedMethodTarget::TraitMethod { dispatch, .. } =
                        &mut call.method.target
                    {
                        *dispatch = HirTraitDispatchKind::TraitBound;
                    }
                }
            }
            let errors = AcceptedHirProgram::revalidate_for_test(forged).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("owner") || error.contains("owned")),
                "{errors:?}"
            );
        }
    }
}
