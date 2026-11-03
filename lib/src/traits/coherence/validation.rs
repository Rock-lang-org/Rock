use super::*;

pub fn validate_coherence<'a>(
    traits: impl IntoIterator<Item = &'a HirTrait>,
    impls: impl IntoIterator<Item = &'a HirImpl>,
    structs: impl IntoIterator<Item = &'a HirStruct>,
    enums: impl IntoIterator<Item = &'a HirEnum>,
    aliases: impl IntoIterator<Item = &'a HirTypeAlias>,
    sized_trait_id: Option<DefId>,
) -> Vec<String> {
    let traits = traits.into_iter().collect::<Vec<_>>();
    let impls = impls.into_iter().collect::<Vec<_>>();
    let structs = structs.into_iter().collect::<Vec<_>>();
    let enums = enums.into_iter().collect::<Vec<_>>();
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    let display_context = coherence_display_context(&traits, &impls, &structs, &enums, &aliases);
    validate_coherence_with_ids(
        traits.iter().copied(),
        impls.iter().copied(),
        structs.iter().copied(),
        enums.iter().copied(),
        aliases.iter().copied(),
        sized_trait_id,
    )
    .into_iter()
    .map(|error| {
        error.render_with(
            |ty| {
                crate::type_services::display::display_type_with_context(ty, &display_context)
                    .to_string()
            },
            |id| {
                display_context
                    .definition_name(id)
                    .unwrap_or("<unknown item>")
                    .to_string()
            },
        )
    })
    .collect()
}

pub(super) fn validate_coherence_with_ids<'a>(
    traits: impl IntoIterator<Item = &'a HirTrait>,
    impls: impl IntoIterator<Item = &'a HirImpl>,
    structs: impl IntoIterator<Item = &'a HirStruct>,
    enums: impl IntoIterator<Item = &'a HirEnum>,
    aliases: impl IntoIterator<Item = &'a HirTypeAlias>,
    sized_trait_id: Option<DefId>,
) -> Vec<CoherenceError> {
    let traits = traits.into_iter().collect::<Vec<_>>();
    let mut impls = impls.into_iter().collect::<Vec<_>>();
    let structs = structs.into_iter().collect::<Vec<_>>();
    let enums = enums.into_iter().collect::<Vec<_>>();
    let aliases = aliases.into_iter().collect::<Vec<_>>();
    impls.sort_by_key(|imp| def_id_key(imp.id));

    let trait_by_id = traits
        .iter()
        .map(|trait_def| (trait_def.id, *trait_def))
        .collect::<HashMap<_, _>>();
    let env = normalization_env(&traits, &impls, &structs, &enums, &aliases);
    let mut diagnostics = Vec::new();
    let mut targets = HashMap::new();
    let mut trait_args = HashMap::new();

    for imp in &impls {
        let Some(trait_id) = imp.trait_id else {
            continue;
        };
        let Some(trait_def) = trait_by_id.get(&trait_id).copied() else {
            continue;
        };

        let target = match normalize_target(&env, &imp.receiver_pattern) {
            Ok(target) => target,
            Err(error) => {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidTarget {
                        trait_id,
                        target: imp.receiver_pattern.clone(),
                        error,
                    },
                });
                continue;
            }
        };
        let expected_kind = trait_def
            .target
            .as_ref()
            .map(|target| target.kind.clone())
            .unwrap_or(Kind::Type);
        let actual_kind = match target.kind(&env) {
            Ok(kind) => kind,
            Err(error) => {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidTarget {
                        trait_id,
                        target: imp.receiver_pattern.clone(),
                        error,
                    },
                });
                continue;
            }
        };
        if actual_kind != expected_kind {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::TargetKindMismatch {
                    trait_id,
                    expected: expected_kind,
                    actual: actual_kind,
                },
            });
            continue;
        }
        if target.is_constructor() != !matches!(expected_kind, Kind::Type) {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::TargetCategoryMismatch {
                    trait_id,
                    expected: expected_kind,
                },
            });
            continue;
        }
        if let CanonicalTarget::Constructor(ty) = &target {
            if let Err(error) = validate_constructor_header(ty, &imp_generic_ids(imp)) {
                diagnostics.push(CoherenceError {
                    impl_id: imp.id,
                    kind: CoherenceErrorKind::InvalidConstructorTarget {
                        trait_id,
                        target: ty.clone(),
                        reason: error,
                    },
                });
                continue;
            }
        }

        let outer = target.outer_nominal();
        if trait_id.crate_id != imp.id.crate_id
            && outer.is_none_or(|outer_id| outer_id.crate_id != imp.id.crate_id)
        {
            diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::Orphan {
                    trait_id,
                    target: target.clone(),
                },
            });
        }

        let normalized_args = imp
            .trait_arg_types
            .iter()
            .map(|arg| TypeNormalizer::new(&env).normalize(arg))
            .collect::<Result<Vec<_>, _>>();
        match normalized_args {
            Ok(args) => {
                targets.insert(imp.id, target);
                trait_args.insert(imp.id, args);
            }
            Err(error) => diagnostics.push(CoherenceError {
                impl_id: imp.id,
                kind: CoherenceErrorKind::InvalidTraitArguments { trait_id, error },
            }),
        }
    }

    for (index, left) in impls.iter().enumerate() {
        let Some(trait_id) = left.trait_id else {
            continue;
        };
        let Some(left_target) = targets.get(&left.id) else {
            continue;
        };
        for right in impls.iter().skip(index + 1) {
            if right.trait_id != Some(trait_id) {
                continue;
            }
            let Some(right_target) = targets.get(&right.id) else {
                continue;
            };
            let left_args = &trait_args[&left.id];
            let right_args = &trait_args[&right.id];
            if left_args.len() != right_args.len() {
                continue;
            }

            let mut unifier = FirstOrderUnifier::new(left, right, sized_trait_id, &env);
            if !left_args
                .iter()
                .zip(right_args)
                .all(|(left, right)| unifier.unify(left, right))
                || !unifier.unify_targets(left_target, right_target)
            {
                continue;
            }
            diagnostics.push(CoherenceError {
                impl_id: left.id,
                kind: CoherenceErrorKind::Overlapping {
                    left_impl_id: left.id,
                    right_impl_id: right.id,
                    trait_id,
                    left_target: left_target.clone(),
                    right_target: right_target.clone(),
                    witness: unifier.witness(),
                },
            });
        }
    }

    diagnostics.sort_by_key(coherence_error_sort_key);
    diagnostics.dedup();
    diagnostics
}
