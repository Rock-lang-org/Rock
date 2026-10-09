//! Canonical stored-callable demands, not measurements or layout inventions.

use std::collections::{HashMap, HashSet};

use super::{MirBackendContract, MirGenericCallableInput, MirNominalLayout};
use crate::ids::{TypeId, VariantId};
use crate::mir::{MirVariantLayoutFields, Projection};
use crate::type_context::TypeContext;
use crate::types::{CallableResultBorrowing, CallableResultSource, Type};

pub(crate) struct CallablePayloadRequirement {
    pub projection: Vec<Projection>,
    pub ty: TypeId,
    pub borrowing: CallableResultBorrowing,
}

pub(crate) fn callable_payload_requirements(
    tc: &TypeContext,
    contract: &MirBackendContract,
    input: &MirGenericCallableInput,
) -> Vec<CallablePayloadRequirement> {
    fn visit(
        tc: &TypeContext,
        contract: &MirBackendContract,
        actual: Type,
        required: Type,
        path: Vec<Projection>,
        seen: &mut HashSet<(Type, Type)>,
        out: &mut Vec<CallablePayloadRequirement>,
    ) {
        let actual = contract.normalize_storage_type(tc, &actual);
        let required = contract.normalize_storage_type(tc, &required);
        let required = if matches!(actual, Type::Function { .. }) {
            let Some(required) = crate::type_services::callable::physical_owned_input_requirement(
                &required, &actual,
            ) else {
                return;
            };
            required
        } else {
            required
        };
        if !seen.insert((actual.clone(), required.clone())) {
            return;
        }
        let measured_fields = match &actual {
            Type::StructRepresentation { fields, .. } => Some(fields.as_slice()),
            _ => None,
        };
        let measured_variants = match &actual {
            Type::EnumRepresentation { variants, .. } => Some(variants.as_slice()),
            _ => None,
        };
        match (&actual, &required) {
            (
                Type::Function {
                    result_borrowing: declared,
                    ..
                },
                Type::Function {
                    params,
                    ret,
                    result_borrowing: borrowing @ CallableResultBorrowing::Sources(sources),
                    ..
                },
            ) => {
                if !crate::type_services::callable::input_shape_matches(&required, &actual)
                    || !borrowing.sources_are_valid(params.len(), |index| {
                        matches!(
                            params[index as usize],
                            Type::Reference { .. } | Type::Pointer(_)
                        )
                    })
                    || sources.iter().any(|source| {
                        matches!(
                            source,
                            CallableResultSource::EnvironmentStorage
                                | CallableResultSource::EnvironmentBorrowedContents
                                | CallableResultSource::ArgumentBorrowedContents(_)
                        )
                    })
                {
                    return;
                }
                let Some(ty) = tc.id_for_type(&actual) else {
                    return;
                };
                let Some(result) = tc.id_for_type(ret) else {
                    return;
                };
                // Raw-memory results also carry dependencies, without becoming
                // safe-reference escape payloads. Scalar results stay opaque;
                // every admitted demand still needs a measured incoming producer.
                if crate::mir::borrowck::return_type_needs_escape_check(tc, contract, result)
                    || crate::mir::borrowck::type_id_tracks_origin(
                        crate::type_context::TypeView::new(tc),
                        contract,
                        result,
                    )
                {
                    out.push(CallablePayloadRequirement {
                        projection: path.clone(),
                        ty,
                        // Both are input obligations: every actual binding must
                        // independently satisfy the definition's protocol and
                        // the field's declared envelope. No producer is seeded
                        // without the enclosing canonical input requirement.
                        borrowing: borrowing.merge(declared, false),
                    });
                }
            }
            (Type::Tuple(actual), Type::Tuple(required)) if actual.len() == required.len() => {
                for (index, (actual, required)) in actual.iter().zip(required).enumerate() {
                    let mut field = path.clone();
                    field.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    visit(
                        tc,
                        contract,
                        actual.clone(),
                        required.clone(),
                        field,
                        seen,
                        out,
                    );
                }
            }
            (
                Type::Struct {
                    id: a,
                    args: actual,
                }
                | Type::StructRepresentation {
                    id: a,
                    args: actual,
                    ..
                },
                Type::Struct {
                    id: b,
                    args: required,
                },
            )
            | (
                Type::Enum {
                    id: a,
                    args: actual,
                }
                | Type::EnumRepresentation {
                    id: a,
                    args: actual,
                    ..
                },
                Type::Enum {
                    id: b,
                    args: required,
                },
            ) if a == b => {
                let Some(layout) = contract.nominal_layouts.get(a) else {
                    return;
                };
                let (generics, variants) = match layout {
                    MirNominalLayout::Struct {
                        generic_params,
                        fields,
                        ..
                    } => (
                        generic_params,
                        vec![(None, fields.iter().map(|(_, ty)| *ty).collect::<Vec<_>>())],
                    ),
                    MirNominalLayout::Enum {
                        generic_params,
                        variants,
                        ..
                    } => (
                        generic_params,
                        variants
                            .iter()
                            .enumerate()
                            .map(|(index, variant)| {
                                let fields = match &variant.fields {
                                    MirVariantLayoutFields::Unit => vec![],
                                    MirVariantLayoutFields::Positional(fields) => fields.clone(),
                                    MirVariantLayoutFields::Named(fields) => {
                                        fields.iter().map(|(_, ty)| *ty).collect()
                                    }
                                };
                                (Some(VariantId(index as u32)), fields)
                            })
                            .collect(),
                    ),
                };
                if actual.len() != generics.len() || required.len() != generics.len() {
                    return;
                }
                let actual = generics
                    .iter()
                    .copied()
                    .zip(actual.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let required = generics
                    .iter()
                    .copied()
                    .zip(required.iter().cloned())
                    .collect::<HashMap<_, _>>();
                for (variant, fields) in variants {
                    let measured_fields = if let Some(variants) = measured_variants {
                        let Some(fields) =
                            variant.and_then(|variant| variants.get(variant.0 as usize))
                        else {
                            return;
                        };
                        Some(fields.as_slice())
                    } else {
                        measured_fields
                    };
                    if measured_fields.is_some_and(|measured| measured.len() != fields.len()) {
                        return;
                    }
                    for (index, ty) in fields.into_iter().enumerate() {
                        let field_type = tc.type_for(ty);
                        let declared_actual = field_type.substitute_generics(&actual);
                        let actual_field = measured_fields
                            .map(|fields| fields[index].clone())
                            .unwrap_or_else(|| declared_actual.clone());
                        if crate::type_services::callable::value_representation(
                            &declared_actual,
                            &actual_field,
                        )
                        .as_ref()
                            != Some(&actual_field)
                        {
                            return;
                        }
                        let mut field = path.clone();
                        if let Some(variant) = variant {
                            field.push(Projection::Downcast(variant));
                        }
                        field.push(Projection::Field {
                            index,
                            identity: None,
                        });
                        visit(
                            tc,
                            contract,
                            actual_field,
                            field_type.substitute_generics(&required),
                            field,
                            seen,
                            out,
                        );
                    }
                }
            }
            _ => {}
        }
        seen.remove(&(actual, required));
    }
    if !tc.type_id_tree_is_valid(input.subject) || !tc.type_id_tree_is_valid(input.invocation) {
        return Vec::new();
    }
    let mut requirements = Vec::new();
    visit(
        tc,
        contract,
        tc.type_for(input.subject),
        tc.type_for(input.invocation),
        Vec::new(),
        &mut HashSet::new(),
        &mut requirements,
    );
    requirements
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_nominal_fields_do_not_discard_definition_owned_callable_demands() {
        let mut tc = TypeContext::default();
        let mut contract = MirBackendContract::default();
        let owner = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(901));
        let parameter = crate::types::GenericParamId { owner, index: 0 };
        let field = tc.intern_type(&Type::Generic(parameter));
        contract.nominal_layouts.insert(
            owner,
            MirNominalLayout::Struct {
                id: owner,
                fields: vec![("callback".into(), field)],
                generic_params: vec![parameter],
            },
        );
        let signature = Type::function(
            vec![],
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let mut stored = signature.clone();
        // Physical storage supplies no producer proof; only inspect whether
        // the definition's independent input obligation survives transport.
        let Type::Function { captures, .. } = &mut stored else {
            unreachable!()
        };
        *captures = vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::Move,
            Type::I64,
        )]
        .into_boxed_slice();
        let mut requirement = signature.clone();
        let Type::Function {
            result_borrowing, ..
        } = &mut requirement
        else {
            unreachable!()
        };
        *result_borrowing = crate::type_services::callable::owned_result_requirement(&[]).unwrap();
        let actual = Type::StructRepresentation {
            id: owner,
            args: vec![signature],
            fields: vec![stored.clone()],
        };
        let required = Type::Struct {
            id: owner,
            args: vec![requirement],
        };
        let input = MirGenericCallableInput {
            subject: tc.intern_type(&actual),
            invocation: tc.intern_type(&required),
        };
        let demands = callable_payload_requirements(&tc, &contract, &input);
        assert_eq!(
            demands.len(),
            1,
            "physical field evidence cannot remove a canonical input obligation"
        );
        assert_eq!(tc.type_for(demands[0].ty), stored);
        assert_eq!(
            demands[0].projection,
            vec![Projection::Field {
                index: 0,
                identity: None
            }]
        );
    }

    #[test]
    fn raw_result_dependency_demands_are_not_scalar_independence_or_escape_proof() {
        let mut tc = TypeContext::default();
        let mut contract = MirBackendContract::default();
        let owner = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(900));
        let pointer = tc.intern_type(&Type::Pointer(Box::new(Type::I64)));
        contract.nominal_layouts.insert(
            owner,
            MirNominalLayout::Struct {
                id: owner,
                fields: vec![("storage".into(), pointer)],
                generic_params: vec![],
            },
        );
        for (result, expected) in [
            (Type::I64, 0),
            (Type::Unit, 0),
            (Type::Pointer(Box::new(Type::I64)), 1),
            (
                Type::Tuple(vec![Type::Pointer(Box::new(Type::I64)), Type::I64]),
                1,
            ),
            (
                Type::Struct {
                    id: owner,
                    args: vec![],
                },
                1,
            ),
        ] {
            let actual = Type::function(vec![], result.clone());
            let mut required = actual.clone();
            let demand = crate::type_services::callable::owned_result_requirement(&[]).unwrap();
            let Type::Function {
                result_borrowing, ..
            } = &mut required
            else {
                unreachable!()
            };
            *result_borrowing = demand.clone();
            let input = MirGenericCallableInput {
                subject: tc.intern_type(&actual),
                invocation: tc.intern_type(&required),
            };
            let result = tc.intern_type(&result);
            assert!(!crate::mir::borrowck::return_type_needs_escape_check(
                &tc, &contract, result
            ));
            let fields = callable_payload_requirements(&tc, &contract, &input);
            assert_eq!(fields.len(), expected);
            if expected != 0 {
                assert_eq!(fields[0].borrowing, demand);
                assert!(!fields[0]
                    .borrowing
                    .accepts(&CallableResultBorrowing::Unknown));
                assert!(!fields[0]
                    .borrowing
                    .accepts(&CallableResultBorrowing::Sources(Box::new(vec![
                        CallableResultSource::EnvironmentStorage
                    ]),)));
            }
        }
    }

    #[test]
    fn measured_enum_fields_preserve_definition_owned_demands_at_variant_paths() {
        let mut tc = TypeContext::default();
        let mut contract = MirBackendContract::default();
        let owner = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(902));
        let parameter = crate::types::GenericParamId { owner, index: 0 };
        let field = tc.intern_type(&Type::Generic(parameter));
        contract.nominal_layouts.insert(
            owner,
            MirNominalLayout::Enum {
                id: owner,
                generic_params: vec![parameter],
                variants: vec![
                    crate::mir::MirEnumVariantLayout {
                        name: "Held".into(),
                        fields: MirVariantLayoutFields::Positional(vec![field]),
                    },
                    crate::mir::MirEnumVariantLayout {
                        name: "Empty".into(),
                        fields: MirVariantLayoutFields::Unit,
                    },
                ],
            },
        );
        let signature = Type::function(
            vec![],
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let mut stored = signature.clone();
        let Type::Function { captures, .. } = &mut stored else {
            unreachable!()
        };
        *captures = vec![crate::types::FunctionCapture::new(
            crate::types::CaptureKind::Move,
            Type::I64,
        )]
        .into_boxed_slice();
        let mut requirement = signature.clone();
        let Type::Function {
            result_borrowing, ..
        } = &mut requirement
        else {
            unreachable!()
        };
        *result_borrowing = crate::type_services::callable::owned_result_requirement(&[]).unwrap();
        let actual = Type::EnumRepresentation {
            id: owner,
            args: vec![signature],
            variants: vec![vec![stored.clone()], vec![]],
        };
        let required = Type::Enum {
            id: owner,
            args: vec![requirement],
        };
        let input = MirGenericCallableInput {
            subject: tc.intern_type(&actual),
            invocation: tc.intern_type(&required),
        };
        let demands = callable_payload_requirements(&tc, &contract, &input);
        assert_eq!(demands.len(), 1);
        assert_eq!(tc.type_for(demands[0].ty), stored);
        assert_eq!(
            demands[0].projection,
            vec![
                Projection::Downcast(VariantId(0)),
                Projection::Field {
                    index: 0,
                    identity: None
                }
            ]
        );
        assert_eq!(
            demands[0].borrowing,
            crate::type_services::callable::owned_result_requirement(&[]).unwrap()
        );
    }

    #[test]
    fn canonical_input_and_declared_field_envelopes_are_joint_obligations() {
        let mut tc = TypeContext::default();
        let mut actual = Type::function(
            vec![],
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::I64),
            },
        );
        let mut required = actual.clone();
        let demand = crate::type_services::callable::owned_result_requirement(&[]).unwrap();
        let Type::Function {
            result_borrowing, ..
        } = &mut required
        else {
            unreachable!()
        };
        *result_borrowing = demand.clone();
        let narrow = CallableResultBorrowing::Sources(Box::new(vec![
            CallableResultSource::EnvironmentValue,
        ]));
        let Type::Function {
            result_borrowing, ..
        } = &mut actual
        else {
            unreachable!()
        };
        *result_borrowing = narrow.clone();
        let input = MirGenericCallableInput {
            subject: tc.intern_type(&actual),
            invocation: tc.intern_type(&required),
        };
        let contract = MirBackendContract::default();
        let fields = callable_payload_requirements(&tc, &contract, &input);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].borrowing, narrow);
        assert!(
            !fields[0]
                .borrowing
                .accepts(&CallableResultBorrowing::Unknown),
            "a narrowed demand still requires actual producer measurement"
        );
        assert!(!fields[0]
            .borrowing
            .accepts(&CallableResultBorrowing::Sources(Box::new(vec![
                CallableResultSource::Unknown
            ]))));

        let Type::Function {
            result_borrowing, ..
        } = &mut actual
        else {
            unreachable!()
        };
        *result_borrowing = CallableResultBorrowing::Unknown;
        let input = MirGenericCallableInput {
            subject: tc.intern_type(&actual),
            invocation: input.invocation,
        };
        let fields = callable_payload_requirements(&tc, &contract, &input);
        assert_eq!(
            fields[0].borrowing, demand,
            "an opaque field cannot strengthen the canonical demand"
        );
    }
}
