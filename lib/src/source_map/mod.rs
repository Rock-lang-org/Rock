use std::collections::HashMap;

use crate::ids::{AssocTypeId, DefId, FieldId, HirLocalId, VariantId};
use crate::lexer::Span;
use crate::types::GenericParamId;

mod declaration_spans;
mod declarations;
mod type_references;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceSymbol {
    Definition(DefId),
    Local {
        owner: DefId,
        local: HirLocalId,
    },
    Field {
        owner: DefId,
        field: FieldId,
    },
    Variant {
        owner: DefId,
        variant: VariantId,
    },
    AssociatedType {
        owner: DefId,
        associated: AssocTypeId,
    },
    Generic(GenericParamId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSymbolInfo {
    pub name_span: Span,
    pub declaration_span: Option<Span>,
    pub lexical_owner: Option<DefId>,
    pub scope_id: Option<u32>,
    inferred_binding: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceReference {
    pub span: Span,
    pub target: SourceSymbol,
    pub lexical_owner: Option<DefId>,
    pub scope_id: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceScope {
    pub id: u32,
    pub owner: DefId,
    pub parent: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticSourceMap {
    symbols: HashMap<SourceSymbol, SourceSymbolInfo>,
    references: Vec<SourceReference>,
    /// Indices of `references` by span offsets. Recording deduplicates every
    /// reference; a linear scan made source-map construction quadratic.
    reference_index: HashMap<(usize, usize), Vec<usize>>,
    expressions: Vec<Span>,
    type_annotations: Vec<Span>,
    scopes: Vec<SourceScope>,
}

impl SemanticSourceMap {
    /// Return the narrowest declaration or reference containing a byte offset.
    pub fn symbol_at(&self, path: &std::path::Path, offset: usize) -> Option<(SourceSymbol, Span)> {
        let references = self.references.iter().filter_map(|reference| {
            span_contains(&reference.span, path, offset)
                .then(|| (reference.target.clone(), reference.span.clone()))
        });
        let declarations = self.symbols.iter().filter_map(|(symbol, info)| {
            span_contains(&info.name_span, path, offset)
                .then(|| (symbol.clone(), info.name_span.clone()))
        });

        references
            .chain(declarations)
            .min_by_key(|(_, span)| span.end.saturating_sub(span.start))
    }

    pub fn insert_symbol(
        &mut self,
        symbol: SourceSymbol,
        name_span: Span,
        declaration_span: Option<Span>,
        lexical_owner: Option<DefId>,
    ) {
        self.symbols
            .entry(symbol)
            .and_modify(|info| {
                if declaration_span.is_some() {
                    info.declaration_span = declaration_span.clone();
                }
                if lexical_owner.is_some() {
                    info.lexical_owner = lexical_owner;
                }
            })
            .or_insert(SourceSymbolInfo {
                name_span,
                declaration_span,
                lexical_owner,
                scope_id: None,
                inferred_binding: false,
            });
    }

    fn insert_definition_symbol(
        &mut self,
        symbol: SourceSymbol,
        name_span: Span,
        declaration_span: Span,
        lexical_owner: Option<DefId>,
    ) {
        self.insert_symbol(symbol, name_span, Some(declaration_span), lexical_owner);
    }

    pub fn symbol(&self, symbol: &SourceSymbol) -> Option<&SourceSymbolInfo> {
        self.symbols.get(symbol)
    }

    pub fn definition_span(&self, id: DefId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Definition(id))
            .map(|info| &info.name_span)
    }

    pub fn definition_declaration_span(&self, id: DefId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Definition(id))
            .and_then(|info| info.declaration_span.as_ref())
    }

    pub fn insert_definition(&mut self, id: DefId, span: Span) {
        self.insert_symbol(SourceSymbol::Definition(id), span.clone(), Some(span), None);
    }

    pub fn insert_local(&mut self, owner: DefId, local: HirLocalId, span: Span) {
        self.insert_local_in_scope(owner, local, span, None);
    }

    pub fn insert_local_in_scope(
        &mut self,
        owner: DefId,
        local: HirLocalId,
        span: Span,
        scope_id: Option<u32>,
    ) {
        self.insert_symbol(
            SourceSymbol::Local { owner, local },
            span.clone(),
            Some(span),
            Some(owner),
        );
        if let Some(info) = self.symbols.get_mut(&SourceSymbol::Local { owner, local }) {
            info.scope_id = scope_id;
        }
    }

    pub fn local_span(&self, owner: DefId, local: HirLocalId) -> Option<&Span> {
        self.symbol(&SourceSymbol::Local { owner, local })
            .map(|info| &info.name_span)
    }

    pub(crate) fn mark_inferred_binding(&mut self, owner: DefId, local: HirLocalId) {
        if let Some(info) = self.symbols.get_mut(&SourceSymbol::Local { owner, local }) {
            info.inferred_binding = true;
        }
    }

    pub(crate) fn inferred_bindings(&self) -> impl Iterator<Item = (DefId, HirLocalId, &Span)> {
        self.symbols.iter().filter_map(|(symbol, info)| match symbol {
            SourceSymbol::Local { owner, local } if info.inferred_binding => {
                Some((*owner, *local, &info.name_span))
            }
            _ => None,
        })
    }

    pub fn insert_field(&mut self, owner: DefId, field: FieldId, span: Span) {
        self.insert_symbol(
            SourceSymbol::Field { owner, field },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_variant(&mut self, owner: DefId, variant: VariantId, span: Span) {
        self.insert_symbol(
            SourceSymbol::Variant { owner, variant },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_associated_type(&mut self, owner: DefId, associated: AssocTypeId, span: Span) {
        self.insert_symbol(
            SourceSymbol::AssociatedType { owner, associated },
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn insert_generic(&mut self, id: GenericParamId, span: Span, owner: DefId) {
        self.insert_symbol(
            SourceSymbol::Generic(id),
            span.clone(),
            Some(span),
            Some(owner),
        );
    }

    pub fn record_reference(
        &mut self,
        span: Span,
        target: SourceSymbol,
        lexical_owner: Option<DefId>,
    ) {
        self.record_reference_in_scope(span, target, lexical_owner, None);
    }

    pub fn record_reference_in_scope(
        &mut self,
        span: Span,
        target: SourceSymbol,
        lexical_owner: Option<DefId>,
        scope_id: Option<u32>,
    ) {
        let duplicate = self
            .reference_index
            .get(&(span.start, span.end))
            .is_some_and(|indices| {
                indices.iter().any(|&index| {
                    let reference = &self.references[index];
                    reference.span == span
                        && reference.target == target
                        && reference.lexical_owner == lexical_owner
                        && reference.scope_id == scope_id
                })
            });
        if duplicate {
            return;
        }
        self.push_reference(SourceReference {
            span,
            target,
            lexical_owner,
            scope_id,
        });
    }

    fn push_reference(&mut self, reference: SourceReference) {
        self.reference_index
            .entry((reference.span.start, reference.span.end))
            .or_default()
            .push(self.references.len());
        self.references.push(reference);
    }

    pub fn references(&self) -> &[SourceReference] {
        &self.references
    }

    pub fn reference_spans<'a>(
        &'a self,
        target: &'a SourceSymbol,
    ) -> impl Iterator<Item = &'a Span> + 'a {
        self.references
            .iter()
            .filter(move |reference| &reference.target == target)
            .map(|reference| &reference.span)
    }

    pub fn record_expression(&mut self, span: Span) {
        self.expressions.push(span);
    }

    pub fn expression_spans(&self) -> &[Span] {
        &self.expressions
    }

    pub fn record_type_annotation(&mut self, span: Span) {
        self.type_annotations.push(span);
    }

    pub fn type_annotation_spans(&self) -> &[Span] {
        &self.type_annotations
    }

    pub fn record_scope(&mut self, id: u32, owner: DefId, parent: Option<u32>) {
        self.scopes.push(SourceScope { id, owner, parent });
    }

    pub fn insert_scope(&mut self, owner: DefId, parent: Option<u32>) -> u32 {
        let id = self.scopes.len() as u32;
        self.record_scope(id, owner, parent);
        id
    }

    pub fn scopes(&self) -> &[SourceScope] {
        &self.scopes
    }

    /// Copy source ownership metadata from a semantic owner to a generated owner.
    ///
    /// Generated trait-default methods receive fresh `DefId`s during conformance,
    /// while their parameters and generic binders still originate in the trait
    /// declaration. This session-only alias keeps those spans available to MIR
    /// without putting editor metadata in artifacts.
    pub fn remap_owner(&mut self, from: DefId, to: DefId) {
        let aliases = self
            .symbols
            .iter()
            .filter_map(|(symbol, info)| {
                remap_source_symbol(symbol, from, to).map(|symbol| (symbol, info.clone()))
            })
            .collect::<Vec<_>>();
        for (symbol, mut info) in aliases {
            // The original declaration owns the hint, not a generated method.
            info.inferred_binding = false;
            if info.lexical_owner == Some(from) {
                info.lexical_owner = Some(to);
            }
            self.symbols.entry(symbol).or_insert(info);
        }

        let references = self
            .references
            .iter()
            .filter_map(|reference| {
                remap_source_symbol(&reference.target, from, to).map(|target| SourceReference {
                    span: reference.span.clone(),
                    target,
                    lexical_owner: reference.lexical_owner.map(|owner| {
                        if owner == from {
                            to
                        } else {
                            owner
                        }
                    }),
                    scope_id: reference.scope_id,
                })
            })
            .collect::<Vec<_>>();
        for reference in references {
            self.push_reference(reference);
        }

        for scope in &mut self.scopes {
            if scope.owner == from {
                scope.owner = to;
            }
        }
    }
}

fn span_contains(span: &Span, path: &std::path::Path, offset: usize) -> bool {
    span.file_path == path && span.start <= offset && offset < span.end
}

fn remap_source_symbol(symbol: &SourceSymbol, from: DefId, to: DefId) -> Option<SourceSymbol> {
    match symbol {
        SourceSymbol::Definition(owner) if *owner == from => Some(SourceSymbol::Definition(to)),
        SourceSymbol::Local { owner, local } if *owner == from => Some(SourceSymbol::Local {
            owner: to,
            local: *local,
        }),
        SourceSymbol::Field { owner, field } if *owner == from => Some(SourceSymbol::Field {
            owner: to,
            field: *field,
        }),
        SourceSymbol::Variant { owner, variant } if *owner == from => Some(SourceSymbol::Variant {
            owner: to,
            variant: *variant,
        }),
        SourceSymbol::AssociatedType { owner, associated } if *owner == from => {
            Some(SourceSymbol::AssociatedType {
                owner: to,
                associated: *associated,
            })
        }
        SourceSymbol::Generic(param) if param.owner == from => {
            Some(SourceSymbol::Generic(GenericParamId {
                owner: to,
                ..*param
            }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests;
