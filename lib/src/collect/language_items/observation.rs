use super::kinds::{child_kind_name, expected_child_kind, root_expected_kind};
use super::{AssociatedTypeRecord, BindingState, MethodRecord, RootRecord, VariantRecord};
use crate::ast;
use crate::ast::LanguageItemMemberKind;
use crate::collect::item_index::ItemKind;
use crate::collect::CollectedIdEnvironment;
use crate::ids::{AssocTypeId, DefId, VariantId};
use crate::language_items::LanguageItemRole;
use crate::lexer::Span;

impl BindingState {
    pub(super) fn observe_trait(
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

    pub(super) fn observe_enum(
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
}
