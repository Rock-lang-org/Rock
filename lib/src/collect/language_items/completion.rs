use super::BindingState;
use crate::collect::item_index::ItemKind;
use crate::language_items::LanguageItemRole;
use crate::lexer::Span;

impl BindingState {
    pub(super) fn complete_sized(&mut self) {
        if let Some(root) = &self.sized {
            debug_assert_eq!(root.kind, ItemKind::Trait);
        }
    }

    pub(super) fn complete_drop(&mut self) {
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

    pub(super) fn complete_index(&mut self) {
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

    pub(super) fn complete_index_mut(&mut self) {
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

    pub(super) fn complete_fn(&mut self, protocol: LanguageItemRole) {
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

    pub(super) fn require_index_for_index_mut(&mut self) {
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

    pub(super) fn complete_try_bundle(&mut self) {
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

    pub(super) fn complete_range(&mut self) {
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
