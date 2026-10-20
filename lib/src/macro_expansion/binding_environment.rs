//! Compiler-owned definition environments, independent of caller dependency aliases.

use std::collections::{BTreeMap, HashMap};

use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::lexer::lossless::SyntaxContext;
use crate::products::ProductCrateIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DefinitionEnvironment {
    SourceScope(Vec<(String, SyntaxContext)>),
    Dependency(ProductCrateIdentity),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collect::resolver::ResolverTables;
    use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
    use crate::crate_system::{ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord};
    use crate::ids::{CrateId, LocalDefId};

    fn context(value: u32) -> SyntaxContext {
        serde::Deserialize::deserialize(serde::de::value::SeqDeserializer::<_, serde::de::value::Error>::new(std::iter::once(value))).unwrap()
    }

    fn dependency(alias: &str) -> (CrateContext, ProductCrateIdentity, DefId) {
        let identity = ProductCrateIdentity::local("runtime".into());
        let mut crates = CrateContext::new();
        let id = DefId::new(CrateId(7), LocalDefId(1));
        crates.product_crate_ids.insert(identity.clone(), CrateId(7));
        let mut interface = ArtifactCrateInterface::default();
        interface.root_export_ids.insert("helper".into(), ArtifactExport { source: "runtime::helper".into(), id });
        let metadata = ExternCrateMetadata::new(interface, ResolverTables::default(), BTreeMap::new());
        crates.add_extern_crate(ExternCrateRecord::new(
            CrateId(7), alias.into(), metadata, ExternCrateBodies::default(), ExternCrateLink::metadata_only(BTreeMap::new()),
        )).unwrap();
        (crates, identity, id)
    }

    #[test]
    fn macro_a_13_definition_dependency_uses_identity_not_alias_or_caller_shadow() {
        for alias in ["runtime", "renamed"] {
            let (crates, identity, id) = dependency(alias);
            let binding_context = context(101);
            let mut environments = BindingEnvironments::default();
            environments.record_dependency(binding_context, &identity, &[identity.clone()]).unwrap();
            let mut resolver = ResolverTables::default();
            resolver.definition_environments = environments.resolve(&crates).unwrap();
            resolver.binding_paths.insert(vec![("runtime".into(), SyntaxContext::ROOT), ("helper".into(), SyntaxContext::ROOT)], DefId::new(CrateId(0), LocalDefId(1)));
            assert_eq!(resolver.resolve_binding_path(None, &[("runtime".into(), binding_context), ("helper".into(), SyntaxContext::ROOT)]), Some(id));
            assert_eq!(resolver.resolve_binding_path(None, &[("other_spelling".into(), binding_context), ("helper".into(), binding_context)]), Some(id));
            assert_eq!(resolver.resolve_binding_path(None, &[("runtime".into(), binding_context), ("private".into(), SyntaxContext::ROOT)]), None);
            assert_eq!(resolver.resolve_binding_path(None, &[("runtime".into(), binding_context), ("helper".into(), context(102))]), None);
        }
    }

    #[test]
    fn macro_a_13_definition_dependency_rejects_undeclared_or_missing_provider() {
        let (crates, identity, _) = dependency("renamed");
        let mut environments = BindingEnvironments::default();
        assert!(environments.record_dependency(context(103), &identity, &[]).is_err());
        assert!(environments.record_dependency(SyntaxContext::ROOT, &identity, &[identity.clone()]).is_err());
        environments.record_dependency(context(103), &identity, &[identity.clone()]).unwrap();
        assert!(environments.record_source_scope(context(103), Vec::new()).is_err());
        assert!(environments.resolve(&crates).is_ok());
        assert!(environments.resolve(&CrateContext::new()).is_err());
        let mut only_transitive = CrateContext::new();
        only_transitive.product_crate_ids.insert(identity, CrateId(7));
        assert!(environments.resolve(&only_transitive).is_err());
    }

    #[test]
    fn macro_a_13_definition_scope_preserves_exact_generated_item_bindings() {
        let binding_context = context(131);
        let helper = DefId::new(CrateId(0), LocalDefId(1));
        let generated = DefId::new(CrateId(0), LocalDefId(2));
        let mut environments = BindingEnvironments::default();
        environments.record_source_scope(binding_context, vec![("tools".into(), SyntaxContext::ROOT)]).unwrap();
        let mut resolver = ResolverTables::default();
        resolver.definition_environments = environments.resolve(&CrateContext::new()).unwrap();
        resolver.binding_paths.insert(vec![("tools".into(), SyntaxContext::ROOT), ("helper".into(), SyntaxContext::ROOT)], helper);
        let path = vec![("helper".into(), binding_context)];
        assert_eq!(resolver.resolve_binding_path(None, &path), Some(helper));
        resolver.binding_paths.insert(path.clone(), generated);
        assert_eq!(resolver.resolve_binding_path(None, &path), Some(generated));
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct BindingEnvironments {
    environments: HashMap<SyntaxContext, DefinitionEnvironment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ResolvedDefinitionEnvironment {
    SourceScope(Vec<(String, SyntaxContext)>),
    PublicDependency(BTreeMap<String, DefId>),
}

impl BindingEnvironments {
    fn insert(&mut self, context: SyntaxContext, environment: DefinitionEnvironment) -> Result<(), String> {
        if context == SyntaxContext::ROOT {
            return Err("source root context cannot carry definition-side authority".into());
        }
        if self.environments.contains_key(&context) {
            return Err("syntax context already has a definition environment".into());
        }
        self.environments.insert(context, environment);
        Ok(())
    }

    pub(crate) fn record_source_scope(&mut self, context: SyntaxContext, scope: Vec<(String, SyntaxContext)>) -> Result<(), String> {
        self.insert(context, DefinitionEnvironment::SourceScope(scope))
    }

    pub(crate) fn record_dependency(
        &mut self,
        context: SyntaxContext,
        identity: &ProductCrateIdentity,
        declared_dependencies: &[ProductCrateIdentity],
    ) -> Result<(), String> {
        if !declared_dependencies.contains(identity) {
            return Err("definition-side path requested an undeclared dependency".into());
        }
        self.insert(context, DefinitionEnvironment::Dependency(identity.clone()))
    }

    pub(crate) fn resolve(&self, crates: &CrateContext) -> Result<HashMap<SyntaxContext, ResolvedDefinitionEnvironment>, String> {
        self.environments.iter().map(|(context, environment)| {
            let resolved = match environment {
                DefinitionEnvironment::SourceScope(path) => ResolvedDefinitionEnvironment::SourceScope(path.clone()),
                DefinitionEnvironment::Dependency(identity) => {
                    let id = crates.product_crate_ids.get(identity)
                        .ok_or_else(|| "definition-side dependency identity is not loaded".to_string())?;
                    let provider = crates.extern_crates().find(|provider| provider.crate_id() == *id)
                        .ok_or_else(|| "definition-side dependency was not explicitly supplied".to_string())?;
                    ResolvedDefinitionEnvironment::PublicDependency(provider.root_exports().iter()
                        .map(|(name, export)| (name.clone(), export.id)).collect())
                }
            };
            Ok((*context, resolved))
        }).collect()
    }
}
