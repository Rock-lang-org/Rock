//! Instance and tuple lowering.

use crate::ast;
use crate::hir::*;
use crate::lower::{path_names, Lowerer};
use crate::types::{GenericParamId, Type};
use std::collections::HashMap;

impl Lowerer {
    pub(crate) fn lower_instance(&mut self, inst: &ast::Instance) -> HirExpr {
        let span = Self::full_path_span(&inst.name.path);
        if matches!(
            inst.name.path.as_slice(),
            [ast::IdentOrType::Type(ast::ParseType::Unit(_))]
        ) && inst.fields.is_empty()
        {
            return HirExpr {
                ty: Type::Unit,
                kind: HirExprKind::Unit,
                span,
            };
        }
        let segments = path_names(&inst.name.path);

        let struct_resolution = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_struct_literal_path(&segments);
        let type_name = struct_resolution.name;

        // Check for Enum::Variant pattern (2-segment path where first is an enum)
        if let Some(resolved) = crate::lower::resolution::LowerResolutionContext::new(self)
            .resolve_enum_variant_path(&segments)
        {
            let enum_info = resolved.owner;
            let variant = resolved.variant;
            self.record_source_reference(
                span.clone(),
                crate::source_map::SourceSymbol::Variant {
                    owner: enum_info.id,
                    variant: variant.id,
                },
            );
            let args: Vec<HirExpr> = match &variant.fields {
                HirVariantFields::Named(fields) => fields
                    .iter()
                    .filter_map(|field| {
                        inst.fields
                            .iter()
                            .find(|(ident, _)| ident.name == field.name)
                            .map(|(_, expr)| self.lower_expression(expr))
                    })
                    .collect(),
                _ => inst
                    .fields
                    .values()
                    .map(|expr| self.lower_expression(expr))
                    .collect(),
            };

            // Handle generic enum type params
            let type_args = if !enum_info.generic_params.is_empty() {
                // Create fresh type vars for each generic param
                let fresh = self
                    .engine
                    .fresh_generic_args(&enum_info.generic_params, &span);
                let type_var_mapping: HashMap<GenericParamId, Type> = enum_info
                    .generic_params
                    .iter()
                    .zip(fresh)
                    .map(|(param, ty)| (param.id, ty))
                    .collect();
                // Unify type vars with arg types based on variant field types
                match &variant.fields {
                    HirVariantFields::Positional(field_types) => {
                        for (arg, field_ty) in args.iter().zip(field_types.iter()) {
                            let expected = field_ty.substitute_generics(&type_var_mapping);
                            let _ = self.engine.unify(&arg.ty, &expected);
                        }
                    }
                    HirVariantFields::Named(fields) => {
                        for (arg, field) in args.iter().zip(fields.iter()) {
                            let expected = field.ty.substitute_generics(&type_var_mapping);
                            let _ = self.engine.unify(&arg.ty, &expected);
                        }
                    }
                    HirVariantFields::Unit => {}
                }
                // Resolve type vars to concrete types
                enum_info
                    .generic_params
                    .iter()
                    .map(|param| {
                        if let Some(tv) = type_var_mapping.get(&param.id) {
                            self.engine.resolve(tv)
                        } else {
                            Type::Error
                        }
                    })
                    .collect()
            } else {
                vec![]
            };

            return HirExpr {
                ty: Type::Enum {
                    id: enum_info.id,
                    args: type_args,
                },
                kind: HirExprKind::EnumVariant(
                    resolved.owner_name,
                    variant.name.clone(),
                    args,
                    Some(HirVariantLocation {
                        owner: enum_info.id,
                        variant_id: variant.id,
                        name: variant.name.clone(),
                    }),
                ),
                span,
            };
        }

        // Check if this is a known struct
        if let Some(hir_struct) = struct_resolution.structure {
            let current_impl_matches = self.current_struct_impl.as_deref()
                == Some(type_name.as_str())
                || self.current_struct_impl.as_ref().is_some_and(|name| {
                    crate::lower::resolution::LowerResolutionContext::new(self)
                        .resolve_item_id(name)
                        .is_some_and(|id| id == hir_struct.id)
                });

            if !current_impl_matches && hir_struct.fields.iter().any(|field| !field.public) {
                self.diagnostics.push_with_span(
                    format!(
                        "Cannot construct struct '{}' because it has private fields",
                        type_name
                    ),
                    span.clone(),
                );
            }

            // For generic structs, create type variables for each generic parameter
            // and infer them from field values
            let generic_params = hir_struct.generic_params.clone();

            // Create a mapping from generic param names to fresh type variables
            let fresh = self.engine.fresh_generic_args(&generic_params, &span);
            let type_var_mapping: HashMap<GenericParamId, Type> = generic_params
                .iter()
                .zip(fresh)
                .map(|(param, ty)| (param.id, ty))
                .collect();

            let mut fields = Vec::new();
            for (ident, expr) in &inst.fields {
                let mut hir_expr = self.lower_expression(expr);

                // Find the expected field type and substitute generic params
                if let Some(field_info) = hir_struct.fields.iter().find(|f| f.name == ident.name) {
                    let expected_ty = field_info.ty.substitute_generics(&type_var_mapping);
                    hir_expr = self.coerce_argument_to_expected(hir_expr, &expected_ty);
                    // Unify expected type with actual type
                    let _ = self.engine.unify(&expected_ty, &hir_expr.ty);
                }

                let field = hir_struct
                    .fields
                    .iter()
                    .find(|f| f.name == ident.name)
                    .map(|f| HirFieldLocation {
                        owner: hir_struct.id,
                        field_id: f.id,
                        name: ident.name.clone(),
                    });
                if let Some(field) = field.as_ref() {
                    self.record_source_reference(
                        ident.span.clone(),
                        crate::source_map::SourceSymbol::Field {
                            owner: field.owner,
                            field: field.field_id,
                        },
                    );
                }
                fields.push(HirStructLiteralField {
                    name: ident.name.clone(),
                    value: hir_expr,
                    field,
                });
            }

            // Resolve the type variables to get actual type arguments
            let type_args: Vec<Type> = generic_params
                .iter()
                .map(|param| {
                    if let Some(tv) = type_var_mapping.get(&param.id) {
                        self.engine.resolve(tv)
                    } else {
                        Type::Error
                    }
                })
                .collect();
            HirExpr {
                ty: Type::Struct {
                    id: hir_struct.id,
                    args: type_args,
                },
                kind: HirExprKind::StructLiteral(type_name, Some(hir_struct.id), fields),
                span,
            }
        } else {
            // Unknown struct - lower fields and use a type variable
            let mut fields = Vec::new();
            for (ident, expr) in &inst.fields {
                let hir_expr = self.lower_expression(expr);
                fields.push(HirStructLiteralField {
                    name: ident.name.clone(),
                    value: hir_expr,
                    field: None,
                });
            }

            // Might be an enum variant with named fields
            let ty = self.engine.fresh_type_var_at(span.clone());
            HirExpr {
                ty,
                kind: HirExprKind::StructLiteral(type_name, None, fields),
                span,
            }
        }
    }

    pub(crate) fn lower_tuple(&mut self, tuple: &ast::Tuple) -> HirExpr {
        let elements: Vec<HirExpr> = tuple
            .elements
            .iter()
            .map(|e| self.lower_expression(e))
            .collect();
        let span = elements
            .first()
            .map(|element| element.span.clone())
            .expect("tuple requires a source element span");
        let types: Vec<Type> = elements.iter().map(|e| e.ty.clone()).collect();
        HirExpr {
            ty: Type::Tuple(types),
            kind: HirExprKind::TupleLiteral(elements),
            span,
        }
    }
}
