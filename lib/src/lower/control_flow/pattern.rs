use std::collections::HashMap;

use crate::ast;
use crate::hir::*;
use crate::lower::path_names;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};

fn identifier_path_span(path: &ast::TypePath) -> crate::lexer::Span {
    match path
        .path
        .last()
        .expect("pattern identifier path must contain a source segment")
    {
        ast::IdentOrType::Ident(ident) => ident.span.clone(),
        ast::IdentOrType::Type(ty) => ty.span(),
    }
}

fn inferred_pattern_span(
    lowerer: &Lowerer,
    pattern: &ast::Pattern,
    expected_ty: &Type,
) -> Option<crate::lexer::Span> {
    lowerer
        .engine
        .span_for_type(expected_ty)
        .or_else(|| Lowerer::pattern_binding_span(pattern))
}

impl Lowerer {
    fn report_pattern_type_mismatch(
        &mut self,
        actual: &Type,
        expected: &str,
        span: crate::lexer::Span,
    ) {
        let actual = self.display_type(actual);
        self.diagnostics.push_type_with_span(
            format!(
                "pattern type mismatch: Type mismatch: {} vs {}",
                actual, expected
            ),
            span,
        );
    }

    pub(crate) fn lower_pattern(
        &mut self,
        pattern: &ast::Pattern,
        expected_ty: &Type,
    ) -> HirPattern {
        match &pattern.kind {
            ast::PatternKind::Wildcard => HirPattern::Wildcard,
            ast::PatternKind::Ident(ident_pat) => {
                let ty = expected_ty.clone();
                let local_id = self.fresh_local_id();
                if let Some(owner) = self.current_body_def_id() {
                    self.source_map.insert_local_in_scope(
                        owner,
                        local_id,
                        ident_pat.name.span.clone(),
                        self.source_scope_stack.last().copied(),
                    );
                }
                self.scope
                    .define_local(ident_pat.name.name.clone(), ty, ident_pat.mut_, local_id);
                HirPattern::Binding {
                    name: ident_pat.name.name.clone(),
                    local_id,
                    mutable: ident_pat.mut_,
                }
            }
            ast::PatternKind::Literal(lit) => match &lit.kind {
                ast::LiteralKind::Bool(b) => HirPattern::Literal(HirLiteralPattern::Bool(*b)),
                ast::LiteralKind::Number(n) => {
                    HirPattern::Literal(HirLiteralPattern::Int(*n as i64))
                }
                ast::LiteralKind::Float(f) => HirPattern::Literal(HirLiteralPattern::Float(*f)),
                ast::LiteralKind::String(s) => {
                    let resolved = self.engine.resolve(expected_ty);
                    let str_ref = Type::Reference {
                        mutable: false,
                        inner: Box::new(Type::Str),
                    };
                    if matches!(&resolved, Type::TypeVar(_)) {
                        let _ = self.engine.unify(expected_ty, &str_ref);
                    } else if !matches!(
                        &resolved,
                        Type::Reference { inner, .. } if matches!(inner.as_ref(), Type::Str)
                    ) {
                        let actual = self.display_type(&resolved);
                        self.diagnostics.push_type_with_span(
                            format!(
                                "string literal pattern requires a &Str scrutinee, got {}; call .as_str! before matching an owned String",
                                actual
                            ),
                            lit.span.clone(),
                        );
                        return HirPattern::Wildcard;
                    }
                    HirPattern::Literal(HirLiteralPattern::String(s.clone()))
                }
                ast::LiteralKind::Char(_) => match self.lower_literal(lit).kind {
                    HirExprKind::CharLiteral(value) => {
                        HirPattern::Literal(HirLiteralPattern::Char(value))
                    }
                    _ => HirPattern::Wildcard,
                },
                _ => HirPattern::Wildcard,
            },
            ast::PatternKind::Tuple(pats) => {
                let span = inferred_pattern_span(self, pattern, expected_ty);
                let elem_types: Vec<Type> = (0..pats.len())
                    .map(|_| {
                        span.clone()
                            .map(|span| self.engine.fresh_type_var_at(span))
                            .unwrap_or_else(|| self.engine.fresh_type_var())
                    })
                    .collect();
                let tuple_ty = Type::Tuple(elem_types.clone());
                let _ = self.engine.unify(expected_ty, &tuple_ty);

                let patterns: Vec<HirPattern> = pats
                    .iter()
                    .zip(elem_types.iter())
                    .map(|(p, t)| self.lower_pattern(p, t))
                    .collect();
                HirPattern::Tuple(patterns)
            }
            ast::PatternKind::Instance(inst_pat) => {
                let type_name: String = path_names(&inst_pat.name.path).join("::");

                match &inst_pat.args {
                    ast::FieldsPatternOrArgumentsPattern::Arguments(args) => {
                        if type_name.contains("::") {
                            let parts: Vec<&str> = type_name.split("::").collect();
                            if parts.len() == 2 {
                                'qualified_enum: {
                                    let enum_name = parts[0].to_string();
                                    let variant_name = parts[1].to_string();
                                    let span = identifier_path_span(&inst_pat.name);

                                    let Some(enum_info) =
                                        crate::lower::resolution::LowerResolutionContext::new(self)
                                            .resolve_enum_type(&enum_name)
                                    else {
                                        break 'qualified_enum;
                                    };

                                    let resolved_scrutinee = self.engine.resolve(expected_ty);
                                    let scrutinee_type_args = match &resolved_scrutinee {
                                        Type::Enum { id, args } if *id == enum_info.id => {
                                            args.clone()
                                        }
                                        Type::Enum { .. } | Type::Struct { .. } => {
                                            self.report_pattern_type_mismatch(
                                                &resolved_scrutinee,
                                                &enum_name,
                                                span,
                                            );
                                            return HirPattern::Wildcard;
                                        }
                                        Type::TypeVar(_) | Type::Apply { .. } => {
                                            let fresh_type_args: Vec<Type> = enum_info
                                                .generic_params
                                                .iter()
                                                .map(|param| {
                                                    self.engine.fresh_type_var_at_kind(
                                                        span.clone(),
                                                        param.kind.clone(),
                                                    )
                                                })
                                                .collect();
                                            let enum_ty = Type::Enum {
                                                id: enum_info.id,
                                                args: fresh_type_args.clone(),
                                            };
                                            if let Err(err) =
                                                self.engine.unify(expected_ty, &enum_ty)
                                            {
                                                self.diagnostics.push_type_with_span(
                                                    format!(
                                                        "pattern type mismatch: {}",
                                                        err.render(&self.engine)
                                                    ),
                                                    span,
                                                );
                                                return HirPattern::Wildcard;
                                            }
                                            fresh_type_args
                                        }
                                        _ => {
                                            self.report_pattern_type_mismatch(
                                                &resolved_scrutinee,
                                                &enum_name,
                                                span,
                                            );
                                            return HirPattern::Wildcard;
                                        }
                                    };

                                    let subst: HashMap<GenericParamId, Type> = enum_info
                                        .generic_params
                                        .iter()
                                        .enumerate()
                                        .zip(scrutinee_type_args.iter())
                                        .map(|((index, _), concrete)| {
                                            (
                                                GenericParamId {
                                                    owner: enum_info.id,
                                                    index: index as u32,
                                                },
                                                concrete.clone(),
                                            )
                                        })
                                        .collect();

                                    let Some(variant) =
                                        enum_info.variants.iter().find(|v| v.name == variant_name)
                                    else {
                                        self.diagnostics.push_type_with_span(
                                            format!(
                                                "pattern type mismatch: enum {} has no variant {}",
                                                enum_name, variant_name
                                            ),
                                            span,
                                        );
                                        return HirPattern::Wildcard;
                                    };

                                    let arg_types: Vec<Type> = match &variant.fields {
                                        HirVariantFields::Positional(field_types) => field_types
                                            .iter()
                                            .map(|ft| ft.substitute_generics(&subst))
                                            .collect(),
                                        _ => args
                                            .iter()
                                            .map(|_| {
                                                self.engine.fresh_type_var_at(identifier_path_span(
                                                    &inst_pat.name,
                                                ))
                                            })
                                            .collect(),
                                    };

                                    let patterns: Vec<HirPattern> = args
                                        .iter()
                                        .zip(arg_types.iter())
                                        .map(|(p, t)| self.lower_pattern(p, t))
                                        .collect();

                                    return HirPattern::Enum(
                                        enum_name,
                                        variant_name.clone(),
                                        Some(HirVariantLocation {
                                            owner: enum_info.id,
                                            variant_id: variant.id,
                                            name: variant_name.clone(),
                                        }),
                                        patterns,
                                    );
                                }
                            }
                        }

                        let resolved_expected_ty = self.engine.resolve(expected_ty);
                        match &resolved_expected_ty {
                            Type::Enum {
                                id: expected_enum_id,
                                args: scrutinee_type_args,
                            } => {
                                if let Some(enum_info) =
                                    self.items.enumeration(*expected_enum_id).cloned()
                                {
                                    let enum_name =
                                        crate::lower::resolution::LowerResolutionContext::new(self)
                                            .canonical_name(*expected_enum_id)
                                            .map(str::to_string)
                                            .unwrap_or_else(|| enum_info.name.clone());
                                    if let Some(variant) =
                                        enum_info.variants.iter().find(|v| v.name == type_name)
                                    {
                                        let subst: HashMap<GenericParamId, Type> = enum_info
                                            .generic_params
                                            .iter()
                                            .enumerate()
                                            .zip(scrutinee_type_args.iter())
                                            .map(|((index, _), concrete)| {
                                                (
                                                    GenericParamId {
                                                        owner: enum_info.id,
                                                        index: index as u32,
                                                    },
                                                    concrete.clone(),
                                                )
                                            })
                                            .collect();

                                        let arg_types: Vec<Type> = match &variant.fields {
                                            HirVariantFields::Positional(field_types) => {
                                                field_types
                                                    .iter()
                                                    .map(|ft| ft.substitute_generics(&subst))
                                                    .collect()
                                            }
                                            _ => args
                                                .iter()
                                                .map(|_| {
                                                    self.engine.fresh_type_var_at(
                                                        identifier_path_span(&inst_pat.name),
                                                    )
                                                })
                                                .collect(),
                                        };

                                        let patterns: Vec<HirPattern> = args
                                            .iter()
                                            .zip(arg_types.iter())
                                            .map(|(p, t)| self.lower_pattern(p, t))
                                            .collect();

                                        return HirPattern::Enum(
                                            enum_name,
                                            type_name.clone(),
                                            Some(HirVariantLocation {
                                                owner: enum_info.id,
                                                variant_id: variant.id,
                                                name: type_name.clone(),
                                            }),
                                            patterns,
                                        );
                                    }

                                    let span = identifier_path_span(&inst_pat.name);
                                    self.diagnostics.push_type_with_span(
                                        format!(
                                            "pattern type mismatch: enum {} has no variant {}",
                                            enum_name, type_name
                                        ),
                                        span,
                                    );
                                    return HirPattern::Wildcard;
                                }

                                let span = identifier_path_span(&inst_pat.name);
                                let actual = self.display_type(&resolved_expected_ty);
                                self.diagnostics.push_type_with_span(
                                    format!(
                                        "pattern type mismatch: unknown enum scrutinee {}",
                                        actual
                                    ),
                                    span,
                                );
                                return HirPattern::Wildcard;
                            }
                            Type::TypeVar(_) | Type::Apply { .. } => {}
                            _ => {
                                let span = identifier_path_span(&inst_pat.name);
                                self.report_pattern_type_mismatch(
                                    &resolved_expected_ty,
                                    &type_name,
                                    span,
                                );
                                return HirPattern::Wildcard;
                            }
                        }

                        let patterns: Vec<HirPattern> = args
                            .iter()
                            .map(|p| {
                                let t = self
                                    .engine
                                    .fresh_type_var_at(identifier_path_span(&inst_pat.name));
                                self.lower_pattern(p, &t)
                            })
                            .collect();

                        let matching_variants: Vec<(String, HirEnum, HirVariant)> = self
                            .items
                            .enumerations()
                            .filter_map(|(enum_id, enum_info)| {
                                enum_info
                                    .variants
                                    .iter()
                                    .find(|variant| variant.name == type_name)
                                    .cloned()
                                    .map(|variant| {
                                        (
                                            crate::lower::resolution::LowerResolutionContext::new(
                                                self,
                                            )
                                            .canonical_name(enum_id)
                                            .map(str::to_string)
                                            .unwrap_or_else(|| enum_info.name.clone()),
                                            enum_info.clone(),
                                            variant,
                                        )
                                    })
                            })
                            .collect();

                        if matching_variants.len() == 1 {
                            let (enum_name, enum_info, variant) = matching_variants
                                .into_iter()
                                .next()
                                .expect("single matching enum variant exists");
                            let enum_ty = Type::Enum {
                                id: enum_info.id,
                                args: vec![],
                            };
                            let _ = self.engine.unify(expected_ty, &enum_ty);
                            return HirPattern::Enum(
                                enum_name,
                                type_name.clone(),
                                Some(HirVariantLocation {
                                    owner: enum_info.id,
                                    variant_id: variant.id,
                                    name: type_name.clone(),
                                }),
                                patterns,
                            );
                        }

                        if !matching_variants.is_empty() {
                            return HirPattern::Enum(type_name.clone(), type_name, None, patterns);
                        }

                        HirPattern::Struct(type_name, None, vec![], vec![])
                    }
                    ast::FieldsPatternOrArgumentsPattern::Fields(fields) => {
                        if type_name.contains("::") {
                            let parts: Vec<&str> = type_name.split("::").collect();
                            if parts.len() == 2 {
                                let enum_name = parts[0].to_string();
                                let variant_name = parts[1].to_string();
                                let span = identifier_path_span(&inst_pat.name);

                                let Some(enum_info) =
                                    crate::lower::resolution::LowerResolutionContext::new(self)
                                        .resolve_enum_type(&enum_name)
                                else {
                                    self.diagnostics.push_type_with_span(
                                        format!(
                                            "pattern type mismatch: unknown enum {}",
                                            enum_name
                                        ),
                                        span,
                                    );
                                    return HirPattern::Wildcard;
                                };
                                let Some(variant) = enum_info
                                    .variants
                                    .iter()
                                    .find(|variant| variant.name == variant_name)
                                    .cloned()
                                else {
                                    self.diagnostics.push_type_with_span(
                                        format!(
                                            "pattern type mismatch: enum {} has no variant {}",
                                            enum_name, variant_name
                                        ),
                                        span,
                                    );
                                    return HirPattern::Wildcard;
                                };

                                let resolved_expected_ty = self.engine.resolve(expected_ty);
                                let scrutinee_type_args = match &resolved_expected_ty {
                                    Type::Enum { id, args } if *id == enum_info.id => args.clone(),
                                    Type::TypeVar(_) | Type::Apply { .. } => {
                                        let fresh_type_args: Vec<Type> = enum_info
                                            .generic_params
                                            .iter()
                                            .map(|param| {
                                                self.engine.fresh_type_var_at_kind(
                                                    span.clone(),
                                                    param.kind.clone(),
                                                )
                                            })
                                            .collect();
                                        let enum_ty = Type::Enum {
                                            id: enum_info.id,
                                            args: fresh_type_args.clone(),
                                        };
                                        if let Err(err) = self.engine.unify(expected_ty, &enum_ty) {
                                            self.diagnostics.push_type_with_span(
                                                format!(
                                                    "pattern type mismatch: {}",
                                                    err.render(&self.engine)
                                                ),
                                                span,
                                            );
                                            return HirPattern::Wildcard;
                                        }
                                        fresh_type_args
                                    }
                                    _ => {
                                        self.report_pattern_type_mismatch(
                                            &resolved_expected_ty,
                                            &enum_name,
                                            span,
                                        );
                                        return HirPattern::Wildcard;
                                    }
                                };

                                let subst: HashMap<GenericParamId, Type> = enum_info
                                    .generic_params
                                    .iter()
                                    .enumerate()
                                    .zip(scrutinee_type_args.iter())
                                    .map(|((index, _), concrete)| {
                                        (
                                            GenericParamId {
                                                owner: enum_info.id,
                                                index: index as u32,
                                            },
                                            concrete.clone(),
                                        )
                                    })
                                    .collect();

                                let HirVariantFields::Named(variant_fields) = &variant.fields
                                else {
                                    self.diagnostics.push_type_with_span(
                                        format!(
                                            "pattern type mismatch: enum variant {}::{} does not have named fields",
                                            enum_name, variant_name
                                        ),
                                        span,
                                    );
                                    return HirPattern::Wildcard;
                                };

                                for field in fields {
                                    if !variant_fields
                                        .iter()
                                        .any(|variant_field| variant_field.name == field.name.name)
                                    {
                                        self.diagnostics.push_type_with_span(
                                            format!(
                                                "pattern type mismatch: enum variant {}::{} has no field '{}'",
                                                enum_name, variant_name, field.name.name
                                            ),
                                            field.name.span.clone(),
                                        );
                                        return HirPattern::Wildcard;
                                    }
                                }

                                let mut patterns = Vec::new();
                                for variant_field in variant_fields {
                                    let Some(field) = fields
                                        .iter()
                                        .find(|field| field.name.name == variant_field.name)
                                    else {
                                        patterns.push(HirPattern::Wildcard);
                                        continue;
                                    };
                                    let field_ty = variant_field.ty.substitute_generics(&subst);
                                    patterns.push(self.lower_pattern(&field.pattern, &field_ty));
                                }

                                return HirPattern::Enum(
                                    enum_name,
                                    variant_name.clone(),
                                    Some(HirVariantLocation {
                                        owner: enum_info.id,
                                        variant_id: variant.id,
                                        name: variant_name,
                                    }),
                                    patterns,
                                );
                            }
                        }

                        let struct_info =
                            crate::lower::resolution::LowerResolutionContext::new(self)
                                .resolve_struct_type(&type_name);
                        let resolved_expected_ty = self.engine.resolve(expected_ty);
                        let type_args = match &resolved_expected_ty {
                            Type::Struct {
                                id,
                                args: type_args,
                            } if struct_info
                                .as_ref()
                                .is_some_and(|struct_def| struct_def.id == *id) =>
                            {
                                type_args.clone()
                            }
                            Type::TypeVar(_) | Type::Apply { .. } => struct_info
                                .as_ref()
                                .map(|hir_struct| {
                                    hir_struct
                                        .generic_params
                                        .iter()
                                        .map(|param| {
                                            self.engine.fresh_type_var_at_kind(
                                                identifier_path_span(&inst_pat.name),
                                                param.kind.clone(),
                                            )
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                            _ => {
                                let span = identifier_path_span(&inst_pat.name);
                                self.report_pattern_type_mismatch(
                                    &resolved_expected_ty,
                                    &type_name,
                                    span,
                                );
                                return HirPattern::Wildcard;
                            }
                        };

                        let Some(struct_info) = struct_info else {
                            return HirPattern::Struct(type_name, None, vec![], vec![]);
                        };
                        let struct_id = struct_info.id;

                        let struct_ty = Type::Struct {
                            id: struct_id,
                            args: type_args.clone(),
                        };
                        if let Err(e) = self.engine.unify(expected_ty, &struct_ty) {
                            let span = identifier_path_span(&inst_pat.name);
                            self.diagnostics.push_type_with_span(
                                format!("pattern type mismatch: {}", e.render(&self.engine)),
                                span,
                            );
                            return HirPattern::Wildcard;
                        }

                        let field_patterns: Vec<HirStructPatternField> = fields
                            .iter()
                            .map(|f| {
                                let field = struct_info.fields.iter().find(|field| field.name == f.name.name).map(|field| {
                                    HirFieldLocation {
                                        owner: struct_info.id,
                                        field_id: field.id,
                                        name: field.name.clone(),
                                    }
                                });
                                let t = self
                                    .lower_struct_field_type(
                                        &type_name,
                                        &type_args,
                                        &f.name.name,
                                        &f.name.span,
                                    )
                                     .unwrap_or_else(|| {
                                         self.engine
                                             .fresh_type_var_at(f.name.span.clone())
                                     });
                                let pat = match &f.pattern.kind {
                                    ast::PatternKind::Wildcard | ast::PatternKind::Ident(_) => {
                                        self.lower_pattern(&f.pattern, &t)
                                    }
                                    _ => {
                                        self.diagnostics.push_type_with_span(
                                            "Struct field patterns currently only support bindings and wildcards"
                                                .to_string(),
                                            f.name.span.clone(),
                                        );
                                        HirPattern::Wildcard
                                    }
                                };
                                HirStructPatternField {
                                    name: f.name.name.clone(),
                                    field,
                                    pattern: pat,
                                }
                            })
                            .collect();
                        HirPattern::Struct(type_name, Some(struct_id), type_args, field_patterns)
                    }
                }
            }
            ast::PatternKind::Array(_) => HirPattern::Wildcard,
            ast::PatternKind::Nested(inner) => self.lower_pattern(inner, expected_ty),
            ast::PatternKind::Reference { .. } => HirPattern::Wildcard,
        }
    }
}

#[cfg(test)]
mod tests;
