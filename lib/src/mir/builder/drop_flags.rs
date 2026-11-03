use crate::lexer::Span;
use crate::types::Type;

use crate::mir::{
    Constant, Local, LocalDecl, LocalSource, Mutability, Operand, Place, Projection, Rvalue,
    StatementData,
};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn ensure_drop_flag(&mut self, local: Local) -> Option<Local> {
        if let Some(flag) = self.drop_flags.get(&local).copied() {
            return Some(flag);
        }

        let ty = self.get_local_type(local)?;
        if !self.type_needs_cleanup(&ty) {
            return None;
        }

        let flag = Local(self.locals.len());
        self.locals.push(LocalDecl {
            ty: self.type_id_for(&Type::Bool),
            mutability: Mutability::Mut,
            name: Some(format!("drop_flag_{}", local.0)),
            span: self.locals.get(local.0).and_then(|decl| decl.span.clone()),
            source: LocalSource::Temporary,
        });
        self.record_temporary_local(flag);
        self.drop_flags.insert(local, flag);
        self.ensure_projection_drop_flags(local, &ty);
        Some(flag)
    }

    fn ensure_projection_drop_flags(&mut self, local: Local, ty: &Type) {
        let mut entries = Vec::new();
        self.collect_projection_drop_flag_entries(ty, Vec::new(), &mut entries);
        for (projection, _) in entries {
            let place = Place { local, projection };
            if self.projection_drop_flags.contains_key(&place) {
                continue;
            }
            let flag = Local(self.locals.len());
            self.locals.push(LocalDecl {
                ty: self.type_id_for(&Type::Bool),
                mutability: Mutability::Mut,
                name: Some(format!("drop_flag_{}_{}", local.0, flag.0)),
                span: self.locals.get(local.0).and_then(|decl| decl.span.clone()),
                source: LocalSource::Temporary,
            });
            self.record_temporary_local(flag);
            self.projection_drop_flags.insert(place, flag);
        }
    }

    fn collect_projection_drop_flag_entries(
        &self,
        ty: &Type,
        prefix: Vec<Projection>,
        entries: &mut Vec<(Vec<Projection>, Type)>,
    ) {
        match ty {
            Type::Struct { id, args } => {
                for (index, (_field_id, field_ty)) in
                    Self::struct_field_types_with_substitution(self.program, *id, args)
                        .into_iter()
                        .enumerate()
                {
                    if !self.type_needs_cleanup(&field_ty) {
                        continue;
                    }
                    let mut projection = prefix.clone();
                    projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    entries.push((projection.clone(), field_ty.clone()));
                    self.collect_projection_drop_flag_entries(&field_ty, projection, entries);
                }
            }
            Type::Tuple(elems) => {
                for (index, field_ty) in elems.iter().enumerate() {
                    if !self.type_needs_cleanup(field_ty) {
                        continue;
                    }
                    let mut projection = prefix.clone();
                    projection.push(Projection::Field {
                        index,
                        identity: None,
                    });
                    entries.push((projection.clone(), field_ty.clone()));
                    self.collect_projection_drop_flag_entries(field_ty, projection, entries);
                }
            }
            Type::Enum { id, args: _ } => {
                let Some((_, enumeration)) = self.program.enum_by_id(*id) else {
                    return;
                };
                let generic_subst = Self::generic_substitution_for_fields(
                    &enumeration
                        .variants
                        .iter()
                        .flat_map(|variant| {
                            Self::enum_variant_field_types_for(self.program, *id, variant.id)
                        })
                        .collect::<Vec<_>>(),
                    ty,
                );
                for variant in &enumeration.variants {
                    for (index, field_ty) in
                        Self::enum_variant_field_types_for(self.program, *id, variant.id)
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&generic_subst))
                            .enumerate()
                    {
                        if !self.type_needs_cleanup(&field_ty) {
                            continue;
                        }
                        let mut projection = prefix.clone();
                        projection.push(Projection::Downcast(variant.id));
                        projection.push(Projection::Field {
                            index,
                            identity: None,
                        });
                        entries.push((projection.clone(), field_ty.clone()));
                        self.collect_projection_drop_flag_entries(&field_ty, projection, entries);
                    }
                }
            }
            Type::Array(_, _) => {}
            _ => {}
        }
    }

    pub(super) fn drop_flag_for(&self, local: Local) -> Option<Local> {
        self.drop_flags.get(&local).copied()
    }

    pub(super) fn drop_flag_for_place(&self, place: &Place) -> Option<Local> {
        if place.projection.is_empty() {
            self.drop_flag_for(place.local)
        } else {
            self.projection_drop_flags
                .iter()
                .find_map(|(candidate, flag)| {
                    (candidate.local == place.local
                        && candidate.projection.len() == place.projection.len()
                        && Self::projection_is_prefix(&candidate.projection, &place.projection))
                    .then_some(*flag)
                })
        }
    }

    pub(super) fn projection_drop_flags_for_local(&self, local: Local) -> Vec<Local> {
        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(place, flag)| (place.local == local).then_some(*flag))
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        flags
    }

    pub(super) fn set_drop_flag_for_place(
        &mut self,
        place: &Place,
        initialized: bool,
        span: Option<Span>,
    ) {
        let Some(flag) = self.drop_flag_for_place(place) else {
            return;
        };
        self.set_drop_flag_local(flag, initialized, span);
    }

    pub(super) fn set_drop_flag(&mut self, local: Local, initialized: bool, span: Option<Span>) {
        let Some(flag) = self.drop_flag_for(local) else {
            return;
        };
        self.set_drop_flag_local(flag, initialized, span);
    }

    pub(super) fn set_projection_drop_flags_for_local(
        &mut self,
        local: Local,
        initialized: bool,
        span: Option<Span>,
    ) {
        for flag in self.projection_drop_flags_for_local(local) {
            self.set_drop_flag_local(flag, initialized, span.clone());
        }
    }

    pub(super) fn set_drop_flags_for_place_tree(
        &mut self,
        place: &Place,
        initialized: bool,
        span: Option<Span>,
    ) {
        if place.projection.is_empty() {
            self.set_drop_flag(place.local, initialized, span.clone());
            self.set_projection_drop_flags_for_local(place.local, initialized, span);
            return;
        }

        let mut flags = self
            .projection_drop_flags
            .iter()
            .filter_map(|(candidate, flag)| {
                Self::place_projection_is_prefix(place, candidate).then_some(*flag)
            })
            .collect::<Vec<_>>();
        flags.sort_by_key(|flag| flag.0);
        for flag in flags {
            self.set_drop_flag_local(flag, initialized, span.clone());
        }
    }

    fn set_drop_flag_local(&mut self, flag: Local, initialized: bool, span: Option<Span>) {
        if let Some(current) = self.current_block {
            self.blocks[current.0]
                .statements
                .push(StatementData::assign(
                    Place {
                        local: flag,
                        projection: vec![],
                    },
                    Rvalue::Use(Operand::Constant(Constant::Bool(initialized))),
                    span,
                ));
        }
    }

    pub(super) fn mark_place_initialized(&mut self, place: &Place, span: Option<Span>) {
        let Some(ty) = self.get_local_type(place.local) else {
            return;
        };
        if self.type_needs_cleanup(&ty) {
            self.set_drop_flags_for_place_tree(place, true, span);
        }
    }
}
