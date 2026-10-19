use crate::ast::{self, LanguageItemMemberKind};
use crate::collect::item_index::{ItemIndex, ItemKind, SourceModuleMap};
use crate::collect::CollectedIdEnvironment;
use crate::crate_system::CrateContext;
use crate::ids::{AssocTypeId, DefId, ModuleId, VariantId};
use crate::language_items::{
    merge_language_item_providers_all, DropLanguageItems, FnLanguageItems, FnMutLanguageItems,
    FnOnceLanguageItems, FoldLanguageItems, IndexLanguageItems, IndexMutLanguageItems,
    LanguageItemRole, LanguageItems, RangeLanguageItems, SendLanguageItems, SizedLanguageItems,
    SyncLanguageItems, TryLanguageItems,
};
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

impl BindingState {
    fn observe_trait(
        &mut self,
        trait_decl: &ast::TraitDecl,
        item: Option<&crate::collect::item_index::ItemRecord>,
        id_environment: &CollectedIdEnvironment,
    ) {
        let Some(root_marker) = &trait_decl.language_items.root else {
            self.reject_orphan_children(&trait_decl.language_items.members);
            return;
        };
        let role = root_marker.role;
        let Some(expected_kind) = root_expected_kind(role) else {
            self.error(
                role,
                None,
                format!("language item {role} cannot mark a root declaration"),
                root_marker.span.clone(),
            );
            return;
        };
        if expected_kind != ItemKind::Trait {
            self.error(
                role,
                None,
                format!("language item {role} root must be an enum"),
                root_marker.span.clone(),
            );
            return;
        }
        let Some(item) = item.filter(|item| item.kind == ItemKind::Trait) else {
            self.error(
                role,
                None,
                format!("language item {role} root has no indexed trait identity"),
                root_marker.span.clone(),
            );
            return;
        };
        if !trait_decl.exported {
            self.error(
                role,
                None,
                format!("language item {role} root must be exported"),
                root_marker.span.clone(),
            );
            return;
        }
        let root = RootRecord {
            id: item.def_id,
            kind: item.kind,
            span: root_marker.span.clone(),
        };
        if !self.insert_root(role, root) {
            return;
        }
        for marker in &trait_decl.language_items.members {
            self.observe_trait_member(role, item.def_id, trait_decl, marker, id_environment);
        }
    }

    fn observe_enum(
        &mut self,
        enum_decl: &ast::EnumDecl,
        item: Option<&crate::collect::item_index::ItemRecord>,
    ) {
        let Some(root_marker) = &enum_decl.language_items.root else {
            self.reject_orphan_children(&enum_decl.language_items.members);
            return;
        };
        let role = root_marker.role;
        let Some(expected_kind) = root_expected_kind(role) else {
            self.error(
                role,
                None,
                format!("language item {role} cannot mark a root declaration"),
                root_marker.span.clone(),
            );
            return;
        };
        if expected_kind != ItemKind::Enum {
            self.error(
                role,
                None,
                format!("language item {role} root must be a trait"),
                root_marker.span.clone(),
            );
            return;
        }
        let Some(item) = item.filter(|item| item.kind == ItemKind::Enum) else {
            self.error(
                role,
                None,
                format!("language item {role} root has no indexed enum identity"),
                root_marker.span.clone(),
            );
            return;
        };
        if !enum_decl.exported {
            self.error(
                role,
                None,
                format!("language item {role} root must be exported"),
                root_marker.span.clone(),
            );
            return;
        }
        let root = RootRecord {
            id: item.def_id,
            kind: item.kind,
            span: root_marker.span.clone(),
        };
        if !self.insert_root(role, root) {
            return;
        }
        for marker in &enum_decl.language_items.members {
            self.observe_enum_member(role, item.def_id, enum_decl, marker);
        }
    }

    fn reject_orphan_children(&mut self, markers: &[ast::LanguageItemMemberMarker]) {
        for marker in markers {
            self.error(
                marker.marker.role,
                Some(marker.marker.role),
                format!(
                    "language item {} must belong to an annotated protocol root",
                    marker.marker.role
                ),
                marker.marker.span.clone(),
            );
        }
    }

    fn observe_trait_member(
        &mut self,
        protocol: LanguageItemRole,
        owner_id: DefId,
        trait_decl: &ast::TraitDecl,
        marker: &ast::LanguageItemMemberMarker,
        id_environment: &CollectedIdEnvironment,
    ) {
        let role = marker.marker.role;
        let Some(expected_kind) = expected_child_kind(protocol, role) else {
            self.error(
                protocol,
                Some(role),
                format!("language item {role} is not valid for {protocol}"),
                marker.marker.span.clone(),
            );
            return;
        };
        if expected_kind != marker.kind {
            self.error(
                protocol,
                Some(role),
                format!(
                    "language item {protocol}.{role} must mark an {}",
                    child_kind_name(expected_kind)
                ),
                marker.marker.span.clone(),
            );
            return;
        }

        match marker.kind {
            LanguageItemMemberKind::AssociatedType => {
                let Some(index) = trait_decl
                    .associated_types
                    .iter()
                    .position(|assoc| assoc.name.name == marker.member_name)
                else {
                    self.missing_member(protocol, role, &marker.member_name, &marker.marker.span);
                    return;
                };
                self.insert_associated_type(
                    protocol,
                    role,
                    AssociatedTypeRecord {
                        id: AssocTypeId(index as u32),
                        owner_id,
                        span: marker.marker.span.clone(),
                    },
                );
            }
            LanguageItemMemberKind::Method => {
                let Some(member_ids) = id_environment.trait_member_ids_by_owner.get(&owner_id)
                else {
                    self.missing_member(protocol, role, &marker.member_name, &marker.marker.span);
                    return;
                };
                let method_id = member_ids
                    .methods
                    .get(&marker.member_name)
                    .or_else(|| member_ids.signatures.get(&marker.member_name));
                let Some(&id) = method_id else {
                    self.missing_member(protocol, role, &marker.member_name, &marker.marker.span);
                    return;
                };
                self.insert_method(
                    protocol,
                    role,
                    MethodRecord {
                        id,
                        owner_id,
                        span: marker.marker.span.clone(),
                    },
                );
            }
            LanguageItemMemberKind::Variant => self.error(
                protocol,
                Some(role),
                format!("language item {protocol}.{role} must mark a trait member"),
                marker.marker.span.clone(),
            ),
        }
    }

    fn observe_enum_member(
        &mut self,
        protocol: LanguageItemRole,
        owner_id: DefId,
        enum_decl: &ast::EnumDecl,
        marker: &ast::LanguageItemMemberMarker,
    ) {
        let role = marker.marker.role;
        let Some(expected_kind) = expected_child_kind(protocol, role) else {
            self.error(
                protocol,
                Some(role),
                format!("language item {role} is not valid for {protocol}"),
                marker.marker.span.clone(),
            );
            return;
        };
        if expected_kind != marker.kind {
            self.error(
                protocol,
                Some(role),
                format!(
                    "language item {protocol}.{role} must mark an {}",
                    child_kind_name(expected_kind)
                ),
                marker.marker.span.clone(),
            );
            return;
        }
        if marker.kind != LanguageItemMemberKind::Variant {
            self.error(
                protocol,
                Some(role),
                format!("language item {protocol}.{role} must mark an enum variant"),
                marker.marker.span.clone(),
            );
            return;
        }
        let Some(index) = enum_decl
            .variants
            .iter()
            .position(|variant| variant.name.name == marker.member_name)
        else {
            self.missing_member(protocol, role, &marker.member_name, &marker.marker.span);
            return;
        };
        self.insert_variant(
            protocol,
            role,
            VariantRecord {
                id: VariantId(index as u32),
                owner_id,
                span: marker.marker.span.clone(),
            },
        );
    }

    fn missing_member(
        &mut self,
        protocol: LanguageItemRole,
        role: LanguageItemRole,
        name: &str,
        span: &Span,
    ) {
        self.error(
            protocol,
            Some(role),
            format!("language item {protocol}.{role} targets missing member {name}"),
            span.clone(),
        );
    }

    fn insert_root(&mut self, role: LanguageItemRole, root: RootRecord) -> bool {
        let span = root.span.clone();
        let slot = match role {
            LanguageItemRole::Sized => &mut self.sized,
            LanguageItemRole::Drop => &mut self.drop.root,
            LanguageItemRole::Fold => &mut self.fold.root,
            LanguageItemRole::Index => &mut self.index.root,
            LanguageItemRole::IndexMut => &mut self.index_mut.root,
            LanguageItemRole::FnOnce => &mut self.fn_once.root,
            LanguageItemRole::FnMut => &mut self.fn_mut.root,
            LanguageItemRole::Fn => &mut self.fn_trait.root,
            LanguageItemRole::Send => &mut self.send,
            LanguageItemRole::Sync => &mut self.sync,
            LanguageItemRole::Try => &mut self.try_protocol.root,
            LanguageItemRole::FromResidual => &mut self.from_residual.root,
            LanguageItemRole::ControlFlow => &mut self.control_flow.root,
            LanguageItemRole::Range => &mut self.range.root,
            _ => unreachable!("only protocol root roles reach insertion"),
        };
        insert_once(
            slot,
            root,
            &mut self.errors,
            role,
            None,
            format!("language item {role} is duplicated"),
            span,
        )
    }

    fn insert_method(
        &mut self,
        protocol: LanguageItemRole,
        role: LanguageItemRole,
        value: MethodRecord,
    ) {
        let span = value.span.clone();
        let slot = match (protocol, role) {
            (LanguageItemRole::Drop, LanguageItemRole::Method) => &mut self.drop.method,
            (LanguageItemRole::Fold, LanguageItemRole::Method) => &mut self.fold.method,
            (LanguageItemRole::Index, LanguageItemRole::Method) => &mut self.index.method,
            (LanguageItemRole::IndexMut, LanguageItemRole::Method) => &mut self.index_mut.method,
            (LanguageItemRole::FnOnce, LanguageItemRole::Method) => &mut self.fn_once.method,
            (LanguageItemRole::FnMut, LanguageItemRole::Method) => &mut self.fn_mut.method,
            (LanguageItemRole::Fn, LanguageItemRole::Method) => &mut self.fn_trait.method,
            (LanguageItemRole::Try, LanguageItemRole::Branch) => &mut self.try_protocol.branch,
            (LanguageItemRole::FromResidual, LanguageItemRole::Method) => {
                &mut self.from_residual.method
            }
            _ => return,
        };
        insert_once(
            slot,
            value,
            &mut self.errors,
            protocol,
            Some(role),
            format!("language item {protocol}.{role} is duplicated"),
            span,
        );
    }

    fn insert_associated_type(
        &mut self,
        protocol: LanguageItemRole,
        role: LanguageItemRole,
        value: AssociatedTypeRecord,
    ) {
        let span = value.span.clone();
        let slot = match (protocol, role) {
            (LanguageItemRole::Index, LanguageItemRole::Output) => &mut self.index.output,
            (LanguageItemRole::IndexMut, LanguageItemRole::Output) => &mut self.index_mut.output,
            (LanguageItemRole::FnOnce, LanguageItemRole::Output) => &mut self.fn_once.output,
            (LanguageItemRole::FnMut, LanguageItemRole::Output) => &mut self.fn_mut.output,
            (LanguageItemRole::Fn, LanguageItemRole::Output) => &mut self.fn_trait.output,
            (LanguageItemRole::Try, LanguageItemRole::Output) => &mut self.try_protocol.output,
            (LanguageItemRole::Try, LanguageItemRole::Residual) => &mut self.try_protocol.residual,
            _ => return,
        };
        insert_once(
            slot,
            value,
            &mut self.errors,
            protocol,
            Some(role),
            format!("language item {protocol}.{role} is duplicated"),
            span,
        );
    }

    fn insert_variant(
        &mut self,
        protocol: LanguageItemRole,
        role: LanguageItemRole,
        value: VariantRecord,
    ) {
        let span = value.span.clone();
        let slot = match (protocol, role) {
            (LanguageItemRole::ControlFlow, LanguageItemRole::Break) => {
                &mut self.control_flow.break_variant
            }
            (LanguageItemRole::ControlFlow, LanguageItemRole::Continue) => {
                &mut self.control_flow.continue_variant
            }
            (LanguageItemRole::Range, LanguageItemRole::RangeFull) => &mut self.range.full_variant,
            (LanguageItemRole::Range, LanguageItemRole::RangeFrom) => &mut self.range.from_variant,
            (LanguageItemRole::Range, LanguageItemRole::RangeTo) => &mut self.range.to_variant,
            (LanguageItemRole::Range, LanguageItemRole::RangeToInclusive) => {
                &mut self.range.to_inclusive_variant
            }
            (LanguageItemRole::Range, LanguageItemRole::RangeExclusive) => {
                &mut self.range.exclusive_variant
            }
            (LanguageItemRole::Range, LanguageItemRole::RangeInclusive) => {
                &mut self.range.inclusive_variant
            }
            _ => return,
        };
        insert_once(
            slot,
            value,
            &mut self.errors,
            protocol,
            Some(role),
            format!("language item {protocol}.{role} is duplicated"),
            span,
        );
    }

    fn error(
        &mut self,
        protocol: LanguageItemRole,
        role: Option<LanguageItemRole>,
        message: String,
        span: Span,
    ) {
        self.errors.push(ProtocolError {
            protocol,
            role,
            error: ResolveError::with_span(message, span),
        });
    }

    fn finish(mut self) -> Result<LanguageItems<DefId>, Vec<ResolveError>> {
        self.complete_sized();
        self.complete_drop();
        if let Some(root) = &self.fold.root {
            if self.fold.method.is_none() {
                self.error(
                    LanguageItemRole::Fold,
                    Some(LanguageItemRole::Method),
                    "language item fold.method is missing".to_string(),
                    root.span.clone(),
                );
            }
        }
        self.complete_index();
        self.complete_index_mut();
        self.complete_fn(LanguageItemRole::FnOnce);
        self.complete_fn(LanguageItemRole::FnMut);
        self.complete_fn(LanguageItemRole::Fn);
        self.require_index_for_index_mut();
        self.complete_try_bundle();
        self.complete_range();
        self.errors.sort_by_key(|error| {
            (
                role_order(error.protocol),
                error.role.map(role_order).unwrap_or_default(),
            )
        });
        if !self.errors.is_empty() {
            return Err(self.errors.into_iter().map(|error| error.error).collect());
        }

        let sized = self
            .sized
            .map(|root| SizedLanguageItems { trait_id: root.id });
        let drop = match (self.drop.root, self.drop.method) {
            (Some(root), Some(method)) if method.owner_id == root.id => Some(DropLanguageItems {
                trait_id: root.id,
                method_id: method.id,
            }),
            _ => None,
        };
        let fold = match (self.fold.root, self.fold.method) {
            (Some(root), Some(method)) if method.owner_id == root.id => Some(FoldLanguageItems {
                trait_id: root.id,
                method_id: method.id,
            }),
            _ => None,
        };
        let index = match (self.index.root, self.index.output, self.index.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(IndexLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let index_mut = match (
            self.index_mut.root,
            self.index_mut.output,
            self.index_mut.method,
        ) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(IndexMutLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let range = match (
            self.range.root,
            self.range.full_variant,
            self.range.from_variant,
            self.range.to_variant,
            self.range.to_inclusive_variant,
            self.range.exclusive_variant,
            self.range.inclusive_variant,
        ) {
            (
                Some(root),
                Some(full),
                Some(from),
                Some(to),
                Some(to_inclusive),
                Some(exclusive),
                Some(inclusive),
            ) if [
                full.owner_id,
                from.owner_id,
                to.owner_id,
                to_inclusive.owner_id,
                exclusive.owner_id,
                inclusive.owner_id,
            ]
            .iter()
            .all(|owner| *owner == root.id) =>
            {
                Some(RangeLanguageItems {
                    enum_id: root.id,
                    full_variant_id: full.id,
                    from_variant_id: from.id,
                    to_variant_id: to.id,
                    to_inclusive_variant_id: to_inclusive.id,
                    exclusive_variant_id: exclusive.id,
                    inclusive_variant_id: inclusive.id,
                })
            }
            _ => None,
        };
        let fn_once = match (self.fn_once.root, self.fn_once.output, self.fn_once.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnOnceLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let fn_mut = match (self.fn_mut.root, self.fn_mut.output, self.fn_mut.method) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnMutLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let fn_trait = match (
            self.fn_trait.root,
            self.fn_trait.output,
            self.fn_trait.method,
        ) {
            (Some(root), Some(output), Some(method))
                if output.owner_id == root.id && method.owner_id == root.id =>
            {
                Some(FnLanguageItems {
                    trait_id: root.id,
                    output_id: output.id,
                    method_id: method.id,
                })
            }
            _ => None,
        };
        let send = self
            .send
            .map(|root| SendLanguageItems { trait_id: root.id });
        let sync = self
            .sync
            .map(|root| SyncLanguageItems { trait_id: root.id });
        let try_protocol = match (
            self.try_protocol.root,
            self.try_protocol.output,
            self.try_protocol.residual,
            self.try_protocol.branch,
            self.from_residual.root,
            self.from_residual.method,
            self.control_flow.root,
            self.control_flow.break_variant,
            self.control_flow.continue_variant,
        ) {
            (
                Some(try_root),
                Some(output),
                Some(residual),
                Some(branch),
                Some(from_residual_root),
                Some(from_residual_method),
                Some(control_flow_root),
                Some(break_variant),
                Some(continue_variant),
            ) if output.owner_id == try_root.id
                && residual.owner_id == try_root.id
                && branch.owner_id == try_root.id
                && from_residual_method.owner_id == from_residual_root.id
                && break_variant.owner_id == control_flow_root.id
                && continue_variant.owner_id == control_flow_root.id =>
            {
                Some(TryLanguageItems {
                    try_trait_id: try_root.id,
                    output_id: output.id,
                    residual_id: residual.id,
                    branch_method_id: branch.id,
                    from_residual_trait_id: from_residual_root.id,
                    from_residual_method_id: from_residual_method.id,
                    control_flow_enum_id: control_flow_root.id,
                    break_variant_id: break_variant.id,
                    continue_variant_id: continue_variant.id,
                })
            }
            _ => None,
        };
        Ok(LanguageItems {
            sized,
            drop,
            fold,
            index,
            index_mut,
            fn_once,
            fn_mut,
            fn_trait,
            send,
            sync,
            try_protocol,
            range,
        })
    }

    fn complete_sized(&mut self) {
        if let Some(root) = &self.sized {
            debug_assert_eq!(root.kind, ItemKind::Trait);
        }
    }

    fn complete_drop(&mut self) {
        if self.drop.root.is_some() && self.drop.method.is_none() {
            let span = self.drop.root.as_ref().unwrap().span.clone();
            self.error(
                LanguageItemRole::Drop,
                Some(LanguageItemRole::Method),
                "language item drop.method is missing".to_string(),
                span,
            );
        }
    }

    fn complete_index(&mut self) {
        let Some(root) = &self.index.root else {
            return;
        };
        let span = root.span.clone();
        if self.index.output.is_none() {
            self.error(
                LanguageItemRole::Index,
                Some(LanguageItemRole::Output),
                "language item index.output is missing".to_string(),
                span.clone(),
            );
        }
        if self.index.method.is_none() {
            self.error(
                LanguageItemRole::Index,
                Some(LanguageItemRole::Method),
                "language item index.method is missing".to_string(),
                span,
            );
        }
    }

    fn complete_index_mut(&mut self) {
        let Some(root) = &self.index_mut.root else {
            return;
        };
        let span = root.span.clone();
        if self.index_mut.output.is_none() {
            self.error(
                LanguageItemRole::IndexMut,
                Some(LanguageItemRole::Output),
                "language item index_mut.output is missing".to_string(),
                span.clone(),
            );
        }
        if self.index_mut.method.is_none() {
            self.error(
                LanguageItemRole::IndexMut,
                Some(LanguageItemRole::Method),
                "language item index_mut.method is missing".to_string(),
                span,
            );
        }
    }

    fn complete_fn(&mut self, protocol: LanguageItemRole) {
        let (span, missing_output, missing_method) = {
            let partial = match protocol {
                LanguageItemRole::FnOnce => &self.fn_once,
                LanguageItemRole::FnMut => &self.fn_mut,
                LanguageItemRole::Fn => &self.fn_trait,
                _ => unreachable!("only callable protocol roots reach completion"),
            };
            let Some(root) = &partial.root else {
                return;
            };
            (
                root.span.clone(),
                partial.output.is_none(),
                partial.method.is_none(),
            )
        };
        if missing_output {
            self.error(
                protocol,
                Some(LanguageItemRole::Output),
                format!("language item {protocol}.output is missing"),
                span.clone(),
            );
        }
        if missing_method {
            self.error(
                protocol,
                Some(LanguageItemRole::Method),
                format!("language item {protocol}.method is missing"),
                span,
            );
        }
    }

    fn require_index_for_index_mut(&mut self) {
        if self.index_mut.root.is_some() && self.index.root.is_none() {
            let span = self.index_mut.root.as_ref().unwrap().span.clone();
            self.error(
                LanguageItemRole::IndexMut,
                None,
                "language item index_mut requires language item index from the same provider"
                    .to_string(),
                span,
            );
        }
    }

    fn complete_try_bundle(&mut self) {
        let Some(span) = self.try_bundle_span() else {
            return;
        };
        if self.try_protocol.root.is_none() {
            self.missing_try_root(LanguageItemRole::Try, span.clone());
        } else {
            if self.try_protocol.output.is_none() {
                self.missing_try_member(LanguageItemRole::Output, span.clone());
            }
            if self.try_protocol.residual.is_none() {
                self.missing_try_member(LanguageItemRole::Residual, span.clone());
            }
            if self.try_protocol.branch.is_none() {
                self.missing_try_member(LanguageItemRole::Branch, span.clone());
            }
        }
        if self.from_residual.root.is_none() {
            self.missing_try_root(LanguageItemRole::FromResidual, span.clone());
        } else if self.from_residual.method.is_none() {
            self.error(
                LanguageItemRole::FromResidual,
                Some(LanguageItemRole::Method),
                "language item from_residual.method is missing".to_string(),
                span.clone(),
            );
        }
        if self.control_flow.root.is_none() {
            self.missing_try_root(LanguageItemRole::ControlFlow, span);
        } else {
            let child_span = self.control_flow.root.as_ref().unwrap().span.clone();
            if self.control_flow.break_variant.is_none() {
                self.error(
                    LanguageItemRole::ControlFlow,
                    Some(LanguageItemRole::Break),
                    "language item control_flow.break is missing".to_string(),
                    child_span.clone(),
                );
            }
            if self.control_flow.continue_variant.is_none() {
                self.error(
                    LanguageItemRole::ControlFlow,
                    Some(LanguageItemRole::Continue),
                    "language item control_flow.continue is missing".to_string(),
                    child_span,
                );
            }
        }
    }

    fn complete_range(&mut self) {
        let Some(root) = self.range.root.as_ref() else {
            return;
        };
        let span = root.span.clone();
        for (role, missing) in [
            (
                LanguageItemRole::RangeFull,
                self.range.full_variant.is_none(),
            ),
            (
                LanguageItemRole::RangeFrom,
                self.range.from_variant.is_none(),
            ),
            (LanguageItemRole::RangeTo, self.range.to_variant.is_none()),
            (
                LanguageItemRole::RangeToInclusive,
                self.range.to_inclusive_variant.is_none(),
            ),
            (
                LanguageItemRole::RangeExclusive,
                self.range.exclusive_variant.is_none(),
            ),
            (
                LanguageItemRole::RangeInclusive,
                self.range.inclusive_variant.is_none(),
            ),
        ] {
            if missing {
                self.error(
                    LanguageItemRole::Range,
                    Some(role),
                    format!("language item range.{role} is missing"),
                    span.clone(),
                );
            }
        }
    }

    fn try_bundle_span(&self) -> Option<Span> {
        self.try_protocol
            .root
            .as_ref()
            .or(self.from_residual.root.as_ref())
            .or(self.control_flow.root.as_ref())
            .map(|root| root.span.clone())
    }

    fn missing_try_root(&mut self, role: LanguageItemRole, span: Span) {
        self.error(role, None, format!("language item {role} is missing"), span);
    }

    fn missing_try_member(&mut self, role: LanguageItemRole, span: Span) {
        self.error(
            LanguageItemRole::Try,
            Some(role),
            format!("language item try.{role} is missing"),
            span,
        );
    }
}

fn insert_once<T>(
    slot: &mut Option<T>,
    value: T,
    errors: &mut Vec<ProtocolError>,
    protocol: LanguageItemRole,
    role: Option<LanguageItemRole>,
    message: String,
    span: Span,
) -> bool {
    if slot.is_some() {
        errors.push(ProtocolError {
            protocol,
            role,
            error: ResolveError::with_span(message, span),
        });
        return false;
    }
    *slot = Some(value);
    true
}

fn root_expected_kind(role: LanguageItemRole) -> Option<ItemKind> {
    match role {
        LanguageItemRole::Sized
        | LanguageItemRole::Drop
        | LanguageItemRole::Fold
        | LanguageItemRole::Index
        | LanguageItemRole::IndexMut
        | LanguageItemRole::FnOnce
        | LanguageItemRole::FnMut
        | LanguageItemRole::Fn
        | LanguageItemRole::Send
        | LanguageItemRole::Sync
        | LanguageItemRole::Try
        | LanguageItemRole::FromResidual => Some(ItemKind::Trait),
        LanguageItemRole::ControlFlow | LanguageItemRole::Range => Some(ItemKind::Enum),
        _ => None,
    }
}

fn expected_child_kind(
    protocol: LanguageItemRole,
    role: LanguageItemRole,
) -> Option<LanguageItemMemberKind> {
    match (protocol, role) {
        (LanguageItemRole::Drop, LanguageItemRole::Method)
        | (LanguageItemRole::Fold, LanguageItemRole::Method)
        | (LanguageItemRole::Index, LanguageItemRole::Method)
        | (LanguageItemRole::IndexMut, LanguageItemRole::Method)
        | (LanguageItemRole::FnOnce, LanguageItemRole::Method)
        | (LanguageItemRole::FnMut, LanguageItemRole::Method)
        | (LanguageItemRole::Fn, LanguageItemRole::Method)
        | (LanguageItemRole::Try, LanguageItemRole::Branch)
        | (LanguageItemRole::FromResidual, LanguageItemRole::Method) => {
            Some(LanguageItemMemberKind::Method)
        }
        (LanguageItemRole::Index, LanguageItemRole::Output)
        | (LanguageItemRole::IndexMut, LanguageItemRole::Output)
        | (LanguageItemRole::FnOnce, LanguageItemRole::Output)
        | (LanguageItemRole::FnMut, LanguageItemRole::Output)
        | (LanguageItemRole::Fn, LanguageItemRole::Output)
        | (LanguageItemRole::Try, LanguageItemRole::Output)
        | (LanguageItemRole::Try, LanguageItemRole::Residual) => {
            Some(LanguageItemMemberKind::AssociatedType)
        }
        (LanguageItemRole::ControlFlow, LanguageItemRole::Break)
        | (LanguageItemRole::ControlFlow, LanguageItemRole::Continue) => {
            Some(LanguageItemMemberKind::Variant)
        }
        (LanguageItemRole::Range, LanguageItemRole::RangeFull)
        | (LanguageItemRole::Range, LanguageItemRole::RangeFrom)
        | (LanguageItemRole::Range, LanguageItemRole::RangeTo)
        | (LanguageItemRole::Range, LanguageItemRole::RangeToInclusive)
        | (LanguageItemRole::Range, LanguageItemRole::RangeExclusive)
        | (LanguageItemRole::Range, LanguageItemRole::RangeInclusive) => {
            Some(LanguageItemMemberKind::Variant)
        }
        _ => None,
    }
}

fn child_kind_name(kind: LanguageItemMemberKind) -> &'static str {
    match kind {
        LanguageItemMemberKind::AssociatedType => "associated type",
        LanguageItemMemberKind::Method => "method",
        LanguageItemMemberKind::Variant => "variant",
    }
}

fn role_order(role: LanguageItemRole) -> u8 {
    match role {
        LanguageItemRole::Sized => 0,
        LanguageItemRole::Drop => 1,
        LanguageItemRole::Fold => 25,
        LanguageItemRole::Index => 2,
        LanguageItemRole::IndexMut => 3,
        LanguageItemRole::FnOnce => 4,
        LanguageItemRole::FnMut => 5,
        LanguageItemRole::Fn => 6,
        LanguageItemRole::Send => 7,
        LanguageItemRole::Sync => 8,
        LanguageItemRole::Try => 9,
        LanguageItemRole::FromResidual => 10,
        LanguageItemRole::ControlFlow => 11,
        LanguageItemRole::Range => 12,
        LanguageItemRole::Method => 13,
        LanguageItemRole::Output => 14,
        LanguageItemRole::Residual => 15,
        LanguageItemRole::Branch => 16,
        LanguageItemRole::Break => 17,
        LanguageItemRole::Continue => 18,
        LanguageItemRole::RangeFull => 19,
        LanguageItemRole::RangeFrom => 20,
        LanguageItemRole::RangeTo => 21,
        LanguageItemRole::RangeToInclusive => 22,
        LanguageItemRole::RangeExclusive => 23,
        LanguageItemRole::RangeInclusive => 24,
    }
}
