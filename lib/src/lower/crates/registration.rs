//! Crate registration methods

#![allow(dead_code)]

use crate::crate_system::{CrateContext, ExternCrateRef};
use crate::lower::Lowerer;
use crate::types::Type;

pub(crate) struct LowerCrateRegistration<'a> {
    ctx: &'a CrateContext,
}

impl<'a> LowerCrateRegistration<'a> {
    pub(crate) fn new(ctx: &'a CrateContext) -> Self {
        Self { ctx }
    }

    pub(crate) fn register_crate_resolvers(&self, lowerer: &mut Lowerer) {
        for message in self.ctx.dependency_errors_for_phase("lowering") {
            lowerer.diagnostics.push_toolchain_once(message);
        }

        for dep in self.ctx.extern_crates() {
            let resolver = dep.metadata().resolver().clone();
            lowerer.resolver.merge_global_inputs(&resolver);
            lowerer
                .dependency_resolvers
                .insert(dep.name().to_string(), resolver);
        }
    }

    /// Register functions from external crates in the scope
    ///
    /// Functions are registered with qualified names only
    /// (e.g., `std::sqrt`, `mycrate::function`).
    /// Unqualified access must come from normal imports or std prelude injection.
    pub(crate) fn register_crate_functions(&self, lowerer: &mut Lowerer) {
        self.register_crate_resolvers(lowerer);

        for dep in self.ctx.extern_crates() {
            self.register_extern_crate(lowerer, dep);
        }
        lowerer.refresh_inference_normalization_env();
    }

    pub(crate) fn register_extern_crate(&self, lowerer: &mut Lowerer, dep: ExternCrateRef<'_>) {
        let crate_name = dep.name();
        let metadata = dep.metadata();
        let interface = metadata.interface();

        lowerer.imported_effective_trait_methods.extend(
            interface
                .effective_trait_methods
                .iter()
                .map(|(&key, &method_id)| (key, method_id)),
        );

        lowerer
            .prelude
            .capture_loaded_prelude_exports(crate_name, dep.prelude_exports());

        for (name, func) in interface.function_items() {
            let param_types: Vec<Type> = func.params.iter().map(|param| param.ty.clone()).collect();
            let func_type = Type::function_with_safety(
                param_types,
                func.ret_type.clone(),
                crate::types::FunctionSafety::from_is_unsafe(func.is_unsafe),
            );
            lowerer
                .scope
                .define_top_level(name.clone(), func_type, false);
            lowerer.items.insert_function(func.clone());
        }

        for ext in interface.extern_items() {
            let Some(name) = lowerer.canonical_name_for_def_id(ext.id).map(str::to_owned) else {
                lowerer.diagnostics.push_toolchain_once(format!(
                    "missing canonical extern declaration name for DefId {:?}",
                    ext.id
                ));
                lowerer.items.insert_extern(ext);
                continue;
            };
            let func_type = Type::function_with_safety(
                ext.params.clone(),
                ext.ret.clone(),
                crate::types::FunctionSafety::from_is_unsafe(ext.is_unsafe),
            );
            lowerer.scope.define_top_level(name, func_type, false);
            lowerer.items.insert_extern(ext);
        }

        for (_, strukt) in interface.struct_items() {
            lowerer.items.insert_structure(strukt.clone());
        }

        for (_, enum_) in interface.enum_items() {
            lowerer.items.insert_enumeration(enum_.clone());
        }

        for (_, alias) in interface.type_alias_items() {
            lowerer.items.insert_type_alias(alias);
        }

        for (_, trait_) in interface.trait_items() {
            lowerer.items.insert_trait_def(trait_.clone());
        }

        for imp in interface.impl_items() {
            if let Err(error) = lowerer.items.insert_impl(imp.clone()) {
                lowerer.diagnostics.push_toolchain(error.message);
            }
        }

        for (name, precedence) in &interface.infix_precedence {
            lowerer.infix_precedence.insert(name.clone(), *precedence);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::path::PathBuf;

    use crate::ast::Module;
    use crate::collect::resolver::ResolverTables;
    use crate::crate_artifact::ArtifactCrateInterface;
    use crate::crate_system::{
        CrateConfig, CrateContext, CrateManifest, ExternCrateBodies, ExternCrateLink,
        ExternCrateMetadata, ExternCrateRecord, LibConfig,
    };
    use crate::hir::{HirBlock, HirFunction, HirImpl, HirImplOwner};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lower::crates::registration::LowerCrateRegistration;
    use crate::lower::Lowerer;
    use crate::types::Type;

    #[test]
    fn source_backed_crate_does_not_register_module_path() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx.register_crate(
            CrateManifest {
                crate_: CrateConfig {
                    name: "dep".to_string(),
                    version: "0.1.0".to_string(),
                    no_std: false,
                },
                lib: LibConfig {
                    path: "lib.rk".to_string(),
                },
                dependencies: None,
            },
            PathBuf::from("/dep"),
            Module {
                name: None,
                top_levels: vec![],
                is_inline: false,
                filepath: None,
            },
        );

        let mut lowerer = Lowerer::new_for_test();

        LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

        assert!(lowerer.modules.source_module_paths().is_empty());
        assert_eq!(lowerer.errors().len(), 1);
        assert!(lowerer.errors()[0]
            .message
            .contains("source-backed external dependency 'dep' is not supported during lowering"));
    }

    #[test]
    fn extern_artifact_crate_does_not_register_module_path() {
        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(1),
                "dep".to_string(),
                ExternCrateMetadata::new(
                    ArtifactCrateInterface::default(),
                    ResolverTables::default(),
                    BTreeMap::new(),
                ),
                ExternCrateBodies::default(),
                ExternCrateLink::metadata_only(BTreeMap::new()),
            ))
            .unwrap();
        let mut lowerer = Lowerer::new_for_test();

        LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

        assert!(lowerer.errors().is_empty());
        assert!(lowerer.modules.source_module_paths().is_empty());
    }

    #[test]
    fn lower_registration_preserves_object_backed_impl_method_source_name() {
        let mut interface = ArtifactCrateInterface::default();
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            HirFunction {
                id: DefId::new(CrateId(0), LocalDefId(2)),
                name: "show".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        interface.insert_impl(HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(1)),
            owner: HirImplOwner::Named("DepThing".to_string()),
            type_name: "DepThing".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: None,
            trait_id: None,
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods,
        });

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(1),
                "dep".to_string(),
                ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new()),
                ExternCrateBodies::default(),
                ExternCrateLink::object(
                    PathBuf::from("/dep/dep.o"),
                    BTreeMap::from([(
                        DefId::new(CrateId(0), LocalDefId(2)),
                        "DepThing_show".to_string(),
                    )]),
                ),
            ))
            .unwrap();

        let mut lowerer = Lowerer::new_for_test();
        LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

        let method = lowerer
            .items
            .impl_def(DefId::new(CrateId(0), LocalDefId(1)))
            .unwrap()
            .methods
            .get("show")
            .expect("object-backed method should remain attached to its impl");
        assert_eq!(method.name, "show");
    }

    #[test]
    fn lower_registration_keeps_trait_impl_methods_attached_to_impl() {
        let mut interface = ArtifactCrateInterface::default();
        let mut methods = HashMap::new();
        methods.insert(
            "show".to_string(),
            HirFunction {
                id: DefId::new(CrateId(0), LocalDefId(3)),
                name: "show".to_string(),
                generic_params: Vec::new(),
                generic_bounds: HashMap::new().into(),
                params: Vec::new(),
                ret_type: Type::I64,
                body: HirBlock {
                    stmts: Vec::new(),
                    ty: Type::I64,
                },
                is_curried: false,
                is_method: true,
                self_receiver: None,
                is_unsafe: false,
            },
        );
        interface.insert_impl(HirImpl {
            id: DefId::new(CrateId(0), LocalDefId(2)),
            owner: HirImplOwner::Named("DepThing".to_string()),
            type_name: "DepThing".to_string(),
            type_generics: Vec::new(),
            receiver_pattern: Vec::new().into(),
            trait_name: Some("Show".to_string()),
            trait_id: Some(DefId::new(CrateId(0), LocalDefId(1))),
            trait_generics: Vec::new(),
            trait_arg_types: Vec::new(),
            associated_types: Vec::new(),
            bounds: std::collections::HashMap::new().into(),
            methods,
        });

        let mut crate_ctx = CrateContext::new();
        crate_ctx
            .add_extern_crate(ExternCrateRecord::new(
                CrateId(1),
                "dep".to_string(),
                ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new()),
                ExternCrateBodies::default(),
                ExternCrateLink::object(PathBuf::from("/dep/dep.o"), BTreeMap::new()),
            ))
            .unwrap();

        let mut lowerer = Lowerer::new_for_test();
        LowerCrateRegistration::new(&crate_ctx).register_crate_functions(&mut lowerer);

        assert!(lowerer
            .items
            .impl_def(DefId::new(CrateId(0), LocalDefId(2)))
            .unwrap()
            .methods
            .contains_key("show"));
    }
}
