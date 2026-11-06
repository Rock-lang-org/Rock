use crate::ids::{DefId, TypeId};
use crate::lexer::Span;
use crate::type_services::facts::TypeFacts;
use crate::types::Type;

use crate::mir::{
    BasicBlock, BasicBlockId, Local, LocalDecl, LocalSource, Mutability, Place, Projection,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn new_block(&mut self) -> BasicBlockId {
        let id = BasicBlockId(self.blocks.len());
        self.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: None,
        });
        id
    }

    pub(super) fn type_id_for(&self, ty: &Type) -> TypeId {
        // Accepted HIR is canonical, including constructor-valued nominal args.
        self.type_context.borrow_mut().intern_canonical_type(ty)
    }

    pub(super) fn new_local(
        &mut self,
        ty: TypeId,
        mutability: Mutability,
        name: Option<String>,
    ) -> Local {
        self.new_local_with_source(ty, mutability, name, LocalSource::Temporary, None)
    }

    pub(super) fn new_local_with_source(
        &mut self,
        ty: TypeId,
        mutability: Mutability,
        name: Option<String>,
        source: LocalSource,
        span: Option<Span>,
    ) -> Local {
        let id = Local(self.locals.len());
        self.locals.push(LocalDecl {
            ty,
            mutability,
            name,
            span,
            source,
        });
        if source.is_temporary() {
            self.record_temporary_local(id);
        }
        id
    }

    pub(super) fn new_local_from_expr(&mut self, ty: Type, expr: &HirExpr) -> Local {
        let id = Local(self.locals.len());
        let ty = self.type_id_for(&ty);
        self.locals.push(LocalDecl {
            ty,
            mutability: Mutability::Not,
            name: None,
            span: Some(expr.span.clone()),
            source: LocalSource::Temporary,
        });
        self.record_temporary_local(id);
        id
    }

    pub(super) fn needs_move(ty: &Type) -> bool {
        !TypeFacts::is_copy(ty)
    }

    pub(super) fn get_local_type(&self, local: Local) -> Option<Type> {
        self.locals
            .get(local.0)
            .map(|decl| self.type_context.borrow().type_for(decl.ty))
    }

    pub(super) fn place_for_borrowed_binding_value(
        &self,
        local: Local,
        hir_ty: &Type,
    ) -> Option<Place> {
        match self.get_local_type(local)? {
            Type::Reference { mutable: _, inner } if inner.as_ref() == hir_ty => Some(Place {
                local,
                projection: vec![Projection::Deref],
            }),
            _ => None,
        }
    }

    pub(super) fn place_for_local_expr(&self, local: Local, hir_ty: &Type) -> Place {
        self.place_for_borrowed_binding_value(local, hir_ty)
            .unwrap_or(Place {
                local,
                projection: vec![],
            })
    }

    pub(super) fn source_local_span(
        &self,
        owner: Option<DefId>,
        local: crate::ids::HirLocalId,
    ) -> Option<Span> {
        owner.and_then(|owner| self.source_map.local_span(owner, local).cloned())
    }

    pub(super) fn source_operation_span(
        &self,
        _owner: Option<DefId>,
        operation: Option<Span>,
    ) -> Option<Span> {
        operation
    }

    pub(super) fn source_origin(&self, span: Option<Span>) -> crate::mir::MirOrigin {
        span.map(crate::mir::MirOrigin::Source).unwrap_or_else(|| {
            crate::mir::MirOrigin::synthetic(crate::mir::MirSyntheticOrigin::ControlFlow, None)
        })
    }

    pub(super) fn cleanup_origin(&self, place: &Place) -> crate::mir::MirOrigin {
        crate::mir::MirOrigin::synthetic(
            crate::mir::MirSyntheticOrigin::Cleanup,
            self.locals
                .get(place.local.0)
                .and_then(|local| local.span.clone()),
        )
    }

    pub(super) fn drop_origin(&self, place: &Place) -> crate::mir::MirOrigin {
        let span = self
            .locals
            .get(place.local.0)
            .and_then(|local| local.span.clone());
        if self.drops.in_cleanup() {
            crate::mir::MirOrigin::synthetic(crate::mir::MirSyntheticOrigin::Cleanup, span)
        } else {
            self.source_origin(span)
        }
    }

    pub(super) fn register_scoped_temp(&mut self, local: Local) {
        if let Some(scope_locals) = self.scope_locals.last_mut() {
            scope_locals.push(local);
        }
    }
}
