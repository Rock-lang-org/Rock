use super::ProtocolError;
use crate::ast::LanguageItemMemberKind;
use crate::collect::item_index::ItemKind;
use crate::language_items::LanguageItemRole;
use crate::lexer::Span;
use crate::lower::ResolveError;

pub(super) fn insert_once<T>(
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

pub(super) fn root_expected_kind(role: LanguageItemRole) -> Option<ItemKind> {
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
        | LanguageItemRole::Owned
        | LanguageItemRole::Sync
        | LanguageItemRole::Try
        | LanguageItemRole::FromResidual => Some(ItemKind::Trait),
        LanguageItemRole::ControlFlow | LanguageItemRole::Range => Some(ItemKind::Enum),
        _ => None,
    }
}

pub(super) fn expected_child_kind(
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

pub(super) fn child_kind_name(kind: LanguageItemMemberKind) -> &'static str {
    match kind {
        LanguageItemMemberKind::AssociatedType => "associated type",
        LanguageItemMemberKind::Method => "method",
        LanguageItemMemberKind::Variant => "variant",
    }
}

pub(super) fn role_order(role: LanguageItemRole) -> u8 {
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
        LanguageItemRole::Owned => 26,
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
