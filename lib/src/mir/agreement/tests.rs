use super::*;

use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, InstanceId, LocalDefId, TypeId, TypeVarId};
use crate::mir::{
    AggregateKind, BasicBlock, BasicBlockId, Constant, Local, LocalDecl, MirAssert, MirAssertKind,
    MirBackendContract, MirCallable, MirCallableDecl, MirCallableKey, MirCallableKind,
    MirCallableSignature, MirEnumVariantLayout, MirFieldIdentity, MirFunction, MirFunctionId,
    MirLinkage, MirNominalLayout, MirPassMode, MirProgram, MirProjectionKey, Mutability, Operand,
    Place, Projection, Rvalue, StatementData, StatementKind, Terminator,
};
use crate::type_context::TypeContext;
use crate::types::{FunctionSafety, GenericParamId, Type};

fn type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn empty_function(function_id: MirFunctionId, type_context: &mut TypeContext) -> MirFunction {
    let unit = type_id(type_context, Type::Unit);
    MirFunction {
        id: function_id,
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    }
}

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

#[test]
fn agreement_validates_contract_projection_output_type_ids() {
    let mut type_context = TypeContext::new();
    let valid_ty = type_id(&mut type_context, Type::I64);
    let invalid_ty = TypeId(999);
    let trait_id = DefId::new(CrateId(0), LocalDefId(41));
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: invalid_ty,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![valid_ty, invalid_ty],
        },
        invalid_ty,
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 3);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_projection_output_without_panicking() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(41)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(42));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let projection_ty = type_id(
        &mut type_context,
        Type::Projection {
            ty: Box::new(Type::Unit),
            trait_id,
            assoc_type: crate::types::AssociatedTypeKey {
                owner: trait_id,
                assoc_type_id: AssocTypeId(0),
            },
            trait_args: vec![],
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "invalid_projection_output".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: projection_ty,
                mutability: Mutability::Not,
                name: Some("projected".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![],
        },
        TypeId(999),
    );
    backend_contract.projection_traits.insert(trait_id);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(report.invalid_type_ids >= 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_validates_all_backend_contract_type_ids() {
    let mut type_context = TypeContext::new();
    let valid_ty = type_id(&mut type_context, Type::I64);
    let callable_key = MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(42)));
    let mut signature =
        MirCallableSignature::from_type_ids(&[TypeId(900)], TypeId(901), MirPassMode::Direct);
    signature.ret.abi_ty = TypeId(902);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key,
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(42))),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
    backend_contract.nominal_layouts.insert(
        DefId::new(CrateId(0), LocalDefId(43)),
        MirNominalLayout::Struct {
            id: DefId::new(CrateId(0), LocalDefId(43)),
            fields: vec![("field".to_string(), TypeId(903))],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        DefId::new(CrateId(0), LocalDefId(44)),
        MirNominalLayout::Enum {
            id: DefId::new(CrateId(0), LocalDefId(44)),
            variants: vec![
                MirEnumVariantLayout {
                    name: "Positional".to_string(),
                    fields: crate::mir::MirVariantLayoutFields::Positional(vec![TypeId(904)]),
                },
                MirEnumVariantLayout {
                    name: "Named".to_string(),
                    fields: crate::mir::MirVariantLayoutFields::Named(vec![(
                        "field".to_string(),
                        TypeId(905),
                    )]),
                },
            ],
            generic_params: Vec::new(),
        },
    );
    backend_contract.drop_glue.insert(
        TypeId(906),
        MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(42))),
    );
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: valid_ty,
            trait_id: DefId::new(CrateId(0), LocalDefId(45)),
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![TypeId(907)],
        },
        TypeId(908),
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 9);
    assert!(!report.is_clean());
}

#[test]
fn agreement_requires_contract_nominal_layout_for_mir_used_struct_types() {
    let id = DefId::new(CrateId(0), LocalDefId(46));
    let function_id = MirFunctionId::Function(id);
    let struct_id = DefId::new(CrateId(0), LocalDefId(47));
    let mut type_context = TypeContext::new();
    let struct_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "returns_struct".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: Vec::new(),
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: struct_ty,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: struct_ty,
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
            llvm_symbol: "returns_struct".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], struct_ty, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 1);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_requires_contract_nominal_layouts_for_callable_signature_types() {
    let callable_id = DefId::new(CrateId(0), LocalDefId(48));
    let param_struct = DefId::new(CrateId(0), LocalDefId(49));
    let ret_struct = DefId::new(CrateId(0), LocalDefId(50));
    let abi_struct = DefId::new(CrateId(0), LocalDefId(51));
    let mut type_context = TypeContext::new();
    let param_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: param_struct,
            args: Vec::new(),
        },
    );
    let ret_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: ret_struct,
            args: Vec::new(),
        },
    );
    let abi_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: abi_struct,
            args: Vec::new(),
        },
    );
    let callable_key = MirCallableKey::Extern(callable_id);
    let mut signature =
        MirCallableSignature::from_type_ids(&[param_ty], ret_ty, MirPassMode::Direct);
    signature.ret.abi_ty = abi_ty;
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key,
            source_def_id: Some(callable_id),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 3);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_validates_contract_layout_keys_and_nested_contract_type_layouts() {
    let callable_id = DefId::new(CrateId(0), LocalDefId(52));
    let holder_id = DefId::new(CrateId(0), LocalDefId(53));
    let enum_id = DefId::new(CrateId(0), LocalDefId(54));
    let nested_struct = DefId::new(CrateId(0), LocalDefId(55));
    let nested_enum = DefId::new(CrateId(0), LocalDefId(56));
    let mismatch_key = DefId::new(CrateId(0), LocalDefId(57));
    let mismatch_value = DefId::new(CrateId(0), LocalDefId(58));
    let projection_base_struct = DefId::new(CrateId(0), LocalDefId(59));
    let projection_arg_struct = DefId::new(CrateId(0), LocalDefId(60));
    let projection_output_struct = DefId::new(CrateId(0), LocalDefId(61));
    let drop_enum = DefId::new(CrateId(0), LocalDefId(62));
    let trait_id = DefId::new(CrateId(0), LocalDefId(63));
    let _trait_member_id = DefId::new(CrateId(0), LocalDefId(64));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let nested_struct_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: nested_struct,
            args: Vec::new(),
        },
    );
    let nested_enum_ty = type_id(
        &mut type_context,
        Type::Enum {
            id: nested_enum,
            args: Vec::new(),
        },
    );
    let projection_base_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_base_struct,
            args: Vec::new(),
        },
    );
    let projection_arg_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_arg_struct,
            args: Vec::new(),
        },
    );
    let projection_output_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: projection_output_struct,
            args: Vec::new(),
        },
    );
    let drop_ty = type_id(
        &mut type_context,
        Type::Enum {
            id: drop_enum,
            args: Vec::new(),
        },
    );
    let callable_key = MirCallableKey::Extern(callable_id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key.clone(),
            source_def_id: Some(callable_id),
            kind: MirCallableKind::Extern {
                link_name: "ffi".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract.nominal_layouts.insert(
        holder_id,
        MirNominalLayout::Struct {
            id: holder_id,
            fields: vec![("nested".to_string(), nested_struct_ty)],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        enum_id,
        MirNominalLayout::Enum {
            id: enum_id,
            variants: vec![MirEnumVariantLayout {
                name: "Nested".to_string(),
                fields: crate::mir::MirVariantLayoutFields::Named(vec![(
                    "value".to_string(),
                    nested_enum_ty,
                )]),
            }],
            generic_params: Vec::new(),
        },
    );
    backend_contract.nominal_layouts.insert(
        mismatch_key,
        MirNominalLayout::Struct {
            id: mismatch_value,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: projection_base_ty,
            trait_id,
            assoc_type_id: AssocTypeId(0),
            trait_args: vec![projection_arg_ty],
        },
        projection_output_ty,
    );
    backend_contract
        .drop_glue
        .insert(drop_ty, callable_key.clone());
    backend_contract.projection_traits.insert(trait_id);
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 7);
    assert_eq!(report.stale_projection_facts, 0);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_resolved_callable_key_missing_from_backend_contract() {
    let caller = DefId::new(CrateId(0), LocalDefId(40));
    let function_id = MirFunctionId::Function(caller);
    let missing_key = MirCallableKey::Instance(InstanceId(401));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    missing_key.clone(),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let caller_key = MirCallableKey::Function(caller);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), caller_key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_backend_contract, 1);
    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_unit_callable_placeholder_assignment() {
    let id = DefId::new(CrateId(0), LocalDefId(1));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&mut type_context, Type::function(vec![], Type::Unit)),
            mutability: Mutability::Not,
            name: Some("callable".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_allows_unit_assignment_to_projected_unit_field() {
    let id = DefId::new(CrateId(0), LocalDefId(2));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let tuple = type_id(&mut type_context, Type::Tuple(vec![Type::Unit, Type::I64]));
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(1),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: None,
                    }],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: tuple,
                mutability: Mutability::Not,
                name: Some("tuple".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let callable_key = MirCallableKey::Function(id);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        callable_key.clone(),
        MirCallableDecl {
            key: callable_key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), callable_key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

#[test]
fn agreement_rejects_field_identity_without_nominal_layout_metadata() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(900)));
    let struct_id = DefId::new(CrateId(0), LocalDefId(901));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "field_identity_without_layout".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: Some(MirFieldIdentity::new(struct_id, FieldId(0))),
                    }],
                },
                Rvalue::Use(Operand::Constant(Constant::Unit)),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_layout_checks_use_contract_not_legacy_metadata() {
    let legacy_id = DefId::new(CrateId(0), LocalDefId(901));
    let contract_id = DefId::new(CrateId(0), LocalDefId(902));
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(903)));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let legacy_ty = type_id(
        &mut type_context,
        Type::Struct {
            id: legacy_id,
            args: Vec::new(),
        },
    );
    let function = MirFunction {
        id: function_id.clone(),
        name: "layout_contract_divergence".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![Projection::Field {
                        index: 0,
                        identity: Some(MirFieldIdentity::new(legacy_id, FieldId(0))),
                    }],
                },
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: legacy_id,
                        display_name: "Legacy".to_string(),
                    },
                    Vec::new(),
                ),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![LocalDecl {
            ty: legacy_ty,
            mutability: Mutability::Mut,
            name: Some("value".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        contract_id,
        MirNominalLayout::Struct {
            id: contract_id,
            fields: vec![("field".to_string(), unit)],
            generic_params: Vec::new(),
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 3);
}

#[test]
fn agreement_rejects_unit_callable_placeholder_call_terminator() {
    let id = DefId::new(CrateId(0), LocalDefId(3));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Unit),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_unit_local_callable_placeholder_call_terminator() {
    let id = DefId::new(CrateId(0), LocalDefId(4));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Copy(Place {
                    local: Local(0),
                    projection: vec![],
                }),
                args: vec![],
                destination: Place {
                    local: Local(1),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![
            LocalDecl {
                ty: type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Not,
                name: Some("callable".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: type_id(&mut type_context, Type::Unit),
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.non_unit_placeholder_units, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_bounds_check_assertion_missing_index_operand() {
    let id = DefId::new(CrateId(0), LocalDefId(5));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let array_base = Place {
        local: Local(1),
        projection: vec![],
    };
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::new(
                StatementKind::Assert(MirAssert {
                    kind: MirAssertKind::BoundsCheck,
                    operands: vec![Operand::Copy(array_base)],
                }),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: type_id(&mut type_context, Type::Array(Box::new(Type::I64), 4)),
                mutability: Mutability::Not,
                name: Some("array".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
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
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.malformed_runtime_requirements, 1);
    assert_eq!(report.missing_runtime_requirements, 0);
    assert_eq!(report.stale_runtime_requirements, 0);
    assert_eq!(report.invalid_backend_contract, 0);
    assert!(
        !report.is_clean(),
        "unexpected clean agreement report: {report:?}"
    );
}

#[test]
fn agreement_rejects_invalid_type_ids_in_mir_and_backend_contract() {
    let id = DefId::new(CrateId(0), LocalDefId(8));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: invalid,
            mutability: Mutability::Not,
            name: Some("bad".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: invalid,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    let extern_key = MirCallableKey::Extern(id);
    backend_contract.callables.insert(
        extern_key.clone(),
        MirCallableDecl {
            key: extern_key,
            source_def_id: Some(id),
            kind: MirCallableKind::Extern {
                link_name: "external".to_string(),
                variadic: false,
            },
            llvm_symbol: "external".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[invalid], unit, MirPassMode::Direct),
        },
    );
    backend_contract.nominal_layouts.insert(
        id,
        MirNominalLayout::Struct {
            id,
            fields: vec![("field".to_string(), invalid)],
            generic_params: Vec::new(),
        },
    );
    backend_contract
        .drop_glue
        .insert(invalid, MirCallableKey::Function(id));
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 5);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_backend_forbidden_type_states() {
    let owner = DefId::new(CrateId(0), LocalDefId(81));
    let mut type_context = TypeContext::new();
    let type_var = type_id(&mut type_context, Type::TypeVar(TypeVarId(0)));
    let generic = type_id(
        &mut type_context,
        Type::Generic(GenericParamId { owner, index: 0 }),
    );
    let error = type_id(&mut type_context, Type::Error);
    let option = DefId::new(CrateId(0), LocalDefId(82));
    let constructor_generic = GenericParamId { owner, index: 1 };
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let mut normalization_env = crate::type_services::normalize::TypeNormalizationEnv::new();
    normalization_env.register_constructor(
        option,
        crate::types::NominalTypeKind::Enum,
        unary.clone(),
    );
    normalization_env.register_generic_kind(constructor_generic, unary);
    let constructor = type_context
        .intern_normalized_type(
            &Type::Constructor {
                id: option,
                flavor: crate::types::NominalTypeKind::Enum,
            },
            &normalization_env,
        )
        .unwrap();
    let application = type_context
        .intern_normalized_type(
            &Type::Apply {
                constructor: Box::new(Type::Generic(constructor_generic)),
                args: vec![Type::I64],
            },
            &normalization_env,
        )
        .unwrap();
    let lambda = type_context
        .intern_normalized_type(
            &Type::Lambda {
                params: vec![crate::type_services::kind::Kind::Type],
                body: Box::new(Type::Tuple(vec![
                    Type::BoundVar {
                        depth: 0,
                        index: 0,
                        kind: crate::type_services::kind::Kind::Type,
                    },
                    Type::I64,
                ])),
            },
            &normalization_env,
        )
        .unwrap();
    let bound = type_context
        .intern_normalized_type(
            &Type::BoundVar {
                depth: 0,
                index: 0,
                kind: crate::type_services::kind::Kind::Type,
            },
            &normalization_env,
        )
        .unwrap();
    let unit = type_id(&mut type_context, Type::Unit);
    let key = MirCallableKey::Extern(owner);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key,
            source_def_id: Some(owner),
            kind: MirCallableKind::Extern {
                link_name: "forbidden_types".to_string(),
                variadic: false,
            },
            llvm_symbol: "forbidden_types".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(
                &[
                    type_var,
                    generic,
                    error,
                    constructor,
                    application,
                    lambda,
                    bound,
                ],
                unit,
                MirPassMode::Direct,
            ),
        },
    );
    let program = MirProgram {
        functions: std::collections::BTreeMap::new(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 7);
    assert!(!report.is_clean());
}

#[test]
fn agreement_does_not_request_layouts_for_closed_constructor_arguments() {
    let option = DefId::new(CrateId(0), LocalDefId(83));
    let compose = DefId::new(CrateId(0), LocalDefId(84));
    let unary = crate::type_services::kind::Kind::arrow(
        crate::type_services::kind::Kind::Type,
        crate::type_services::kind::Kind::Type,
    );
    let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
    env.register_constructor(option, crate::types::NominalTypeKind::Enum, unary.clone());
    env.register_constructor(
        compose,
        crate::types::NominalTypeKind::Struct,
        crate::type_services::kind::Kind::arrow(
            unary,
            crate::type_services::kind::Kind::arrow(
                crate::type_services::kind::Kind::Type,
                crate::type_services::kind::Kind::Type,
            ),
        ),
    );
    let mut type_context = TypeContext::new();
    let compose_id = type_context
        .intern_normalized_type(
            &Type::Apply {
                constructor: Box::new(Type::Constructor {
                    id: compose,
                    flavor: crate::types::NominalTypeKind::Struct,
                }),
                args: vec![
                    Type::Lambda {
                        params: vec![crate::type_services::kind::Kind::Type],
                        body: Box::new(Type::Tuple(vec![
                            Type::Enum {
                                id: option,
                                args: vec![Type::BoundVar {
                                    depth: 0,
                                    index: 0,
                                    kind: crate::type_services::kind::Kind::Type,
                                }],
                            },
                            Type::I64,
                        ])),
                    },
                    Type::I64,
                ],
            },
            &env,
        )
        .unwrap();
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        compose,
        MirNominalLayout::Struct {
            id: compose,
            fields: Vec::new(),
            generic_params: Vec::new(),
        },
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };
    let mut report = MirAgreementReport::default();

    validate_type_id(&program, compose_id, &mut report);
    validate_type_nominal_layout_id(
        &program,
        &BackendContract::new(&program),
        compose_id,
        &mut HashSet::new(),
        &mut report,
    );

    assert_eq!(report.invalid_type_ids, 0);
    assert_eq!(report.invalid_layout_ids, 0);
}

#[test]
fn agreement_rejects_missing_function_callable_metadata() {
    let id = DefId::new(CrateId(0), LocalDefId(9));
    let missing = DefId::new(CrateId(0), LocalDefId(10));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(missing),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: type_id(&mut type_context, Type::Unit),
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_accepts_resolved_instance_callable_backed_by_contract() {
    let id = DefId::new(CrateId(0), LocalDefId(11));
    let dependency = DefId::new(CrateId(1), LocalDefId(12));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Instance(InstanceId(2)),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    let dependency_key = MirCallableKey::Instance(InstanceId(2));
    backend_contract.callables.insert(
        dependency_key.clone(),
        MirCallableDecl {
            key: dependency_key,
            source_def_id: Some(dependency),
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: "dependency::make".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    let caller_key = MirCallableKey::Function(id);
    backend_contract.callables.insert(
        caller_key.clone(),
        MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], unit, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), caller_key);
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

#[test]
fn agreement_rejects_function_body_without_callable_symbol_metadata() {
    let caller = DefId::new(CrateId(0), LocalDefId(13));
    let callee = DefId::new(CrateId(0), LocalDefId(14));
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let caller_function = MirFunction {
        id: MirFunctionId::Function(caller),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Function(callee),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let callee_function = MirFunction {
        id: MirFunctionId::Function(callee),
        name: "callee".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([
            (MirFunctionId::Function(caller), caller_function),
            (MirFunctionId::Function(callee), callee_function),
        ]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_instance_body_without_callable_symbol_metadata() {
    let caller = DefId::new(CrateId(0), LocalDefId(15));
    let instance = InstanceId(16);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let caller_function = MirFunction {
        id: MirFunctionId::Function(caller),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::Instance(instance),
                ))),
                args: vec![],
                destination: Place {
                    local: Local(0),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let instance_function = MirFunction {
        id: MirFunctionId::Instance(instance),
        name: "instance".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([
            (MirFunctionId::Function(caller), caller_function),
            (MirFunctionId::Instance(instance), instance_function),
        ]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_cast_target_type_id() {
    let id = DefId::new(CrateId(0), LocalDefId(17));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Cast(Operand::Constant(Constant::Int(1)), invalid),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_invalid_nested_type_id() {
    let id = DefId::new(CrateId(0), LocalDefId(18));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let function_ty = type_context.intern_ty(Ty::Function {
        params: vec![invalid],
        ret: unit,
        safety: FunctionSafety::Safe,
        callable_kind: crate::types::CallableKind::Fn,
        captures: Vec::new(),
    });
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Intrinsic(
                        crate::mir::MirIntrinsicId::SizeOf,
                    )),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: function_ty,
            mutability: Mutability::Not,
            name: Some("callable".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_callable_metadata_in_assignment_operand() {
    let id = DefId::new(CrateId(0), LocalDefId(22));
    let missing = DefId::new(CrateId(0), LocalDefId(23));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let callable_ty = type_id(&mut type_context, Type::function(vec![], Type::Unit));
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Use(Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Function(missing)),
                ))),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: callable_ty,
            mutability: Mutability::Not,
            name: Some("callable".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: type_id(&mut type_context, Type::Unit),
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_callable_metadata_in_indirect_call_argument() {
    let id = DefId::new(CrateId(0), LocalDefId(24));
    let missing = DefId::new(CrateId(0), LocalDefId(25));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let callable_ty = type_id(&mut type_context, Type::function(vec![], Type::Unit));
    let callee_ty = type_context.intern_ty(Ty::Function {
        params: vec![callable_ty],
        ret: unit,
        safety: FunctionSafety::Safe,
        callable_kind: crate::types::CallableKind::Fn,
        captures: Vec::new(),
    });
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![],
            terminator: Some(Terminator::Call {
                func: Operand::Copy(Place {
                    local: Local(0),
                    projection: vec![],
                }),
                args: vec![Operand::Constant(Constant::Callable(
                    MirCallable::Resolved(MirCallableKey::Function(missing)),
                ))],
                destination: Place {
                    local: Local(1),
                    projection: vec![],
                },
                target: BasicBlockId(0),
                span: None,
            }),
        }],
        local_decls: vec![
            LocalDecl {
                ty: callee_ty,
                mutability: Mutability::Not,
                name: Some("callee".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: unit,
                mutability: Mutability::Not,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
        ],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_callable_metadata, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_aggregate_with_missing_struct_layout_metadata() {
    let id = DefId::new(CrateId(0), LocalDefId(32));
    let missing_struct = DefId::new(CrateId(0), LocalDefId(33));
    let _other_struct = DefId::new(CrateId(0), LocalDefId(34));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(0),
                    projection: vec![],
                },
                Rvalue::Aggregate(
                    AggregateKind::Struct {
                        id: missing_struct,
                        display_name: "Missing".to_string(),
                    },
                    vec![],
                ),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![LocalDecl {
            ty: unit,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_layout_ids, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_function_type_with_missing_projection_output() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(35)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(36));
    let mut type_context = TypeContext::new();
    let projection = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    let projection_id = type_id(&mut type_context, projection);
    let mut function = empty_function(function_id.clone(), &mut type_context);
    function.local_decls.push(LocalDecl {
        ty: projection_id,
        mutability: Mutability::Not,
        name: Some("projected".to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    });
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_missing_projection_outputs_in_cast_and_type_metadata() {
    let function_id = MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(37)));
    let trait_id = DefId::new(CrateId(0), LocalDefId(38));
    let mut type_context = TypeContext::new();
    let projection = Type::Projection {
        ty: Box::new(Type::I64),
        trait_id,
        assoc_type: crate::types::AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: Vec::new(),
    };
    let projection_id = type_id(&mut type_context, projection);
    let mut function = empty_function(function_id.clone(), &mut type_context);
    function.basic_blocks[0].statements = vec![
        StatementData::assign(
            Place {
                local: Local(0),
                projection: Vec::new(),
            },
            Rvalue::Cast(Operand::Constant(Constant::Unit), projection_id),
            None,
        ),
        StatementData::assert(
            MirAssert {
                kind: MirAssertKind::BoundsCheck,
                operands: vec![
                    Operand::Constant(Constant::TypeId(projection_id)),
                    Operand::Constant(Constant::Int(0)),
                ],
            },
            None,
        ),
    ];
    let program = MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.missing_projection_facts, 2);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_projection_output_without_contract_trait() {
    let stale_trait = DefId::new(CrateId(0), LocalDefId(35));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id: stale_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![],
        },
        unit,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.stale_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_rejects_stale_contract_projection_output_without_projection_trait() {
    let stale_trait = DefId::new(CrateId(0), LocalDefId(37));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let valid = type_id(&mut type_context, Type::Unit);
    let invalid = TypeId(999);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: invalid,
            trait_id: stale_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![valid, invalid],
        },
        invalid,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_type_ids, 3);
    assert_eq!(report.stale_projection_facts, 1);
    assert!(!report.is_clean());
}

#[test]
fn agreement_accepts_contract_projection_output_with_projection_trait() {
    let known_trait = DefId::new(CrateId(0), LocalDefId(39));
    let assoc_type = AssocTypeId(0);
    let mut type_context = TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_traits.insert(known_trait);
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base: unit,
            trait_id: known_trait,
            assoc_type_id: assoc_type,
            trait_args: vec![],
        },
        unit,
    );
    let program = MirProgram {
        functions: Default::default(),
        type_context,
        backend_contract,
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.stale_projection_facts, 0);
    assert!(
        report.is_clean(),
        "unexpected MIR agreement report: {report:?}"
    );
}

fn intrinsic_call_program(intrinsic: crate::mir::MirIntrinsicId, args: Vec<Operand>) -> MirProgram {
    let id = DefId::new(CrateId(0), LocalDefId(36));
    let function_id = MirFunctionId::Function(id);
    let mut type_context = TypeContext::new();
    let i64_ty = type_id(&mut type_context, Type::I64);
    let intrinsic_key = MirCallableKey::Intrinsic(intrinsic.clone());
    let function_key = MirCallableKey::Function(id);
    let function = MirFunction {
        id: function_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![
            BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        intrinsic_key.clone(),
                    ))),
                    args,
                    destination: Place {
                        local: Local(0),
                        projection: vec![],
                    },
                    target: BasicBlockId(1),
                    span: None,
                }),
            },
            BasicBlock {
                statements: vec![],
                terminator: Some(Terminator::Return),
            },
        ],
        local_decls: vec![LocalDecl {
            ty: i64_ty,
            mutability: Mutability::Not,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: vec![],
        arg_count: 0,
        ret_type: i64_ty,
        ownership: Default::default(),
    };
    let mut backend_contract = MirBackendContract::default();
    backend_contract.callables.insert(
        function_key.clone(),
        MirCallableDecl {
            key: function_key.clone(),
            source_def_id: Some(id),
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: "main".to_string(),
            linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[], i64_ty, MirPassMode::Direct),
        },
    );
    backend_contract.callables.insert(
        intrinsic_key.clone(),
        MirCallableDecl {
            key: intrinsic_key.clone(),
            source_def_id: None,
            kind: MirCallableKind::Intrinsic(intrinsic.clone()),
            llvm_symbol: intrinsic.as_str().to_string(),
            linkage: MirLinkage::Internal,
            signature: MirCallableSignature::from_type_ids(&[i64_ty], i64_ty, MirPassMode::Direct),
        },
    );
    backend_contract
        .function_bodies
        .insert(function_id.clone(), function_key);

    MirProgram {
        functions: std::collections::BTreeMap::from([(function_id, function)]),
        type_context,
        backend_contract,
    }
}

#[test]
fn agreement_accepts_type_id_metadata_for_type_only_intrinsic() {
    let program = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );

    let report = check_mir_runtime_agreement(&program);

    assert!(report.is_clean(), "unexpected report: {report:?}");
}

#[test]
fn agreement_rejects_misplaced_and_malformed_type_id_metadata() {
    let runtime_intrinsic = intrinsic_call_program(
        crate::mir::MirIntrinsicId::I64Add,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let malformed_type_only = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::Int(1))],
    );
    let unknown_type = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(999)))],
    );
    let misplaced_callable = intrinsic_call_program(
        crate::mir::MirIntrinsicId::I64Add,
        vec![Operand::Constant(Constant::Callable(
            MirCallable::Resolved(MirCallableKey::Intrinsic(
                crate::mir::MirIntrinsicId::SizeOf,
            )),
        ))],
    );
    let empty_array_len = intrinsic_call_program(crate::mir::MirIntrinsicId::ArrayLen, vec![]);
    let scalar_array_len_metadata = intrinsic_call_program(
        crate::mir::MirIntrinsicId::ArrayLen,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let extra_array_len_args = intrinsic_call_program(
        crate::mir::MirIntrinsicId::ArrayLen,
        vec![
            Operand::Constant(Constant::Int(1)),
            Operand::Constant(Constant::Int(2)),
        ],
    );

    assert_eq!(
        check_mir_runtime_agreement(&runtime_intrinsic).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&malformed_type_only).invalid_constant_operands,
        1
    );
    assert!(check_mir_runtime_agreement(&unknown_type).invalid_type_ids > 0);
    assert_eq!(
        check_mir_runtime_agreement(&misplaced_callable).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&empty_array_len).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&scalar_array_len_metadata).invalid_constant_operands,
        1
    );
    assert_eq!(
        check_mir_runtime_agreement(&extra_array_len_args).invalid_constant_operands,
        1
    );
}

#[test]
fn agreement_rejects_callable_assignment_to_non_function_destination() {
    let mut program = intrinsic_call_program(
        crate::mir::MirIntrinsicId::SizeOf,
        vec![Operand::Constant(Constant::TypeId(TypeId(0)))],
    );
    let function = program.functions.values_mut().next().unwrap();
    function.basic_blocks[0] = BasicBlock {
        statements: vec![StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Use(Operand::Constant(Constant::Callable(
                MirCallable::Resolved(MirCallableKey::Intrinsic(
                    crate::mir::MirIntrinsicId::SizeOf,
                )),
            ))),
            None,
        )],
        terminator: Some(Terminator::Return),
    };

    let report = check_mir_runtime_agreement(&program);

    assert_eq!(report.invalid_constant_operands, 1);
}
