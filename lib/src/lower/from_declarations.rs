//! Construction of a `Lowerer` from collected declarations.

use super::*;

impl Lowerer {
    /// Construct a Lowerer from collected declarations (output of collect stage).
    /// Builds scope from the declarations so body lowering can proceed.
    pub fn from_declarations(
        decls: crate::collect::Declarations,
    ) -> Result<Self, Vec<ResolveError>> {
        use crate::types::Type;

        let crate::collect::Declarations {
            indexing_ids,
            item_index,
            resolver,
            current_def_ids,
            items,
            function_type_vars,
            infix_precedence,
            loaded_module_paths: _,
            type_vars,
            inject_prelude,
            loaded_prelude_export_ids,
            module_file_cache: _,
            source_modules,
            dependency_root_export_ids: _,
            source_map,
            language_items,
        } = decls;

        let lower_items = LowerItems::from_declarations(items)?;

        let mut scope = scope::Scope::new();

        // Populate scope from canonical resolver names, then fetch the exact function ID.
        let mut function_names: Vec<_> = resolver
            .item_names_by_id
            .iter()
            .map(|(id, name)| (name, id))
            .collect();
        function_names.sort_by_key(|(name, _)| *name);
        for (name, id) in function_names {
            let Some(func) = lower_items.function(*id) else {
                continue;
            };
            let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            scope.define_top_level(name.clone(), func_type, false);
        }
        let mut function_ids: Vec<_> = lower_items.functions().map(|(id, _)| id).collect();
        function_ids.sort();
        if let Some(id) = function_ids
            .into_iter()
            .find(|id| !resolver.item_names_by_id.contains_key(id))
        {
            return Err(vec![ResolveError::non_source(format!(
                "missing canonical function declaration name for DefId {id:?}"
            ))]);
        }
        // Note: struct names are NOT put in scope as values.
        // Use Type::method syntax (double-colon) for associated functions.
        // Populate scope from canonical resolver names, then fetch the exact enum ID.
        let mut enum_names: Vec<_> = lower_items.enumerations().map(|(id, _)| id).collect();
        enum_names.sort();
        for id in enum_names {
            let Some(name) = resolver.item_names_by_id.get(&id) else {
                return Err(vec![ResolveError::non_source(format!(
                    "missing canonical enum declaration name for DefId {id:?}"
                ))]);
            };
            let enum_def = lower_items
                .enumeration(id)
                .expect("enum ID collected from enum declaration map");
            scope.define_top_level(
                name.clone(),
                Type::Enum {
                    id: enum_def.id,
                    args: vec![],
                },
                false,
            );
        }
        // Populate scope from import aliases so short names (e.g. `add` for `utils::add`)
        // are visible during body lowering.
        for (short_name, def_id) in &resolver.import_aliases {
            if let Some(func) = lower_items.function(*def_id) {
                let param_types: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
                let func_type = Type::function_with_safety(
                    param_types,
                    func.ret_type.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
                );
                scope.define_alias(short_name.clone(), func_type, false);
            } else if let Some(ext) = lower_items.extern_def(*def_id) {
                let func_type = Type::function_with_safety(
                    ext.params.clone(),
                    ext.ret.clone(),
                    crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
                );
                scope.define_alias(short_name.clone(), func_type, false);
            }
        }

        // Populate scope from resolver canonical names and exact extern IDs. Short names come
        // from imports or module-local alias injection during body lowering.
        let mut extern_names: Vec<_> = resolver
            .item_names_by_id
            .iter()
            .filter_map(|(id, name)| {
                lower_items
                    .extern_def(*id)
                    .map(|extern_def| (name, extern_def))
            })
            .collect();
        extern_names.sort_by_key(|(name, _)| *name);
        for (name, ext) in extern_names {
            let func_type = Type::function_with_safety(
                ext.params.clone(),
                ext.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
            );
            scope.define_top_level(name.clone(), func_type, false);
        }
        let mut extern_ids: Vec<_> = lower_items.externs().map(|(id, _)| id).collect();
        extern_ids.sort();
        if let Some(id) = extern_ids
            .into_iter()
            .find(|id| !resolver.item_names_by_id.contains_key(id))
        {
            return Err(vec![ResolveError::non_source(format!(
                "missing canonical extern declaration name for DefId {id:?}"
            ))]);
        }

        let mut inference_engine =
            crate::infer::InferenceEngine::with_next_type_var(type_vars.next_raw());
        inference_engine.seed_var_spans(type_vars.var_spans().clone());
        let services = LowererServices {
            engine: LowerInferenceService::new(inference_engine),
            scope: LowerScopeService::new(scope),
            items: LowerItemService::new(lower_items),
            diagnostics: LowerDiagnosticService::new(
                crate::lower::diagnostics::LowerDiagnosticSink::new(),
            ),
            modules: LowerModuleService::from_source_modules(source_modules),
            prelude: LowerPreludeService::new(crate::lower::prelude::PreludeImports::with_exports(
                inject_prelude,
                loaded_prelude_export_ids,
            )),
            resolver: LowerResolverService::new(resolver),
            dependency_resolvers: LowerDependencyResolverService::new(HashMap::new()),
            constraint_store: LowerConstraintService::new(
                crate::infer::constraints::ConstraintStore::new(),
            ),
        };

        let mut lowerer = Self::from_services(services);
        lowerer.function_type_vars = function_type_vars;
        lowerer.infix_precedence = infix_precedence;
        lowerer.current_def_ids = current_def_ids;
        lowerer.root_crate_id = indexing_ids.root_crate_id();
        lowerer.local_def_ids = indexing_ids.into_local_def_ids();
        lowerer.item_index = item_index;
        lowerer.source_map = source_map;
        lowerer.language_items = language_items;
        lowerer.refresh_inference_normalization_env();
        lowerer.refresh_type_display_context();
        Ok(lowerer)
    }
}
