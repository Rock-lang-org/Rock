use std::collections::{BTreeSet, HashMap};

use crate::collect::resolver::ResolverTables;
use crate::hir::{HirBlock, HirFunction, HirFunctionSig, HirImpl, HirImplOwner, HirTrait};
use crate::ids::{CrateId, DefId, IdGen, LocalDefId};
use crate::infer::ConstraintStore;
use crate::lexer::Span;
use crate::types::{GenericParamDecl, Type};

use super::*;

fn def_id(raw: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(raw))
}

fn function(id: DefId, name: &str, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: true,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn partial_hir(engine: InferenceEngine) -> PartialHir {
    PartialHir {
        functions: HashMap::new(),
        structs: HashMap::new(),
        enums: HashMap::new(),
        traits: HashMap::new(),
        impls: HashMap::new(),
        externs: HashMap::new(),
        type_aliases: HashMap::new(),
        engine,
        function_type_vars: HashMap::new(),
        import_aliases: HashMap::new(),
        loaded_module_paths: Vec::new(),
        constraint_store: ConstraintStore::new(),
        resolver: ResolverTables::default(),
        current_def_ids: BTreeSet::new(),
        root_crate_id: CrateId(0),
        local_def_ids: IdGen::new(),
        language_items: HirLanguageItems::default(),
        imported_effective_trait_methods: HashMap::new(),
        inference_sccs: HashMap::new(),
        inference_scc_order: Vec::new(),
        source_map: Default::default(),
    }
}

#[test]
fn all_impl_methods_finalize_strictly_without_ownership_metadata() {
    let mut engine = InferenceEngine::new();
    let own_ret = engine.fresh_type_var_at(Span::test());
    let external_ret = engine.fresh_type_var_at(Span::test());
    let own_method_id = def_id(1);
    let external_method_id = def_id(2);
    let mut hir = partial_hir(engine);
    hir.impls.insert(
        def_id(10),
        HirImpl {
            id: def_id(10),
            owner: HirImplOwner::Named("Point".to_string()),
            type_name: "Point".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: HashMap::new().into(),
            methods: HashMap::from([
                ("own".to_string(), function(own_method_id, "own", own_ret)),
                (
                    "external".to_string(),
                    function(external_method_id, "external", external_ret),
                ),
            ]),
        },
    );

    let errors = apply_finalization(&mut hir);

    assert_eq!(errors.len(), 2);
    assert_eq!(hir.impls[&def_id(10)].methods["own"].ret_type, Type::Error);
    assert_eq!(
        hir.impls[&def_id(10)].methods["external"].ret_type,
        Type::Error
    );
}

#[test]
fn trait_signatures_and_declaration_bounds_finalize_strictly() {
    let mut engine = InferenceEngine::new();
    let signature_param = engine.fresh_type_var();
    let signature_return = engine.fresh_type_var();
    let function_id = def_id(11);
    let signature_id = def_id(12);
    let generic = crate::types::GenericParamId {
        owner: signature_id,
        index: 0,
    };
    let mut hir = partial_hir(engine);
    hir.traits.insert(
        def_id(13),
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id: def_id(13),
            name: "Trait".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::from([(
                "method".to_string(),
                HirFunctionSig {
                    id: signature_id,
                    name: "method".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(generic, "T")],
                    params: vec![signature_param],
                    ret: signature_return,
                    generic_bounds: HashMap::from([(
                        generic,
                        vec![crate::types::TraitBound {
                            trait_id: def_id(14),
                            type_args: vec![Type::TypeVar(crate::ids::TypeVarId(2))],
                        }],
                    )])
                    .into(),
                    self_receiver: None,
                    is_unsafe: false,
                },
            )]),
        },
    );
    hir.functions.insert(
        function_id,
        function(function_id, "function", Type::Generic(generic)),
    );

    let errors = apply_finalization(&mut hir);

    assert_eq!(errors.len(), 3);
    assert_eq!(
        hir.traits[&def_id(13)].signatures["method"].params[0],
        Type::Error
    );
    assert_eq!(
        hir.traits[&def_id(13)].signatures["method"].ret,
        Type::Error
    );
    assert_eq!(hir.functions[&function_id].ret_type, Type::Generic(generic));
}
