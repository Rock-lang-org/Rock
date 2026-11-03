use crate::hir::HirVariantLocation;
use crate::ids::DefId;
use crate::lexer::Span;
use crate::types::Type;

use crate::hir::AcceptedHir;
use crate::mir::builder::{HirExpr, MirBuilder};
use crate::mir::{AggregateKind, Operand, Place, Rvalue};

impl<'a> MirBuilder<'a> {
    pub(super) fn lower_struct_literal_expr(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        span: Option<Span>,
        name: &String,
        explicit_id: &Option<DefId>,
        fields: &[crate::hir::HirStructLiteralFieldFor<AcceptedHir>],
    ) {
        let mut lowered_fields = Vec::new();
        for field in fields {
            let field_expr = &field.value;
            let field_temp = self.new_local_from_expr(field_expr.ty.clone(), field_expr);
            let field_place = Place {
                local: field_temp,
                projection: vec![],
            };
            self.lower_expr(field_expr, field_place.clone());
            lowered_fields.push((field, Operand::Copy(field_place)));
        }

        let struct_id = explicit_id.or_else(|| match &expr.ty {
            Type::Struct { id, .. } => Some(*id),
            _ => None,
        });
        let Some(struct_id) = struct_id else {
            panic!("struct literal MIR lowering requires canonical struct id for {name}");
        };

        let Some((_, struct_def)) = self.program.struct_by_id(struct_id) else {
            panic!("struct literal MIR lowering missing struct definition for {struct_id:?}");
        };

        let mut ordered: Vec<Option<Operand>> = vec![None; struct_def.fields.len()];
        for (field, operand) in lowered_fields {
            let Some(location) = field.field.as_ref() else {
                panic!(
                    "struct literal MIR lowering requires HIR field location for {}.{}",
                    name, field.name
                );
            };
            let Some(field_idx) = struct_def.fields.iter().position(|struct_field| {
                location.owner == struct_def.id
                    && struct_field.id == location.field_id
                    && struct_field.name == location.name
                    && struct_field.name == field.name
            }) else {
                panic!(
                    "struct literal MIR lowering found mismatched HIR field location for {}.{}",
                    name, field.name
                );
            };

            if let Some(slot) = ordered.get_mut(field_idx) {
                *slot = Some(operand);
            }
        }

        let field_operands = ordered
            .into_iter()
            .enumerate()
            .map(|(field_idx, operand)| {
                operand.unwrap_or_else(|| {
                    panic!(
                        "struct literal MIR lowering missing operand for field index {field_idx} of {name}"
                    )
                })
            })
            .collect();

        let rvalue = Rvalue::Aggregate(
            AggregateKind::Struct {
                id: struct_id,
                display_name: name.clone(),
            },
            field_operands,
        );
        self.emit_assign(dest, rvalue, span);
    }

    pub(super) fn lower_enum_variant_expr(
        &mut self,
        expr: &HirExpr,
        dest: Place,
        span: Option<Span>,
        enum_name: &String,
        variant_name: &String,
        args: &[HirExpr],
        location: &Option<HirVariantLocation>,
    ) {
        let field_types = location
            .as_ref()
            .map(|location| {
                let field_types =
                    self.enum_variant_field_types(location.owner, location.variant_id);
                let generic_subst = match &expr.ty {
                    Type::Enum { args, .. } => {
                        let mut generic_params = std::collections::HashSet::new();
                        for field_ty in &field_types {
                            field_ty.collect_generic_params(&mut generic_params);
                        }
                        generic_params
                            .into_iter()
                            .filter_map(|param| {
                                args.get(param.index as usize)
                                    .cloned()
                                    .map(|arg| (param, arg))
                            })
                            .collect::<std::collections::HashMap<_, _>>()
                    }
                    _ => std::collections::HashMap::new(),
                };
                field_types
                    .into_iter()
                    .map(|ty| ty.substitute_generics(&generic_subst))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut arg_operands = Vec::new();
        for (index, arg) in args.iter().enumerate() {
            let arg_ty = field_types
                .get(index)
                .cloned()
                .unwrap_or_else(|| arg.ty.clone());
            let arg_temp = self.new_local_from_expr(arg_ty, arg);
            let arg_place = Place {
                local: arg_temp,
                projection: vec![],
            };
            self.lower_expr(arg, arg_place.clone());
            arg_operands.push(Operand::Copy(arg_place));
        }

        if let Some(location) = location {
            self.emit_assign(
                dest,
                Rvalue::Aggregate(
                    AggregateKind::EnumVariant {
                        enum_id: location.owner,
                        variant_id: location.variant_id,
                        enum_name: enum_name.clone(),
                        variant_name: variant_name.clone(),
                    },
                    arg_operands,
                ),
                span,
            );
        }
    }
}
