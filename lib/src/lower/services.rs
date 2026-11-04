use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::collect::resolver::ResolverTables;
use crate::infer::{constraints::ConstraintStore, InferenceEngine};
use crate::lower::diagnostics::LowerDiagnosticSink;
use crate::lower::items::LowerItems;
use crate::lower::module_context::{ModuleLoweringContext, ModuleSourceProvider};
use crate::lower::prelude::PreludeImports;
use crate::lower::scope::Scope;

macro_rules! lower_service_wrapper {
    ($name:ident, $inner:ty) => {
        pub(crate) struct $name($inner);

        impl $name {
            pub(crate) fn new(inner: $inner) -> Self {
                Self(inner)
            }

            // Generated for every service; only some of them are unwrapped.
            #[allow(dead_code)]
            pub(crate) fn into_inner(self) -> $inner {
                self.0
            }
        }

        impl Deref for $name {
            type Target = $inner;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}

lower_service_wrapper!(LowerInferenceService, InferenceEngine);
lower_service_wrapper!(LowerScopeService, Scope);
lower_service_wrapper!(LowerItemService, LowerItems);
lower_service_wrapper!(LowerDiagnosticService, LowerDiagnosticSink);
lower_service_wrapper!(LowerPreludeService, PreludeImports);
lower_service_wrapper!(LowerResolverService, ResolverTables);
lower_service_wrapper!(LowerDependencyResolverService, HashMap<String, ResolverTables>);
lower_service_wrapper!(LowerConstraintService, ConstraintStore);

pub(crate) struct LowerModuleService {
    context: ModuleLoweringContext,
    source_provider: ModuleSourceProvider,
}

impl LowerModuleService {
    pub(crate) fn new(context: ModuleLoweringContext) -> Self {
        Self {
            context,
            source_provider: ModuleSourceProvider::empty(),
        }
    }

    pub(crate) fn from_source_modules(
        source_modules: impl Into<crate::source_loader::SourceModuleSet>,
    ) -> Self {
        Self {
            context: ModuleLoweringContext::new(),
            source_provider: ModuleSourceProvider::new(source_modules),
        }
    }

    pub(crate) fn source_provider(&self) -> &ModuleSourceProvider {
        &self.source_provider
    }

    pub(crate) fn associate_current_crate_module_ids(
        &mut self,
        item_index: &crate::collect::item_index::ItemIndex,
    ) {
        self.context.associate_current_crate_module_ids(item_index);
    }

    pub(crate) fn into_loaded_module_paths(self) -> Vec<(String, std::path::PathBuf)> {
        self.source_provider.loaded_module_paths()
    }

    pub(crate) fn current_module_prefix(&self) -> Option<String> {
        self.context.current_module_prefix(&self.source_provider)
    }

    pub(crate) fn source_module_for_qualified_name(
        &self,
        qualified_name: &str,
    ) -> Option<crate::ast::Module> {
        self.source_provider
            .source_module_for_qualified_name(&self.context, qualified_name)
    }

    pub(crate) fn source_module_for_path(
        &self,
        path: &std::path::Path,
    ) -> Option<crate::ast::Module> {
        self.source_provider.source_module_for_path(path)
    }

    pub(crate) fn source_path_for_module_name(
        &self,
        qualified_module_name: &str,
    ) -> Option<std::path::PathBuf> {
        self.context
            .source_path_for_module_name(&self.source_provider, qualified_module_name)
    }

    pub(crate) fn has_loaded_root_name(&self, root_name: &str) -> bool {
        self.source_provider.has_loaded_root_name(root_name)
    }

    pub(crate) fn source_root_path(&self, root_name: &str) -> Option<std::path::PathBuf> {
        self.source_provider.source_root_path(root_name)
    }
}

impl Deref for LowerModuleService {
    type Target = ModuleLoweringContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl DerefMut for LowerModuleService {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

pub(crate) struct LowererServices {
    pub(crate) engine: LowerInferenceService,
    pub(crate) scope: LowerScopeService,
    pub(crate) items: LowerItemService,
    pub(crate) diagnostics: LowerDiagnosticService,
    pub(crate) modules: LowerModuleService,
    pub(crate) prelude: LowerPreludeService,
    pub(crate) resolver: LowerResolverService,
    pub(crate) dependency_resolvers: LowerDependencyResolverService,
    pub(crate) constraint_store: LowerConstraintService,
}

impl LowererServices {
    pub(crate) fn new(inject_prelude: bool) -> Self {
        Self {
            engine: LowerInferenceService::new(InferenceEngine::new()),
            scope: LowerScopeService::new(Scope::new()),
            items: LowerItemService::new(LowerItems::new()),
            diagnostics: LowerDiagnosticService::new(LowerDiagnosticSink::new()),
            modules: LowerModuleService::new(ModuleLoweringContext::new()),
            prelude: LowerPreludeService::new(PreludeImports::new(inject_prelude)),
            resolver: LowerResolverService::new(ResolverTables::default()),
            dependency_resolvers: LowerDependencyResolverService::new(HashMap::new()),
            constraint_store: LowerConstraintService::new(ConstraintStore::new()),
        }
    }
}
