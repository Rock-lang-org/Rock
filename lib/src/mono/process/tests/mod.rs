mod callee_params;
mod generic_functions;
mod method_calls;
mod static_and_object_backed;

use super::super::hir_types::HirImpl;

use super::*;

use crate::hir::{HirCallTarget, HirImplOwner, HirMethodCallTarget, HirParam};

use crate::ids::{CrateId, DefId, HirLocalId, LocalDefId};

use crate::mono::{
    InstanceImplOwner, InstanceKey, InstanceOrigin, InstanceRecord, InstanceSymbols,
};

use crate::types::ReceiverMode;

use crate::types::{GenericParamDecl, GenericParamId, Type};

use std::collections::HashMap;

fn empty_function(receiver: Option<ReceiverMode>) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "f".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: crate::types::Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: crate::types::Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: receiver,
        is_unsafe: false,
    }
}

fn generic_identity_for_process_test(id: DefId, name: &str) -> HirFunction {
    let generic_id = GenericParamId {
        owner: id,
        index: 0,
    };
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![GenericParamDecl::type_param(generic_id, "T")],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "value".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Generic(generic_id),
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::Generic(generic_id),
        body: HirBlock {
            stmts: vec![HirStmt::Return(Some(HirExpr {
                kind: HirExprKind::Var("value".to_string()),
                ty: Type::Generic(generic_id),
                span: crate::lexer::Span::test(),
            }))],
            ty: Type::Generic(generic_id),
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn callback_acceptor_for_process_test(id: DefId, name: &str, callback_ty: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "callback".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: callback_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

impl Monomorphizer {
    pub(crate) fn lookup_self_receiver(
        &self,
        target: Option<&HirMethodCallTarget>,
    ) -> Option<ReceiverMode> {
        let target = target?;
        if let Some(impl_id) = target.impl_id() {
            let method_id = target.method_id()?;
            return self
                .generic_impls
                .values()
                .chain(self.trait_impls.values().flat_map(|impls| impls.iter()))
                .find(|imp| imp.id == impl_id)
                .and_then(|imp| imp.methods.values().find(|method| method.id == method_id))
                .and_then(|method| method.self_receiver);
        }

        let trait_id = target.trait_id()?;
        let member_id = target.method_id()?;
        self.trait_impls
            .values()
            .flat_map(|impls| impls.iter())
            .filter(|imp| imp.trait_id == Some(trait_id))
            .find_map(|imp| imp.methods.values().find(|method| method.id == member_id))
            .and_then(|method| method.self_receiver)
    }
}
