use super::{
    type_id_contains_reference, type_id_is_mut_reference, type_id_is_pointer, type_id_tracks_origin,
};
use crate::diagnostic::{DiagnosticCode, Diagnostics};
use crate::ids::{AssocTypeId, CrateId, DefId, InstanceId, LocalDefId};
use crate::mir::borrowck::BorrowChecker;
use crate::mir::{
    BasicBlock, Constant, DropObligationKind, Local, LocalDecl, MirAssert, MirAssertKind,
    MirBackendContract, MirCallable, MirCallableKey, MirClosure, MirClosureCapture,
    MirClosureCaptureKind, MirClosureId, MirDropObligation, MirEnumVariantLayout, MirFunction,
    MirFunctionId, MirNominalLayout, MirOwnershipMetadata, MirProgram, MirProjectionKey,
    MirVariantLayoutFields, Mutability, Operand, Place, Projection, ReferenceOrigin, Rvalue,
    StatementData, Terminator,
};
use crate::type_context::{TypeContext, TypeView};
use crate::types::{GenericParamId, Type};
use std::collections::{BTreeMap, HashMap, HashSet};

fn test_type_id(type_context: &mut TypeContext, ty: Type) -> crate::ids::TypeId {
    type_context.intern_type(&ty)
}

fn move_or_copy_out_of_reference_function(
    value_type: crate::ids::TypeId,
    reference_type: crate::ids::TypeId,
    source: Operand,
) -> MirFunction {
    MirFunction {
        id: MirFunctionId::Function(DefId::new(CrateId(0), LocalDefId(131))),
        name: "move_or_copy_out_of_reference".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                Place {
                    local: Local(2),
                    projection: Vec::new(),
                },
                Rvalue::Use(source),
                None,
            )],
            terminator: Some(Terminator::Return),
        }],
        local_decls: vec![
            LocalDecl {
                ty: value_type,
                mutability: Mutability::Mut,
                name: Some("return_place".to_string()),
                span: None,
                source: crate::mir::LocalSource::ReturnPlace,
            },
            LocalDecl {
                ty: reference_type,
                mutability: Mutability::Not,
                name: Some("reference".to_string()),
                span: None,
                source: crate::mir::LocalSource::Argument,
            },
            LocalDecl {
                ty: value_type,
                mutability: Mutability::Mut,
                name: Some("destination".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 1,
        ret_type: value_type,
        ownership: MirOwnershipMetadata::default(),
    }
}

mod conflicts;
mod drops;
mod escapes;
mod moves;
mod projections;
mod reference_returns;

impl BorrowChecker {
    pub(crate) fn check_function(
        func: &MirFunction,
        type_context: &crate::type_context::TypeContext,
    ) -> Result<(), Diagnostics> {
        Self::check_function_with_contract(
            func,
            type_context,
            &MirBackendContract::default(),
            &HashMap::new(),
            &Default::default(),
        )
    }
}
