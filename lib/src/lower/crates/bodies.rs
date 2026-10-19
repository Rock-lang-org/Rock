//! Crate body lowering methods

use crate::ast;
use crate::crate_system::CrateContext;

use crate::lower::Lowerer;

impl Lowerer {
    /// Report dependency capability errors before trait conformance.
    /// Executable dependency bodies remain accepted HIR and enter at monomorphization.
    pub(crate) fn lower_crate_trait_bodies(&mut self, ctx: &CrateContext) {
        for message in ctx.dependency_errors_for_phase("lowering") {
            self.diagnostics.push_toolchain_once(message);
        }
    }

    /// Report dependency capability errors before local body lowering.
    /// Dependency interfaces already supply declarations for name and type resolution.
    pub(crate) fn lower_crate_module_bodies(&mut self, ctx: &CrateContext) {
        for message in ctx.dependency_errors_for_phase("lowering") {
            self.diagnostics.push_toolchain_once(message);
        }
    }

    /// Inject module-local items as short-name aliases.
    /// Returns the list of alias keys added (for cleanup after body lowering).
    pub(crate) fn inject_module_local_aliases(
        &mut self,
        module: &ast::Module,
        prefix: &str,
        include_local_items: bool,
        record_resolver_aliases: bool,
    ) -> Vec<(String, Option<crate::ids::DefId>)> {
        let added: Vec<(String, Option<crate::ids::DefId>)> = Vec::new();
        let _ = record_resolver_aliases;
        for top_level in &module.top_levels {
            match top_level {
                ast::TopLevel::FunctionDecl(fd) => {
                    if !include_local_items || prefix.is_empty() {
                        continue;
                    }
                    let short = fd.name.name.clone();
                    let qualified = qualify_module_local_name(prefix, &short);
                    if self
                        .scope
                        .define_alias_to_existing(short.clone(), &qualified)
                    {}
                }
                ast::TopLevel::Extern(sig) => {
                    if !include_local_items || prefix.is_empty() {
                        continue;
                    }
                    let short = sig.name.name.clone();
                    let qualified = qualify_module_local_name(prefix, &short);
                    if self
                        .scope
                        .define_alias_to_existing(short.clone(), &qualified)
                    {}
                }
                ast::TopLevel::Import(path) => {
                    // Bring an explicitly imported name into scope as a short alias.
                    // e.g. `> std::ffi::libc::malloc` resolves to its canonical extern in HIR.
                    let names = match path {
                        crate::ast::Path::Ident(ip) => crate::lower::path_names(&ip.path),
                        crate::ast::Path::Type(tp) => crate::lower::path_names(&tp.path),
                    };
                    if let Some(short) = names.last().cloned() {
                        let qualified = qualify_module_local_path(self, prefix, &names);
                        if self
                            .scope
                            .define_alias_to_existing(short.clone(), &qualified)
                        {
                            self.record_module_local_alias(
                                prefix,
                                &short,
                                &qualified,
                                record_resolver_aliases,
                            );
                        }
                    }
                }
                ast::TopLevel::GlobImport(module_path) => {
                    if let Ok(targets) = self.glob_import_targets(module_path) {
                        for (short, qualified) in targets {
                            let qualified = qualify_module_local_target(self, prefix, &qualified);
                            if self
                                .scope
                                .define_alias_to_existing(short.clone(), &qualified)
                            {
                                self.record_module_local_alias(
                                    prefix,
                                    &short,
                                    &qualified,
                                    record_resolver_aliases,
                                );
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        added
    }

    fn record_module_local_alias(
        &mut self,
        prefix: &str,
        short: &str,
        qualified: &str,
        record_resolver_aliases: bool,
    ) {
        if !record_resolver_aliases {
            return;
        }

        let Some(id) = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_item_id_for_path(qualified)
        else {
            return;
        };
        let source = crate::lower::resolution::LowerResolutionContext::new(self)
            .canonical_name(id)
            .map(ToString::to_string)
            .unwrap_or_else(|| qualified.to_string());

        self.resolver
            .scoped_module_aliases
            .entry(prefix.to_string())
            .or_default()
            .insert(short.to_string(), id);
        self.resolver.item_names_by_id.entry(id).or_insert(source);
    }
}

fn qualify_module_local_path(lowerer: &Lowerer, prefix: &str, names: &[String]) -> String {
    let qualified = names.join("::");
    qualify_module_local_target(lowerer, prefix, &qualified)
}

fn qualify_module_local_name(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", prefix, name)
    }
}

fn qualify_module_local_target(lowerer: &Lowerer, prefix: &str, qualified: &str) -> String {
    if prefix.is_empty() {
        return qualified.to_string();
    }

    if qualified == prefix || qualified.starts_with(&format!("{}::", prefix)) {
        return qualified.to_string();
    }

    let root_prefix = prefix.split("::").next().unwrap_or(prefix);
    if qualified == root_prefix || qualified.starts_with(&format!("{}::", root_prefix)) {
        return qualified.to_string();
    }

    let first_segment = qualified.split("::").next().unwrap_or(qualified);
    let is_absolute = lowerer.modules.has_loaded_root_name(first_segment);

    if is_absolute {
        qualified.to_string()
    } else {
        format!("{}::{}", prefix, qualified)
    }
}
