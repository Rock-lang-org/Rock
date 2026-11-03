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

mod callables;
mod layouts;
mod malformed_mir;
mod projection_outputs;
mod runtime_requirements;
mod type_ids;
