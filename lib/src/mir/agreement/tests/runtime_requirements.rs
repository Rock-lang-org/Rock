use super::*;

fn runtime_requirement_program(
    runtime_requirements: std::collections::BTreeSet<crate::mir::MirRuntimeHelper>,
    captures: bool,
) -> MirProgram {
    let id = DefId::new(CrateId(0), LocalDefId(100));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let _unit = type_id(&mut type_context, Type::Unit);
    let i64_ty = type_id(&mut type_context, Type::I64);
    let closure_ty = type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
    let mut statements = vec![StatementData::assert(
        MirAssert {
            kind: MirAssertKind::BoundsCheck,
            operands: vec![
                Operand::Copy(Place {
                    local: Local(1),
                    projection: vec![],
                }),
                Operand::Copy(Place {
                    local: Local(2),
                    projection: vec![],
                }),
            ],
        },
        None,
    )];
    if captures {
        statements.push(StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Closure(crate::mir::MirClosure {
                id: crate::mir::MirClosureId {
                    owner: function_id.clone(),
                    local_index: 0,
                },
                display_name: "captured".to_string(),
                captures: vec![crate::mir::MirClosureCapture {
                    name: "captured".to_string(),
                    local: Local(2),
                    kind: crate::mir::MirClosureCaptureKind::ByValue,
                    span: None,
                }],
            }),
            None,
        ));
    }
    let function = MirFunction {
        id: function_id.clone(),
        name: "runtime_requirement_fixture".to_string(),
        basic_blocks: vec![BasicBlock {
            statements,
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: closure_ty,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Not,
                name: Some("base".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_ty,
                mutability: Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: closure_ty,
        ownership: Default::default(),
    };
    let key = MirCallableKey::Function(id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "runtime_requirement_fixture".to_string(),
            linkage: MirLinkage::Internal,
            signature: MirCallableSignature::from_type_ids(&[], closure_ty, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), key);
    backend_contract.runtime_requirements = runtime_requirements;

    MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    }
}

#[test]
fn agreement_rejects_missing_runtime_requirement() {
    let program = runtime_requirement_program(
        std::collections::BTreeSet::from([crate::mir::MirRuntimeHelper::BoundsCheck]),
        true,
    );

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_runtime_requirements, 1);
    assert_eq!(report.stale_runtime_requirements, 0);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_stale_runtime_requirement() {
    let program = runtime_requirement_program(
        std::collections::BTreeSet::from([
            crate::mir::MirRuntimeHelper::BoundsCheck,
            crate::mir::MirRuntimeHelper::HeapAlloc,
        ]),
        false,
    );

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_runtime_requirements, 0);
    assert_eq!(report.stale_runtime_requirements, 1);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_reports_backend_contract_body_without_callable_mapping() {
    let id = DefId::new(CrateId(0), LocalDefId(37));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = empty_function(function_id.clone(), &mut type_context);
    let mut backend_contract = MirBackendContract::default();
    let ext_key = MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(38)));
    backend_contract.callables.insert(
        ext_key.clone(),
        MirCallableDecl {
            key: ext_key,
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(38))),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], TypeId(0), MirPassMode::Direct),
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_backend_contract, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_reports_empty_backend_contract_for_backend_program() {
    let id = DefId::new(CrateId(0), LocalDefId(39));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = empty_function(function_id.clone(), &mut type_context);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_backend_contract, 1);
    assert!(!report.is_clean());
}
