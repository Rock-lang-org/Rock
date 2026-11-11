use super::*;

impl HirProgram {
    pub(super) fn type_normalization_env(
        &self,
    ) -> crate::type_services::normalize::TypeNormalizationEnv {
        let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
        for structure in self.structs.values() {
            normalization_env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for enumeration in self.enums.values() {
            normalization_env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        crate::type_lowering::register_type_aliases(
            &mut normalization_env,
            self.type_aliases.values(),
        );
        for trait_def in self.traits.values() {
            for param in &trait_def.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
            if let Some(target) = &trait_def.target {
                normalization_env.register_generic_kind(target.id, target.kind.clone());
            }
            for associated_type in &trait_def.associated_types {
                normalization_env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
            for function in trait_def.methods.values() {
                for param in &function.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for param in &signature.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        for function in self.functions.values() {
            for param in &function.generic_params {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for imp in self.impls.values() {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                normalization_env.register_generic_kind(param.id, param.kind.clone());
            }
            for function in imp.methods.values() {
                for param in &function.generic_params {
                    normalization_env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }

        normalization_env
    }

    pub(crate) fn validate_accepted_types(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let normalization_env = self.type_normalization_env();
        let mut predicate_rows = Vec::new();
        for trait_def in self.traits.values() {
            for (index, predicate) in trait_def.predicates.iter().enumerate() {
                predicate_rows.push((trait_def.id, index, predicate));
            }
            for signature in trait_def.signatures.values() {
                for (index, predicate) in signature.generic_bounds.predicates.iter().enumerate() {
                    predicate_rows.push((signature.id, index, predicate));
                }
            }
            for method in trait_def.methods.values() {
                for (index, predicate) in method.generic_bounds.predicates.iter().enumerate() {
                    predicate_rows.push((method.id, index, predicate));
                }
            }
        }
        for function in self.functions.values() {
            for (index, predicate) in function.generic_bounds.predicates.iter().enumerate() {
                predicate_rows.push((function.id, index, predicate));
            }
        }
        for imp in self.impls.values() {
            for (index, predicate) in imp.bounds.predicates.iter().enumerate() {
                predicate_rows.push((imp.id, index, predicate));
            }
        }
        predicate_rows.sort_by_key(|(owner, index, _)| (*owner, *index));
        for (owner, index, predicate) in predicate_rows {
            let crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } = predicate;
            let Some(required_trait) = self.traits.get(trait_id) else {
                errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] references unknown trait {trait_id:?}"
                ));
                continue;
            };
            let expected_subject_kind = required_trait
                .target
                .as_ref()
                .map(|target| target.kind.clone())
                .unwrap_or(crate::type_services::kind::Kind::Type);
            let normalizer =
                crate::type_services::normalize::TypeNormalizer::new(&normalization_env);
            match normalizer.kind_of(subject) {
                Ok(actual) if actual != expected_subject_kind => errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] subject has kind {actual}, expected {expected_subject_kind} for trait '{}'",
                    required_trait.name
                )),
                Err(error) => errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] subject kind is invalid: {error}"
                )),
                _ => {}
            }
            if args.len() != required_trait.generic_params.len() {
                errors.push(format!(
                    "accepted HIR predicate {owner:?}[{index}] supplies {} arguments to trait '{}', expected {}",
                    args.len(),
                    required_trait.name,
                    required_trait.generic_params.len()
                ));
                continue;
            }
            for (arg_index, (arg, param)) in
                args.iter().zip(&required_trait.generic_params).enumerate()
            {
                match crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                    .kind_of(arg)
                {
                    Ok(actual) if actual != param.kind => errors.push(format!(
                        "accepted HIR predicate {owner:?}[{index}] argument {arg_index} has kind {actual}, expected {}",
                        param.kind
                    )),
                    Err(error) => errors.push(format!(
                        "accepted HIR predicate {owner:?}[{index}] argument {arg_index} kind is invalid: {error}"
                    )),
                    _ => {}
                }
            }
        }

        let mut type_context = crate::type_context::TypeContext::new();
        let type_ids = collect_hir_type_ids(self, &mut type_context);
        let mut locations = type_ids.iter().collect::<Vec<_>>();
        locations.sort_by_key(|(location, _)| format!("{location:?}"));
        for (location, type_id) in locations {
            let ty = type_context.type_for(type_id);
            if type_contains_backend_forbidden_sentinel(&ty) {
                errors.push(format!(
                    "accepted HIR contains unresolved type at {location:?}: {ty:?}"
                ));
            }
            let normalizer =
                crate::type_services::normalize::TypeNormalizer::new(&normalization_env);
            if let Err(error) = normalizer.validate_canonical(&ty) {
                errors.push(format!(
                    "accepted HIR contains noncanonical type at {location:?}: {error}"
                ));
            }
            let expected_kind = match location {
                HirTypeLocation::ArrayRepeatLength { .. } => Some(crate::type_services::kind::Kind::Usize),
                HirTypeLocation::TypeAliasBody { .. } => None,
                HirTypeLocation::AssociatedTypeDef {
                    impl_id,
                    assoc_type,
                } => self.impls.get(impl_id).and_then(|imp| {
                    imp.associated_types
                        .iter()
                        .find(|associated| associated.id == *assoc_type)
                        .map(|associated| associated.kind.clone())
                }),
                location if hir_type_location_requires_runtime_type(location) => {
                    Some(crate::type_services::kind::Kind::Type)
                }
                _ => None,
            };
            if let Some(expected_kind) = expected_kind {
                match crate::type_services::normalize::TypeNormalizer::new(&normalization_env)
                    .kind_of(&ty)
                {
                    Ok(actual_kind) if actual_kind != expected_kind => errors.push(format!(
                        "accepted HIR type at {location:?} has kind {actual_kind}, expected {expected_kind}"
                    )),
                    Err(error) => errors.push(format!(
                        "accepted HIR cannot determine kind at {location:?}: {error}"
                    )),
                    _ => {}
                }
            }
            validate_projection_ambiguities(self, &ty, &format!("{location:?}"), &mut errors);
        }

        errors
    }
}

fn hir_type_location_requires_runtime_type(location: &HirTypeLocation) -> bool {
    matches!(
        location,
        HirTypeLocation::FunctionReturn { .. }
            | HirTypeLocation::FunctionParam { .. }
            | HirTypeLocation::ExternReturn { .. }
            | HirTypeLocation::ExternParam { .. }
            | HirTypeLocation::StructField { .. }
            | HirTypeLocation::EnumVariantNamedField { .. }
            | HirTypeLocation::EnumVariantPositionalField { .. }
            | HirTypeLocation::TraitSignatureReturn { .. }
            | HirTypeLocation::TraitSignatureParam { .. }
            | HirTypeLocation::Block { .. }
            | HirTypeLocation::LetStmt { .. }
            | HirTypeLocation::Expr { .. }
            | HirTypeLocation::ClosureCapture { .. }
            | HirTypeLocation::ClosureParam { .. }
            | HirTypeLocation::CastTarget { .. }
            | HirTypeLocation::TryOutput { .. }
            | HirTypeLocation::TryResidual { .. }
            | HirTypeLocation::TryReturn { .. }
    )
}

pub(super) fn type_contains_backend_forbidden_sentinel(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| {
        matches!(nested, Type::TypeVar(_) | Type::Error)
    })
}

fn projection_input_is_concrete(ty: &Type) -> bool {
    crate::type_services::facts::TypeFacts::is_codegen_concrete(ty)
}

fn projection_impl_matches(
    imp: &HirImpl,
    base_ty: &Type,
    trait_id: DefId,
    trait_args: &[Type],
    assoc_type_id: AssocTypeId,
) -> bool {
    if imp.trait_id != Some(trait_id)
        || imp.trait_arg_types.len() != trait_args.len()
        || !imp
            .associated_types
            .iter()
            .any(|associated| associated.id == assoc_type_id)
    {
        return false;
    }
    let Some(mut substitution) =
        crate::selection::receiver_pattern_substitution(&imp.receiver_pattern, base_ty)
    else {
        return false;
    };
    imp.trait_arg_types
        .iter()
        .zip(trait_args)
        .all(|(expected, actual)| {
            crate::selection::type_pattern_matches(expected, actual, &mut substitution)
        })
}

fn validate_projection_ambiguities(
    program: &HirProgram,
    ty: &Type,
    location: &str,
    errors: &mut Vec<String>,
) {
    match ty {
        Type::Projection {
            ty: base_ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            if assoc_type.owner == *trait_id
                && projection_input_is_concrete(base_ty)
                && trait_args.iter().all(projection_input_is_concrete)
            {
                let mut matching = program
                    .impls
                    .values()
                    .filter(|imp| {
                        projection_impl_matches(
                            imp,
                            base_ty,
                            *trait_id,
                            trait_args,
                            assoc_type.assoc_type_id,
                        )
                    })
                    .map(|imp| imp.id)
                    .collect::<Vec<_>>();
                matching.sort();
                matching.dedup();
                if matching.len() > 1 {
                    errors.push(format!(
                        "accepted HIR contains multiple matching typed projection implementations at {location}: {matching:?}"
                    ));
                }
            }
            validate_projection_ambiguities(program, base_ty, location, errors);
            for arg in trait_args {
                validate_projection_ambiguities(program, arg, location, errors);
            }
        }
        Type::Slice(inner) | Type::Pointer(inner) | Type::Array(inner, _) => {
            validate_projection_ambiguities(program, inner, location, errors)
        }
        Type::Reference { inner, .. } => {
            validate_projection_ambiguities(program, inner, location, errors)
        }
        Type::Tuple(types) => {
            for ty in types {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Function {
            params,
            ret,
            captures,
            ..
        } => {
            for ty in params {
                validate_projection_ambiguities(program, ty, location, errors);
            }
            validate_projection_ambiguities(program, ret, location, errors);
            for capture in captures {
                validate_projection_ambiguities(program, &capture.ty, location, errors);
            }
        }
        Type::Struct { args, .. } | Type::Enum { args, .. } => {
            for ty in args {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Apply { constructor, args } => {
            validate_projection_ambiguities(program, constructor, location, errors);
            for ty in args {
                validate_projection_ambiguities(program, ty, location, errors);
            }
        }
        Type::Lambda { body, .. } => {
            validate_projection_ambiguities(program, body, location, errors)
        }
        _ => {}
    }
}
