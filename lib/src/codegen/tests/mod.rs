mod contract_requirements;
mod declarations;
mod drop_glue;
mod errors;
mod method_receivers;

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
