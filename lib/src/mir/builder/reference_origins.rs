use crate::ids::TypeId;
use crate::types::Type;

use crate::mir::{
    Local, LocalSource, MirDropObligation, Operand, Place, Projection, ReferenceOrigin, Rvalue,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn record_reference_assignment(&mut self, dest: &Place, rvalue: &Rvalue) {
        if !dest.projection.is_empty() || !self.local_type_is_reference(dest.local) {
            return;
        }

        let origin = match rvalue {
            Rvalue::Ref(_, place) => self.reference_origin_for_place(place),
            Rvalue::Use(Operand::Copy(place) | Operand::Move(place))
                if place.projection.is_empty() =>
            {
                self.reference_origin_for_local(place.local)
            }
            _ => None,
        };

        if let Some(origin) = origin {
            self.record_reference_origin(dest.local, origin);
        }
    }

    pub(super) fn record_reference_origin(&mut self, local: Local, origin: ReferenceOrigin) {
        self.ownership
            .reference_origins
            .retain(|(origin_local, _)| *origin_local != local);
        self.ownership.reference_origins.push((local, origin));
    }

    pub(super) fn record_temporary_local(&mut self, local: Local) {
        self.ownership.temporary_locals.push(local);
    }

    pub(super) fn record_drop_obligation(
        &mut self,
        place: Place,
        ty: TypeId,
        kind: crate::mir::DropObligationKind,
    ) {
        self.ownership
            .drop_obligations
            .push(MirDropObligation { place, ty, kind });
    }

    fn local_type_is_reference(&self, local: Local) -> bool {
        self.get_local_type(local)
            .is_some_and(|ty| matches!(ty, Type::Reference { .. }))
    }

    fn local_type_is_pointer(&self, local: Local) -> bool {
        self.get_local_type(local)
            .is_some_and(|ty| matches!(ty, Type::Pointer(_)))
    }

    fn reference_origin_for_place(&self, place: &Place) -> Option<ReferenceOrigin> {
        if matches!(place.projection.first(), Some(Projection::Deref)) {
            if self.local_type_is_pointer(place.local) {
                return Some(ReferenceOrigin::UnknownExternal);
            }
            if let Some(origin) = self.reference_origin_for_local(place.local) {
                return Some(origin);
            }
        }

        if !place.projection.is_empty() {
            if let Some(ReferenceOrigin::Param(local)) =
                self.reference_origin_for_local(place.local)
            {
                return Some(ReferenceOrigin::Param(local));
            }
        }

        self.locals
            .get(place.local.0)
            .map(|decl| match decl.source {
                LocalSource::Argument => ReferenceOrigin::Local(place.local),
                LocalSource::UserBinding
                | LocalSource::ReturnPlace
                | LocalSource::ClosureCapture => ReferenceOrigin::Local(place.local),
                LocalSource::Temporary => ReferenceOrigin::Temporary(place.local),
            })
    }

    fn reference_origin_for_local(&self, local: Local) -> Option<ReferenceOrigin> {
        self.ownership
            .reference_origins
            .iter()
            .rev()
            .find_map(|(origin_local, origin)| (*origin_local == local).then_some(*origin))
            .or_else(|| {
                self.locals.get(local.0).and_then(|decl| {
                    if !self.local_type_is_reference(local) {
                        return None;
                    }
                    Some(match decl.source {
                        LocalSource::Argument => ReferenceOrigin::Param(local),
                        LocalSource::UserBinding
                        | LocalSource::ReturnPlace
                        | LocalSource::ClosureCapture => ReferenceOrigin::Local(local),
                        LocalSource::Temporary => ReferenceOrigin::Temporary(local),
                    })
                })
            })
    }
}
