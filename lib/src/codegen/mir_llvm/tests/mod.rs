mod aggregates;
mod bounds_checks;
mod casts;
mod closures;
mod drop_glue;
mod functions;
mod places;
mod references;
mod rvalues;

use std::cell::RefCell;

use std::collections::BTreeMap;

use std::panic::{catch_unwind, AssertUnwindSafe};

use inkwell::context::Context;

use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId, TypeId, VariantId};

use crate::mir::{
    AggregateKind, BasicBlock, Constant, Local, LocalDecl, MirAssert, MirAssertKind,
    MirBackendContract, MirBinOp, MirCallable, MirCallableDecl, MirCallableKey, MirCallableKind,
    MirCallableSignature, MirClosure, MirClosureCapture, MirClosureCaptureKind, MirClosureId,
    MirEnumVariantLayout, MirFunction, MirFunctionId, MirLinkage, MirNominalLayout, MirPassMode,
    MirProgram, MirProjectionKey, MirRuntimeHelper, MirUnaryOp, MirVariantLayoutFields, Mutability,
    Operand, Place, Projection, Rvalue, StatementData, Terminator,
};

use crate::type_context::TypeContext;

use crate::types::{AssociatedTypeKey, Type};

use super::super::{CodeGen, CodegenError};

fn root_place(local: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![],
    }
}

fn type_id(type_context: &RefCell<TypeContext>, ty: Type) -> crate::ids::TypeId {
    type_context.borrow_mut().intern_type(&ty)
}

fn local_body_callable_key(function_id: &MirFunctionId) -> MirCallableKey {
    match function_id {
        MirFunctionId::Function(id) => MirCallableKey::Function(*id),
        MirFunctionId::Extern(id) => MirCallableKey::Extern(*id),
        MirFunctionId::Instance(id) => MirCallableKey::Instance(*id),
        MirFunctionId::Closure(id) => MirCallableKey::Closure(MirFunctionId::Closure(id.clone())),
        MirFunctionId::ArrayTailDrop(ty) => MirCallableKey::RuntimeHelper(crate::mir::MirRuntimeHelper::ArrayTailDrop(*ty)),
    }
}

fn insert_local_body_contract(
    contract: &mut MirBackendContract,
    function_id: MirFunctionId,
    symbol: &str,
    params: &[TypeId],
    ret_ty: TypeId,
) {
    insert_local_body_contract_with_signature(
        contract,
        function_id,
        symbol,
        MirCallableSignature::from_type_ids(params, ret_ty, MirPassMode::Direct),
    );
}

fn insert_local_body_contract_with_signature(
    contract: &mut MirBackendContract,
    function_id: MirFunctionId,
    symbol: &str,
    signature: MirCallableSignature,
) {
    let key = local_body_callable_key(&function_id);
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key: key.clone(),
            source_def_id: None,
            kind: MirCallableKind::LocalBody {
                function_id: function_id.clone(),
            },
            llvm_symbol: symbol.to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
    contract.function_bodies.insert(function_id, key);
}

fn local_body_contract_with_params(
    function_id: MirFunctionId,
    symbol: &str,
    params: &[TypeId],
    ret_ty: TypeId,
) -> MirBackendContract {
    let mut contract = MirBackendContract::default();
    insert_local_body_contract(&mut contract, function_id, symbol, params, ret_ty);
    contract
}

fn local_body_contract(
    function_id: MirFunctionId,
    symbol: &str,
    ret_ty: TypeId,
) -> MirBackendContract {
    local_body_contract_with_params(function_id, symbol, &[], ret_ty)
}

fn local_body_contract_with_return_abi(
    function_id: MirFunctionId,
    symbol: &str,
    ret_ty: TypeId,
    ret_abi_ty: TypeId,
) -> MirBackendContract {
    let mut signature = MirCallableSignature::from_type_ids(&[], ret_ty, MirPassMode::Direct);
    signature.ret.abi_ty = ret_abi_ty;
    let mut contract = MirBackendContract::default();
    insert_local_body_contract_with_signature(&mut contract, function_id, symbol, signature);
    contract
}

fn insert_struct_layout_contract(
    contract: &mut MirBackendContract,
    id: DefId,
    fields: Vec<(String, TypeId)>,
) {
    contract.nominal_layouts.insert(
        id,
        MirNominalLayout::Struct {
            id,
            fields,
            generic_params: Vec::new(),
        },
    );
}

fn insert_enum_layout_contract(
    contract: &mut MirBackendContract,
    id: DefId,
    variants: Vec<MirEnumVariantLayout>,
) {
    contract.nominal_layouts.insert(
        id,
        MirNominalLayout::Enum {
            id,
            variants,
            generic_params: Vec::new(),
        },
    );
}

fn insert_object_callable_contract(
    contract: &mut MirBackendContract,
    key: MirCallableKey,
    symbol: &str,
    params: &[TypeId],
    ret_ty: TypeId,
) {
    insert_object_callable_contract_with_signature(
        contract,
        key,
        symbol,
        MirCallableSignature::from_type_ids(params, ret_ty, MirPassMode::Direct),
    );
}

fn insert_object_callable_contract_with_signature(
    contract: &mut MirBackendContract,
    key: MirCallableKey,
    symbol: &str,
    signature: MirCallableSignature,
) {
    contract.callables.insert(
        key.clone(),
        MirCallableDecl {
            key,
            source_def_id: None,
            kind: MirCallableKind::ObjectProvided,
            llvm_symbol: symbol.to_string(),
            linkage: MirLinkage::External,
            signature,
        },
    );
}

fn assert_valid_mir_fixture(program: &MirProgram) {
    let report = crate::mir::agreement::check_mir_runtime_agreement(program);
    assert!(
        report.is_clean(),
        "invalid MIR program fixture: {report:#?}"
    );
}

fn compile_valid_mir_program(
    codegen: &mut CodeGen<'_>,
    program: &MirProgram,
) -> Result<(), CodegenError> {
    assert_valid_mir_fixture(program);
    codegen.compile_program_from_mir(program)
}

struct MirProgramFixtureBuilder {
    functions: BTreeMap<MirFunctionId, MirFunction>,
    type_context: TypeContext,
    backend_contract: MirBackendContract,
}

impl MirProgramFixtureBuilder {
    pub(crate) fn new(type_context: TypeContext, backend_contract: MirBackendContract) -> Self {
        Self {
            functions: BTreeMap::new(),
            type_context,
            backend_contract,
        }
    }

    pub(crate) fn with_local_body(
        mut self,
        function: MirFunction,
        symbol: &str,
        params: &[TypeId],
    ) -> Self {
        let function_id = function.id.clone();
        let callable_key = local_body_callable_key(&function_id);
        assert!(
            !self.backend_contract.callables.contains_key(&callable_key)
                && !self
                    .backend_contract
                    .function_bodies
                    .contains_key(&function_id),
            "duplicate MIR local-body contract fixture"
        );
        insert_local_body_contract(
            &mut self.backend_contract,
            function_id.clone(),
            symbol,
            params,
            function.ret_type,
        );
        assert!(
            self.functions.insert(function_id, function).is_none(),
            "duplicate MIR function fixture"
        );
        self
    }

    pub(crate) fn build(self) -> MirProgram {
        let program = MirProgram {
            functions: self.functions,
            type_context: self.type_context,
            backend_contract: self.backend_contract,
        };
        assert_valid_mir_fixture(&program);
        program
    }
}

fn compile_test_function(
    codegen: &mut CodeGen<'_>,
    type_context: &RefCell<TypeContext>,
    name: &str,
    ret_ty: Type,
    params: Vec<Type>,
    local_decls: Vec<LocalDecl>,
    statements: Vec<StatementData>,
) {
    compile_test_function_result(
        codegen,
        type_context,
        name,
        ret_ty,
        params,
        local_decls,
        statements,
    )
    .unwrap();
}

fn compile_test_function_result(
    codegen: &mut CodeGen<'_>,
    type_context: &RefCell<TypeContext>,
    name: &str,
    ret_ty: Type,
    params: Vec<Type>,
    local_decls: Vec<LocalDecl>,
    statements: Vec<StatementData>,
) -> Result<(), CodegenError> {
    compile_test_function_result_with_contract(
        codegen,
        type_context,
        name,
        ret_ty,
        params,
        local_decls,
        statements,
        MirBackendContract::default(),
    )
}

fn compile_test_function_with_contract(
    codegen: &mut CodeGen<'_>,
    type_context: &RefCell<TypeContext>,
    name: &str,
    ret_ty: Type,
    params: Vec<Type>,
    local_decls: Vec<LocalDecl>,
    statements: Vec<StatementData>,
    backend_contract: MirBackendContract,
) {
    compile_test_function_result_with_contract(
        codegen,
        type_context,
        name,
        ret_ty,
        params,
        local_decls,
        statements,
        backend_contract,
    )
    .unwrap();
}

fn compile_test_function_result_with_contract(
    codegen: &mut CodeGen<'_>,
    type_context: &RefCell<TypeContext>,
    name: &str,
    ret_ty: Type,
    params: Vec<Type>,
    local_decls: Vec<LocalDecl>,
    statements: Vec<StatementData>,
    backend_contract: MirBackendContract,
) -> Result<(), CodegenError> {
    let function_id = MirFunctionId::Instance(InstanceId(0));
    let param_tys = params
        .iter()
        .cloned()
        .map(|param| type_id(type_context, param))
        .collect::<Vec<_>>();
    let ret_type = type_id(type_context, ret_ty.clone());
    let function = MirFunction {
        id: function_id.clone(),
        name: name.to_string(),
        basic_blocks: vec![BasicBlock {
            statements,
            terminator: Some(Terminator::Return),
        }],
        local_decls,
        closure_captures: Vec::new(),
        arg_count: params.len(),
        ret_type,
        ownership: Default::default(),
    };
    let mir = MirProgramFixtureBuilder::new(type_context.borrow().clone(), backend_contract)
        .with_local_body(function, name, &param_tys)
        .build();

    codegen.compile_program_from_mir(&mir)
}

fn local_decl(type_context: &RefCell<TypeContext>, ty: Type, name: &str) -> LocalDecl {
    LocalDecl {
        ty: type_id(type_context, ty),
        mutability: Mutability::Mut,
        name: Some(name.to_string()),
        span: None,
        source: crate::mir::LocalSource::UserBinding,
    }
}

fn bounds_check_contract() -> MirBackendContract {
    let mut contract = MirBackendContract::default();
    contract
        .runtime_requirements
        .insert(crate::mir::MirRuntimeHelper::BoundsCheck);
    contract
}

fn assert_ir_order(ir: &str, snippets: &[&str]) {
    let mut start = 0;
    for snippet in snippets {
        let found = ir[start..].find(snippet).unwrap_or_else(|| {
            let remaining: String = ir[start..].chars().take(1200).collect();
            panic!(
                "expected IR to contain {:?} after byte {}\nremaining IR:\n{}",
                snippet, start, remaining
            )
        });
        start += found + snippet.len();
    }
}

fn assert_ir_order_ignoring_aggregate_gep_index_widths(ir: &str, snippets: &[&str]) {
    // LLVM may print constant aggregate GEP indices as target pointer-sized.
    let normalized_ir = ir
        .lines()
        .map(|line| {
            if line.contains("getelementptr inbounds {") {
                normalize_i64_constant_gep_indices(line)
            } else if line.contains("getelementptr inbounds nuw {") {
                normalize_i64_constant_gep_indices(
                    &line.replace("getelementptr inbounds nuw {", "getelementptr inbounds {"),
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ir_order(&normalized_ir, snippets);
}

fn normalize_i64_constant_gep_indices(line: &str) -> String {
    let mut normalized = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(index) = rest.find(", i64 ") {
        normalized.push_str(&rest[..index]);
        let after_prefix = &rest[index + ", i64 ".len()..];
        let digits_len: usize = after_prefix
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .map(char::len_utf8)
            .sum();

        if digits_len == 0 {
            normalized.push_str(", i64 ");
        } else {
            normalized.push_str(", i32 ");
            normalized.push_str(&after_prefix[..digits_len]);
        }
        rest = &after_prefix[digits_len..];
    }

    normalized.push_str(rest);
    normalized
}

fn field_place(local: usize, index: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    }
}

fn index_place(local: usize, index_local: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![Projection::Index(Local(index_local))],
    }
}

fn downcast_field_place(local: usize, variant_id: VariantId, index: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![
            Projection::Downcast(variant_id),
            Projection::Field {
                index,
                identity: None,
            },
        ],
    }
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn mir_closure_symbol(
        parent_symbol: &str,
        closure_id: &crate::mir::identity::MirClosureId,
    ) -> String {
        format!(
            "__mir_closure_{}_{}_{}",
            Self::sanitize_symbol(parent_symbol),
            Self::mir_function_id_symbol_key(&closure_id.owner),
            closure_id.local_index
        )
    }
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn mir_function_id_symbol_key(id: &MirFunctionId) -> String {
        match id {
            MirFunctionId::Function(id) => format!("fn_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Extern(id) => format!("extern_{}_{}", id.crate_id.0, id.local.0),
            MirFunctionId::Instance(id) => format!("instance_{}", id.0),
            MirFunctionId::ArrayTailDrop(ty) => format!("array_tail_drop_{}", ty.0),
            MirFunctionId::Closure(id) => format!(
                "closure_{}_{}",
                Self::mir_function_id_symbol_key(&id.owner),
                id.local_index
            ),
        }
    }
}
