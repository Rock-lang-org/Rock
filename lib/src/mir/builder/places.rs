use std::collections::{HashMap, HashSet};

use crate::hir::{HirVarTarget, HirVariantFields};
use crate::ids::VariantId;
use crate::types::Type;

use crate::mir::{Operand, Place, Projection};

use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn generic_substitution_for_fields(
        fields: &[Type],
        source_ty: &Type,
    ) -> HashMap<crate::types::GenericParamId, Type> {
        let args = match source_ty {
            Type::Enum { args, .. } | Type::Struct { args, .. } => args,
            _ => return HashMap::new(),
        };
        let mut generic_params = HashSet::new();
        for field_ty in fields {
            field_ty.collect_generic_params(&mut generic_params);
        }
        generic_params
            .into_iter()
            .filter_map(|param| {
                args.get(param.index as usize)
                    .cloned()
                    .map(|arg| (param, arg))
            })
            .collect()
    }

    pub(super) fn struct_field_types_with_substitution(
        program: &HirProgram,
        def_id: crate::ids::DefId,
        generic_args: &[Type],
    ) -> Vec<(crate::ids::FieldId, Type)> {
        let Some((_, structure)) = program.struct_by_id(def_id) else {
            return Vec::new();
        };
        let generic_subst = Self::generic_substitution_for_fields(
            &structure
                .fields
                .iter()
                .map(|f| f.ty.clone())
                .collect::<Vec<_>>(),
            &Type::Struct {
                id: def_id,
                args: generic_args.to_vec(),
            },
        );
        structure
            .fields
            .iter()
            .map(|field| {
                let field_ty = field.ty.substitute_generics(&generic_subst);
                (field.id, field_ty)
            })
            .collect()
    }

    pub(super) fn lower_place(&mut self, expr: &HirExpr) -> Option<Place> {
        match &expr.kind {
            HirExprKind::Var(name) => {
                if let Some(local) = self.var_map.get(name) {
                    Some(self.place_for_local_expr(*local, &expr.ty))
                } else {
                    None
                }
            }
            HirExprKind::ResolvedVar(reference)
                if matches!(reference.target, HirVarTarget::Local(_)) =>
            {
                self.var_map
                    .get(&reference.name)
                    .map(|local| self.place_for_local_expr(*local, &expr.ty))
            }
            HirExprKind::Deref(base) => {
                let mut place = if let Some(place) = self.lower_place(base) {
                    place
                } else {
                    let base_temp = self.new_local_from_expr(base.ty.clone(), base);
                    self.register_scoped_temp(base_temp);
                    self.emit_storage_live(base_temp, Some(base.span.clone()));
                    let base_place = Place {
                        local: base_temp,
                        projection: vec![],
                    };
                    self.lower_expr(base, base_place.clone());
                    base_place
                };
                place.projection.push(Projection::Deref);
                Some(place)
            }
            HirExprKind::FieldAccess(base, field_name, location) => {
                let mut place = self.lower_place(base)?;
                let struct_id = location
                    .as_ref()
                    .map(|location| location.owner)
                    .or_else(|| match &base.ty {
                        Type::Struct { id, .. } => Some(*id),
                        _ => None,
                    })?;
                let (_, fields) = self.program.struct_by_id(struct_id)?;
                let location = location.as_ref().filter(|location| {
                    location.owner == fields.id
                        && fields.fields.iter().any(|field| {
                            field.id == location.field_id
                                && field.name == location.name
                                && field.name == *field_name
                        })
                })?;
                let field_idx = fields
                    .fields
                    .iter()
                    .position(|field| field.id == location.field_id)?;
                place.projection.push(self.field_projection(
                    field_idx,
                    Some(location.owner),
                    Some(location.field_id),
                ));
                Some(place)
            }
            HirExprKind::TupleIndex(base, idx) => {
                let mut place = self.lower_place(base)?;
                match &base.ty {
                    Type::Tuple(elems) if (*idx as usize) < elems.len() => {
                        place
                            .projection
                            .push(self.field_projection(*idx as usize, None, None));
                        Some(place)
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn operand_for_place(
        &self,
        ty: &Type,
        place: Place,
        borrow_context: bool,
    ) -> Operand {
        if borrow_context {
            if matches!(ty, Type::Reference { mutable: true, .. }) {
                Operand::Move(place)
            } else {
                Operand::Copy(place)
            }
        } else if matches!(
            self.get_local_type(place.local),
            Some(Type::Reference { mutable: false, .. })
        ) && matches!(place.projection.first(), Some(Projection::Deref))
            && !self.type_needs_cleanup(ty)
        {
            Operand::Copy(place)
        } else if Self::needs_move(ty) {
            Operand::Move(place)
        } else {
            Operand::Copy(place)
        }
    }

    pub(super) fn enum_variant_field_types(
        &self,
        enum_id: crate::ids::DefId,
        variant_id: VariantId,
    ) -> Vec<Type> {
        Self::enum_variant_field_types_for(self.program, enum_id, variant_id)
    }

    pub(super) fn enum_variant_field_types_for(
        program: &HirProgram,
        enum_id: crate::ids::DefId,
        variant_id: VariantId,
    ) -> Vec<Type> {
        let Some((_, enumeration)) = program.enum_by_id(enum_id) else {
            return Vec::new();
        };
        let Some(variant) = enumeration
            .variants
            .iter()
            .find(|variant| variant.id == variant_id)
        else {
            return Vec::new();
        };
        match &variant.fields {
            HirVariantFields::Named(fields) => {
                fields.iter().map(|field| field.ty.clone()).collect()
            }
            HirVariantFields::Positional(fields) => fields.clone(),
            HirVariantFields::Unit => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
