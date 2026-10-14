//! Canonical owner transport and unique-owner evidence for consuming dispatch.
use std::collections::{HashMap, HashSet};

use super::*;
use crate::selection::{ReceiverAdjustment, ReceiverCandidate, SelectedMethod, SelectionService};
use crate::types::{FunctionSafety, NominalTypeKind, ObjectType};

pub(crate) fn object(ty: &Type) -> Option<&ObjectType> {
    let Type::Struct { args, .. } = ty else { return None; };
    // Discovery only: the canonical transport application must bind this
    // exact object before the signature probe can become an owned call.
    let mut objects = args.iter().filter_map(|arg| match arg {
        Type::Object(object) => Some(object.as_ref()),
        _ => None,
    });
    let object = objects.next()?;
    objects.next().is_none().then_some(object)
}

/// Prove the exact payload slot and independent State without granting unique access.
pub(super) fn owner_application(
    owner: &Type,
    payload: &Type,
    items: &HirLanguageItems,
    service: &SelectionService<'_>,
) -> Result<(Type, Vec<Type>), String> {
    let transport = items.object_owner.as_ref().ok_or("consuming object dispatch requires object_owner")?;
    let (constructor, actual_payload, state_args) = service
        .infer_unary_constructor_application(owner, transport.trait_id)
        .map_err(|error| error.message())?
        .ok_or("owner transport State is not uniquely determined")?;
    if &actual_payload != payload {
        return Err("owner transport application has a different object payload".into());
    }
    // Phase inputs are canonical. Beta-apply the recovered section using the
    // normalizer's checked binder substitution, without solving any equation.
    let applied = match &constructor {
        Type::Lambda { .. } => crate::type_services::normalize::apply_type_lambda(&constructor, std::slice::from_ref(payload)),
        Type::Constructor { id, flavor: NominalTypeKind::Struct } => Some(Type::Struct { id: *id, args: vec![payload.clone()] }),
        _ => None,
    };
    if applied.as_ref() != Some(owner) {
        return Err("owner transport section does not reconstruct the exact canonical owner".into());
    }
    let [state] = state_args.as_slice() else { return Err("invalid owner transport State arity".into()); };
    struct IndependentState<'a> {
        payload: &'a Type,
        constructor: &'a Type,
        depth: u32,
        valid: bool,
    }
    impl crate::type_services::visit::TypeVisitor for IndependentState<'_> {
        fn enter_binders(&mut self, _: &[crate::type_services::kind::Kind]) { self.depth += 1; }
        fn exit_binders(&mut self) { self.depth -= 1; }
        fn visit_type(&mut self, ty: &Type) {
            self.valid &= ty != self.payload && match ty {
                Type::Generic(parameter) => crate::type_services::visit::type_any(self.constructor, |fixed| fixed == &Type::Generic(*parameter)),
                Type::BoundVar { depth, .. } => *depth < self.depth,
                Type::TypeVar(_) | Type::Error | Type::ObjectSelf { .. } | Type::Witness(_) => false,
                _ => true,
            };
            crate::type_services::visit::visit_type_children(ty, self);
        }
    }
    let mut independent = IndependentState { payload, constructor: &constructor, depth: 0, valid: true };
    crate::type_services::visit::visit_type(state, &mut independent);
    let sized = items.sized.as_ref().ok_or("unique owner State requires a canonical Sized capability")?;
    if !independent.valid || !service.trait_bound_satisfied(state, &TraitBound { trait_id: sized.trait_id, type_args: Vec::new() }) {
        return Err("unique owner State must be sized and independent of the payload".into());
    }
    Ok((constructor, state_args))
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
    if !binding.matches_owner(&owner.ty) {
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
    let Type::Struct { .. } = owner else {
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
    let (constructor, state_args) = owner_application(
        owner,
        &witness.map(Type::Witness).unwrap_or_else(|| Type::Object(Box::new(object.clone()))),
        items,
        service,
    )?;
    let [state] = state_args.as_slice() else {
        return Err("invalid owner transport State arity".into());
    };
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
    let mut context_owners = HashSet::new();
    crate::type_services::visit::visit_type(&owner.ty, &mut |ty: &Type| {
        if let Type::Generic(parameter) = ty {
            if !context_owners.insert(parameter.owner) {
                return;
            }
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
            if let Some(bounds) = declared {
                // Reuse the actual declaration context, including relaxation;
                // no default Sized evidence is reconstructed from the owner.
                assumptions.extend(
                    bounds.iter().map(|(parameter, bounds)| (*parameter, bounds.clone())),
                );
                assumptions
                    .relaxed_sized
                    .extend(bounds.relaxed_sized.iter().copied());
                for predicate in &bounds.predicates {
                    if !assumptions.predicates.contains(predicate) {
                        assumptions.predicates.push(predicate.clone());
                    }
                }
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
        program_from_source(source)
    }

    fn source_config(source: &str) -> crate::Config {
        let path = std::env::temp_dir().join("rock_owned_hir/main.rk");
        crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: source.into(),
            }],
            ..Default::default()
        }
    }

    fn program_from_source(source: &str) -> HirProgramFor<AcceptedHir> {
        crate::analyze(&source_config(source))
            .unwrap()
            .hir
            .program
            .into_program()
    }

    fn closed_owner_source(payload_first: bool, unique_state: Option<&str>) -> String {
        let args = |allocator: &str, payload: &str| {
            if payload_first { format!("{payload}, {allocator}") }
            else { format!("{allocator}, {payload}") }
        };
        let unique = unique_state.map(|state| format!(r#"
impl Unique ({state}) for Shell {section} where A: Memory
    unsafe release = _, state !-> ()
"#, section = args("A", "_"))).unwrap_or_default();
        format!(r#"
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
trait Memory
    @key: I64
struct Small
    < tag: I64
impl Memory for Small
    @key = -> self.tag
struct Large
    < padding: (I64, I64)
    < tag: I64
impl Memory for Large
    @key = -> self.tag
struct State A
    < allocator: A
struct OtherState A
    < allocator: A
struct Shell {header}
    < raw: *T
    < state: State A
impl Transport (State A) for Shell {section} where A: Memory
    unsafe split = owner -> (owner.raw, owner.state)
    unsafe build = pointer, state -> Shell
        raw: pointer
        state: state
{unique}
trait Take
    ~@take: I64
consume_small: Shell {small} -> I64
consume_small = owner -> owner.take!
consume_large: Shell {large} -> I64
consume_large = owner -> owner.take!
consume_generic: Shell {generic} -> I64 where A: Memory, A: Layout
consume_generic = owner -> owner.take!
main = !-> ()
"#,
            header = args("A", "(T: ?Layout)"), section = args("A", "_"),
            small = args("Small", "Take"), large = args("Large", "Take"), generic = args("A", "Take"))
    }

    fn opened_owner_source(payload_first: bool) -> String {
        let mut source = closed_owner_source(payload_first, Some("State A"));
        for name in ["consume_small", "consume_large", "consume_generic"] {
            source = source.replace(
                &format!("{name} = owner -> owner.take!"),
                &format!("{name} = owner ->\n    open owner as Hidden, value\n        value.take!"),
            );
        }
        source
    }

    #[test]
    fn opened_owner_lookup_requires_the_exact_constructor_and_fixed_arguments() {
        let program = program_from_source(&opened_owner_source(false));
        let function = program.functions.values().find(|f| f.name == "consume_small").unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { unreachable!() };
        let HirExprKindFor::Open { binding, .. } = &expression.kind else { unreachable!() };
        assert!(binding.matches_owner(&binding.value.ty));
        let Type::Struct { id, args } = &binding.value.ty else { unreachable!() };
        let other = program.structs.values().find(|s| s.name == "Large").unwrap().id;
        let mut changed = args.clone();
        changed[0] = Type::Struct { id: other, args: Vec::new() };
        assert!(!binding.matches_owner(&Type::Struct { id: *id, args: changed }));
        assert!(!binding.matches_owner(&Type::Struct { id: other, args: args.clone() }));
        assert!(!binding.matches_owner(&Type::Reference {
            inner: Box::new(binding.value.ty.clone()),
            mutable: true,
        }));

        // A one-argument unrelated constructor must not acquire the old unary
        // binding just because it mentions the same rigid witness.
        let program = self::program();
        let function = program.functions.values().find(|f| f.name == "consume_opened").unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { unreachable!() };
        let HirExprKindFor::Open { binding, .. } = &expression.kind else { unreachable!() };
        let foreign = program.traits.values().find(|t| t.name == "Take").unwrap().id;
        assert!(!binding.matches_owner(&Type::Struct {
            id: foreign,
            args: vec![Type::Witness(binding.witness)],
        }));
    }

    #[test]
    fn opened_owner_revalidation_rejects_forged_constructor_state_and_payload_slots() {
        for payload_first in [false, true] {
            for mutation in 0..4 {
                let mut program = program_from_source(&opened_owner_source(payload_first));
                AcceptedHirProgram::revalidate_for_test(program.clone()).unwrap();
                let other = program.structs.values().find(|s| s.name == "Large").unwrap().id;
                let function = program.functions.values_mut().find(|f| f.name == "consume_small").unwrap();
                let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else { unreachable!() };
                let HirExprKindFor::Open { binding, .. } = &mut expression.kind else { unreachable!() };
                let owner = binding.owner.as_mut().unwrap();
                match mutation {
                    0 => owner.state_ty = Type::I64,
                    1 => {
                        let Type::Lambda { body, .. } = &mut owner.from_parts.owner_ty else { unreachable!() };
                        let Type::Struct { args, .. } = body.as_mut() else { unreachable!() };
                        args[usize::from(payload_first)] = Type::Struct { id: other, args: Vec::new() };
                    }
                    2 => {
                        let Type::Struct { args, .. } = &mut binding.value.ty else { unreachable!() };
                        args[usize::from(payload_first)] = Type::Struct { id: other, args: Vec::new() };
                    }
                    _ => {
                        let Type::Struct { args, .. } = &mut binding.value.ty else { unreachable!() };
                        args.swap(0, 1);
                    }
                }
                let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
                assert!(errors.iter().any(|error| {
                    error.contains("opened owner split/rebuild authority")
                        || error.contains("opening must preserve the exact owner constructor")
                        || error.contains("opening handle does not preserve its exact source access")
                }), "payload_first={payload_first}, mutation={mutation}: {errors:?}");
            }
        }
    }

    #[test]
    fn opened_owner_revalidation_requires_actual_contextual_allocator_bounds() {
        for payload_first in [false, true] {
            let mut program = program_from_source(&opened_owner_source(payload_first));
            AcceptedHirProgram::revalidate_for_test(program.clone()).unwrap();
            let memory = program.traits.values().find(|t| t.name == "Memory").unwrap().id;
            let function = program.functions.values_mut().find(|f| f.name == "consume_generic").unwrap();
            let parameter = function.generic_params[0].id;
            function.generic_bounds.get_mut(&parameter).unwrap().retain(|bound| bound.trait_id != memory);
            function.generic_bounds.predicates.retain(|predicate| {
                let crate::types::Predicate::Trait { trait_id, .. } = predicate;
                *trait_id != memory
            });
            let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
            assert!(errors.iter().any(|error| error.contains("owner transport State is not uniquely determined")), "{errors:?}");
        }
    }

    #[test]
    fn closed_owner_fixed_sections_preserve_allocator_state_and_operations() {
        use crate::type_services::kind::Kind;
        for payload_first in [false, true] {
            let program = program_from_source(&closed_owner_source(payload_first, Some("State A")));
            AcceptedHirProgram::revalidate_for_test(program.clone()).unwrap();
            let shell = program.structs.values().find(|definition| definition.name == "Shell").unwrap().id;
            let state = program.structs.values().find(|definition| definition.name == "State").unwrap().id;
            let take = program.traits.values().find(|definition| definition.name == "Take").unwrap().id;
            let transport = program.language_items.object_owner.as_ref().unwrap();
            let unique = program.language_items.object_owner_unique.as_ref().unwrap();
            for (name, allocator) in [("consume_small", "Small"), ("consume_large", "Large")] {
                let function = program.functions.values().find(|function| function.name == name).unwrap();
                assert!(hir_function_is_codegen_concrete(function));
                let allocator = Type::Struct { id: program.structs.values().find(|definition| definition.name == allocator).unwrap().id, args: Vec::new() };
                let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { panic!("consuming tail"); };
                let HirExprKindFor::OwnedObjectCall { owner, args, call } = &expression.kind else { panic!("actual accepted owned call"); };
                assert!(args.is_empty());
                assert_eq!(expression.ty, Type::I64);
                let Type::Object(object) = &call.object else { panic!("object signature"); };
                assert_eq!(object.principal.trait_id, take);
                assert!(matches!(call.method.target, HirSelectedMethodTarget::TraitMethod { dispatch: HirTraitDispatchKind::Object, .. }));
                let payload_position = usize::from(!payload_first);
                let fixed_position = usize::from(payload_first);
                let mut owner_args = vec![allocator.clone(); 2];
                owner_args[payload_position] = call.object.clone();
                assert_eq!(owner.ty, Type::Struct { id: shell, args: owner_args });
                assert_eq!(call.state, Type::Struct { id: state, args: vec![allocator.clone()] });
                for (operation, trait_id, member_id, expected_params, expected_ret) in [
                    (&call.into_parts, transport.trait_id, transport.into_parts_id, vec![owner.ty.clone()],
                        Type::Tuple(vec![Type::Pointer(Box::new(call.object.clone())), call.state.clone()])),
                    (&call.release, unique.trait_id, unique.method_id, vec![Type::Pointer(Box::new(Type::U8)), call.state.clone()], Type::Unit),
                ] {
                    let Type::Lambda { params, body } = &operation.owner_ty else { panic!("fixed constructor section"); };
                    assert_eq!(params, &vec![Kind::Type]);
                    let Type::Struct { id, args } = body.as_ref() else { panic!("nominal section body"); };
                    assert_eq!(*id, shell);
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[fixed_position], allocator);
                    assert_eq!(args[payload_position], Type::BoundVar { depth: 0, index: 0, kind: Kind::Type });
                    assert_eq!(crate::type_services::normalize::apply_type_lambda(&operation.owner_ty, std::slice::from_ref(&call.object)), Some(owner.ty.clone()));
                    let HirSelectedMethodTarget::ImplMethod { impl_id, method_id, selected_trait: Some(selected) } = &operation.method.target else { panic!("canonical selected operation"); };
                    assert_eq!(selected.trait_id, trait_id);
                    assert_eq!(selected.member_id, member_id);
                    assert_eq!(selected.trait_args, vec![call.state.clone()]);
                    let implementation = &program.impls[impl_id];
                    let method = implementation.methods.values().find(|method| method.id == *method_id).unwrap();
                    assert!(method.is_unsafe && method.self_receiver.is_none());
                    assert_eq!(operation.method.owner_substitution, vec![HirTypeBinding { param: implementation.type_generics[0].id, ty: allocator.clone() }]);
                    if member_id == transport.into_parts_id {
                        let payload = method.generic_params.iter().find(|param| param.id.owner == method.id).unwrap();
                        assert!(payload.maybe_unsized);
                        assert_eq!(operation.method.method_substitution, vec![HirTypeBinding { param: payload.id, ty: call.object.clone() }]);
                    } else {
                        assert!(operation.method.method_substitution.is_empty());
                    }
                    let substitution = operation.method.owner_substitution.iter().chain(&operation.method.method_substitution)
                        .map(|binding| (binding.param, binding.ty.clone())).collect::<HashMap<_, _>>();
                    assert_eq!(method.params.iter().map(|param| param.ty.substitute_generics(&substitution)).collect::<Vec<_>>(), expected_params);
                    assert_eq!(method.ret_type.substitute_generics(&substitution), expected_ret);
                    eprintln!("CLOSED_OWNER fixture={name} payloadFirst={payload_first} actualAcceptedTarget={operation:?} State={:?} execution=NONE", call.state);
                }
            }
        }
    }

    fn closed_owner_unique_source_rejection(state: Option<&str>) {
        for payload_first in [false, true] {
            let source = closed_owner_source(payload_first, state);
            let errors = crate::analyze(&source_config(&source)).unwrap_err();
            assert!(errors.0.iter().any(|error| error.message.starts_with(
                "consuming object call requires unique release for the same owner constructor and State:"
            ) && error.primary.as_ref().is_some_and(|label| {
                source.get(label.span.start..label.span.end) == Some("owner")
                    && source.get(label.span.end..).is_some_and(|text| text.starts_with(".take!"))
            })), "{errors:?}");
        }
    }

    #[test]
    fn closed_owner_missing_unique_rejects_at_consuming_source() {
        closed_owner_unique_source_rejection(None);
    }

    #[test]
    fn closed_owner_cross_state_release_rejects_at_consuming_source() {
        closed_owner_unique_source_rejection(Some("OtherState A"));
    }

    #[test]
    fn closed_owner_missing_canonical_state_sized_rejects_revalidation() {
        let mut program = program_from_source(&closed_owner_source(false, Some("State A")));
        program.language_items.sized = None;
        let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("unique owner State requires a canonical Sized capability")), "{errors:?}");
    }

    #[test]
    fn closed_owner_declared_generic_state_sized_is_reproved() {
        let mut program = program_from_source(&closed_owner_source(false, Some("State A")));
        AcceptedHirProgram::revalidate_for_test(program.clone()).unwrap();
        let sized = program.language_items.sized.as_ref().unwrap().trait_id;
        let function = program.functions.values_mut().find(|function| function.name == "consume_generic").unwrap();
        let parameter = function.generic_params[0].id;
        assert!(function.generic_bounds[&parameter].iter().any(|bound| bound.trait_id == sized));
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { panic!("generic consuming tail"); };
        let HirExprKindFor::OwnedObjectCall { call, .. } = &expression.kind else { panic!("generic accepted owned call"); };
        assert!(crate::type_services::visit::type_any(&call.state, |ty| ty == &Type::Generic(parameter)));
        for operation in [&call.into_parts, &call.release] {
            assert!(operation.method.owner_substitution.iter().any(|binding| binding.ty == Type::Generic(parameter)));
        }
        // Remove the actual context evidence, including its source predicate;
        // do not make a State equation stand in for the missing Sized bound.
        function.generic_bounds.get_mut(&parameter).unwrap().retain(|bound| bound.trait_id != sized);
        function.generic_bounds.predicates.retain(|predicate| {
            let crate::types::Predicate::Trait { trait_id, .. } = predicate;
            *trait_id != sized
        });
        function.generic_params[0].maybe_unsized = true;
        function.generic_bounds.relaxed_sized.insert(parameter);
        let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("owner transport State is not uniquely determined")), "{errors:?}");
    }

    #[test]
    fn closed_owner_payload_dependent_state_rejects_revalidation() {
        let mut program = program_from_source(&closed_owner_source(false, Some("State A")));
        let function = program.functions.values().find(|function| function.name == "consume_small").unwrap();
        let HirStmtFor::Expr(expression) = &function.body.stmts[0] else { panic!("consuming tail"); };
        let HirExprKindFor::OwnedObjectCall { call, .. } = &expression.kind else { panic!("accepted owned call"); };
        let object = call.object.clone();
        let transport = program.language_items.object_owner.as_ref().unwrap().trait_id;
        let unique = program.language_items.object_owner_unique.as_ref().unwrap().trait_id;
        for implementation in program.impls.values_mut().filter(|implementation| matches!(implementation.trait_id, Some(id) if id == transport || id == unique)) {
            implementation.trait_arg_types = vec![object.clone()];
        }
        let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
        assert!(errors.iter().any(|error| error.contains("unique owner State must be sized and independent of the payload")), "{errors:?}");
    }

    #[test]
    fn closed_owner_same_layout_state_and_fixed_bindings_cannot_be_forged() {
        let valid = program_from_source(&closed_owner_source(false, Some("State A")));
        let other_state = valid.structs.values().find(|definition| definition.name == "OtherState").unwrap().id;
        let other_allocator = Type::Struct { id: valid.structs.values().find(|definition| definition.name == "Large").unwrap().id, args: Vec::new() };
        for mutation in 0..4 {
            let mut program = valid.clone();
            let function = program.functions.values_mut().find(|function| function.name == "consume_small").unwrap();
            let HirStmtFor::Expr(expression) = &mut function.body.stmts[0] else { panic!("consuming tail"); };
            let HirExprKindFor::OwnedObjectCall { owner, call, .. } = &mut expression.kind else { panic!("accepted owned call"); };
            match mutation {
                0 => {
                    let Type::Struct { args, .. } = &call.state else { panic!("State allocator"); };
                    call.state = Type::Struct { id: other_state, args: args.clone() };
                }
                1 => call.release.method.owner_substitution[0].ty = other_allocator.clone(),
                2 => call.into_parts.method.method_substitution[0].ty = Type::I64,
                _ => {
                    let Type::Struct { args, .. } = &mut owner.ty else { panic!("nominal owner"); };
                    args[0] = call.object.clone();
                    assert!(object(&owner.ty).is_none(), "two object arguments are ambiguous discovery");
                }
            }
            let errors = AcceptedHirProgram::revalidate_for_test(program).unwrap_err();
            let expected = if mutation == 3 { "consuming object dispatch requires an owning value, not a reference or raw pointer" }
                else { "owned object call has invalid transport or unique-release evidence" };
            assert!(errors.iter().any(|error| error.contains(expected)), "mutation={mutation}: {errors:?}");
        }
    }

    #[test]
    fn forward_owner_protocol_rebinds_the_complete_relaxed_payload_scheme() {
        let contract = r#"
lang object_owner
< trait Transport State for F _
    lang owner_into_parts
    unsafe split: F T -> (*T, State) where T: ?Layout
    lang owner_from_parts
    unsafe build: *T -> State -> F T where T: ?Layout
"#;
        for forward in [false, true] {
            let source = format!(
                r#"
lang sized
< trait Layout
{before}
struct Own (T: ?Layout)
    < raw: *T
impl Transport I64 for Own
    unsafe split = owner ->
        parts = (owner.raw, 0)
        ~Forget owner
        parts
    unsafe build = pointer, _ -> Own
        raw: pointer
{after}
lang object_owner_unique
< trait Unique State for F _
    lang owner_release
    unsafe release: *U8 -> State -> ()
impl Unique I64 for Own
    unsafe release = _, _ !-> ()
trait Take
    ~@take: I64
consume: Own Take -> I64
consume = owner -> owner.take!
main = !-> ()
"#,
                before = if forward { "" } else { contract },
                after = if forward { contract } else { "" }
            );
            let program = program_from_source(&source);
            let transport = program.language_items.object_owner.as_ref().unwrap();
            let implementation = program
                .impls
                .values()
                .find(|implementation| implementation.trait_id == Some(transport.trait_id))
                .unwrap();
            for name in ["split", "build"] {
                let method = &implementation.methods[name];
                let [parameter] = method.generic_params.as_slice() else {
                    panic!("expected one payload parameter");
                };
                assert_eq!(parameter.id.owner, method.id, "forward={forward}, {name}");
                assert!(parameter.maybe_unsized);
                assert!(method.generic_bounds.relaxed_sized.contains(&parameter.id));
                assert!(method
                    .generic_bounds
                    .get(&parameter.id)
                    .is_none_or(Vec::is_empty));
                for ty in method
                    .params
                    .iter()
                    .map(|parameter| &parameter.ty)
                    .chain(std::iter::once(&method.ret_type))
                {
                    assert!(!crate::type_services::visit::type_any(
                        ty,
                        |ty| matches!(ty, Type::Generic(id) if id.owner == transport.into_parts_id || id.owner == transport.from_parts_id)
                    ));
                }
            }
            let mut narrowed = program.clone();
            let split = narrowed
                .impls
                .get_mut(&implementation.id)
                .unwrap()
                .methods
                .get_mut("split")
                .unwrap();
            split.generic_params[0].maybe_unsized = false;
            split.generic_bounds.relaxed_sized.clear();
            let errors = AcceptedHirProgram::revalidate_for_test(narrowed).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("unrestricted ?Sized payload contract")),
                "{errors:?}"
            );
        }
    }

    #[test]
    fn closed_owned_call_keeps_static_constructor_authority_codegen_concrete() {
        let program = program();
        let consume = program
            .functions
            .values()
            .find(|function| function.name == "consume")
            .unwrap();
        assert!(consume.generic_params.is_empty());
        assert!(hir_function_is_codegen_concrete(consume));
        let opened = program
            .functions
            .values()
            .find(|function| function.name == "consume_opened")
            .unwrap();
        assert!(!hir_function_is_codegen_concrete(opened));
        let HirStmtFor::Expr(expression) = &consume.body.stmts[0] else {
            panic!("expected owned call");
        };
        let HirExprKindFor::OwnedObjectCall { call, .. } = &expression.kind else {
            panic!("expected owned call");
        };
        for operation in [&call.into_parts, &call.release] {
            assert!(matches!(operation.owner_ty, Type::Constructor { .. }));
            assert!(
                !crate::type_services::facts::TypeFacts::is_codegen_concrete(&operation.owner_ty)
            );
        }

        for mutation in 0..6 {
            let mut unresolved = consume.clone();
            let HirStmtFor::Expr(expression) = &mut unresolved.body.stmts[0] else {
                unreachable!();
            };
            let HirExprKindFor::OwnedObjectCall { owner, call, .. } = &mut expression.kind else {
                unreachable!();
            };
            let generic = Type::Generic(crate::types::GenericParamId {
                owner: consume.id,
                index: 0,
            });
            match mutation {
                0 => call.state = call.into_parts.owner_ty.clone(),
                1 => call.object = generic.clone(),
                2 => owner.ty = generic.clone(),
                3 => call.into_parts.owner_ty = generic.clone(),
                4 => call.release.owner_ty = generic.clone(),
                _ => call.method.method_substitution.push(HirTypeBinding {
                    param: crate::types::GenericParamId {
                        owner: consume.id,
                        index: 0,
                    },
                    ty: generic,
                }),
            }
            assert!(
                !hir_function_is_codegen_concrete(&unresolved),
                "mutation {mutation}"
            );
        }
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
            if mutation == 2 {
                assert!(
                    errors.iter().any(|error| error.contains(
                        "consuming object dispatch requires an owning value, not a reference or raw pointer"
                    )),
                    "{errors:?}"
                );
                continue;
            }
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("owner") || error.contains("owned")),
                "{errors:?}"
            );
        }
    }
}
