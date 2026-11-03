use std::collections::HashMap;

use crate::ids::TypeId;
use crate::source_map::SourceSymbol;
use crate::types::{GenericParamId, Type};

use super::hir_types::{HirProgram, HirVariantFields};
use super::Monomorphizer;

impl Monomorphizer {
    pub(super) fn register_nominal_field_types(&mut self, program: &HirProgram) {
        self.nominal_field_types.clear();
        self.nominal_field_spans.clear();
        for strukt in program.structs.values() {
            self.nominal_field_types.insert(
                strukt.id,
                strukt.fields.iter().map(|field| field.ty.clone()).collect(),
            );
            self.nominal_field_spans.insert(
                strukt.id,
                strukt
                    .fields
                    .iter()
                    .map(|field| {
                        self.source_map
                            .symbol(&SourceSymbol::Field {
                                owner: strukt.id,
                                field: field.id,
                            })
                            .map(|info| info.name_span.clone())
                    })
                    .collect(),
            );
        }
        for enm in program.enums.values() {
            let mut fields = Vec::new();
            let mut spans = Vec::new();
            for variant in &enm.variants {
                match &variant.fields {
                    HirVariantFields::Named(named) => {
                        fields.extend(named.iter().map(|field| field.ty.clone()));
                        spans.extend(named.iter().map(|field| {
                            self.source_map
                                .symbol(&SourceSymbol::Field {
                                    owner: enm.id,
                                    field: field.id,
                                })
                                .map(|info| info.name_span.clone())
                        }));
                    }
                    HirVariantFields::Positional(positional) => {
                        fields.extend(positional.iter().cloned());
                        spans.extend(std::iter::repeat(None).take(positional.len()));
                    }
                    HirVariantFields::Unit => {}
                }
            }
            self.nominal_field_types.insert(enm.id, fields);
            self.nominal_field_spans.insert(enm.id, spans);
        }
    }

    pub(super) fn monomorphize_drop_fields_for_type(&mut self, ty: &Type) {
        let (Type::Struct { id, args } | Type::Enum { id, args }) = ty else {
            return;
        };
        let Some(field_types) = self.nominal_field_types.get(id).cloned() else {
            return;
        };
        let type_key = self.intern_type(ty);
        if !self
            .drop_field_monomorphization_in_progress
            .insert(type_key)
        {
            return;
        }

        let subst: HashMap<_, _> = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                (
                    GenericParamId {
                        owner: *id,
                        index: index as u32,
                    },
                    arg.clone(),
                )
            })
            .collect();
        let field_spans = self
            .nominal_field_spans
            .get(id)
            .cloned()
            .unwrap_or_default();
        for (index, field_ty) in field_types.into_iter().enumerate() {
            self.monomorphize_drop_for_type(
                &field_ty.substitute_generics(&subst),
                field_spans.get(index).cloned().flatten(),
            );
        }

        self.drop_field_monomorphization_in_progress
            .remove(&type_key);
    }

    pub(super) fn monomorphize_known_drop_types(&mut self) {
        let mut index = 0;
        while index < self.type_context.len() {
            let ty = self.type_for(TypeId(index as u32));
            index += 1;
            self.monomorphize_drop_for_type(&ty, None);
        }
    }
}
