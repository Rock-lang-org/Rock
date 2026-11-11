mod completion;
mod finish;
mod kinds;
mod observation;
mod recording;

use crate::ast;
use crate::collect::item_index::{ItemIndex, ItemKind, SourceModuleMap};
use crate::collect::CollectedIdEnvironment;
use crate::crate_system::CrateContext;
use crate::ids::{AssocTypeId, DefId, ModuleId, VariantId};
use crate::language_items::{merge_language_item_providers_all, LanguageItemRole, LanguageItems};
use crate::lexer::Span;
use crate::lower::ResolveError;

#[derive(Clone)]
struct RootRecord {
    id: DefId,
    kind: ItemKind,
    span: Span,
}

#[derive(Clone)]
struct MethodRecord {
    id: DefId,
    owner_id: DefId,
    span: Span,
}

#[derive(Clone)]
struct AssociatedTypeRecord {
    id: AssocTypeId,
    owner_id: DefId,
    span: Span,
}

#[derive(Clone)]
struct VariantRecord {
    id: VariantId,
    owner_id: DefId,
    span: Span,
}

#[derive(Default)]
struct DropPartial {
    root: Option<RootRecord>,
    method: Option<MethodRecord>,
}

#[derive(Default)]
struct IndexPartial {
    root: Option<RootRecord>,
    output: Option<AssociatedTypeRecord>,
    method: Option<MethodRecord>,
}

#[derive(Default)]
struct FnPartial {
    root: Option<RootRecord>,
    output: Option<AssociatedTypeRecord>,
    method: Option<MethodRecord>,
}

#[derive(Default)]
struct TryPartial {
    root: Option<RootRecord>,
    output: Option<AssociatedTypeRecord>,
    residual: Option<AssociatedTypeRecord>,
    branch: Option<MethodRecord>,
}

#[derive(Default)]
struct FromResidualPartial {
    root: Option<RootRecord>,
    method: Option<MethodRecord>,
}

#[derive(Default)]
struct ControlFlowPartial {
    root: Option<RootRecord>,
    break_variant: Option<VariantRecord>,
    continue_variant: Option<VariantRecord>,
}

#[derive(Default)]
struct RangePartial {
    root: Option<RootRecord>,
    full_variant: Option<VariantRecord>,
    from_variant: Option<VariantRecord>,
    to_variant: Option<VariantRecord>,
    to_inclusive_variant: Option<VariantRecord>,
    exclusive_variant: Option<VariantRecord>,
    inclusive_variant: Option<VariantRecord>,
}

struct ProtocolError {
    protocol: LanguageItemRole,
    role: Option<LanguageItemRole>,
    error: ResolveError,
}

#[derive(Default)]
struct BindingState {
    sized: Option<RootRecord>,
    drop: DropPartial,
    fold: DropPartial,
    index: IndexPartial,
    index_mut: IndexPartial,
    fn_once: FnPartial,
    fn_mut: FnPartial,
    fn_trait: FnPartial,
    send: Option<RootRecord>,
    owned: Option<RootRecord>,
    sync: Option<RootRecord>,
    try_protocol: TryPartial,
    from_residual: FromResidualPartial,
    control_flow: ControlFlowPartial,
    range: RangePartial,
    errors: Vec<ProtocolError>,
}

pub(super) fn bind_current_language_items(
    module: &ast::Module,
    item_index: &ItemIndex,
    id_environment: &CollectedIdEnvironment,
    current_crate_name: Option<&str>,
    source_modules: &SourceModuleMap,
) -> Result<LanguageItems<DefId>, Vec<ResolveError>> {
    let Some(root_module_id) = item_index.module_id_by_path(&[]) else {
        return Err(vec![ResolveError::non_source(
            "language item collection requires an indexed root module".to_string(),
        )]);
    };
    let mut module_path = current_crate_name
        .map(|name| vec![name.to_string()])
        .unwrap_or_default();
    let mut state = BindingState::default();
    collect_language_items_in_module(
        &mut state,
        module,
        root_module_id,
        &mut module_path,
        item_index,
        id_environment,
        source_modules,
    );
    state.finish()
}

pub(super) fn merge_provided_language_items(
    current_crate_name: Option<&str>,
    provided: &LanguageItems<DefId>,
    crate_ctx: &CrateContext,
) -> Result<LanguageItems<DefId>, Vec<ResolveError>> {
    let providers = std::iter::once((current_crate_name.unwrap_or("<current>"), provided)).chain(
        crate_ctx
            .extern_crates()
            .filter(|dependency| {
                !crate_ctx.is_current_source_self_artifact(current_crate_name, dependency.name())
            })
            .map(|dependency| (dependency.name(), dependency.metadata().language_items())),
    );

    merge_language_item_providers_all(providers).map_err(|conflicts| {
        conflicts
            .into_iter()
            .map(|conflict| ResolveError::non_source(conflict.to_string()))
            .collect()
    })
}

fn collect_language_items_in_module(
    state: &mut BindingState,
    module: &ast::Module,
    module_id: ModuleId,
    module_path: &mut Vec<String>,
    item_index: &ItemIndex,
    id_environment: &CollectedIdEnvironment,
    source_modules: &SourceModuleMap,
) {
    for (top_level_index, top_level) in module.top_levels.iter().enumerate() {
        match top_level {
            ast::TopLevel::Module(module_decl) => {
                let inner = &module_decl.0;
                let Some(name) = &inner.name else {
                    continue;
                };
                let Some(child_module_id) = item_index.child_module_id(module_id, &name.name)
                else {
                    continue;
                };
                module_path.push(name.name.clone());
                collect_language_items_in_module(
                    state,
                    inner,
                    child_module_id,
                    module_path,
                    item_index,
                    id_environment,
                    source_modules,
                );
                module_path.pop();
            }
            ast::TopLevel::Mod(name, _) => {
                let Some(child_module_id) = item_index.child_module_id(module_id, &name.name)
                else {
                    continue;
                };
                module_path.push(name.name.clone());
                if let Some(source_module) = source_modules.get(&*module_path) {
                    collect_language_items_in_module(
                        state,
                        source_module,
                        child_module_id,
                        module_path,
                        item_index,
                        id_environment,
                        source_modules,
                    );
                }
                module_path.pop();
            }
            ast::TopLevel::TraitDecl(trait_decl) => state.observe_trait(
                trait_decl,
                item_index.item_at_source(module_id, top_level_index),
                id_environment,
            ),
            ast::TopLevel::EnumDecl(enum_decl) => state.observe_enum(
                enum_decl,
                item_index.item_at_source(module_id, top_level_index),
            ),
            _ => {}
        }
    }
}
