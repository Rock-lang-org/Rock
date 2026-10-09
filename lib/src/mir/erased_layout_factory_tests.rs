//! Guard the real body-local factory boundary without fabricating evidence.
use super::*;
use crate::ids::{CrateId, LocalDefId};
use crate::mir::{MirBackendContract, MirLinkage};
use crate::type_context::TypeContext;

fn body_local_tuple() -> (MirErasedFunction, MirBackendContract, TypeContext) {
    let mut tc = TypeContext::new();
    let unit = tc.intern_type(&Type::Unit);
    let mut contract = MirBackendContract::default();
    let drop = crate::mir::payload_drop_plan(&contract, &mut tc, unit).unwrap();
    contract.erased.descriptors.insert(unit, MirTypeDescriptor { ty: unit, drop, fields: Vec::new() });
    let definition = DefId::new(CrateId(1), LocalDefId(1));
    let parameter = MirErasedType::Parameter(0);
    let tuple = MirErasedType::Tuple(vec![parameter.clone(), parameter.clone()]);
    let result = MirErasedType::Concrete(unit);
    let function = MirErasedFunction {
        key: MirErasedKey { definition, static_args: Vec::new() },
        symbol: "body_local_tuple".into(), linkage: MirLinkage::Internal,
        signature: MirErasedSignature {
            parameters: vec![MirErasedParameter { source: GenericParamId { owner: definition, index: 0 }, kind: Kind::Type }],
            params: vec![parameter.clone(), parameter.clone()], ret: result.clone(),
            dictionaries: Vec::new(), layouts: vec![parameter.clone()], require_borrow_free: BTreeSet::new(),
        },
        body: Some(MirErasedBody {
            locals: vec![parameter.clone(), parameter, tuple, result],
            mutable_locals: BTreeSet::new(), scopes: Vec::new(),
            blocks: vec![MirErasedBasicBlock {
                statements: vec![
                    MirErasedAssignment {
                        destination: MirErasedLocal(2),
                        value: MirErasedRvalue::Aggregate(vec![
                            MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0))),
                            MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(1))),
                        ]),
                    },
                    MirErasedAssignment {
                        destination: MirErasedLocal(3), value: MirErasedRvalue::Literal(MirErasedLiteral::Unit),
                    },
                ],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(3)))),
            }],
        }),
    };
    (function, contract, tc)
}

#[test]
fn composed_tuple_without_factory_or_supplied_layout_is_rejected() {
    let (function, contract, tc) = body_local_tuple();
    let tuple = &function.body.as_ref().unwrap().locals[2];
    assert!(!function.signature.layouts.contains(tuple));
    assert!(function.signature.params.iter().chain(std::iter::once(&function.signature.ret))
        .all(|ty| !matches!(ty, MirErasedType::Tuple(_))));
    assert_eq!(validate_erased_function(&function, &contract, &tc), Err(MirErasedError::MissingLayout));
}

#[test]
fn malformed_composed_child_cannot_replace_bound_factory_evidence() {
    let (mut function, contract, tc) = body_local_tuple();
    function.body.as_mut().unwrap().locals[2] = MirErasedType::Tuple(vec![
        MirErasedType::Parameter(0), MirErasedType::Parameter(99),
    ]);
    assert_eq!(validate_erased_function(&function, &contract, &tc), Err(MirErasedError::UnboundParameter));
}

#[test]
fn another_supplied_tuple_layout_is_not_evidence_for_the_body_local_tuple() {
    let (mut function, contract, tc) = body_local_tuple();
    function.signature.layouts.push(MirErasedType::Tuple(vec![
        MirErasedType::Parameter(0),
        MirErasedType::Reference { inner: Box::new(MirErasedType::Parameter(0)), mutable: false },
    ]));
    assert_eq!(validate_erased_function(&function, &contract, &tc), Err(MirErasedError::MissingLayout));
}
