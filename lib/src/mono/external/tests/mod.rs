mod current_crate_instances;
mod loading;
mod object_backed_impls;

use std::collections::{BTreeMap, HashMap};

use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;

use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir};

use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};

use crate::hir::{
    HirCallTarget, HirImplOwner, HirMethodCallTarget, HirParam, HirVarRef, HirVarTarget,
};

use crate::ids::{CrateId, DefId, LocalDefId};

use crate::lexer::Span;

use crate::mono::hir_types::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirStmt};

use crate::types::Type;

use crate::types::{GenericParamDecl, ReceiverMode};

use super::*;

fn println_method(receiver_ty: Type) -> HirFunction {
    HirFunction {
        id: DefId::new(CrateId(0), LocalDefId(0)),
        name: "println".to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: receiver_ty,
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Move),
        is_unsafe: false,
    }
}

fn concrete_function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: ret_type,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn loaded_object_backed_crate(
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
) -> ExternCrateRecord {
    loaded_object_backed_crate_with_backend_symbols(
        interface_impls,
        cross_crate_impls,
        BTreeMap::new(),
    )
}

fn loaded_object_backed_crate_with_backend_symbols(
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    loaded_object_backed_crate_named_with_backend_symbols(
        CrateId(1),
        "std",
        interface_impls,
        cross_crate_impls,
        backend_symbols,
    )
}

fn loaded_object_backed_crate_named_with_backend_symbols(
    crate_id: CrateId,
    crate_name: &str,
    interface_impls: Vec<HirImpl>,
    cross_crate_impls: Vec<HirImpl>,
    backend_symbols: BTreeMap<DefId, String>,
) -> ExternCrateRecord {
    let interface_impls = interface_impls;

    let mut resolver = ResolverTables::default();
    let mut next_local_id = 0u32;
    for imp in &interface_impls {
        let owner_name = format!("{}::{}", crate_name, imp.type_name);
        let owner_def_id = DefId::new(CrateId(0), LocalDefId(next_local_id));
        next_local_id += 1;
        resolver.item_paths.insert(owner_name.clone(), owner_def_id);
        resolver.item_names_by_id.insert(owner_def_id, owner_name);

        for method in imp.methods.values() {
            let Some(backend_name) = backend_symbols.get(&method.id).cloned() else {
                continue;
            };
            let def_id = DefId::new(CrateId(0), LocalDefId(next_local_id));
            next_local_id += 1;
            let canonical_name = format!("{}::{}", crate_name, backend_name);
            resolver.item_paths.insert(canonical_name.clone(), def_id);
            resolver.item_names_by_id.insert(def_id, canonical_name);
        }
    }

    let mut interface = ArtifactCrateInterface::default();
    for imp in interface_impls {
        interface.insert_impl(imp);
    }

    ExternCrateRecord::new(
        crate_id,
        crate_name.to_string(),
        ExternCrateMetadata::new(interface, resolver, BTreeMap::new()),
        ExternCrateBodies::from_cross_crate_hir(ArtifactCrossCrateHir {
            generic_functions: BTreeMap::new(),
            traits_with_defaults: BTreeMap::new(),
            generic_impls: cross_crate_impls,
        }),
        ExternCrateLink::object(PathBuf::from("std.o"), backend_symbols),
    )
}

fn simple_hir_function(name: &str, _canonical_name: Option<&str>, id: DefId) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![],
        generic_bounds: HashMap::new().into(),
        params: vec![],
        ret_type: Type::I32,
        body: HirBlock {
            stmts: vec![],
            ty: Type::I32,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn wrapped_process_with_crates(
    mono: &mut Monomorphizer,
    program: crate::mono::hir_types::HirProgram,
    crate_ctx: &CrateContext,
) -> crate::mono::MonomorphizedProgram {
    let program = mono.process_with_crates(program, crate_ctx);
    let (instances, pre_mir_instance_bodies) =
        std::mem::replace(&mut mono.instances, crate::mono::InstanceRegistry::new()).into_parts();
    crate::mono::MonomorphizedProgram {
        program,
        instances,
        pre_mir_instance_bodies,
        generated_drop_instances: Default::default(),
        type_context: mono.type_context.clone(),
        source_map: crate::source_map::SemanticSourceMap::default(),
    }
}
