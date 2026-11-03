use std::collections::{HashMap, HashSet};

use crate::diagnostic::Diagnostics;
use crate::ids::TypeId;
use crate::lexer::Span;
use crate::mir::{MirBackendContract, MirFunction, Place, Projection};
use crate::types::Type;

use super::*;

pub(super) struct MoveValidationContext<'a> {
    type_context: &'a crate::type_context::TypeContext,
    backend_contract: &'a MirBackendContract,
    direct_drop_types: HashSet<TypeId>,
    cleanup_cache: HashMap<Type, bool>,
}

impl<'a> MoveValidationContext<'a> {
    pub(super) fn new(
        type_context: &'a crate::type_context::TypeContext,
        backend_contract: &'a MirBackendContract,
    ) -> Self {
        Self {
            type_context,
            backend_contract,
            direct_drop_types: backend_contract.drop_glue.keys().copied().collect(),
            cleanup_cache: HashMap::new(),
        }
    }

    pub(super) fn validate_move(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
    ) {
        self.validate_place(place, func, span, diagnostics, true);
    }

    pub(super) fn validate_copy(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
    ) {
        self.validate_place(place, func, span, diagnostics, false);
    }

    fn validate_place(
        &mut self,
        place: &Place,
        func: &MirFunction,
        span: Option<Span>,
        diagnostics: &mut Diagnostics,
        validate_move_only_rules: bool,
    ) {
        let Some(local) = func.local_decls.get(place.local.0) else {
            return;
        };
        let mut current_ty = self
            .backend_contract
            .normalize_type(self.type_context, &self.type_context.type_for(local.ty));
        let mut reference_owned_path = false;

        for projection in &place.projection {
            current_ty = self
                .backend_contract
                .normalize_type(self.type_context, &current_ty);
            match projection {
                Projection::Deref => match &current_ty {
                    Type::Reference { .. } => reference_owned_path = true,
                    Type::Pointer(_) => reference_owned_path = false,
                    _ => {}
                },
                Projection::Field { .. }
                    if validate_move_only_rules && self.has_direct_drop(&current_ty) =>
                {
                    diagnostics.push(mir_diagnostic(
                        format!(
                            "Cannot move field out of type '{}' because it implements Drop",
                            self.type_display_name(&current_ty)
                        ),
                        span.clone(),
                    ));
                    return;
                }
                Projection::Index(_) if validate_move_only_rules => {
                    if let Type::Array(elem_ty, _) = &current_ty {
                        let elem_ty = elem_ty.as_ref().clone();
                        if self.type_needs_cleanup(&elem_ty) {
                            diagnostics.push(mir_diagnostic(
                                "Cannot move cleanup value out of array element".to_string(),
                                span.clone(),
                            ));
                            return;
                        }
                    }
                }
                _ => {}
            }

            let Some(next_ty) = self.project_type(&current_ty, projection) else {
                return;
            };
            current_ty = self
                .backend_contract
                .normalize_type(self.type_context, &next_ty);
        }

        if reference_owned_path && !current_ty.is_copy() {
            diagnostics.push(mir_diagnostic(
                format!(
                    "Cannot move non-copy value of type '{}' out of a reference",
                    self.type_display_name(&current_ty)
                ),
                span,
            ));
        }
    }

    fn has_direct_drop(&self, ty: &Type) -> bool {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        self.type_context
            .id_for_type(&ty)
            .is_some_and(|id| self.direct_drop_types.contains(&id))
    }

    fn type_needs_cleanup(&mut self, ty: &Type) -> bool {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        if let Some(needs_cleanup) = self.cleanup_cache.get(&ty) {
            return *needs_cleanup;
        }

        let needs_cleanup = if self.has_direct_drop(&ty) {
            true
        } else {
            match &ty {
                Type::Struct { id, args } => contract_struct_fields(self.backend_contract, *id)
                    .is_some_and(|layout_fields| {
                        let fields = layout_fields
                            .iter()
                            .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                            .collect::<Vec<_>>();
                        let subst = generic_substitution_for_fields(
                            &fields,
                            &Type::Struct {
                                id: *id,
                                args: args.clone(),
                            },
                        );
                        fields
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&subst))
                            .any(|field_ty| self.type_needs_cleanup(&field_ty))
                    }),
                Type::Enum { id, args } => contract_enum_variants(self.backend_contract, *id)
                    .is_some_and(|variants| {
                        let fields = variants
                            .iter()
                            .flat_map(|variant| match &variant.fields {
                                crate::mir::MirVariantLayoutFields::Unit => Vec::new(),
                                crate::mir::MirVariantLayoutFields::Positional(fields) => fields
                                    .iter()
                                    .map(|field_ty| self.type_context.type_for(*field_ty))
                                    .collect(),
                                crate::mir::MirVariantLayoutFields::Named(fields) => fields
                                    .iter()
                                    .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                                    .collect(),
                            })
                            .collect::<Vec<_>>();
                        let subst = generic_substitution_for_fields(
                            &fields,
                            &Type::Enum {
                                id: *id,
                                args: args.clone(),
                            },
                        );
                        fields
                            .into_iter()
                            .map(|field_ty| field_ty.substitute_generics(&subst))
                            .any(|field_ty| self.type_needs_cleanup(&field_ty))
                    }),
                Type::Array(elem, _) => self.type_needs_cleanup(elem),
                Type::Tuple(elems) => elems.iter().any(|elem| self.type_needs_cleanup(elem)),
                Type::Function { .. } => true,
                _ => false,
            }
        };

        self.cleanup_cache.insert(ty, needs_cleanup);
        needs_cleanup
    }

    fn project_type(&self, ty: &Type, projection: &Projection) -> Option<Type> {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        let projected = match projection {
            Projection::Deref => match &ty {
                Type::Reference { inner, .. } | Type::Pointer(inner) => {
                    Some(inner.as_ref().clone())
                }
                _ => None,
            },
            Projection::Field { index, .. } => match &ty {
                Type::Tuple(elems) => elems.get(*index).cloned(),
                Type::Struct { id, args } => {
                    let layout_fields = contract_struct_fields(self.backend_contract, *id)?;
                    let field_ty = layout_fields.get(*index).map(|(_, ty)| *ty)?;
                    let fields = layout_fields
                        .iter()
                        .map(|(_, field_ty)| self.type_context.type_for(*field_ty))
                        .collect::<Vec<_>>();
                    let subst = generic_substitution_for_fields(
                        &fields,
                        &Type::Struct {
                            id: *id,
                            args: args.clone(),
                        },
                    );
                    Some(
                        self.type_context
                            .type_for(field_ty)
                            .substitute_generics(&subst),
                    )
                }
                _ => None,
            },
            Projection::Index(_) => match &ty {
                Type::Array(elem, _) | Type::Slice(elem) => Some(elem.as_ref().clone()),
                Type::Str => Some(Type::U8),
                _ => None,
            },
            Projection::Downcast(_) => Some(ty.clone()),
        }?;
        Some(
            self.backend_contract
                .normalize_type(self.type_context, &projected),
        )
    }

    fn type_display_name(&self, ty: &Type) -> String {
        let ty = self.backend_contract.normalize_type(self.type_context, ty);
        let context = crate::type_services::display::TypeDisplayContext::default();
        crate::type_services::display::display_type_with_context(&ty, &context).to_string()
    }
}
