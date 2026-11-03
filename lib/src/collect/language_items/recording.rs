use super::kinds::insert_once;
use super::{
    AssociatedTypeRecord, BindingState, MethodRecord, ProtocolError, RootRecord, VariantRecord,
};
use crate::language_items::LanguageItemRole;
use crate::lexer::Span;
use crate::lower::ResolveError;

impl BindingState {
    pub(super) fn insert_root(&mut self, role: LanguageItemRole, root: RootRecord) -> bool {
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

    pub(super) fn insert_method(
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

    pub(super) fn insert_associated_type(
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

    pub(super) fn insert_variant(
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

    pub(super) fn error(
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
}
