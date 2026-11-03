use std::collections::{BTreeMap, BTreeSet};

use inkwell::context::Context;

use crate::diagnostic::{DiagnosticCode, DiagnosticLocation};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mono::InstanceId;
use crate::types::Type;

use super::{CodeGen, CodegenError};

fn type_id(type_context: &mut crate::type_context::TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn local_instance_contract(
    type_context: &crate::type_context::TypeContext,
    instance_id: InstanceId,
    def_id: DefId,
    symbol: &str,
    params: Vec<crate::ids::TypeId>,
    ret: crate::ids::TypeId,
    is_method: bool,
) -> crate::mir::MirBackendContract {
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    let function_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mut contract = crate::mir::MirBackendContract::default();
    let signature = crate::mir::MirCallableSignature::from_instance_type_ids(
        &params,
        ret,
        is_method,
        type_context,
    );
    contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(def_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: symbol.to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature,
        },
    );
    contract.function_bodies.insert(function_id, key);
    contract
}

#[test]
fn declare_runtime_only_declares_contract_requirements() {
    let context = Context::create();

    let mut bounds_codegen = CodeGen::new(&context, "bounds");
    bounds_codegen.declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::BoundsCheck]));
    assert!(bounds_codegen.module.get_function("puts").is_some());
    assert!(bounds_codegen.module.get_function("exit").is_some());
    assert!(bounds_codegen.module.get_function("malloc").is_none());

    let mut heap_codegen = CodeGen::new(&context, "heap");
    heap_codegen.declare_runtime(&BTreeSet::from([crate::mir::MirRuntimeHelper::HeapAlloc]));
    assert!(heap_codegen.module.get_function("puts").is_none());
    assert!(heap_codegen.module.get_function("exit").is_none());
    assert!(heap_codegen.module.get_function("malloc").is_some());

    let mut empty_codegen = CodeGen::new(&context, "empty");
    empty_codegen.declare_runtime(&BTreeSet::new());
    assert!(empty_codegen.module.get_function("puts").is_none());
    assert!(empty_codegen.module.get_function("exit").is_none());
    assert!(empty_codegen.module.get_function("malloc").is_none());
}

#[test]
fn compile_mir_program_rejects_missing_heap_allocation_requirement_before_lowering() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "missing_heap_requirement");
    let instance_id = InstanceId(301);
    let function_id = DefId::new(CrateId(0), LocalDefId(301));
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let closure_id = crate::mir::MirClosureId {
        owner: mir_id.clone(),
        local_index: 0,
    };
    let mut type_context = crate::type_context::TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let closure_ty = type_id(&mut type_context, Type::function(Vec::new(), Type::Unit));
    let function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(1),
                    projection: vec![],
                },
                crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                    id: closure_id,
                    display_name: "captured".to_string(),
                    captures: vec![crate::mir::MirClosureCapture {
                        name: "capture".to_string(),
                        local: crate::mir::Local(0),
                        kind: crate::mir::MirClosureCaptureKind::ByValue,
                        span: None,
                    }],
                }),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: unit,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: closure_ty,
                mutability: crate::mir::Mutability::Mut,
                name: Some("closure".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, function)]),
        type_context: type_context.clone(),
        backend_contract: local_instance_contract(
            &type_context,
            instance_id,
            function_id,
            "main",
            Vec::new(),
            unit,
            false,
        ),
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error.message.contains("runtime requirement mismatch")
            && error.message.contains("HeapAlloc"),
        "unexpected error: {error}"
    );
}

#[test]
fn compile_mir_program_rejects_stale_heap_allocation_requirement_before_lowering() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "stale_heap_requirement");
    let instance_id = InstanceId(302);
    let function_id = DefId::new(CrateId(0), LocalDefId(302));
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mut type_context = crate::type_context::TypeContext::new();
    let unit = type_id(&mut type_context, Type::Unit);
    let function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: unit,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: unit,
        ownership: Default::default(),
    };
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        unit,
        false,
    );
    backend_contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, function)]),
        type_context,
        backend_contract,
    };

    let error = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        error.message.contains("runtime requirement mismatch")
            && error.message.contains("HeapAlloc"),
        "unexpected error: {error}"
    );
}

#[test]
fn mangle_function_symbol_distinguishes_separator_and_underscore_paths() {
    let first = CodeGen::mangle_function_symbol("foo::bar__baz");
    let second = CodeGen::mangle_function_symbol("foo__bar::baz");

    assert_eq!(first, "s3_666f6f_s8_6261725f5f62617a");
    assert_eq!(second, "s8_666f6f5f5f626172_s3_62617a");
    assert_ne!(first, second);
}

#[test]
fn mir_callable_resolved_key_missing_from_contract_fails_without_name_lookup() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let instance = InstanceId(210);
    let present_key = crate::mir::MirCallableKey::Instance(instance);
    let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(211));
    codegen.functions.insert(
        "Box_show_exact".to_string(),
        codegen.module.add_function(
            "Box_show_exact",
            context.i64_type().fn_type(&[], false),
            None,
        ),
    );
    codegen.functions.insert(
        "show".to_string(),
        codegen
            .module
            .add_function("show", context.i64_type().fn_type(&[], false), None),
    );
    codegen
        .callable_symbols_by_key
        .insert(present_key.clone(), "Box_show_exact".to_string());

    let symbol = codegen
        .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(present_key))
        .unwrap();

    assert_eq!(symbol, "Box_show_exact");

    let err = codegen
        .resolve_mir_callable_symbol(&crate::mir::MirCallable::Resolved(missing_key))
        .unwrap_err();

    assert!(
        err.message.contains("Unknown MIR callable key"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_uses_mir_body() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(1));
    let instance_id = InstanceId(0);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    99,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 99"), "IR was:\n{}", ir);
    assert!(!ir.contains("ret i64 1"), "IR was:\n{}", ir);
}

#[test]
fn mir_llvm_module_exposes_focused_compile_api() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(91));
    let instance_id = InstanceId(91);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    7,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    super::mir_llvm::compile_mir_program(&mut codegen, &mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 7"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_instance_from_mir_body_without_hir_body() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(7));
    let instance_id = InstanceId(7);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    type_id(&mut type_context, Type::I32);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "main".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    42,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "main",
        Vec::new(),
        i64_type,
        false,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("ret i64 42"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_instance_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(97));
    let instance_id = InstanceId(97);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "contract_instance".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    123,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "contract_instance_symbol".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.function_bodies.insert(mir_id.clone(), key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @contract_instance_symbol()"),
        "IR was:\n{}",
        ir
    );
    assert!(ir.contains("ret i64 123"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_rejects_missing_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let instance_id = InstanceId(98);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "missing_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    1,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract: Default::default(),
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_resolved_callable_missing_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(198));
    let instance_id = InstanceId(198);
    let missing_key = crate::mir::MirCallableKey::Instance(InstanceId(199));
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "missing_resolved_callable".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(missing_key.clone()),
                    )),
                    args: vec![],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "missing_resolved_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("ResolvedCallableWithoutContract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_wrong_callable_key_even_when_source_alias_registered() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let caller_def = DefId::new(CrateId(0), LocalDefId(201));
    let callee_def = DefId::new(CrateId(0), LocalDefId(202));
    let callee_instance = InstanceId(202);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Function(caller_def);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "source_alias_callable".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Function(
                            callee_def,
                        )),
                    )),
                    args: vec![],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Function(caller_def);
    let callee_key = crate::mir::MirCallableKey::Instance(callee_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(caller_def),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "source_alias_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        callee_key.clone(),
        crate::mir::MirCallableDecl {
            key: callee_key,
            source_def_id: Some(callee_def),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "callee_alias_callable".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("ResolvedCallableWithoutContract"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_direct_intrinsic_callable_without_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(199));
    let instance_id = InstanceId(200);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "direct_intrinsic_bypass".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(crate::mir::MirCallableKey::Intrinsic(
                            crate::mir::MirIntrinsicId::I64Add,
                        )),
                    )),
                    args: vec![
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "direct_intrinsic_bypass".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            || err.message.contains("Unknown MIR callable"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_lowers_resolved_intrinsic_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let function_id = DefId::new(CrateId(0), LocalDefId(200));
    let instance_id = InstanceId(201);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let intrinsic_key = crate::mir::MirCallableKey::Intrinsic(crate::mir::MirIntrinsicId::I64Add);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "resolved_intrinsic_contract".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Call {
                    func: crate::mir::Operand::Constant(crate::mir::Constant::Callable(
                        crate::mir::MirCallable::Resolved(intrinsic_key.clone()),
                    )),
                    args: vec![
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                    destination: crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                    span: None,
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "resolved_intrinsic_contract".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        intrinsic_key.clone(),
        crate::mir::MirCallableDecl {
            key: intrinsic_key.clone(),
            source_def_id: None,
            kind: crate::mir::MirCallableKind::Intrinsic(crate::mir::MirIntrinsicId::I64Add),
            llvm_symbol: "I64Add".to_string(),
            linkage: crate::mir::MirLinkage::Internal,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type, i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(codegen.callable_symbols_by_key.contains_key(&intrinsic_key));
    assert!(!ir.contains("call i64 @I64Add"), "IR was:\n{}", ir);
    assert!(ir.contains("ret i64"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_declares_extern_from_backend_contract_without_hir_extern() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let extern_id = DefId::new(CrateId(0), LocalDefId(90));
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let extern_key = crate::mir::MirCallableKey::Extern(extern_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        extern_key.clone(),
        crate::mir::MirCallableDecl {
            key: extern_key,
            source_def_id: Some(extern_id),
            kind: crate::mir::MirCallableKind::Extern {
                link_name: "ffi::answer".to_string(),
                variadic: false,
            },
            llvm_symbol: "ffi::answer".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::new(),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    assert!(codegen.module.get_function("answer").is_some());
    assert_eq!(
        codegen
            .callable_symbols_by_key
            .get(&crate::mir::MirCallableKey::Extern(extern_id))
            .map(String::as_str),
        Some("ffi::answer")
    );
    assert!(codegen.functions.contains_key("ffi::answer"));
}

#[test]
fn compile_mir_program_registers_struct_layout_from_backend_contract_without_hir_struct() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let struct_id = DefId::new(CrateId(0), LocalDefId(91));
    let instance_id = InstanceId(91);
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let point_type = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "make_point".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: Vec::new(),
                },
                crate::mir::Rvalue::Aggregate(
                    crate::mir::AggregateKind::Struct {
                        id: struct_id,
                        display_name: "Point".to_string(),
                    },
                    vec![
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(1)),
                        crate::mir::Operand::Constant(crate::mir::Constant::Int(2)),
                    ],
                ),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: point_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::UserBinding,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: point_type,
        ownership: Default::default(),
    };
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        DefId::new(CrateId(0), LocalDefId(92)),
        "make_point",
        Vec::new(),
        point_type,
        false,
    );
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("x".to_string(), i64_type), ("y".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(ir.contains("{ i64 1, i64 2 }"), "IR was:\n{}", ir);
    assert_eq!(
        codegen.struct_layouts_by_id.get(&struct_id),
        Some(&vec![
            ("x".to_string(), i64_type),
            ("y".to_string(), i64_type),
        ])
    );
}

#[test]
fn compile_mir_program_binds_shared_scalar_method_receiver_by_pointer() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let self_ref = Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    };
    let self_ref_type = type_id(&mut type_context, self_ref.clone());
    let function_id = DefId::new(CrateId(0), LocalDefId(80));
    let instance_id = InstanceId(0);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let self_place = crate::mir::Place {
        local: crate::mir::Local(1),
        projection: vec![],
    };
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "I64_identity".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: self_place.local,
                    projection: vec![crate::mir::Projection::Deref],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            crate::mir::LocalDecl {
                ty: self_ref_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("self".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "I64_identity",
        vec![self_ref_type],
        i64_type,
        true,
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @I64_identity(ptr %0)"),
        "IR was:\n{}",
        ir
    );
    assert!(ir.contains("%self = alloca ptr"), "IR was:\n{}", ir);
    assert!(ir.contains("store ptr %0, ptr %self"), "IR was:\n{}", ir);
    assert!(ir.contains("load i64"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_uses_contract_pass_mode_for_method_receiver_without_legacy_metadata() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let function_id = DefId::new(CrateId(0), LocalDefId(81));
    let instance_id = InstanceId(81);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "contract_only_shared_scalar_identity".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: crate::mir::Local(1),
                    projection: vec![],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("self".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let key = crate::mir::MirCallableKey::Instance(instance_id);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        key.clone(),
        crate::mir::MirCallableDecl {
            key: key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "contract_only_shared_scalar_identity".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature {
                params: vec![crate::mir::MirParamAbi {
                    semantic_ty: i64_type,
                    pass_mode: crate::mir::MirPassMode::Pointer,
                }],
                ret: crate::mir::MirReturnAbi {
                    semantic_ty: i64_type,
                    abi_ty: i64_type,
                },
            },
        },
    );
    backend_contract.function_bodies.insert(mir_id.clone(), key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define i64 @contract_only_shared_scalar_identity(ptr %0)"),
        "IR was:\n{}",
        ir,
    );
    assert!(ir.contains("load i64, ptr %0"), "IR was:\n{}", ir);
    assert!(ir.contains("store i64"), "IR was:\n{}", ir);
}

#[test]
fn compile_mir_program_rejects_drop_glue_contract_without_receiver_param() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(82));
    let box_type = Type::Struct {
        id: struct_id,
        args: Vec::new(),
    };
    let box_type_id = type_id(&mut type_context, box_type);
    let function_id = DefId::new(CrateId(0), LocalDefId(83));
    let instance_id = InstanceId(83);
    let drop_instance = InstanceId(84);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "drop_contract_missing_receiver".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Drop {
                    place: crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: box_type_id,
                mutability: crate::mir::Mutability::Not,
                name: Some("box_value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "drop_contract_missing_receiver".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(84))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                type_id(&mut type_context, Type::Unit),
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("receiver parameter"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_bad_signature() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(182));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function_id = DefId::new(CrateId(0), LocalDefId(183));
    let instance_id = InstanceId(183);
    let drop_instance = InstanceId(184);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_bad_drop_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    0,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_bad_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(184))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id, i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("expected 1 receiver parameter")
            && err.message.contains("expected ()"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unit_type = type_id(&mut type_context, Type::Unit);
    let struct_id = DefId::new(CrateId(0), LocalDefId(185));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(186));
    let instance_id = InstanceId(186);
    let drop_instance = InstanceId(187);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_drop_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    0,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(187))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id],
                unit_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(unknown_type, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("unknown dropped type"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_drop_glue_contract_with_unknown_return_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let struct_id = DefId::new(CrateId(0), LocalDefId(188));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(189));
    let instance_id = InstanceId(189);
    let drop_instance = InstanceId(190);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_return_drop_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    0,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_return_drop_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(190))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[box_type_id],
                unknown_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("unknown return semantic type"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_unused_callable_contract_with_unknown_type_id() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unknown_type = crate::ids::TypeId(u32::MAX);
    let function_id = DefId::new(CrateId(0), LocalDefId(191));
    let instance_id = InstanceId(191);
    let extern_instance = InstanceId(192);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "unused_unknown_callable_contract".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Constant(crate::mir::Constant::Int(
                    0,
                ))),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![crate::mir::LocalDecl {
            ty: i64_type,
            mutability: crate::mir::Mutability::Mut,
            name: Some("return_place".to_string()),
            span: None,
            source: crate::mir::LocalSource::ReturnPlace,
        }],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let extern_key = crate::mir::MirCallableKey::Instance(extern_instance);
    let mut backend_contract = local_instance_contract(
        &type_context,
        instance_id,
        function_id,
        "unused_unknown_callable_contract",
        Vec::new(),
        i64_type,
        false,
    );
    backend_contract.callables.insert(
        extern_key.clone(),
        crate::mir::MirCallableDecl {
            key: extern_key,
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(192))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "unknown_return_extern".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                unknown_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("unknown return semantic type"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_rejects_drop_glue_contract_with_wrong_receiver_pass_mode() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let unit_type = type_id(&mut type_context, Type::Unit);
    let struct_id = DefId::new(CrateId(0), LocalDefId(85));
    let box_type_id = type_id(
        &mut type_context,
        Type::Struct {
            id: struct_id,
            args: Vec::new(),
        },
    );
    let function_id = DefId::new(CrateId(0), LocalDefId(86));
    let instance_id = InstanceId(86);
    let drop_instance = InstanceId(87);
    let mir_id = crate::mir::MirFunctionId::Instance(instance_id);
    let mir_function = crate::mir::MirFunction {
        id: mir_id.clone(),
        name: "drop_contract_wrong_receiver".to_string(),
        basic_blocks: vec![
            crate::mir::BasicBlock {
                statements: vec![],
                terminator: Some(crate::mir::Terminator::Drop {
                    place: crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    target: crate::mir::BasicBlockId(1),
                }),
            },
            crate::mir::BasicBlock {
                statements: vec![crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(0),
                    )),
                    None,
                )],
                terminator: Some(crate::mir::Terminator::Return),
            },
        ],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: box_type_id,
                mutability: crate::mir::Mutability::Not,
                name: Some("box_value".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let caller_key = crate::mir::MirCallableKey::Instance(instance_id);
    let drop_key = crate::mir::MirCallableKey::Instance(drop_instance);
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        caller_key.clone(),
        crate::mir::MirCallableDecl {
            key: caller_key.clone(),
            source_def_id: Some(function_id),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: mir_id.clone(),
            },
            llvm_symbol: "drop_contract_wrong_receiver".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        drop_key.clone(),
        crate::mir::MirCallableDecl {
            key: drop_key.clone(),
            source_def_id: Some(DefId::new(CrateId(0), LocalDefId(87))),
            kind: crate::mir::MirCallableKind::ObjectProvided,
            llvm_symbol: "Box_drop".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature {
                params: vec![crate::mir::MirParamAbi {
                    semantic_ty: box_type_id,
                    pass_mode: crate::mir::MirPassMode::Pointer,
                }],
                ret: crate::mir::MirReturnAbi {
                    semantic_ty: unit_type,
                    abi_ty: unit_type,
                },
            },
        },
    );
    backend_contract
        .function_bodies
        .insert(mir_id.clone(), caller_key);
    backend_contract.drop_glue.insert(box_type_id, drop_key);
    backend_contract.nominal_layouts.insert(
        struct_id,
        crate::mir::MirNominalLayout::Struct {
            id: struct_id,
            fields: vec![("value".to_string(), i64_type)],
            generic_params: Vec::new(),
        },
    );
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(mir_id, mir_function)]),
        type_context,
        backend_contract,
    };

    let err = codegen.compile_program_from_mir(&mir).unwrap_err();

    assert!(
        err.message.contains("Invalid MIR backend contract")
            && err.message.contains("receiver pass mode"),
        "unexpected error: {err}"
    );
}

#[test]
fn compile_mir_program_declares_closure_callable_from_backend_contract() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let mut type_context = crate::type_context::TypeContext::new();
    let i64_type = type_id(&mut type_context, Type::I64);
    let callable_ty = Type::function(vec![Type::I64], Type::I64);
    let callable_type = type_id(&mut type_context, callable_ty.clone());
    let parent_def = DefId::new(CrateId(0), LocalDefId(88));
    let parent_id = crate::mir::MirFunctionId::Function(parent_def);
    let closure_id = crate::mir::MirClosureId {
        owner: parent_id.clone(),
        local_index: 0,
    };
    let closure_function_id = crate::mir::MirFunctionId::Closure(Box::new(closure_id.clone()));
    let parent_capture = crate::mir::MirClosureCapture {
        name: "captured".to_string(),
        local: crate::mir::Local(1),
        kind: crate::mir::MirClosureCaptureKind::ByValue,
        span: None,
    };
    let closure_capture = crate::mir::MirClosureCapture {
        name: "captured".to_string(),
        local: crate::mir::Local(2),
        kind: crate::mir::MirClosureCaptureKind::ByValue,
        span: None,
    };
    let parent = crate::mir::MirFunction {
        id: parent_id.clone(),
        name: "make_contract_closure".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![
                crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(1),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Use(crate::mir::Operand::Constant(
                        crate::mir::Constant::Int(5),
                    )),
                    None,
                ),
                crate::mir::StatementData::assign(
                    crate::mir::Place {
                        local: crate::mir::Local(0),
                        projection: vec![],
                    },
                    crate::mir::Rvalue::Closure(crate::mir::MirClosure {
                        id: closure_id.clone(),
                        display_name: "make_contract_closure.closure0".to_string(),
                        captures: vec![parent_capture],
                    }),
                    None,
                ),
            ],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: callable_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: callable_type,
        ownership: Default::default(),
    };
    let closure = crate::mir::MirFunction {
        id: closure_function_id.clone(),
        name: "make_contract_closure.closure0".to_string(),
        basic_blocks: vec![crate::mir::BasicBlock {
            statements: vec![crate::mir::StatementData::assign(
                crate::mir::Place {
                    local: crate::mir::Local(0),
                    projection: vec![],
                },
                crate::mir::Rvalue::Use(crate::mir::Operand::Copy(crate::mir::Place {
                    local: crate::mir::Local(2),
                    projection: vec![],
                })),
                None,
            )],
            terminator: Some(crate::mir::Terminator::Return),
        }],
        local_decls: vec![
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("arg".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            crate::mir::LocalDecl {
                ty: i64_type,
                mutability: crate::mir::Mutability::Not,
                name: Some("captured".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: vec![closure_capture],
        arg_count: 1,
        ret_type: i64_type,
        ownership: Default::default(),
    };
    let parent_key = crate::mir::MirCallableKey::Function(parent_def);
    let closure_key = crate::mir::MirCallableKey::Closure(closure_function_id.clone());
    let mut backend_contract = crate::mir::MirBackendContract::default();
    backend_contract.callables.insert(
        parent_key.clone(),
        crate::mir::MirCallableDecl {
            key: parent_key.clone(),
            source_def_id: Some(parent_def),
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: parent_id.clone(),
            },
            llvm_symbol: "make_contract_closure".to_string(),
            linkage: crate::mir::MirLinkage::External,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[],
                callable_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract.callables.insert(
        closure_key.clone(),
        crate::mir::MirCallableDecl {
            key: closure_key.clone(),
            source_def_id: None,
            kind: crate::mir::MirCallableKind::LocalBody {
                function_id: closure_function_id.clone(),
            },
            llvm_symbol: "contract_closure_symbol".to_string(),
            linkage: crate::mir::MirLinkage::Internal,
            signature: crate::mir::MirCallableSignature::from_type_ids(
                &[i64_type],
                i64_type,
                crate::mir::MirPassMode::Direct,
            ),
        },
    );
    backend_contract
        .function_bodies
        .insert(parent_id.clone(), parent_key);
    backend_contract
        .function_bodies
        .insert(closure_function_id.clone(), closure_key);
    backend_contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::HeapAlloc);
    let mir = crate::mir::MirProgram {
        functions: BTreeMap::from([(parent_id, parent), (closure_function_id, closure)]),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();

    let ir = codegen.get_ir();
    assert!(
        ir.contains("define internal i64 @contract_closure_symbol(ptr %0, i64 %1)"),
        "IR was:\n{}",
        ir,
    );
    assert!(!ir.contains("__mir_closure"), "IR was:\n{}", ir);
}

#[test]
fn codegen_source_operation_keeps_exact_origin_and_code() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
    let diagnostic =
        CodegenError::with_span("operation failed".to_string(), span.clone()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert!(diagnostic.primary.is_some());
}

#[test]
fn codegen_source_operation_hides_debug_identity_payloads() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 4, 9);
    let diagnostic = CodegenError::with_span(
        "MIR callable DefId { local: 1 } uses GenericParamId { index: 0 }".to_string(),
        span.clone(),
    )
    .into_diagnostic();

    assert_eq!(diagnostic.location, DiagnosticLocation::Source(span));
    assert_eq!(
        diagnostic.message,
        "Internal compiler error during MIR code generation"
    );
    assert!(diagnostic.primary.is_some());
}

#[test]
fn codegen_backend_contract_is_internal_and_non_source() {
    let diagnostic = CodegenError::backend_contract("invalid contract").into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Internal));
    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
}

#[test]
fn codegen_output_failure_is_file_classified() {
    let path = std::path::PathBuf::from("/tmp/main.o");
    let diagnostic = CodegenError::output("cannot write object", path.clone()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::File(path));
    assert!(diagnostic.primary.is_none());
}

#[test]
fn codegen_project_and_artifact_failures_keep_public_location_codes() {
    let project_path = std::path::PathBuf::from("/project/rock.toml");
    let artifact_path = std::path::PathBuf::from("/project/dep.rkca");

    let project = CodegenError::project("invalid project", project_path.clone()).into_diagnostic();
    let artifact =
        CodegenError::artifact("invalid artifact", artifact_path.clone()).into_diagnostic();

    assert_eq!(project.code, Some(DiagnosticCode::Project));
    assert_eq!(project.location, DiagnosticLocation::Project(project_path));
    assert_eq!(artifact.code, Some(DiagnosticCode::Artifact));
    assert_eq!(
        artifact.location,
        DiagnosticLocation::Artifact(artifact_path)
    );
}

#[test]
fn llvm_details_are_kept_as_notes_without_a_fabricated_source_span() {
    let diagnostic = CodegenError::new("LLVM rejected the module".to_string()).into_diagnostic();

    assert_eq!(diagnostic.code, Some(DiagnosticCode::Codegen));
    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
    assert_eq!(diagnostic.notes, vec!["LLVM rejected the module"]);
}

#[test]
fn backend_errors_cannot_be_reclassified_as_source_operations() {
    let span = crate::lexer::Span::new(std::path::PathBuf::from("/virtual/main.rk"), 1, 2);
    let diagnostic = CodegenError::new("backend failed".to_string())
        .with_operation_span(Some(span))
        .into_diagnostic();

    assert_eq!(diagnostic.location, DiagnosticLocation::Toolchain);
    assert!(diagnostic.primary.is_none());
}
