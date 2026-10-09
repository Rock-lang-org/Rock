use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use inkwell::{context::Context, OptimizationLevel};

use crate::ids::{CrateId, DefId, InstanceId, LocalDefId};
use crate::mir::*;
use crate::type_context::TypeContext;
use crate::type_services::kind::Kind;
use crate::types::{GenericParamId, Type};

use super::object_tests::{insert_body, local, place};

#[cfg(test)]
mod package_validation_tests {
    use super::*;
    use crate::ids::HirLocalId;
    use crate::types::{ExistentialBinder, ExistentialType, WitnessId};

    fn erased_place(index: u32) -> MirErasedPlace {
        MirErasedPlace::local(MirErasedLocal(index))
    }

    fn assignment(index: u32, value: MirErasedRvalue) -> MirErasedAssignment {
        MirErasedAssignment {
            destination: MirErasedLocal(index),
            value,
        }
    }

    // Keep the existing production-shaped native bodies, schemas, and vtables;
    // add only a package type, its exact descriptors, and a scoped relay.
    fn package_program(
        mutable: bool,
        owner: bool,
    ) -> (MirProgram, MirErasedKey, MirPackagePackPlan) {
        let mut program = super::super::object_tests::object_program(mutable);
        let object = program.backend_contract.vtables[&MirVtableId(0)].object;
        let witness = program.backend_contract.vtables[&MirVtableId(0)].concrete;
        let Type::Object(bounds) = program.type_context.type_for(object) else {
            panic!("object fixture")
        };
        let hidden = Type::BoundVar {
            depth: 0,
            index: 0,
            kind: Kind::Type,
        };
        let payload = if owner {
            let definition = super::super::object_tests::definition(120);
            let parameter = GenericParamId {
                owner: definition,
                index: 0,
            };
            let field = program
                .type_context
                .intern_type(&Type::Pointer(Box::new(Type::Generic(parameter))));
            program.backend_contract.nominal_layouts.insert(
                definition,
                MirNominalLayout::Struct {
                    id: definition,
                    fields: vec![("pointer".into(), field)],
                    generic_params: vec![parameter],
                },
            );
            program
                .backend_contract
                .erased
                .constructor_kinds
                .insert(definition, Kind::arrow(Kind::Type, Kind::Type));
            Type::Struct {
                id: definition,
                args: vec![hidden],
            }
        } else {
            Type::Reference {
                inner: Box::new(hidden),
                mutable,
            }
        };
        let package = program
            .type_context
            .intern_canonical_type(&Type::Exists(Box::new(ExistentialType {
                binder: ExistentialBinder {
                    kind: Kind::Type,
                    maybe_unsized: false,
                },
                bounds: *bounds,
                payload,
            })));
        let schema = package_schema(
            &program.backend_contract,
            &mut program.type_context,
            package,
        )
        .unwrap();
        program.backend_contract.packages.insert(package, schema);
        let actual = package_payload_type(
            package,
            &program.type_context.type_for(witness),
            &program.type_context,
        )
        .unwrap();
        let actual = program.type_context.intern_type(&actual);
        if owner {
            let unit = program.type_context.intern_type(&Type::Unit);
            let pointer = program.type_context.intern_type(&Type::Pointer(Box::new(
                program.type_context.type_for(witness),
            )));
            insert_body(
                &mut program,
                MirFunction {
                    id: MirFunctionId::Instance(InstanceId(4)),
                    name: "package_owner_drop".into(),
                    local_decls: vec![
                        local(unit, LocalSource::ReturnPlace),
                        local(actual, LocalSource::Argument),
                    ],
                    arg_count: 1,
                    ret_type: unit,
                    closure_captures: vec![],
                    ownership: Default::default(),
                    basic_blocks: vec![BasicBlock {
                        statements: vec![StatementData::assign(
                            place(0),
                            Rvalue::Use(Operand::Constant(Constant::Unit)),
                            None,
                        )],
                        terminator: Some(Terminator::Return),
                    }],
                },
                "package_owner_drop",
                &[actual],
                true,
            );
            program
                .backend_contract
                .drop_glue
                .insert(actual, MirCallableKey::Instance(InstanceId(4)));
            for ty in [unit, pointer] {
                super::super::object_tests::insert_descriptor(&mut program, ty);
            }
        }
        for ty in [actual, package] {
            super::super::object_tests::insert_descriptor(&mut program, ty);
        }
        let key = MirErasedKey {
            definition: super::super::object_tests::definition(121),
            static_args: vec![],
        };
        let scope = MirErasedScope {
            witness: WitnessId {
                owner: key.definition,
                local: HirLocalId(0),
            },
            object,
            source: MirErasedScopeSource::Package { package },
        };
        let scoped =
            scoped_package_payload(package, 0, scope.witness, &mut program.type_context).unwrap();
        let packed = MirErasedType::Concrete(package);
        program.backend_contract.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol: "package_validator_relay".into(),
                linkage: MirLinkage::Internal,
                signature: MirErasedSignature {
                    parameters: vec![],
                    params: vec![packed.clone()],
                    ret: packed.clone(),
                    dictionaries: vec![],
                    layouts: vec![],
                    require_borrow_free: BTreeSet::new(),
                },
                body: Some(MirErasedBody {
                    locals: vec![packed.clone(), scoped, packed],
                    mutable_locals: BTreeSet::new(),
                    scopes: vec![scope],
                    blocks: vec![MirErasedBasicBlock {
                        statements: vec![
                            assignment(
                                1,
                                MirErasedRvalue::PackageOpen {
                                    scope: 0,
                                    source: MirErasedOperand::Move(erased_place(0)),
                                },
                            ),
                            assignment(
                                2,
                                MirErasedRvalue::PackagePack {
                                    payload: MirErasedOperand::Move(erased_place(1)),
                                    plan: MirPackagePackPlan {
                                        package,
                                        witness: MirPackageWitnessEvidence::Scoped {
                                            scope: 0,
                                            view: object,
                                        },
                                        payload_descriptor:
                                            MirPackagePayloadDescriptor::FromPackage(0),
                                    },
                                },
                            ),
                        ],
                        terminator: MirErasedTerminator::Return(MirErasedOperand::Move(
                            erased_place(2),
                        )),
                    }],
                }),
            },
        );
        let plan = MirPackagePackPlan {
            package,
            witness: MirPackageWitnessEvidence::Concrete {
                witness,
                table: MirVtableId(0),
            },
            payload_descriptor: MirPackagePayloadDescriptor::Concrete(actual),
        };
        (program, key, plan)
    }

    fn assert_valid(program: &MirProgram) {
        assert_eq!(validate_erased_contract(program), vec![]);
        assert_eq!(validate_object_contract(program), vec![]);
        assert_eq!(
            validate_backend_contract_against_mir(
                &program.backend_contract,
                program.functions.values(),
            ),
            vec![],
        );
    }

    fn body_mut<'a>(program: &'a mut MirProgram, key: &MirErasedKey) -> &'a mut MirErasedBody {
        program
            .backend_contract
            .erased
            .functions
            .get_mut(key)
            .unwrap()
            .body
            .as_mut()
            .unwrap()
    }

    fn repack_plan_mut<'a>(
        program: &'a mut MirProgram,
        key: &MirErasedKey,
    ) -> &'a mut MirPackagePackPlan {
        let MirErasedRvalue::PackagePack { plan, .. } = &mut body_mut(program, key).blocks[0]
            .statements
            .last_mut()
            .unwrap()
            .value
        else {
            panic!("package repack fixture")
        };
        plan
    }

    fn assert_function_error(program: &MirProgram, key: &MirErasedKey, error: MirErasedError) {
        assert_eq!(
            validate_erased_function(
                &program.backend_contract.erased.functions[key],
                &program.backend_contract,
                &program.type_context,
            ),
            Err(error.clone()),
        );
        assert_eq!(
            validate_erased_contract(program),
            vec![(Some(key.clone()), error)],
        );
    }

    #[test]
    fn package_closed_binder_rejects_corrupt_index_kind_and_depth() {
        let (program, _, plan) = package_program(false, false);
        assert_valid(&program);
        assert!(package_schema_is_valid(
            &program.backend_contract,
            &program.type_context,
            plan.package,
        ));
        for (depth, index, kind) in [
            (0, 1, Kind::Type),
            (0, 0, Kind::arrow(Kind::Type, Kind::Type)),
            (1, 0, Kind::Type),
        ] {
            let mut invalid = program.clone();
            let Type::Exists(mut package) = invalid.type_context.type_for(plan.package) else {
                panic!("package fixture")
            };
            package.payload = Type::Reference {
                inner: Box::new(Type::BoundVar { depth, index, kind }),
                mutable: false,
            };
            // Simulate a corrupt backend/artifact record without asking frontend
            // normalization to reject it before the backend authority is tested.
            let id = invalid
                .type_context
                .intern_canonical_type(&Type::Exists(package));
            assert!(invalid.type_context.type_id_tree_is_valid(id));
            assert_eq!(
                package_schema(&invalid.backend_contract, &mut invalid.type_context, id),
                Err("package has an invalid lexical binder or free type".into()),
            );
            let mut schema = invalid.backend_contract.packages[&plan.package].clone();
            schema.package = id;
            invalid.backend_contract.packages.insert(id, schema);
            assert_eq!(
                validate_erased_contract(&invalid),
                vec![(None, MirErasedError::InvalidType)],
            );
        }
    }

    #[test]
    fn package_schema_rejects_forged_dispatch_identity_and_uniform_layout() {
        let (program, _, plan) = package_program(false, false);
        assert_valid(&program);
        for mutation in 0..3 {
            let mut invalid = program.clone();
            let other = invalid.backend_contract.vtables[&MirVtableId(1)].object;
            let schema = invalid
                .backend_contract
                .packages
                .get_mut(&plan.package)
                .unwrap();
            match mutation {
                0 => schema.object = other,
                1 => schema.payload_layout = MirUniformLayout::Tuple(vec![]),
                2 => schema.package = other,
                _ => unreachable!(),
            }
            assert!(!package_schema_is_valid(
                &invalid.backend_contract,
                &invalid.type_context,
                plan.package,
            ));
            // Its retained descriptor also loses runtime-layout authority once
            // the schema is corrupt; keep that deliberate cascading error exact.
            assert_eq!(
                validate_erased_contract(&invalid),
                vec![
                    (None, MirErasedError::InvalidType),
                    (None, MirErasedError::MissingLayout),
                ],
            );
        }
    }

    #[test]
    fn package_concrete_pack_requires_exact_witness_table_and_payload_descriptor() {
        let (mut program, _, plan) = package_program(false, false);
        let MirPackagePayloadDescriptor::Concrete(actual) = plan.payload_descriptor else {
            panic!("concrete descriptor fixture")
        };
        let wrong_descriptor = program.type_context.intern_type(&Type::Reference {
            inner: Box::new(Type::I64),
            mutable: true,
        });
        super::super::object_tests::insert_descriptor(&mut program, wrong_descriptor);
        let wrong_witness = program.type_context.intern_type(&Type::Bool);
        super::super::object_tests::insert_descriptor(&mut program, wrong_witness);
        insert_body(
            &mut program,
            MirFunction {
                id: MirFunctionId::Instance(InstanceId(5)),
                name: "package_validator_pack".into(),
                local_decls: vec![
                    local(plan.package, LocalSource::ReturnPlace),
                    local(actual, LocalSource::Argument),
                ],
                arg_count: 1,
                ret_type: plan.package,
                closure_captures: vec![],
                ownership: Default::default(),
                basic_blocks: vec![BasicBlock {
                    statements: vec![StatementData::assign(
                        place(0),
                        Rvalue::Package(Operand::Move(place(1)), plan.clone()),
                        None,
                    )],
                    terminator: Some(Terminator::Return),
                }],
            },
            "package_validator_pack",
            &[actual],
            false,
        );
        assert_valid(&program);
        assert!(package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            &plan,
            &[],
            &[],
        ));
        for mutation in 0..3 {
            let mut forged = plan.clone();
            match mutation {
                0 => {
                    forged.witness = MirPackageWitnessEvidence::Concrete {
                        witness: wrong_witness,
                        table: MirVtableId(0),
                    };
                }
                1 => {
                    let MirPackageWitnessEvidence::Concrete { table, .. } = &mut forged.witness
                    else {
                        unreachable!()
                    };
                    *table = MirVtableId(1);
                }
                2 => {
                    forged.payload_descriptor =
                        MirPackagePayloadDescriptor::Concrete(wrong_descriptor);
                }
                _ => unreachable!(),
            }
            assert!(!package_plan_is_valid(
                &program.backend_contract,
                &program.type_context,
                &forged,
                &[],
                &[],
            ));
            let mut invalid = program.clone();
            invalid
                .functions
                .get_mut(&MirFunctionId::Instance(InstanceId(5)))
                .unwrap()
                .basic_blocks[0]
                .statements[0] = StatementData::assign(
                place(0),
                Rvalue::Package(Operand::Move(place(1)), forged),
                None,
            );
            assert_eq!(
                validate_erased_contract(&invalid),
                vec![(None, MirErasedError::InvalidType)],
            );
        }
    }

    #[test]
    fn package_owner_cannot_replace_its_actual_drop_descriptor_with_uniform_drop_free() {
        let (mut shared, key, _) = package_program(false, false);
        repack_plan_mut(&mut shared, &key).payload_descriptor =
            MirPackagePayloadDescriptor::UniformDropFree;
        assert_valid(&shared);
        let (program, key, plan) = package_program(false, true);
        assert_valid(&program);
        let MirPackagePayloadDescriptor::Concrete(actual) = plan.payload_descriptor else {
            unreachable!()
        };
        assert!(program.backend_contract.erased.descriptors[&actual]
            .drop
            .has_drop());
        assert!(matches!(
            program.backend_contract.packages[&plan.package].payload_layout,
            MirUniformLayout::Struct { .. },
        ));
        assert!(!package_payload_drop_free(
            &program.backend_contract,
            &program.type_context,
            plan.package,
        ));
        let mut invalid = program.clone();
        repack_plan_mut(&mut invalid, &key).payload_descriptor =
            MirPackagePayloadDescriptor::UniformDropFree;
        assert_function_error(&invalid, &key, MirErasedError::InvalidType);
        let mut concrete = plan;
        assert!(package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            &concrete,
            &[],
            &[],
        ));
        concrete.payload_descriptor = MirPackagePayloadDescriptor::UniformDropFree;
        assert!(!package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            &concrete,
            &[],
            &[],
        ));
    }

    #[test]
    fn package_descriptor_provenance_does_not_follow_a_projected_or_reconstructed_payload() {
        for reconstruct in [false, true] {
            let (mut program, key, _) = package_program(false, false);
            assert_valid(&program);
            let body = body_mut(&mut program, &key);
            let payload = body.locals[1].clone();
            body.locals
                .push(MirErasedType::Tuple(vec![payload.clone()]));
            body.blocks[0].statements.insert(
                1,
                assignment(
                    3,
                    MirErasedRvalue::Aggregate(vec![MirErasedOperand::Move(erased_place(1))]),
                ),
            );
            let projected = MirErasedPlace {
                local: MirErasedLocal(3),
                projection: vec![MirErasedProjection::Field(0)],
            };
            let source = if reconstruct {
                body.locals.push(payload);
                body.blocks[0].statements.insert(
                    2,
                    assignment(4, MirErasedRvalue::Use(MirErasedOperand::Copy(projected))),
                );
                MirErasedOperand::Move(erased_place(4))
            } else {
                MirErasedOperand::Copy(projected)
            };
            let MirErasedRvalue::PackagePack { payload, plan } =
                &mut body.blocks[0].statements.last_mut().unwrap().value
            else {
                unreachable!()
            };
            *payload = source;
            plan.payload_descriptor = MirPackagePayloadDescriptor::UniformDropFree;
            // Same projection/type/initialization is legal with independently
            // proved drop-free metadata, isolating the FromPackage restriction.
            assert_valid(&program);
            repack_plan_mut(&mut program, &key).payload_descriptor =
                MirPackagePayloadDescriptor::FromPackage(0);
            assert_function_error(&program, &key, MirErasedError::InvalidType);
        }
    }

    #[test]
    fn package_descriptor_requires_the_matching_opened_package_scope() {
        let (program, key, plan) = package_program(false, false);
        assert_valid(&program);
        let function = &program.backend_contract.erased.functions[&key];
        let body = function.body.as_ref().unwrap();
        let mut scopes = body.scopes.clone();
        let mut second = scopes[0].clone();
        second.witness.local = HirLocalId(1);
        scopes.push(second);
        let MirErasedRvalue::PackagePack { plan: scoped, .. } = &body.blocks[0].statements[1].value
        else {
            unreachable!()
        };
        assert!(package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            scoped,
            &scopes,
            &[true, true]
        ));
        let mut second_plan = scoped.clone();
        second_plan.witness = MirPackageWitnessEvidence::Scoped {
            scope: 1,
            view: program.backend_contract.packages[&plan.package].object,
        };
        second_plan.payload_descriptor = MirPackagePayloadDescriptor::FromPackage(1);
        assert!(package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            &second_plan,
            &scopes,
            &[true, true]
        ));
        let mut wrong = scoped.clone();
        wrong.payload_descriptor = MirPackagePayloadDescriptor::FromPackage(1);
        assert!(!package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            &wrong,
            &scopes,
            &[true, true]
        ));
        assert!(!package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            scoped,
            &scopes,
            &[false, true]
        ));
        scopes[0].source = MirErasedScopeSource::Object { mutable: false };
        assert!(!package_plan_is_valid(
            &program.backend_contract,
            &program.type_context,
            scoped,
            &scopes,
            &[true, true]
        ));

        // A second valid package may share dispatch bounds without sharing its
        // actual payload descriptor: a one-field tuple is not a bare reference.
        let mut other_payload = program.clone();
        let Type::Exists(mut other) = other_payload.type_context.type_for(plan.package) else {
            unreachable!()
        };
        other.payload = Type::Tuple(vec![other.payload.clone()]);
        let other = other_payload
            .type_context
            .intern_canonical_type(&Type::Exists(other));
        let schema = package_schema(
            &other_payload.backend_contract,
            &mut other_payload.type_context,
            other,
        )
        .unwrap();
        other_payload
            .backend_contract
            .packages
            .insert(other, schema);
        super::super::object_tests::insert_descriptor(&mut other_payload, other);
        assert_valid(&other_payload);
        scopes[0].source = MirErasedScopeSource::Package { package: other };
        assert!(!package_plan_is_valid(
            &other_payload.backend_contract,
            &other_payload.type_context,
            scoped,
            &scopes,
            &[true, true]
        ));

        let mut invalid = program.clone();
        repack_plan_mut(&mut invalid, &key).payload_descriptor =
            MirPackagePayloadDescriptor::FromPackage(1);
        assert_function_error(&invalid, &key, MirErasedError::InvalidType);
    }

    #[test]
    fn package_open_consumes_the_source_and_repack_preserves_move_initialization() {
        for owner in [false, true] {
            let (mut program, key, _) = package_program(false, owner);
            let body = body_mut(&mut program, &key);
            body.locals.push(body.locals[1].clone());
            body.blocks[0].statements.insert(
                1,
                assignment(
                    3,
                    MirErasedRvalue::Use(MirErasedOperand::Move(erased_place(1))),
                ),
            );
            let MirErasedRvalue::PackagePack { payload, .. } =
                &mut body.blocks[0].statements[2].value
            else {
                unreachable!()
            };
            *payload = MirErasedOperand::Move(erased_place(3));
            assert_valid(&program);
            for source in [0, 1, 3] {
                let mut invalid = program.clone();
                let body = body_mut(&mut invalid, &key);
                body.locals.push(body.locals[source as usize].clone());
                body.blocks[0].statements.push(assignment(
                    4,
                    MirErasedRvalue::Use(MirErasedOperand::Move(erased_place(source))),
                ));
                assert_function_error(&invalid, &key, MirErasedError::UninitializedOrMoved);
            }
        }
    }

    #[test]
    fn package_unique_mutable_payload_and_package_cannot_be_copied() {
        let (program, key, _) = package_program(true, false);
        assert_valid(&program);
        for package_copy in [false, true] {
            let mut invalid = program.clone();
            let body = body_mut(&mut invalid, &key);
            if package_copy {
                body.blocks[0].terminator =
                    MirErasedTerminator::Return(MirErasedOperand::Copy(erased_place(2)));
            } else {
                let MirErasedRvalue::PackagePack { payload, .. } =
                    &mut body.blocks[0].statements[1].value
                else {
                    unreachable!()
                };
                *payload = MirErasedOperand::Copy(erased_place(1));
            }
            assert_function_error(&invalid, &key, MirErasedError::CopyWithoutProof);
        }
    }

    fn native_apply_program() -> (MirProgram, MirErasedKey, MirCallableKey) {
        let (mut program, key, _) = package_program(false, false);
        let target = MirCallableKey::Instance(InstanceId(1));
        let native = program.backend_contract.callables[&target]
            .signature
            .clone();
        let function = program
            .backend_contract
            .erased
            .functions
            .get_mut(&key)
            .unwrap();
        let reference = MirErasedType::Concrete(native.params[0].semantic_ty);
        let result = MirErasedType::Concrete(native.ret.semantic_ty);
        function.signature.params.push(reference.clone());
        function.signature.ret = result.clone();
        let body = function.body.as_mut().unwrap();
        let package = body.locals[0].clone();
        let payload = body.locals[1].clone();
        body.locals = vec![package, reference, payload, result];
        body.blocks[0].statements = vec![
            assignment(
                2,
                MirErasedRvalue::PackageOpen {
                    scope: 0,
                    source: MirErasedOperand::Move(erased_place(0)),
                },
            ),
            assignment(
                3,
                MirErasedRvalue::NativeApply {
                    target: target.clone(),
                    args: vec![MirErasedOperand::Copy(erased_place(1))],
                },
            ),
        ];
        body.blocks[0].terminator =
            MirErasedTerminator::Return(MirErasedOperand::Move(erased_place(3)));
        (program, key, target)
    }

    #[test]
    fn package_native_apply_requires_concrete_arguments_exact_body_and_native_abi() {
        let (program, key, target) = native_apply_program();
        assert_valid(&program);
        for mutation in 0..7 {
            let mut invalid = program.clone();
            match mutation {
                0 => {
                    let MirErasedRvalue::NativeApply { args, .. } =
                        &mut body_mut(&mut invalid, &key).blocks[0].statements[1].value
                    else {
                        unreachable!()
                    };
                    args[0] = MirErasedOperand::Copy(erased_place(2));
                    assert_function_error(&invalid, &key, MirErasedError::InvalidType);
                }
                1 => {
                    invalid
                        .functions
                        .remove(&MirFunctionId::Instance(InstanceId(1)));
                    assert_eq!(
                        validate_erased_contract(&invalid),
                        vec![(Some(key.clone()), MirErasedError::MissingCallable)]
                    );
                }
                2 => {
                    invalid
                        .backend_contract
                        .function_bodies
                        .remove(&MirFunctionId::Instance(InstanceId(1)));
                    assert_eq!(
                        validate_erased_contract(&invalid),
                        vec![(Some(key.clone()), MirErasedError::MissingCallable)]
                    );
                }
                3 => {
                    invalid
                        .backend_contract
                        .callables
                        .get_mut(&target)
                        .unwrap()
                        .signature
                        .params[0]
                        .pass_mode = MirPassMode::FatDirect;
                    assert_eq!(
                        validate_erased_contract(&invalid),
                        vec![(Some(key.clone()), MirErasedError::MissingCallable)]
                    );
                }
                4 => {
                    let boolean = invalid.type_context.intern_type(&Type::Bool);
                    invalid
                        .functions
                        .get_mut(&MirFunctionId::Instance(InstanceId(1)))
                        .unwrap()
                        .local_decls[1]
                        .ty = boolean;
                    assert_eq!(
                        validate_erased_contract(&invalid),
                        vec![(Some(key.clone()), MirErasedError::MissingCallable)]
                    );
                }
                5 => {
                    // NativeApply must not weaken NativeCall's producer bridge:
                    // this unrelated scoped source has no bridge authority.
                    let MirErasedRvalue::NativeApply { target, args } =
                        body_mut(&mut invalid, &key).blocks[0].statements[1]
                            .value
                            .clone()
                    else {
                        unreachable!()
                    };
                    body_mut(&mut invalid, &key).blocks[0].statements[1].value =
                        MirErasedRvalue::NativeCall { target, args };
                    assert_function_error(&invalid, &key, MirErasedError::InvalidType);
                }
                6 => {
                    let boolean = invalid.type_context.intern_type(&Type::Bool);
                    invalid
                        .backend_contract
                        .callables
                        .get_mut(&target)
                        .unwrap()
                        .signature
                        .ret
                        .abi_ty = boolean;
                    assert_eq!(
                        validate_erased_function(
                            &invalid.backend_contract.erased.functions[&key],
                            &invalid.backend_contract,
                            &invalid.type_context,
                        ),
                        Err(MirErasedError::InvalidType)
                    );
                    assert_eq!(
                        validate_erased_contract(&invalid),
                        vec![
                            (Some(key.clone()), MirErasedError::InvalidType),
                            (Some(key.clone()), MirErasedError::MissingCallable),
                        ]
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn package_native_apply_requires_an_exact_imported_native_link_proof() {
        let (mut program, key, target) = native_apply_program();
        let declaration = program.backend_contract.callables.get_mut(&target).unwrap();
        declaration.kind = MirCallableKind::ObjectProvided;
        program
            .backend_contract
            .artifact_exports
            .push(MirArtifactExport {
                origin_def_id: declaration.source_def_id,
                source_name: "producer::read_or_increment".into(),
                backend_symbol: declaration.llvm_symbol.clone(),
                substitution_empty: true,
                has_body: false,
                provided_by_object: true,
                is_specialization: false,
                is_drop_glue: false,
            });
        program
            .functions
            .remove(&MirFunctionId::Instance(InstanceId(1)));
        program
            .backend_contract
            .function_bodies
            .remove(&MirFunctionId::Instance(InstanceId(1)));
        assert_valid(&program);
        for mutation in 0..9 {
            let mut invalid = program.clone();
            match mutation {
                0 => invalid.backend_contract.artifact_exports.clear(),
                1 => {
                    invalid.backend_contract.artifact_exports[0].origin_def_id =
                        Some(key.definition)
                }
                2 => {
                    invalid.backend_contract.artifact_exports[0].backend_symbol = "unrelated".into()
                }
                3 => invalid.backend_contract.artifact_exports[0].substitution_empty = false,
                4 => invalid.backend_contract.artifact_exports[0].has_body = true,
                5 => invalid.backend_contract.artifact_exports[0].provided_by_object = false,
                6 => invalid.backend_contract.artifact_exports[0].is_specialization = true,
                7 => invalid.backend_contract.artifact_exports[0].is_drop_glue = true,
                8 => invalid
                    .backend_contract
                    .artifact_exports
                    .push(invalid.backend_contract.artifact_exports[0].clone()),
                _ => unreachable!(),
            }
            assert_eq!(
                validate_erased_function(
                    &invalid.backend_contract.erased.functions[&key],
                    &invalid.backend_contract,
                    &invalid.type_context,
                ),
                Err(MirErasedError::MissingCallable)
            );
            // Body and whole-contract link preflights each report the same
            // missing authority; neither may silently authorize this call.
            assert_eq!(
                validate_erased_contract(&invalid),
                vec![
                    (Some(key.clone()), MirErasedError::MissingCallable),
                    (Some(key.clone()), MirErasedError::MissingCallable),
                ]
            );
        }
    }

    #[test]
    fn package_native_link_tightening_also_rejects_drop_rows_in_existing_native_bridges() {
        let (mut program, _, producer) = scoped_erased_sum_program();
        let target = MirCallableKey::Instance(InstanceId(3));
        let declaration = program.backend_contract.callables.get_mut(&target).unwrap();
        declaration.kind = MirCallableKind::ObjectProvided;
        program
            .backend_contract
            .artifact_exports
            .push(MirArtifactExport {
                origin_def_id: declaration.source_def_id,
                source_name: "producer::sum".into(),
                backend_symbol: declaration.llvm_symbol.clone(),
                substitution_empty: true,
                has_body: false,
                provided_by_object: true,
                is_specialization: false,
                is_drop_glue: false,
            });
        program
            .functions
            .remove(&MirFunctionId::Instance(InstanceId(3)));
        program
            .backend_contract
            .function_bodies
            .remove(&MirFunctionId::Instance(InstanceId(3)));
        assert_valid(&program);
        program.backend_contract.artifact_exports[0].is_drop_glue = true;
        assert_eq!(
            validate_erased_function(
                &program.backend_contract.erased.functions[&producer],
                &program.backend_contract,
                &program.type_context,
            ),
            Err(MirErasedError::MissingCallable),
        );
        assert_eq!(
            validate_erased_contract(&program),
            vec![
                (Some(producer.clone()), MirErasedError::MissingCallable),
                (Some(producer), MirErasedError::MissingCallable),
            ],
        );
    }
    #[test]
    fn package_backend_interning_preserves_closed_compounds_without_admitting_free_binders() {
        let (mut program, _, plan) = package_program(false, false);
        assert_valid(&program);
        let package = program.type_context.type_for(plan.package);
        for ty in [
            package.clone(),
            Type::Reference {
                inner: Box::new(package.clone()),
                mutable: false,
            },
            Type::Tuple(vec![package.clone(), Type::I64]),
            Type::Function {
                params: vec![package.clone()],
                result_borrowing: Default::default(),
                ret: Box::new(package),
                captures: Box::default(),
                callable_kind: crate::types::CallableKind::Fn,
                safety: crate::types::FunctionSafety::Safe,
            },
        ] {
            let id = intern_backend_type(&mut program.type_context, &ty);
            assert_eq!(program.type_context.type_for(id), ty);
            assert_eq!(intern_backend_type(&mut program.type_context, &ty), id);
        }
        let free = intern_backend_type(
            &mut program.type_context,
            &Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            },
        );
        assert!(!object_runtime_type_is_valid(
            &program.backend_contract,
            &program.type_context,
            free
        ));
        assert_valid(&program);
    }

    #[test]
    fn package_projected_reference_pointee_retains_parameter_region_not_field_storage() {
        let (mut program, key, plan) = package_program(false, false);
        let MirPackagePayloadDescriptor::Concrete(reference) = plan.payload_descriptor else {
            panic!("reference fixture")
        };
        let reference_type = program.type_context.type_for(reference);
        let tuple = intern_backend_type(
            &mut program.type_context,
            &Type::Tuple(vec![reference_type.clone()]),
        );
        let address = intern_backend_type(
            &mut program.type_context,
            &Type::Reference {
                inner: Box::new(reference_type),
                mutable: false,
            },
        );
        for ty in [tuple, address] {
            super::super::object_tests::insert_descriptor(&mut program, ty);
        }
        let function = program
            .backend_contract
            .erased
            .functions
            .get_mut(&key)
            .unwrap();
        function.signature.params = vec![MirErasedType::Concrete(tuple)];
        function.signature.ret = MirErasedType::Concrete(reference);
        function.body = Some(MirErasedBody {
            locals: vec![
                MirErasedType::Concrete(tuple),
                MirErasedType::Concrete(reference),
            ],
            mutable_locals: BTreeSet::new(),
            scopes: vec![],
            blocks: vec![MirErasedBasicBlock {
                statements: vec![assignment(
                    1,
                    MirErasedRvalue::Borrow {
                        place: MirErasedPlace {
                            local: MirErasedLocal(0),
                            projection: vec![
                                MirErasedProjection::Field(0),
                                MirErasedProjection::Deref,
                            ],
                        },
                        mutable: false,
                    },
                )],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(erased_place(1))),
            }],
        });
        assert_valid(&program);
        let function = program
            .backend_contract
            .erased
            .functions
            .get_mut(&key)
            .unwrap();
        function.signature.ret = MirErasedType::Concrete(address);
        let body = function.body.as_mut().unwrap();
        body.locals[1] = MirErasedType::Concrete(address);
        let MirErasedRvalue::Borrow { place, .. } = &mut body.blocks[0].statements[0].value else {
            unreachable!()
        };
        place.projection.pop();
        assert_function_error(&program, &key, MirErasedError::BorrowEscape);
    }

    #[test]
    fn package_opened_erased_shared_call_cannot_read_a_uniquely_borrowed_receiver_cell() {
        for mutable in [false, true] {
            let (mut program, key, _) = scoped_erased_sum_program();
            assert_valid(&program);
            let body = body_mut(&mut program, &key);
            // A mutable reference binding does not make its shared pointee
            // mutable; the new loan here protects only that binding's cell.
            body.mutable_locals.insert(MirErasedLocal(1));
            body.locals.push(MirErasedType::Reference {
                inner: Box::new(body.locals[1].clone()),
                mutable,
            });
            // Supply the existing shared reference directly instead of adding
            // an unrelated reborrow that would mask the call's alias check.
            body.blocks[0].statements.remove(1);
            let MirErasedRvalue::OpenedErasedCall { args, .. } =
                &mut body.blocks[0].statements[1].value
            else {
                panic!("shared erased call fixture")
            };
            args[0] = MirErasedOperand::Copy(erased_place(1));
            body.blocks[0].statements.insert(
                1,
                assignment(
                    4,
                    MirErasedRvalue::Borrow {
                        place: erased_place(1),
                        mutable,
                    },
                ),
            );
            if mutable {
                assert_function_error(&program, &key, MirErasedError::BorrowConflict);
            } else {
                assert_valid(&program);
            }
        }
    }
}

fn erased_identity() -> MirErasedFunction {
    let definition = DefId::new(CrateId(1), LocalDefId(7));
    let ty = MirErasedType::Parameter(0);
    MirErasedFunction {
        key: MirErasedKey {
            definition,
            static_args: vec![],
        },
        symbol: "aot.identity".into(),
        linkage: MirLinkage::External,
        signature: MirErasedSignature {
            parameters: vec![MirErasedParameter {
                source: GenericParamId {
                    owner: definition,
                    index: 0,
                },
                kind: Kind::Type,
            }],
            params: vec![ty.clone()],
            ret: ty.clone(),
            dictionaries: vec![],
            layouts: vec![ty.clone()],
            require_borrow_free: BTreeSet::new(),
        },
        body: Some(MirErasedBody {
            locals: vec![ty.clone(), ty],
            mutable_locals: BTreeSet::new(),
            scopes: vec![],
            blocks: vec![MirErasedBasicBlock {
                statements: vec![MirErasedAssignment {
                    destination: MirErasedLocal(1),
                    value: MirErasedRvalue::Use(MirErasedOperand::Move(MirErasedPlace::local(
                        MirErasedLocal(0),
                    ))),
                }],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Move(
                    MirErasedPlace::local(MirErasedLocal(1)),
                )),
            }],
        }),
    }
}

fn identity_producer() -> MirProgram {
    let function = erased_identity();
    let mut contract = MirBackendContract::default();
    contract
        .erased
        .functions
        .insert(function.key.clone(), function);
    MirProgram {
        functions: BTreeMap::new(),
        type_context: TypeContext::new(),
        backend_contract: contract,
    }
}

#[test]
fn aot_identity_executes_with_later_noncopy_type_and_descriptor_drop() {
    static DROPS: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn record(value: i64) {
        DROPS.fetch_add(value as u64, Ordering::SeqCst);
    }
    let context = Context::create();
    let producer = identity_producer();
    let mut producer_codegen = super::CodeGen::new(&context, "producer");
    producer_codegen
        .compile_program_from_mir(&producer)
        .unwrap();
    producer_codegen.module.verify().unwrap();
    let before = producer_codegen.get_ir();
    assert!(before.contains("erased.storage.aligned"));
    assert!(before.contains("llvm.memmove"));
    let mut tc = TypeContext::new();
    let scalar = tc.intern_type(&Type::I64);
    let unit = tc.intern_type(&Type::Unit);
    let late_id = DefId::new(CrateId(9), LocalDefId(1));
    let late_ty = Type::Struct {
        id: late_id,
        args: vec![],
    };
    assert!(!late_ty.is_copy());
    let late = tc.intern_type(&late_ty);
    let mut contract = MirBackendContract::default();
    contract.nominal_layouts.insert(
        late_id,
        MirNominalLayout::Struct {
            id: late_id,
            fields: vec![
                ("a".into(), scalar),
                ("b".into(), scalar),
                ("c".into(), scalar),
            ],
            generic_params: vec![],
        },
    );
    let record_key = MirCallableKey::Extern(DefId::new(CrateId(9), LocalDefId(2)));
    contract.callables.insert(
        record_key.clone(),
        MirCallableDecl {
            key: record_key.clone(),
            source_def_id: None,
            kind: MirCallableKind::Extern {
                link_name: "record_late_drop".into(),
                variadic: false,
            },
            llvm_symbol: "record_late_drop".into(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[scalar], unit, MirPassMode::Direct),
        },
    );
    let mut caller = MirProgram {
        functions: BTreeMap::new(),
        type_context: tc,
        backend_contract: contract,
    };
    let field = |index| Place {
        local: Local(1),
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    };
    insert_body(
        &mut caller,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(1)),
            name: "drop_late".into(),
            local_decls: vec![
                local(unit, LocalSource::ReturnPlace),
                local(late, LocalSource::Argument),
            ],
            arg_count: 1,
            ret_type: unit,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                            record_key,
                        ))),
                        args: vec![Operand::Copy(field(0))],
                        destination: place(0),
                        target: BasicBlockId(1),
                        span: None,
                    }),
                },
                BasicBlock {
                    statements: vec![],
                    terminator: Some(Terminator::Return),
                },
            ],
        },
        "drop_late",
        &[late],
        true,
    );
    caller
        .backend_contract
        .drop_glue
        .insert(late, MirCallableKey::Instance(InstanceId(1)));
    let drop = payload_drop_plan(&caller.backend_contract, &mut caller.type_context, late).unwrap();
    caller.backend_contract.erased.descriptors.insert(
        late,
        MirTypeDescriptor {
            ty: late,
            drop,
            fields: vec![scalar; 3],
        },
    );
    let mut declaration = erased_identity();
    declaration.body = None;
    let target = declaration.key.clone();
    caller
        .backend_contract
        .erased
        .functions
        .insert(target.clone(), declaration);
    let out_field = |index| Place {
        local: Local(2),
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    };
    insert_body(
        &mut caller,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(0)),
            name: "run_late_identity".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(late, LocalSource::UserBinding),
                local(late, LocalSource::UserBinding),
                local(scalar, LocalSource::Temporary),
            ],
            arg_count: 0,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![
                BasicBlock {
                    statements: vec![StatementData::assign(
                        place(1),
                        Rvalue::Aggregate(
                            AggregateKind::Struct {
                                id: late_id,
                                display_name: "Late".into(),
                            },
                            vec![
                                Operand::Constant(Constant::Int(40)),
                                Operand::Constant(Constant::Int(1)),
                                Operand::Constant(Constant::Int(1)),
                            ],
                        ),
                        None,
                    )],
                    terminator: Some(Terminator::Call {
                        func: Operand::Constant(Constant::ErasedCall(MirErasedCall {
                            target: MirErasedCallTarget::Direct(target),
                            type_args: vec![late],
                            dictionaries: vec![],
                        })),
                        args: vec![Operand::Move(place(1))],
                        destination: place(2),
                        target: BasicBlockId(1),
                        span: None,
                    }),
                },
                BasicBlock {
                    statements: vec![
                        StatementData::assign(
                            place(3),
                            Rvalue::BinaryOp(
                                MirBinOp::Add,
                                Operand::Copy(out_field(0)),
                                Operand::Copy(out_field(1)),
                            ),
                            None,
                        ),
                        StatementData::assign(
                            place(0),
                            Rvalue::BinaryOp(
                                MirBinOp::Add,
                                Operand::Copy(place(3)),
                                Operand::Copy(out_field(2)),
                            ),
                            None,
                        ),
                    ],
                    terminator: Some(Terminator::Drop {
                        place: place(2),
                        target: BasicBlockId(2),
                    }),
                },
                BasicBlock {
                    statements: vec![],
                    terminator: Some(Terminator::Return),
                },
            ],
        },
        "run_late_identity",
        &[],
        false,
    );
    assert!(
        validate_erased_contract(&caller).is_empty(),
        "{:?}",
        validate_erased_contract(&caller)
    );
    let mut caller_codegen = super::CodeGen::new(&context, "caller");
    caller_codegen.compile_program_from_mir(&caller).unwrap();
    caller_codegen.module.verify().unwrap();
    assert_eq!(
        producer_codegen.get_ir(),
        before,
        "producer entry must not be rebuilt for a future type"
    );
    let recorder = caller_codegen
        .module
        .get_function("record_late_drop")
        .unwrap();
    let engine = producer_codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    engine.add_module(&caller_codegen.module).unwrap();
    engine.add_global_mapping(&recorder, record as *const () as usize);
    DROPS.store(0, Ordering::SeqCst);
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_late_identity")
                .unwrap()
                .call(),
            42
        );
    }
    assert_eq!(
        DROPS.load(Ordering::SeqCst),
        40,
        "returned ownership must be destroyed exactly once by its caller"
    );
}

#[test]
fn erased_contract_rejects_unbound_parameters_and_unproved_copy() {
    for unbound in [false, true] {
        let mut program = identity_producer();
        let function = program
            .backend_contract
            .erased
            .functions
            .values_mut()
            .next()
            .unwrap();
        if unbound {
            function.body.as_mut().unwrap().locals[1] = MirErasedType::Parameter(99);
        } else {
            function.body.as_mut().unwrap().blocks[0].statements[0].value = MirErasedRvalue::Use(
                MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(0))),
            );
        }
        let errors = validate_erased_contract(&program);
        let report = agreement::check_mir_runtime_agreement(&program);
        assert_eq!(report.erased_contract_errors, errors);
        assert_eq!(report.invalid_backend_contract, errors.len());
        assert!(!report.is_clean());
        assert!(
            errors.iter().any(|(_, error)| if unbound {
                *error == MirErasedError::UnboundParameter
            } else {
                *error == MirErasedError::CopyWithoutProof
            }),
            "{errors:?}"
        );
        let context = Context::create();
        assert!(super::CodeGen::new(&context, "invalid-erased")
            .compile_program_from_mir(&program)
            .is_err());
    }
}

fn dictionary_callback_producer(receiver: crate::types::ReceiverMode) -> MirProgram {
    let mut program = identity_producer();
    let scalar = program.type_context.intern_type(&Type::I64);
    let drop =
        payload_drop_plan(&program.backend_contract, &mut program.type_context, scalar).unwrap();
    program.backend_contract.erased.descriptors.insert(
        scalar,
        MirTypeDescriptor {
            ty: scalar,
            drop,
            fields: vec![],
        },
    );
    let function = program
        .backend_contract
        .erased
        .functions
        .values_mut()
        .next()
        .unwrap();
    let callback = MirErasedType::Parameter(0);
    let argument = MirErasedType::Concrete(scalar);
    function.signature.params = vec![callback.clone(), argument.clone()];
    function.signature.ret = argument.clone();
    function.signature.dictionaries = vec![MirErasedDictionarySchema {
        subject: callback.clone(),
        trait_id: DefId::new(CrateId(1), LocalDefId(8)),
        trait_args: vec![argument.clone(), argument.clone()],
        members: vec![MirErasedDictionaryMember {
            binder: Default::default(),
            member_id: DefId::new(CrateId(1), LocalDefId(9)),
            receiver: Some(receiver),
            params: vec![callback.clone(), argument.clone()],
            ret: argument.clone(),
            borrowing: crate::mir::erased_borrow_contract(
                &[callback.clone(), argument.clone()], &argument, Some(receiver), &program.type_context,
            ).unwrap(),
        }],
    }];
    function.body = Some(MirErasedBody {
        locals: vec![callback, argument.clone(), argument.clone(), argument],
        mutable_locals: [MirErasedLocal(0)].into_iter().collect(),
        scopes: vec![],
        blocks: vec![MirErasedBasicBlock {
            statements: vec![MirErasedAssignment {
                destination: MirErasedLocal(2),
                value: MirErasedRvalue::DictionaryCall {
                    type_args: vec![], dictionaries: vec![],
                    dictionary: 0,
                    member: 0,
                    receiver: Some(MirErasedPlace::local(MirErasedLocal(0))),
                    args: vec![MirErasedOperand::Copy(MirErasedPlace::local(
                        MirErasedLocal(1),
                    ))],
                },
            }],
            terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(
                MirErasedLocal(2),
            ))),
        }],
    });
    program
}

#[test]
fn erased_mutable_callback_requires_write_access_and_excludes_live_loans() {
    let producer = dictionary_callback_producer(crate::types::ReceiverMode::Mut);
    assert!(validate_erased_contract(&producer).is_empty());
    let mut immutable = producer.clone();
    immutable
        .backend_contract
        .erased
        .functions
        .values_mut()
        .next()
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .mutable_locals
        .clear();
    assert!(validate_erased_contract(&immutable)
        .iter()
        .any(|(_, error)| *error == MirErasedError::BorrowConflict));
    let mut borrowed = producer;
    let function = borrowed
        .backend_contract
        .erased
        .functions
        .values_mut()
        .next()
        .unwrap();
    let reference = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Parameter(0)),
        mutable: false,
    };
    function.signature.layouts.push(reference.clone());
    let body = function.body.as_mut().unwrap();
    body.locals.push(reference);
    body.blocks[0].statements.insert(
        0,
        MirErasedAssignment {
            destination: MirErasedLocal(4),
            value: MirErasedRvalue::Borrow {
                place: MirErasedPlace::local(MirErasedLocal(0)),
                mutable: false,
            },
        },
    );
    assert!(validate_erased_contract(&borrowed)
        .iter()
        .any(|(_, error)| *error == MirErasedError::BorrowConflict));
}

#[test]
fn erased_consuming_callback_cannot_be_invoked_twice() {
    let mut producer = dictionary_callback_producer(crate::types::ReceiverMode::Move);
    assert!(validate_erased_contract(&producer).is_empty());
    let body = producer
        .backend_contract
        .erased
        .functions
        .values_mut()
        .next()
        .unwrap()
        .body
        .as_mut()
        .unwrap();
    let mut second = body.blocks[0].statements[0].clone();
    second.destination = MirErasedLocal(3);
    body.blocks[0].statements.push(second);
    assert!(validate_erased_contract(&producer)
        .iter()
        .any(|(_, error)| *error == MirErasedError::UninitializedOrMoved));
}

#[test]
fn erased_cfg_join_rejects_values_initialized_on_only_one_predecessor() {
    let mut program = identity_producer();
    let boolean = program.type_context.intern_type(&Type::Bool);
    super::object_tests::insert_descriptor(&mut program, boolean);
    let function = program
        .backend_contract
        .erased
        .functions
        .values_mut()
        .next()
        .unwrap();
    function
        .signature
        .params
        .push(MirErasedType::Concrete(boolean));
    let body = function.body.as_mut().unwrap();
    body.locals = vec![
        MirErasedType::Parameter(0),
        MirErasedType::Concrete(boolean),
        MirErasedType::Parameter(0),
    ];
    body.blocks = vec![
        MirErasedBasicBlock {
            statements: vec![],
            terminator: MirErasedTerminator::Branch {
                condition: MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(1))),
                then_block: MirErasedBlock(1),
                else_block: MirErasedBlock(2),
            },
        },
        MirErasedBasicBlock {
            statements: vec![MirErasedAssignment {
                destination: MirErasedLocal(2),
                value: MirErasedRvalue::Use(MirErasedOperand::Move(MirErasedPlace::local(
                    MirErasedLocal(0),
                ))),
            }],
            terminator: MirErasedTerminator::Goto(MirErasedBlock(3)),
        },
        MirErasedBasicBlock {
            statements: vec![],
            terminator: MirErasedTerminator::Goto(MirErasedBlock(3)),
        },
        MirErasedBasicBlock {
            statements: vec![],
            terminator: MirErasedTerminator::Return(MirErasedOperand::Move(MirErasedPlace::local(
                MirErasedLocal(2),
            ))),
        },
    ];
    assert!(validate_erased_contract(&program)
        .iter()
        .any(|(_, error)| *error == MirErasedError::UninitializedOrMoved));
}

fn scoped_concrete_dispatch_program(mutable: bool) -> (MirProgram, MirErasedKey) {
    let mut program = super::object_tests::object_program(mutable);
    // The shared object fixture reads 41 without mutation; make this producer
    // explicitly compute 41 + 1 rather than expecting a mutable-slot effect.
    if !mutable {
        program
            .functions
            .get_mut(&MirFunctionId::Instance(InstanceId(1)))
            .unwrap()
            .basic_blocks[0]
            .statements
            .push(StatementData::assign(
                place(0),
                Rvalue::BinaryOp(
                    MirBinOp::Add,
                    Operand::Copy(place(0)),
                    Operand::Constant(Constant::Int(1)),
                ),
                None,
            ));
    }
    let caller = program
        .functions
        .get(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    let reference = caller.local_decls[4].ty;
    let scalar = caller.ret_type;
    let Type::Reference { inner, .. } = program.type_context.type_for(reference) else {
        panic!("object reference fixture")
    };
    let object = program.type_context.id_for_type(&inner).unwrap();
    super::object_tests::insert_descriptor(&mut program, reference);
    let key = MirErasedKey {
        definition: super::object_tests::definition(90),
        static_args: vec![],
    };
    let witness = crate::types::WitnessId {
        owner: key.definition,
        local: crate::ids::HirLocalId(0),
    };
    let scoped_reference = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Scoped(0)),
        mutable,
    };
    let entry = MirErasedFunction {
        key: key.clone(),
        symbol: "scoped.read".into(),
        linkage: MirLinkage::Internal,
        signature: MirErasedSignature {
            parameters: vec![],
            params: vec![MirErasedType::Concrete(reference)],
            ret: MirErasedType::Concrete(scalar),
            dictionaries: vec![],
            layouts: vec![],
            require_borrow_free: BTreeSet::new(),
        },
        body: Some(MirErasedBody {
            locals: vec![
                MirErasedType::Concrete(reference),
                scoped_reference,
                MirErasedType::Concrete(scalar),
            ],
            mutable_locals: BTreeSet::new(),
            scopes: vec![MirErasedScope {
                witness,
                object,
                source: MirErasedScopeSource::Object { mutable },
            }],
            blocks: vec![MirErasedBasicBlock {
                statements: vec![
                    MirErasedAssignment {
                        destination: MirErasedLocal(1),
                        value: MirErasedRvalue::Open {
                            scope: 0,
                            source: if mutable {
                                MirErasedOperand::Move(MirErasedPlace::local(MirErasedLocal(0)))
                            } else {
                                MirErasedOperand::Copy(MirErasedPlace::local(MirErasedLocal(0)))
                            },
                        },
                    },
                    MirErasedAssignment {
                        destination: MirErasedLocal(2),
                        value: MirErasedRvalue::OpenedCall {
                            scope: 0,
                            slot: 0,
                            receiver: MirErasedPlace {
                                local: MirErasedLocal(1),
                                projection: vec![MirErasedProjection::Deref],
                            },
                            args: vec![],
                        },
                    },
                ],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(
                    MirErasedPlace::local(MirErasedLocal(2)),
                )),
            }],
        }),
    };
    program
        .backend_contract
        .erased
        .functions
        .insert(key.clone(), entry);
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    let Some(Terminator::Call { func, .. }) = &mut caller.basic_blocks[0].terminator else {
        panic!("caller fixture")
    };
    *func = Operand::Constant(Constant::ErasedCall(MirErasedCall {
        target: MirErasedCallTarget::Direct(key.clone()),
        type_args: vec![],
        dictionaries: vec![],
    }));
    (program, key)
}

#[test]
fn scoped_open_dispatch_executes_with_dominating_producer_descriptor() {
    let (program, key) = scoped_concrete_dispatch_program(false);
    assert!(
        validate_erased_contract(&program).is_empty(),
        "{:?}",
        validate_erased_contract(&program)
    );
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "scoped-open");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                .unwrap()
                .call(),
            42
        );
    }
    let mut forged = program;
    let function = forged
        .backend_contract
        .erased
        .functions
        .get_mut(&key)
        .unwrap();
    function.body.as_mut().unwrap().blocks[0]
        .statements
        .remove(0);
    assert!(
        !validate_erased_contract(&forged).is_empty(),
        "an opened call without its dominating Open must fail"
    );
}

#[test]
fn scoped_mutable_dispatch_requires_real_place_capability_and_no_shared_reborrow() {
    let (program, key) = scoped_concrete_dispatch_program(true);
    assert!(
        validate_erased_contract(&program).is_empty(),
        "{:?}",
        validate_erased_contract(&program)
    );
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "scoped-mut-dispatch");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                .unwrap()
                .call(),
            42
        );
    }
    for use_shared_place in [false, true] {
        let mut invalid = program.clone();
        let body = invalid
            .backend_contract
            .erased
            .functions
            .get_mut(&key)
            .unwrap()
            .body
            .as_mut()
            .unwrap();
        body.locals.push(MirErasedType::Reference {
            inner: Box::new(MirErasedType::Scoped(0)),
            mutable: false,
        });
        body.blocks[0].statements.insert(
            1,
            MirErasedAssignment {
                destination: MirErasedLocal(3),
                value: MirErasedRvalue::Borrow {
                    place: MirErasedPlace {
                        local: MirErasedLocal(1),
                        projection: vec![MirErasedProjection::Deref],
                    },
                    mutable: false,
                },
            },
        );
        if use_shared_place {
            let MirErasedRvalue::OpenedCall { receiver, .. } =
                &mut body.blocks[0].statements[2].value
            else {
                panic!("opened call fixture")
            };
            receiver.local = MirErasedLocal(3);
        }
        let errors = validate_erased_contract(&invalid);
        assert!(
            errors
                .iter()
                .any(|(_, error)| *error == MirErasedError::BorrowConflict),
            "{errors:?}"
        );
    }
}

/// The hidden Self input uses the producer's erased ordinal and descriptor,
/// while arithmetic stays in an exact, concrete native implementation body.
fn scoped_erased_sum_program() -> (MirProgram, MirErasedKey, MirErasedKey) {
    let mut program = super::object_tests::object_program(false);
    let caller = &program.functions[&MirFunctionId::Instance(InstanceId(0))];
    let object_ref = caller.local_decls[4].ty;
    let scalar = caller.ret_type;
    let native_ref = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    });
    let Type::Reference { inner, .. } = program.type_context.type_for(object_ref) else {
        panic!("object fixture")
    };
    let object = program.type_context.id_for_type(&inner).unwrap();
    super::object_tests::insert_descriptor(&mut program, object_ref);
    super::object_tests::insert_descriptor(&mut program, native_ref);
    let deref = |index| Place {
        local: Local(index),
        projection: vec![Projection::Deref],
    };
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(3)),
            name: "sum_native".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(native_ref, LocalSource::Argument),
                local(native_ref, LocalSource::Argument),
            ],
            arg_count: 2,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    place(0),
                    Rvalue::BinaryOp(
                        MirBinOp::Add,
                        Operand::Copy(deref(1)),
                        Operand::Copy(deref(2)),
                    ),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
        },
        "sum_native",
        &[native_ref, native_ref],
        true,
    );
    let member = super::object_tests::definition(94);
    let parameter = MirErasedParameter {
        source: GenericParamId {
            owner: member,
            index: 0,
        },
        kind: Kind::Type,
    };
    let hidden_ref = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Parameter(0)),
        mutable: false,
    };
    let slot_signature = MirErasedSignature {
        parameters: vec![parameter],
        params: vec![hidden_ref.clone(), hidden_ref.clone()],
        ret: MirErasedType::Concrete(scalar),
        dictionaries: vec![],
        layouts: vec![
            MirErasedType::Parameter(0),
            hidden_ref,
            MirErasedType::Concrete(scalar),
        ],
        require_borrow_free: BTreeSet::new(),
    };
    for schema in program.backend_contract.object_schemas.values_mut() {
        schema.erased_slots.push(MirErasedObjectSlot {
            trait_id: super::object_tests::definition(11),
            member_id: member,
            trait_args: vec![],
            receiver: crate::types::ReceiverMode::Shared,
            signature: slot_signature.clone(),
            result: MirObjectResult::Direct,
        });
    }
    let producer_key = MirErasedKey {
        definition: super::object_tests::definition(3),
        static_args: vec![scalar],
    };
    let native_ref = MirErasedType::Concrete(native_ref);
    let mut producer_signature = slot_signature;
    producer_signature.params = vec![native_ref.clone(), native_ref.clone()];
    producer_signature.layouts[1] = native_ref.clone();
    let erased_place = |index| MirErasedPlace::local(MirErasedLocal(index));
    program.backend_contract.erased.functions.insert(
        producer_key.clone(),
        MirErasedFunction {
            key: producer_key.clone(),
            symbol: "sum_erased_producer".into(),
            linkage: MirLinkage::Internal,
            signature: producer_signature,
            body: Some(MirErasedBody {
                locals: vec![
                    native_ref.clone(),
                    native_ref,
                    MirErasedType::Concrete(scalar),
                ],
                mutable_locals: BTreeSet::new(),
                scopes: vec![],
                blocks: vec![MirErasedBasicBlock {
                    statements: vec![MirErasedAssignment {
                        destination: MirErasedLocal(2),
                        value: MirErasedRvalue::NativeCall {
                            target: MirCallableKey::Instance(InstanceId(3)),
                            args: vec![
                                MirErasedOperand::Copy(erased_place(0)),
                                MirErasedOperand::Copy(erased_place(1)),
                            ],
                        },
                    }],
                    terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(erased_place(
                        2,
                    ))),
                }],
            }),
        },
    );
    for table in program.backend_contract.vtables.values_mut() {
        table.erased_methods.push(producer_key.clone());
    }
    let caller_key = MirErasedKey {
        definition: super::object_tests::definition(96),
        static_args: vec![],
    };
    let scoped_ref = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Scoped(0)),
        mutable: false,
    };
    let scoped_value = MirErasedPlace {
        local: MirErasedLocal(1),
        projection: vec![MirErasedProjection::Deref],
    };
    program.backend_contract.erased.functions.insert(
        caller_key.clone(),
        MirErasedFunction {
            key: caller_key.clone(),
            symbol: "scoped_sum".into(),
            linkage: MirLinkage::Internal,
            signature: MirErasedSignature {
                parameters: vec![],
                params: vec![MirErasedType::Concrete(object_ref)],
                ret: MirErasedType::Concrete(scalar),
                dictionaries: vec![],
                layouts: vec![],
                require_borrow_free: BTreeSet::new(),
            },
            body: Some(MirErasedBody {
                locals: vec![
                    MirErasedType::Concrete(object_ref),
                    scoped_ref.clone(),
                    scoped_ref,
                    MirErasedType::Concrete(scalar),
                ],
                mutable_locals: BTreeSet::new(),
                scopes: vec![MirErasedScope {
                    witness: crate::types::WitnessId {
                        owner: caller_key.definition,
                        local: crate::ids::HirLocalId(0),
                    },
                    object,
                    source: MirErasedScopeSource::Object { mutable: false },
                }],
                blocks: vec![MirErasedBasicBlock {
                    statements: vec![
                        MirErasedAssignment {
                            destination: MirErasedLocal(1),
                            value: MirErasedRvalue::Open {
                                scope: 0,
                                source: MirErasedOperand::Copy(erased_place(0)),
                            },
                        },
                        MirErasedAssignment {
                            destination: MirErasedLocal(2),
                            value: MirErasedRvalue::Borrow {
                                place: scoped_value.clone(),
                                mutable: false,
                            },
                        },
                        MirErasedAssignment {
                            destination: MirErasedLocal(3),
                            value: MirErasedRvalue::OpenedErasedCall {
                                scope: 0,
                                slot: 0,
                                type_args: vec![MirErasedType::Scoped(0)],
                                dictionaries: vec![],
                                receiver: scoped_value,
                                args: vec![MirErasedOperand::Copy(erased_place(2))],
                            },
                        },
                    ],
                    terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(erased_place(
                        3,
                    ))),
                }],
            }),
        },
    );
    let Some(Terminator::Call { func, .. }) = &mut program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap()
        .basic_blocks[0]
        .terminator
    else {
        panic!("caller fixture")
    };
    *func = Operand::Constant(Constant::ErasedCall(MirErasedCall {
        target: MirErasedCallTarget::Direct(caller_key.clone()),
        type_args: vec![],
        dictionaries: vec![],
    }));
    (program, caller_key, producer_key)
}

#[test]
fn scoped_erased_self_input_executes_exact_native_producer_bridge() {
    for optimization in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        let (program, _, _) = scoped_erased_sum_program();
        let report = agreement::check_mir_runtime_agreement(&program);
        assert!(report.is_clean(), "{report:?}");
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "scoped-erased-sum");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let ir = codegen.get_ir();
        // Void calls have no SSA result name; verify the actual indirect slot
        // call and forwarded evidence instead of its discarded builder label.
        assert!(
            ir.lines().any(|line| {
                line.trim_start()
                    .starts_with("call void %object.entry.load(")
                    && line.contains("ptr %opened.descriptor, ptr @rock.descriptor.thin_reference")
            }),
            "{ir}"
        );
        assert!(ir.contains("erased.native.call"), "{ir}");
        let engine = codegen
            .module
            .create_jit_execution_engine(optimization)
            .unwrap();
        unsafe {
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                    .unwrap()
                    .call(),
                82
            );
        }
    }
}

#[test]
fn scoped_erased_self_input_uses_exact_imported_native_link_without_local_body() {
    for optimization in [OptimizationLevel::None, OptimizationLevel::Aggressive] {
        let (mut consumer, _, _) = scoped_erased_sum_program();
        let target = MirCallableKey::Instance(InstanceId(3));
        let native = consumer
            .functions
            .remove(&MirFunctionId::Instance(InstanceId(3)))
            .unwrap();
        let mut producer = MirProgram {
            functions: BTreeMap::new(),
            type_context: consumer.type_context.clone(),
            backend_contract: MirBackendContract::default(),
        };
        let params = consumer.backend_contract.callables[&target]
            .signature
            .params
            .iter()
            .map(|param| param.semantic_ty)
            .collect::<Vec<_>>();
        insert_body(&mut producer, native, "sum_native", &params, true);
        consumer
            .backend_contract
            .function_bodies
            .remove(&MirFunctionId::Instance(InstanceId(3)));
        consumer
            .backend_contract
            .callables
            .get_mut(&target)
            .unwrap()
            .kind = MirCallableKind::ObjectProvided;
        consumer
            .backend_contract
            .artifact_exports
            .push(MirArtifactExport {
                origin_def_id: consumer.backend_contract.callables[&target].source_def_id,
                source_name: "producer::sum".into(),
                backend_symbol: "sum_native".into(),
                substitution_empty: true,
                has_body: false,
                provided_by_object: true,
                is_specialization: false,
                is_drop_glue: false,
            });
        assert!(
            validate_erased_contract(&consumer).is_empty(),
            "{:?}",
            validate_erased_contract(&consumer)
        );
        assert!(agreement::check_mir_runtime_agreement(&consumer).is_clean());
        let context = Context::create();
        let mut producer_codegen = super::CodeGen::new(&context, "imported-native-producer");
        producer_codegen
            .compile_program_from_mir(&producer)
            .unwrap();
        producer_codegen.module.verify().unwrap();
        let mut consumer_codegen = super::CodeGen::new(&context, "imported-native-consumer");
        consumer_codegen
            .compile_program_from_mir(&consumer)
            .unwrap();
        consumer_codegen.module.verify().unwrap();
        assert!(consumer_codegen
            .module
            .get_function("sum_native")
            .unwrap()
            .get_first_basic_block()
            .is_none());
        let engine = consumer_codegen
            .module
            .create_jit_execution_engine(optimization)
            .unwrap();
        engine.add_module(&producer_codegen.module).unwrap();
        unsafe {
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                    .unwrap()
                    .call(),
                82
            );
        }
        for mutation in 0..10 {
            let mut invalid = consumer.clone();
            match mutation {
                0 => {
                    invalid.backend_contract.artifact_exports.clear();
                }
                1 => {
                    invalid.backend_contract.artifact_exports[0].origin_def_id =
                        Some(super::object_tests::definition(99));
                }
                2 => {
                    invalid.backend_contract.artifact_exports[0].backend_symbol =
                        "unrelated_native".into();
                }
                3 => {
                    invalid.backend_contract.artifact_exports[0].substitution_empty = false;
                }
                4 => {
                    invalid.backend_contract.artifact_exports[0].has_body = true;
                }
                5 => {
                    invalid.backend_contract.artifact_exports[0].provided_by_object = false;
                }
                6 => {
                    invalid.backend_contract.artifact_exports[0].is_specialization = true;
                }
                7 => {
                    invalid
                        .backend_contract
                        .callables
                        .get_mut(&target)
                        .unwrap()
                        .signature
                        .params[0]
                        .pass_mode = MirPassMode::FatDirect;
                }
                8 => {
                    invalid
                        .backend_contract
                        .callables
                        .get_mut(&target)
                        .unwrap()
                        .source_def_id = Some(super::object_tests::definition(99));
                }
                9 => {
                    invalid
                        .backend_contract
                        .artifact_exports
                        .push(invalid.backend_contract.artifact_exports[0].clone());
                }
                _ => unreachable!(),
            }
            assert!(
                !validate_erased_contract(&invalid).is_empty(),
                "forged imported native authority {mutation}"
            );
        }
    }
}

#[test]
fn scoped_erased_self_input_rejects_forged_scope_slot_and_native_abi() {
    for mutation in 0..5 {
        let (mut program, caller, producer) = scoped_erased_sum_program();
        match mutation {
            0 => {
                program
                    .backend_contract
                    .erased
                    .functions
                    .get_mut(&caller)
                    .unwrap()
                    .body
                    .as_mut()
                    .unwrap()
                    .blocks[0]
                    .statements
                    .remove(0);
            }
            1 | 2 => {
                let MirErasedRvalue::OpenedErasedCall {
                    slot, type_args, ..
                } = &mut program
                    .backend_contract
                    .erased
                    .functions
                    .get_mut(&caller)
                    .unwrap()
                    .body
                    .as_mut()
                    .unwrap()
                    .blocks[0]
                    .statements[2]
                    .value
                else {
                    panic!("opened fixture")
                };
                if mutation == 1 {
                    *slot = 99;
                } else {
                    type_args[0] =
                        MirErasedType::Concrete(program.type_context.intern_type(&Type::I64));
                }
            }
            3 => {
                program
                    .backend_contract
                    .callables
                    .get_mut(&MirCallableKey::Instance(InstanceId(3)))
                    .unwrap()
                    .signature
                    .params[0]
                    .pass_mode = MirPassMode::FatDirect;
            }
            4 => {
                let MirErasedRvalue::NativeCall { args, .. } = &mut program
                    .backend_contract
                    .erased
                    .functions
                    .get_mut(&producer)
                    .unwrap()
                    .body
                    .as_mut()
                    .unwrap()
                    .blocks[0]
                    .statements[0]
                    .value
                else {
                    panic!("native fixture")
                };
                args.pop();
            }
            _ => unreachable!(),
        }
        assert!(
            !validate_erased_contract(&program).is_empty(),
            "forged erased authority {mutation}"
        );
        let context = Context::create();
        assert!(super::CodeGen::new(&context, "invalid-scoped-erased")
            .compile_program_from_mir(&program)
            .is_err());
    }
}

#[test]
fn native_erased_bridge_rejects_same_width_unrelated_nominal_receiver() {
    let (mut program, _, producer) = scoped_erased_sum_program();
    let scalar = program.type_context.intern_type(&Type::I64);
    let wrong_id = super::object_tests::definition(97);
    let wrong_ty = Type::Struct {
        id: wrong_id,
        args: vec![],
    };
    let wrong_source = program.type_context.intern_type(&wrong_ty);
    let wrong_ref = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(wrong_ty),
        mutable: false,
    });
    let native_ref = program.type_context.intern_type(&Type::Reference {
        inner: Box::new(Type::I64),
        mutable: false,
    });
    program.backend_contract.nominal_layouts.insert(
        wrong_id,
        MirNominalLayout::Struct {
            id: wrong_id,
            fields: vec![("same_width".into(), scalar)],
            generic_params: vec![],
        },
    );
    insert_body(
        &mut program,
        MirFunction {
            id: MirFunctionId::Instance(InstanceId(4)),
            name: "wrong_nominal_sum".into(),
            local_decls: vec![
                local(scalar, LocalSource::ReturnPlace),
                local(wrong_ref, LocalSource::Argument),
                local(native_ref, LocalSource::Argument),
            ],
            arg_count: 2,
            ret_type: scalar,
            closure_captures: vec![],
            ownership: Default::default(),
            basic_blocks: vec![BasicBlock {
                statements: vec![StatementData::assign(
                    place(0),
                    Rvalue::BinaryOp(
                        MirBinOp::Add,
                        Operand::Copy(Place {
                            local: Local(1),
                            projection: vec![
                                Projection::Deref,
                                Projection::Field {
                                    index: 0,
                                    identity: None,
                                },
                            ],
                        }),
                        Operand::Copy(Place {
                            local: Local(2),
                            projection: vec![Projection::Deref],
                        }),
                    ),
                    None,
                )],
                terminator: Some(Terminator::Return),
            }],
        },
        "wrong_nominal_sum",
        &[wrong_ref, native_ref],
        true,
    );
    let function = program
        .backend_contract
        .erased
        .functions
        .get_mut(&producer)
        .unwrap();
    let MirErasedRvalue::NativeCall { target, .. } =
        &mut function.body.as_mut().unwrap().blocks[0].statements[0].value
    else {
        panic!("native fixture")
    };
    *target = MirCallableKey::Instance(InstanceId(4));
    // Even a forged matching definition ID cannot turn scalar Self into this
    // nominal Self; the argument and static receiver identities must agree.
    program
        .backend_contract
        .callables
        .get_mut(&MirCallableKey::Instance(InstanceId(4)))
        .unwrap()
        .source_def_id = Some(producer.definition);
    assert_eq!(
        validate_erased_function(
            &program.backend_contract.erased.functions[&producer],
            &program.backend_contract,
            &program.type_context
        ),
        Err(MirErasedError::InvalidType)
    );
    let context = Context::create();
    assert!(super::CodeGen::new(&context, "wrong-nominal-bridge")
        .compile_program_from_mir(&program)
        .is_err());
    let MirErasedRvalue::NativeCall { target, .. } = &mut program
        .backend_contract
        .erased
        .functions
        .get_mut(&producer)
        .unwrap()
        .body
        .as_mut()
        .unwrap()
        .blocks[0]
        .statements[0]
        .value
    else {
        panic!("native fixture")
    };
    *target = MirCallableKey::Instance(InstanceId(3));
    super::object_tests::insert_descriptor(&mut program, wrong_source);
    assert!(validate_erased_function(
        &program.backend_contract.erased.functions[&producer],
        &program.backend_contract,
        &program.type_context
    )
    .is_ok());
    let wrong_descriptor_call = MirErasedCall {
        target: MirErasedCallTarget::Direct(producer.clone()),
        type_args: vec![wrong_source],
        dictionaries: vec![],
    };
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    // Keep the actual scalar initialization and shared reference, rather than
    // using a reference already transferred into the object-conversion fixture.
    caller.basic_blocks[0].statements.truncate(2);
    let Some(Terminator::Call { func, args, .. }) = &mut caller.basic_blocks[0].terminator else {
        panic!("caller fixture")
    };
    *args = vec![Operand::Copy(place(2)), Operand::Copy(place(2))];
    let mut valid_call = wrong_descriptor_call.clone();
    valid_call.type_args[0] = scalar;
    *func = Operand::Constant(Constant::ErasedCall(valid_call));
    let errors = validate_erased_contract(&program);
    assert!(errors.is_empty(), "{errors:?}");
    let Some(Terminator::Call { func, .. }) = &mut program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap()
        .basic_blocks[0]
        .terminator
    else {
        panic!("caller fixture")
    };
    *func = Operand::Constant(Constant::ErasedCall(wrong_descriptor_call));
    let errors = validate_erased_contract(&program);
    assert!(
        errors.contains(&(Some(producer), MirErasedError::InvalidType)),
        "the native receiver's descriptor must match its static Self identity: {errors:?}"
    );
    assert!(super::CodeGen::new(&context, "wrong-native-descriptor")
        .compile_program_from_mir(&program)
        .is_err());
}

#[test]
fn scoped_reference_forwards_through_erased_generic_and_recloses_with_same_evidence() {
    let (mut program, caller_key, _) = scoped_erased_sum_program();
    let mut forwarder = erased_identity();
    forwarder.symbol = "scoped_reference_identity".into();
    let generic_ref = MirErasedType::Reference {
        inner: Box::new(MirErasedType::Parameter(0)),
        mutable: false,
    };
    forwarder.signature.params = vec![generic_ref.clone()];
    forwarder.signature.ret = generic_ref.clone();
    forwarder.signature.layouts.push(generic_ref.clone());
    forwarder.body = Some(MirErasedBody {
        locals: vec![generic_ref],
        mutable_locals: BTreeSet::new(),
        scopes: vec![],
        blocks: vec![MirErasedBasicBlock {
            statements: vec![],
            terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(MirErasedPlace::local(
                MirErasedLocal(0),
            ))),
        }],
    });
    let forwarder_key = forwarder.key.clone();
    program
        .backend_contract
        .erased
        .functions
        .insert(forwarder_key.clone(), forwarder);
    let caller = program
        .backend_contract
        .erased
        .functions
        .get_mut(&caller_key)
        .unwrap();
    let object_ref = caller.signature.params[0].clone();
    caller.signature.ret = object_ref.clone();
    let body = caller.body.as_mut().unwrap();
    let object = body.scopes[0].object;
    body.locals.push(body.locals[2].clone());
    body.locals.push(object_ref);
    let place = |index| MirErasedPlace::local(MirErasedLocal(index));
    body.blocks[0].statements.extend([
        MirErasedAssignment {
            destination: MirErasedLocal(4),
            value: MirErasedRvalue::Call {
                target: forwarder_key,
                type_args: vec![MirErasedType::Scoped(0)],
                dictionaries: vec![],
                args: vec![MirErasedOperand::Copy(place(2))],
            },
        },
        MirErasedAssignment {
            destination: MirErasedLocal(5),
            value: MirErasedRvalue::Reclose {
                scope: 0,
                value: MirErasedOperand::Copy(place(4)),
                object,
            },
        },
    ]);
    body.blocks[0].terminator = MirErasedTerminator::Return(MirErasedOperand::Copy(place(5)));
    let caller = program
        .functions
        .get_mut(&MirFunctionId::Instance(InstanceId(0)))
        .unwrap();
    let Some(Terminator::Call { destination, .. }) = &mut caller.basic_blocks[0].terminator else {
        panic!("caller fixture")
    };
    *destination = super::object_tests::place(4);
    caller.basic_blocks[1].terminator = Some(Terminator::Call {
        func: Operand::Constant(Constant::VirtualTarget(MirVirtualTarget {
            object,
            slot: 0,
        })),
        args: vec![Operand::Move(super::object_tests::place(4))],
        destination: super::object_tests::place(0),
        target: BasicBlockId(2),
        span: None,
    });
    caller.basic_blocks.push(BasicBlock {
        statements: vec![],
        terminator: Some(Terminator::Return),
    });
    let report = agreement::check_mir_runtime_agreement(&program);
    assert!(report.is_clean(), "{report:?}");
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "scoped-generic-reclose");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let engine = codegen
        .module
        .create_jit_execution_engine(OptimizationLevel::None)
        .unwrap();
    unsafe {
        assert_eq!(
            engine
                .get_function::<unsafe extern "C" fn() -> i64>("run_object")
                .unwrap()
                .call(),
            41
        );
    }
}

#[test]
fn erased_reborrow_returns_parameter_region_but_not_local_storage() {
    for local_storage in [false, true] {
        let mut program = identity_producer();
        let scalar = program.type_context.intern_type(&Type::I64);
        let reference = program.type_context.intern_type(&Type::Reference {
            inner: Box::new(Type::I64),
            mutable: false,
        });
        super::object_tests::insert_descriptor(&mut program, scalar);
        super::object_tests::insert_descriptor(&mut program, reference);
        let function = program
            .backend_contract
            .erased
            .functions
            .values_mut()
            .next()
            .unwrap();
        function.signature.parameters.clear();
        function.signature.layouts.clear();
        function.signature.params = vec![MirErasedType::Concrete(if local_storage {
            scalar
        } else {
            reference
        })];
        function.signature.ret = MirErasedType::Concrete(reference);
        function.body = Some(MirErasedBody {
            locals: vec![
                function.signature.params[0].clone(),
                MirErasedType::Concrete(reference),
            ],
            mutable_locals: BTreeSet::new(),
            scopes: vec![],
            blocks: vec![MirErasedBasicBlock {
                statements: vec![MirErasedAssignment {
                    destination: MirErasedLocal(1),
                    value: MirErasedRvalue::Borrow {
                        place: MirErasedPlace {
                            local: MirErasedLocal(0),
                            projection: if local_storage {
                                vec![]
                            } else {
                                vec![MirErasedProjection::Deref]
                            },
                        },
                        mutable: false,
                    },
                }],
                terminator: MirErasedTerminator::Return(MirErasedOperand::Copy(
                    MirErasedPlace::local(MirErasedLocal(1)),
                )),
            }],
        });
        let errors = validate_erased_contract(&program);
        if local_storage {
            assert!(
                errors
                    .iter()
                    .any(|(_, error)| *error == MirErasedError::BorrowEscape),
                "{errors:?}"
            );
        } else {
            assert!(errors.is_empty(), "{errors:?}");
            let context = Context::create();
            let mut codegen = super::CodeGen::new(&context, "parameter-reborrow");
            codegen.compile_program_from_mir(&program).unwrap();
            codegen.module.verify().unwrap();
        }
    }
}

#[test]
fn native_erased_bridge_cfg_transfers_and_drops_each_noncopy_payload_once() {
    static DROPS: AtomicU64 = AtomicU64::new(0);
    static TOTAL: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn record(value: i64) {
        DROPS.fetch_add(1, Ordering::SeqCst);
        TOTAL.fetch_add(value as u64, Ordering::SeqCst);
    }
    for native_path in [false, true] {
        let mut program = identity_producer();
        let scalar = program.type_context.intern_type(&Type::I64);
        let boolean = program.type_context.intern_type(&Type::Bool);
        let unit = program.type_context.intern_type(&Type::Unit);
        let payload_id = super::object_tests::definition(98);
        let payload = program.type_context.intern_type(&Type::Struct {
            id: payload_id,
            args: vec![],
        });
        program.backend_contract.nominal_layouts.insert(
            payload_id,
            MirNominalLayout::Struct {
                id: payload_id,
                fields: vec![("value".into(), scalar)],
                generic_params: vec![],
            },
        );
        let record_key = MirCallableKey::Extern(super::object_tests::definition(99));
        program.backend_contract.callables.insert(
            record_key.clone(),
            MirCallableDecl {
                key: record_key.clone(),
                source_def_id: None,
                kind: MirCallableKind::Extern {
                    link_name: "record_native_transfer".into(),
                    variadic: false,
                },
                llvm_symbol: "record_native_transfer".into(),
                linkage: MirLinkage::External,
                signature: MirCallableSignature::from_type_ids(
                    &[scalar],
                    unit,
                    MirPassMode::Direct,
                ),
            },
        );
        let field = |index| Place {
            local: Local(index),
            projection: vec![Projection::Field {
                index: 0,
                identity: None,
            }],
        };
        insert_body(
            &mut program,
            MirFunction {
                id: MirFunctionId::Instance(InstanceId(1)),
                name: "drop_native_payload".into(),
                local_decls: vec![
                    local(unit, LocalSource::ReturnPlace),
                    local(payload, LocalSource::Argument),
                ],
                arg_count: 1,
                ret_type: unit,
                closure_captures: vec![],
                ownership: Default::default(),
                basic_blocks: vec![
                    BasicBlock {
                        statements: vec![],
                        terminator: Some(Terminator::Call {
                            func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                                record_key,
                            ))),
                            args: vec![Operand::Copy(field(1))],
                            destination: place(0),
                            target: BasicBlockId(1),
                            span: None,
                        }),
                    },
                    BasicBlock {
                        statements: vec![],
                        terminator: Some(Terminator::Return),
                    },
                ],
            },
            "drop_native_payload",
            &[payload],
            true,
        );
        program
            .backend_contract
            .drop_glue
            .insert(payload, MirCallableKey::Instance(InstanceId(1)));
        for ty in [scalar, boolean, unit, payload] {
            super::object_tests::insert_descriptor(&mut program, ty);
        }
        insert_body(
            &mut program,
            MirFunction {
                id: MirFunctionId::Instance(InstanceId(3)),
                name: "native_transfer".into(),
                local_decls: vec![
                    local(payload, LocalSource::ReturnPlace),
                    local(payload, LocalSource::Argument),
                    local(payload, LocalSource::Argument),
                    local(boolean, LocalSource::Argument),
                ],
                arg_count: 3,
                ret_type: payload,
                closure_captures: vec![],
                ownership: Default::default(),
                basic_blocks: vec![
                    BasicBlock {
                        statements: vec![StatementData::assign(
                            place(0),
                            Rvalue::Use(Operand::Move(place(1))),
                            None,
                        )],
                        terminator: Some(Terminator::Drop {
                            place: place(2),
                            target: BasicBlockId(1),
                        }),
                    },
                    BasicBlock {
                        statements: vec![],
                        terminator: Some(Terminator::Return),
                    },
                ],
            },
            "native_transfer",
            &[payload, payload, boolean],
            true,
        );
        program.backend_contract.erased.functions.clear();
        let key = MirErasedKey {
            definition: super::object_tests::definition(3),
            static_args: vec![payload],
        };
        let erased_place = |index| MirErasedPlace::local(MirErasedLocal(index));
        program.backend_contract.erased.functions.insert(
            key.clone(),
            MirErasedFunction {
                key: key.clone(),
                symbol: "native_transfer_erased".into(),
                linkage: MirLinkage::Internal,
                signature: MirErasedSignature {
                    parameters: vec![MirErasedParameter {
                        source: GenericParamId {
                            owner: key.definition,
                            index: 0,
                        },
                        kind: Kind::Type,
                    }],
                    params: vec![
                        MirErasedType::Concrete(payload),
                        MirErasedType::Concrete(payload),
                        MirErasedType::Concrete(boolean),
                    ],
                    ret: MirErasedType::Concrete(payload),
                    layouts: vec![MirErasedType::Parameter(0)],
                    dictionaries: vec![],
                    require_borrow_free: BTreeSet::new(),
                },
                body: Some(MirErasedBody {
                    locals: vec![
                        MirErasedType::Concrete(payload),
                        MirErasedType::Concrete(payload),
                        MirErasedType::Concrete(boolean),
                        MirErasedType::Concrete(payload),
                    ],
                    mutable_locals: BTreeSet::new(),
                    scopes: vec![],
                    blocks: vec![
                        MirErasedBasicBlock {
                            statements: vec![],
                            terminator: MirErasedTerminator::Branch {
                                condition: MirErasedOperand::Copy(erased_place(2)),
                                then_block: MirErasedBlock(1),
                                else_block: MirErasedBlock(2),
                            },
                        },
                        MirErasedBasicBlock {
                            statements: vec![MirErasedAssignment {
                                destination: MirErasedLocal(3),
                                value: MirErasedRvalue::NativeCall {
                                    target: MirCallableKey::Instance(InstanceId(3)),
                                    args: vec![
                                        MirErasedOperand::Move(erased_place(0)),
                                        MirErasedOperand::Move(erased_place(1)),
                                        MirErasedOperand::Copy(erased_place(2)),
                                    ],
                                },
                            }],
                            terminator: MirErasedTerminator::Return(MirErasedOperand::Move(
                                erased_place(3),
                            )),
                        },
                        MirErasedBasicBlock {
                            statements: vec![],
                            terminator: MirErasedTerminator::Return(MirErasedOperand::Move(
                                erased_place(0),
                            )),
                        },
                    ],
                }),
            },
        );
        insert_body(
            &mut program,
            MirFunction {
                id: MirFunctionId::Instance(InstanceId(0)),
                name: "run_native_transfer".into(),
                local_decls: vec![
                    local(scalar, LocalSource::ReturnPlace),
                    local(payload, LocalSource::UserBinding),
                    local(payload, LocalSource::UserBinding),
                    local(payload, LocalSource::Temporary),
                    local(boolean, LocalSource::Temporary),
                ],
                arg_count: 0,
                ret_type: scalar,
                closure_captures: vec![],
                ownership: Default::default(),
                basic_blocks: vec![
                    BasicBlock {
                        statements: vec![
                            StatementData::assign(
                                place(1),
                                Rvalue::Aggregate(
                                    AggregateKind::Struct {
                                        id: payload_id,
                                        display_name: "Payload".into(),
                                    },
                                    vec![Operand::Constant(Constant::Int(41))],
                                ),
                                None,
                            ),
                            StatementData::assign(
                                place(2),
                                Rvalue::Aggregate(
                                    AggregateKind::Struct {
                                        id: payload_id,
                                        display_name: "Payload".into(),
                                    },
                                    vec![Operand::Constant(Constant::Int(1))],
                                ),
                                None,
                            ),
                            StatementData::assign(
                                place(4),
                                Rvalue::Use(Operand::Constant(Constant::Bool(native_path))),
                                None,
                            ),
                        ],
                        terminator: Some(Terminator::Call {
                            func: Operand::Constant(Constant::ErasedCall(MirErasedCall {
                                target: MirErasedCallTarget::Direct(key.clone()),
                                type_args: vec![payload],
                                dictionaries: vec![],
                            })),
                            args: vec![
                                Operand::Move(place(1)),
                                Operand::Move(place(2)),
                                Operand::Copy(place(4)),
                            ],
                            destination: place(3),
                            target: BasicBlockId(1),
                            span: None,
                        }),
                    },
                    BasicBlock {
                        statements: vec![StatementData::assign(
                            place(0),
                            Rvalue::BinaryOp(
                                MirBinOp::Add,
                                Operand::Copy(field(3)),
                                Operand::Constant(Constant::Int(1)),
                            ),
                            None,
                        )],
                        terminator: Some(Terminator::Drop {
                            place: place(3),
                            target: BasicBlockId(2),
                        }),
                    },
                    BasicBlock {
                        statements: vec![],
                        terminator: Some(Terminator::Return),
                    },
                ],
            },
            "run_native_transfer",
            &[],
            false,
        );
        let erased_errors = validate_erased_contract(&program);
        assert!(erased_errors.is_empty(), "{erased_errors:?}");
        let report = agreement::check_mir_runtime_agreement(&program);
        assert!(report.is_clean(), "{report:?}");
        let context = Context::create();
        let mut codegen = super::CodeGen::new(&context, "native-transfer-cfg");
        codegen.compile_program_from_mir(&program).unwrap();
        codegen.module.verify().unwrap();
        let engine = codegen
            .module
            .create_jit_execution_engine(OptimizationLevel::None)
            .unwrap();
        let recorder = codegen
            .module
            .get_function("record_native_transfer")
            .unwrap();
        engine.add_global_mapping(&recorder, record as *const () as usize);
        DROPS.store(0, Ordering::SeqCst);
        TOTAL.store(0, Ordering::SeqCst);
        unsafe {
            assert_eq!(
                engine
                    .get_function::<unsafe extern "C" fn() -> i64>("run_native_transfer")
                    .unwrap()
                    .call(),
                42
            );
        }
        assert_eq!(DROPS.load(Ordering::SeqCst), 2, "native_path={native_path}");
        assert_eq!(
            TOTAL.load(Ordering::SeqCst),
            42,
            "native_path={native_path}"
        );
        let mut invalid = program;
        let MirErasedRvalue::NativeCall { args, .. } = &mut invalid
            .backend_contract
            .erased
            .functions
            .get_mut(&key)
            .unwrap()
            .body
            .as_mut()
            .unwrap()
            .blocks[1]
            .statements[0]
            .value
        else {
            panic!("native transfer fixture")
        };
        args[1] = args[0].clone();
        assert!(validate_erased_contract(&invalid)
            .iter()
            .any(|(_, error)| *error == MirErasedError::UninitializedOrMoved));
    }
}

fn static_arity(body: &str) -> usize {
    body.lines().next().unwrap().split_once('(').unwrap().1.split_once(')').unwrap().0.split(',').count()
}

#[test]
fn static_dictionary_llvm_uses_receiver_free_native_and_public_call_abis() {
    let source = format!("{}{}", crate::mir::static_dictionary::tests::PRODUCER, r#"
struct LaterProduct
    < value: I64
impl Factory for LaterProduct
    make = seed ->
        LaterProduct
            value: seed
take: LaterProduct -> I64
take = value -> value.value
main = ->
    service = Service
        marker: 0
    object = as_dispatch &service
    take (object.build 42)
"#);
    let program = crate::mir::static_dictionary::tests::accepted_program(&source);
    let context = Context::create();
    let mut codegen = super::CodeGen::new(&context, "static-native-dictionary");
    codegen.compile_program_from_mir(&program).unwrap();
    codegen.module.verify().unwrap();
    let ir = codegen.module.print_to_string().to_string();
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    let producer = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.") && body.contains("%dictionary.entry = load ptr")).unwrap();
    assert_eq!(static_arity(producer), 6, "out, Self/T descriptors, Factory, actual Dispatch receiver, seed");
    let call = producer.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
    assert_eq!(static_arity(call), 3, "out, dictionary environment, seed: no Self receiver");
    assert!(producer.lines().filter(|line| line.contains("call ")).all(|line| {
        let callee = line.split_once('(').unwrap().0.split_whitespace().last().unwrap();
        callee.starts_with('%') || callee.starts_with("@llvm.")
    }));
    let adapter = bodies.iter().find(|body| body.starts_with("internal void @rock.dictionary.")).unwrap();
    assert_eq!(static_arity(adapter), 3);
    let native = adapter.lines().find(|line| line.contains("call ") && line.contains("@__rock_")).unwrap();
    assert_eq!(static_arity(native), 1, "native leaf has only the declared seed argument");
    assert_eq!(program.backend_contract.erased.dictionaries.len(), 1);
    let member = &program.backend_contract.erased.dictionaries.values().next().unwrap().members[0];
    assert!(member.receiver.is_none() && member.target.native().is_some());
}

#[test]
fn static_dictionary_llvm_selected_adapter_opens_only_frozen_parent_evidence() {
    let program = crate::mir::static_dictionary::tests::fixture(true);
    let context = Context::create(); let mut codegen = super::CodeGen::new(&context, "static-selected-dictionary");
    codegen.compile_program_from_mir(&program).unwrap(); codegen.module.verify().unwrap();
    let ir = codegen.module.print_to_string().to_string();
    let bodies = ir.split("define ").skip(1).map(|body| body.split_once("\n}").unwrap().0).collect::<Vec<_>>();
    let adapter = bodies.iter().find(|body| body.starts_with("internal void @rock.dictionary.selected.")).unwrap();
    assert_eq!(static_arity(adapter), 3);
    assert!(adapter.contains("dictionary.environment") && adapter.contains("call void @rock.erased.selected."));
    let entry = bodies.iter().find(|body| body.starts_with("internal void @rock.erased.selected.")).unwrap();
    assert_eq!(static_arity(entry), 4, "out, parent descriptor, captured child dictionary, seed");
    let call = entry.lines().find(|line| line.trim_start().starts_with("call void %dictionary.entry(")).unwrap();
    assert_eq!(static_arity(call), 3);
    assert!(entry.lines().filter(|line| line.contains("call ")).all(|line| {
        let callee = line.split_once('(').unwrap().0.split_whitespace().last().unwrap();
        callee.starts_with('%') || callee.starts_with("@llvm.")
    }));
    assert!(ir.contains("@rock_dictionary_selected"));
    assert!(!ir.contains("LaterProduct"));
}
