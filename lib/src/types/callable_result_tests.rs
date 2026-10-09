use super::*;

#[test]
fn callable_result_optional_authority_remains_pointer_sized() {
    assert_eq!(
        std::mem::size_of::<CallableResultBorrowing>(),
        std::mem::size_of::<usize>()
    );
    // Preserve the pre-contract recursive type footprint on 64-bit targets.
    assert!(std::mem::size_of::<Type>() <= 8 * std::mem::size_of::<usize>());
}

#[test]
fn callable_result_absent_authority_is_not_checked_independence() {
    let absent = CallableResultBorrowing::Unknown;
    let independent = CallableResultBorrowing::Sources(Box::new(vec![]));
    let captured =
        CallableResultBorrowing::Sources(Box::new(vec![CallableResultSource::EnvironmentValue]));
    let storage =
        CallableResultBorrowing::Sources(Box::new(vec![CallableResultSource::EnvironmentStorage]));
    assert_ne!(absent, independent);
    assert!(absent.accepts(&independent));
    assert!(!independent.accepts(&absent));
    assert!(captured.accepts(&independent));
    assert!(!independent.accepts(&captured));
    assert!(!captured.accepts(&storage));
    assert!(!storage.accepts(&captured));
}

#[test]
fn callable_result_dependencies_require_canonical_valid_ordinals_and_views() {
    use CallableResultSource as S;
    for (sources, valid) in [
        (vec![], true),
        (
            vec![S::ArgumentValue(0), S::EnvironmentValue, S::Static],
            true,
        ),
        (vec![S::ArgumentContents(0)], true),
        (vec![S::ArgumentBorrowedContents(0)], true),
        (vec![S::ArgumentContents(1)], false),
        (vec![S::ArgumentValue(2)], false),
        (vec![S::Static, S::EnvironmentValue], false),
        (vec![S::EnvironmentValue, S::EnvironmentValue], false),
        (vec![S::EnvironmentStorage, S::Unknown], true),
    ] {
        assert_eq!(
            CallableResultBorrowing::Sources(Box::new(sources))
                .sources_are_valid(2, |index| index == 0),
            valid
        );
    }
}

#[test]
fn callable_result_joins_keep_every_alternative_and_unknown() {
    use CallableResultSource as S;
    let values =
        CallableResultBorrowing::Sources(Box::new(vec![S::ArgumentValue(0), S::EnvironmentValue]));
    let storage = CallableResultBorrowing::Sources(Box::new(vec![
        S::ArgumentValue(0),
        S::EnvironmentStorage,
    ]));
    let joined = values.merge(&storage, true);
    assert_eq!(
        joined,
        CallableResultBorrowing::Sources(Box::new(vec![
            S::ArgumentValue(0),
            S::EnvironmentValue,
            S::EnvironmentStorage
        ]))
    );
    assert!(joined.accepts(&values) && joined.accepts(&storage));
    assert_eq!(
        values.merge(&storage, false),
        CallableResultBorrowing::Sources(Box::new(vec![S::ArgumentValue(0)]))
    );
    assert_eq!(
        values.merge(&CallableResultBorrowing::Unknown, true),
        CallableResultBorrowing::Unknown
    );
    assert_eq!(
        values.merge(&CallableResultBorrowing::Unknown, false),
        values
    );
}

#[test]
fn callable_result_contract_survives_generic_substitution_and_identity_remapping() {
    let owner = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(3));
    let generic = GenericParamId { owner, index: 0 };
    let mut ty = Type::function(vec![Type::Generic(generic)], Type::Generic(generic));
    let expected = CallableResultBorrowing::Sources(Box::new(vec![
        CallableResultSource::ArgumentValue(0),
        CallableResultSource::EnvironmentValue,
    ]));
    let Type::Function {
        result_borrowing, ..
    } = &mut ty
    else {
        unreachable!()
    };
    *result_borrowing = expected.clone();
    let substituted = ty.substitute_generics(&HashMap::from([(generic, Type::I64)]));
    let mut context = crate::type_context::TypeContext::new();
    let id = context.intern_type(&substituted);
    assert_eq!(context.type_for(id), substituted);
    assert_eq!(
        crate::type_context::TypeView::new(&context).result_borrowing(id),
        Some(&expected)
    );
    ty.try_remap_def_ids(&mut |id: DefId| -> Result<DefId, String> {
        Ok(DefId::new(crate::ids::CrateId(id.crate_id.0 + 1), id.local))
    })
    .unwrap();
    let Type::Function {
        result_borrowing,
        params,
        ..
    } = ty
    else {
        unreachable!()
    };
    assert_eq!(result_borrowing, expected);
    assert_eq!(
        params,
        vec![Type::Generic(GenericParamId {
            owner: DefId::new(crate::ids::CrateId(1), owner.local),
            index: 0
        })]
    );
}
