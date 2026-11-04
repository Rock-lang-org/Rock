use std::collections::HashMap;

use super::*;
use crate::hir::HirBlock;
use crate::ids::{CrateId, LocalDefId};
use crate::types::{GenericParamDecl, GenericParamId, TraitBound, Type};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

fn function(
    id: DefId,
    name: &str,
    generic_bounds: HashMap<GenericParamId, Vec<TraitBound>>,
) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: vec![
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: 0,
                },
                "T",
            ),
            GenericParamDecl::type_param(
                GenericParamId {
                    owner: id,
                    index: 1,
                },
                "U",
            ),
        ],
        generic_bounds: generic_bounds.into(),
        params: vec![],
        ret_type: Type::Unit,
        body: HirBlock {
            stmts: vec![],
            ty: Type::Unit,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

#[test]
fn declaration_items_accept_equivalent_alias_clones_with_different_bound_insertion_order() {
    let id = def_id(1);
    let first_bound = TraitBound {
        trait_id: def_id(2),
        type_args: vec![],
    };
    let second_bound = TraitBound {
        trait_id: def_id(3),
        type_args: vec![],
    };
    let first_bounds = HashMap::from([
        (
            GenericParamId {
                owner: id,
                index: 0,
            },
            vec![first_bound.clone()],
        ),
        (
            GenericParamId {
                owner: id,
                index: 1,
            },
            vec![second_bound.clone()],
        ),
    ]);
    let second_bounds = HashMap::from([
        (
            GenericParamId {
                owner: id,
                index: 1,
            },
            vec![second_bound],
        ),
        (
            GenericParamId {
                owner: id,
                index: 0,
            },
            vec![first_bound],
        ),
    ]);

    let items = DeclarationItems::from_named_maps(
        HashMap::from([
            ("alias".to_string(), function(id, "alias", first_bounds)),
            (
                "canonical".to_string(),
                function(id, "canonical", second_bounds),
            ),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        vec![],
        vec![],
        &HashMap::from([(id, "canonical".to_string())]),
    )
    .expect("equivalent aliases should collapse");

    assert_eq!(items.function(id).unwrap().name, "canonical");
}

#[test]
fn declaration_items_reject_conflicting_alias_clones() {
    let id = def_id(1);
    let mut conflicting = function(id, "canonical", HashMap::new());
    conflicting.ret_type = Type::I64;

    let errors = DeclarationItems::from_named_maps(
        HashMap::from([
            ("alias".to_string(), function(id, "alias", HashMap::new())),
            ("canonical".to_string(), conflicting),
        ]),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        HashMap::new(),
        vec![],
        vec![],
        &HashMap::from([(id, "canonical".to_string())]),
    )
    .unwrap_err();

    assert!(errors[0]
        .message
        .contains("conflicting function declaration payloads"));
}

#[test]
fn declaration_item_diagnostics_are_deterministic_across_input_orders() {
    let function_ids = [def_id(7), def_id(4)];
    let extern_ids = [def_id(6), def_id(2)];
    let expected_named_errors = vec![
        format!(
            "conflicting function declaration payloads for canonical DefId {:?}",
            function_ids[1]
        ),
        format!(
            "conflicting function declaration payloads for canonical DefId {:?}",
            function_ids[0]
        ),
        format!(
            "duplicate extern declaration payload for canonical DefId {:?}",
            extern_ids[1]
        ),
        format!(
            "duplicate extern declaration payload for canonical DefId {:?}",
            extern_ids[0]
        ),
    ];

    for reverse in [false, true] {
        let mut function_entries = vec![
            ("seven-alias", function_ids[0], false),
            ("seven-canonical", function_ids[0], true),
            ("four-alias", function_ids[1], false),
            ("four-canonical", function_ids[1], true),
        ];
        let mut extern_entries = vec![
            HirExtern {
                id: extern_ids[0],
                name: "six-first".to_string(),
                params: vec![],
                ret: Type::Unit,
                variadic: false,
                is_unsafe: false,
            },
            HirExtern {
                id: extern_ids[1],
                name: "two-first".to_string(),
                params: vec![],
                ret: Type::Unit,
                variadic: false,
                is_unsafe: false,
            },
            HirExtern {
                id: extern_ids[0],
                name: "six-second".to_string(),
                params: vec![],
                ret: Type::Unit,
                variadic: false,
                is_unsafe: false,
            },
            HirExtern {
                id: extern_ids[1],
                name: "two-second".to_string(),
                params: vec![],
                ret: Type::Unit,
                variadic: false,
                is_unsafe: false,
            },
        ];
        if reverse {
            function_entries.reverse();
            extern_entries.reverse();
        }

        let functions = function_entries
            .into_iter()
            .map(|(name, id, conflicting)| {
                let mut function = function(id, name, HashMap::new());
                if conflicting {
                    function.ret_type = Type::I64;
                }
                (name.to_string(), function)
            })
            .collect();
        let errors = DeclarationItems::from_named_maps(
            functions,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            vec![],
            extern_entries,
            &HashMap::from([
                (function_ids[0], "seven-canonical".to_string()),
                (function_ids[1], "four-canonical".to_string()),
            ]),
        )
        .unwrap_err();
        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            expected_named_errors
        );
    }

    let expected_validation_errors = vec![
        format!(
            "function declaration key {:?} does not match payload DefId {:?}",
            def_id(3),
            def_id(12)
        ),
        format!(
            "function declaration key {:?} does not match payload DefId {:?}",
            def_id(9),
            def_id(11)
        ),
        format!(
            "struct declaration key {:?} does not match payload DefId {:?}",
            def_id(2),
            def_id(14)
        ),
        format!(
            "struct declaration key {:?} does not match payload DefId {:?}",
            def_id(8),
            def_id(13)
        ),
    ];

    for reverse in [false, true] {
        let mut functions = vec![(def_id(9), def_id(11)), (def_id(3), def_id(12))];
        let mut structs = vec![(def_id(8), def_id(13)), (def_id(2), def_id(14))];
        if reverse {
            functions.reverse();
            structs.reverse();
        }
        let functions = functions
            .into_iter()
            .map(|(key, id)| (key, function(id, "function", HashMap::new())))
            .collect();
        let structs = structs
            .into_iter()
            .map(|(key, id)| {
                (
                    key,
                    HirStruct {
                        id,
                        name: "structure".to_string(),
                        generic_params: vec![],
                        fields: vec![],
                    },
                )
            })
            .collect();
        let errors = DeclarationItems::from_id_maps(
            functions,
            HashMap::new(),
            structs,
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
            HashMap::new(),
        )
        .unwrap_err();
        assert_eq!(
            errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>(),
            expected_validation_errors
        );
    }
}

impl DeclarationItems {
    pub(crate) fn from_named_maps(
        functions: HashMap<String, HirFunction>,
        function_sigs: HashMap<String, HirFunctionSig>,
        structs: HashMap<String, HirStruct>,
        enums: HashMap<String, HirEnum>,
        traits: HashMap<String, HirTrait>,
        impls: Vec<HirImpl>,
        externs: Vec<HirExtern>,
        canonical_names_by_id: &HashMap<DefId, String>,
    ) -> Result<Self, Vec<ResolveError>> {
        Self::from_named_maps_with_aliases(
            functions,
            function_sigs,
            structs,
            enums,
            traits,
            impls,
            externs,
            HashMap::new(),
            canonical_names_by_id,
        )
    }
}

impl DeclarationItems {
    pub(crate) fn insert_trait_def(&mut self, trait_def: HirTrait) -> Option<HirTrait> {
        self.traits.insert(trait_def.id, trait_def)
    }

    pub(crate) fn insert_structure(&mut self, structure: HirStruct) -> Option<HirStruct> {
        self.structs.insert(structure.id, structure)
    }

    pub(crate) fn insert_enumeration(&mut self, enumeration: HirEnum) -> Option<HirEnum> {
        self.enums.insert(enumeration.id, enumeration)
    }

    pub(crate) fn insert_extern(&mut self, extern_def: HirExtern) -> Option<HirExtern> {
        self.externs.insert(extern_def.id, extern_def)
    }

    pub(crate) fn function_mut(&mut self, id: DefId) -> Option<&mut HirFunction> {
        self.functions.get_mut(&id)
    }

    pub(crate) fn structure_mut(&mut self, id: DefId) -> Option<&mut HirStruct> {
        self.structs.get_mut(&id)
    }

    pub(crate) fn extern_mut(&mut self, id: DefId) -> Option<&mut HirExtern> {
        self.externs.get_mut(&id)
    }

    pub(crate) fn enumeration_mut(&mut self, id: DefId) -> Option<&mut HirEnum> {
        self.enums.get_mut(&id)
    }
}
