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
                ast::LiteralKind::Char(c) => {
                    HirPattern::Literal(HirLiteralPattern::Char(c.chars().next().unwrap_or('\0')))
                }
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
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    use crate::crate_system::CrateContext;
    use crate::ids::{CrateId, DefId, FieldId, HirLocalId, LocalDefId, VariantId};
    use crate::infer::ResolvedHirProgram;
    use crate::{collect, infer, lower, macro_expansion, parser, Config};

    static LOWER_PATTERN_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn ident(name: &str) -> ast::Ident {
        ast::Ident {
            name: name.to_string(),
            span: crate::lexer::Span::test(),
        }
    }

    fn ident_pattern(name: &str, mutable: bool) -> ast::Pattern {
        ast::Pattern {
            binding: Some(ident(name)),
            kind: ast::PatternKind::Ident(ast::IdentPattern {
                name: ident(name),
                mut_: mutable,
            }),
        }
    }

    fn compile_source_to_resolved_hir_for_test(source: &str) -> ResolvedHirProgram {
        let id = LOWER_PATTERN_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rock_lower_pattern_hir_ref_test_{}_{}",
            std::process::id(),
            id
        ));
        let entry_file = dir.join("main.rk");

        let config = Config {
            entry_file: entry_file.clone(),
            output_dir: dir,
            debug_print: Vec::new(),
            meta_files: Vec::new(),
            extern_artifacts: Vec::new(),
            source_providers: Vec::new(),
            current_crate_name: Some("test".to_string()),
            opt_level: 0,
            emit_llvm: false,
            no_link: true,
            emit_object: None,
            no_prelude: true,
            no_std: true,
            sysroot: None,
        };

        let ast = parser::parse_source(entry_file, source, &config)
            .map(|module| crate::ast::Program { module })
            .expect("test source parses");
        let macro_context = macro_expansion::MacroExpansionContext::new(&config);
        let ast = macro_expansion::expand_macros_with_context(ast, &macro_context)
            .expect("test macros expand");
        let crate_ctx = CrateContext::new();
        let decls = collect::collect(&ast, &crate_ctx, false, Some("test"))
            .expect("test declarations collect");
        let partial =
            lower::program::lower_from_declarations(&ast, decls, &crate_ctx, Some("test"))
                .expect("test source lowers");
        infer::finalize(partial).expect("test HIR finalizes")
    }

    fn find_first_struct_pattern(
        block: &crate::hir::AcceptedHirBlock,
    ) -> Option<(
        &String,
        &Option<DefId>,
        &Vec<Type>,
        &Vec<HirStructPatternField>,
    )> {
        block.stmts.iter().find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
                find_first_struct_pattern_expr(expr)
            }
            _ => None,
        })
    }

    fn find_first_struct_pattern_expr(
        expr: &crate::hir::AcceptedHirExpr,
    ) -> Option<(
        &String,
        &Option<DefId>,
        &Vec<Type>,
        &Vec<HirStructPatternField>,
    )> {
        match &expr.kind {
            crate::hir::HirExprKindFor::Match { arms, .. } => {
                arms.iter().find_map(|arm| match &arm.pattern {
                    HirPattern::Struct(name, id, type_args, fields) => {
                        Some((name, id, type_args, fields))
                    }
                    _ => None,
                })
            }
            crate::hir::HirExprKindFor::Block(block)
            | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_struct_pattern(block),
            _ => None,
        }
    }

    fn find_first_enum_pattern(
        block: &crate::hir::AcceptedHirBlock,
    ) -> Option<(
        &String,
        &String,
        &Option<HirVariantLocation>,
        &Vec<HirPattern>,
    )> {
        block.stmts.iter().find_map(|stmt| match stmt {
            crate::hir::HirStmtFor::Expr(expr) | crate::hir::HirStmtFor::Return(Some(expr)) => {
                find_first_enum_pattern_expr(expr)
            }
            _ => None,
        })
    }

    fn find_first_enum_pattern_expr(
        expr: &crate::hir::AcceptedHirExpr,
    ) -> Option<(
        &String,
        &String,
        &Option<HirVariantLocation>,
        &Vec<HirPattern>,
    )> {
        match &expr.kind {
            crate::hir::HirExprKindFor::Match { arms, .. } => {
                arms.iter().find_map(|arm| match &arm.pattern {
                    HirPattern::Enum(enum_name, variant_name, location, items) => {
                        Some((enum_name, variant_name, location, items))
                    }
                    _ => None,
                })
            }
            crate::hir::HirExprKindFor::Block(block)
            | crate::hir::HirExprKindFor::UnsafeBlock(block) => find_first_enum_pattern(block),
            _ => None,
        }
    }

    fn unit_enum(id: DefId, name: &str, variant_id: u32, variant_name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(variant_id),
                name: variant_name.to_string(),
                fields: HirVariantFields::Unit,
            }],
        }
    }

    fn named_field_enum() -> HirEnum {
        HirEnum {
            id: def_id(40),
            name: "Record".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Pair".to_string(),
                fields: HirVariantFields::Named(vec![HirField {
                    id: FieldId(0),
                    name: "first".to_string(),
                    ty: Type::I64,
                    public: true,
                }]),
            }],
        }
    }

    fn two_field_named_enum() -> HirEnum {
        HirEnum {
            id: def_id(41),
            name: "Record2".to_string(),
            generic_params: Vec::new(),
            variants: vec![HirVariant {
                id: VariantId(0),
                name: "Pair".to_string(),
                fields: HirVariantFields::Named(vec![
                    HirField {
                        id: FieldId(0),
                        name: "first".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                    HirField {
                        id: FieldId(1),
                        name: "second".to_string(),
                        ty: Type::I64,
                        public: true,
                    },
                ]),
            }],
        }
    }

    fn qualified_named_enum_field_pattern() -> ast::Pattern {
        ast::Pattern {
            binding: None,
            kind: ast::PatternKind::Instance(ast::InstancePattern {
                name: ast::TypePath {
                    path: vec![
                        ast::IdentOrType::Ident(ident("Record")),
                        ast::IdentOrType::Ident(ident("Pair")),
                    ],
                },
                args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                    name: ident("first"),
                    pattern: ident_pattern("value", false),
                }]),
            }),
        }
    }

    fn qualified_second_named_enum_field_pattern() -> ast::Pattern {
        ast::Pattern {
            binding: None,
            kind: ast::PatternKind::Instance(ast::InstancePattern {
                name: ast::TypePath {
                    path: vec![
                        ast::IdentOrType::Ident(ident("Record2")),
                        ast::IdentOrType::Ident(ident("Pair")),
                    ],
                },
                args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                    name: ident("second"),
                    pattern: ident_pattern("second", false),
                }]),
            }),
        }
    }

    fn qualified_unknown_named_enum_field_pattern() -> ast::Pattern {
        ast::Pattern {
            binding: None,
            kind: ast::PatternKind::Instance(ast::InstancePattern {
                name: ast::TypePath {
                    path: vec![
                        ast::IdentOrType::Ident(ident("Record2")),
                        ast::IdentOrType::Ident(ident("Pair")),
                    ],
                },
                args: ast::FieldsPatternOrArgumentsPattern::Fields(vec![ast::FieldPattern {
                    name: ident("typo"),
                    pattern: ident_pattern("value", false),
                }]),
            }),
        }
    }

    fn unqualified_unit_pattern(name: &str) -> ast::Pattern {
        ast::Pattern {
            binding: None,
            kind: ast::PatternKind::Instance(ast::InstancePattern {
                name: ast::TypePath {
                    path: vec![ast::IdentOrType::Ident(ast::Ident {
                        name: name.to_string(),
                        span: crate::lexer::Span::test(),
                    })],
                },
                args: ast::FieldsPatternOrArgumentsPattern::Arguments(Vec::new()),
            }),
        }
    }

    #[test]
    fn pattern_binding_allocates_local_id_and_defines_scope() {
        let mut lowerer = Lowerer::new_for_test();
        let mut pattern = ident_pattern("value", false);
        let binding_span = crate::lexer::Span {
            file_path: "main.rk".into(),
            start: 8,
            end: 13,
        };
        if let ast::PatternKind::Ident(binding) = &mut pattern.kind {
            binding.name.span = binding_span.clone();
        }

        let (hir, scoped_local_id, owner) = lowerer.with_test_body_context(|lowerer| {
            let hir = lowerer.lower_pattern(&pattern, &Type::I64);
            let scoped_local_id = lowerer.scope.lookup("value").unwrap().local_id;
            (hir, scoped_local_id, lowerer.current_body_def_id().unwrap())
        });

        match hir {
            HirPattern::Binding {
                name,
                local_id,
                mutable,
            } => {
                assert_eq!(name, "value");
                assert!(!mutable);
                assert_ne!(local_id, HirLocalId(u32::MAX));
                assert_eq!(scoped_local_id, Some(local_id));
                let span = lowerer
                    .source_map
                    .local_span(owner, local_id)
                    .expect("local source span");
                assert_eq!(span.file_path, binding_span.file_path);
                assert_eq!((span.start, span.end), (8, 13));
            }
            other => panic!("expected binding pattern, got {other:?}"),
        }
    }

    #[test]
    fn lower_struct_pattern_records_struct_and_field_ids() {
        let source = r#"
struct Box
    < value: I64

read: Box -> I64
read = box ->
    match box
        Box value: value => value
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let (_, read) = resolved.program.test_function_by_name("read").unwrap();
        let (struct_id, struct_def) = resolved.program.test_struct_by_name("Box").unwrap();
        let expected_field_id = struct_def
            .fields
            .iter()
            .find(|field| field.name == "value")
            .unwrap()
            .id;

        let pattern = find_first_struct_pattern(&read.body).unwrap();
        assert_eq!(*pattern.1, Some(struct_id));
        let location = pattern.3[0]
            .field
            .as_ref()
            .expect("struct pattern field must be resolved");
        assert_eq!(location.owner, struct_id);
        assert_eq!(location.field_id, expected_field_id);
        assert_eq!(location.name, "value");
        assert_eq!(pattern.3[0].name, "value");
    }

    #[test]
    fn unresolved_unqualified_enum_pattern_keeps_ambiguous_location_unresolved() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_enumeration(unit_enum(def_id(30), "First", 0, "Hit"));
        lowerer
            .items
            .insert_enumeration(unit_enum(def_id(31), "Second", 1, "Hit"));
        let expected_ty = lowerer.engine.fresh_type_var();
        let pattern = unqualified_unit_pattern("Hit");

        let lowered =
            lowerer.with_test_body_context(|lowerer| lowerer.lower_pattern(&pattern, &expected_ty));

        match lowered {
            HirPattern::Enum(_, variant_name, location, _) => {
                assert_eq!(variant_name, "Hit");
                assert!(location.is_none());
            }
            other => panic!("expected enum pattern, got {other:?}"),
        }
    }

    #[test]
    fn lower_enum_pattern_records_enum_and_variant_ids() {
        let source = r#"
enum Maybe
    Some I64
    None

read: Maybe -> I64
read = value ->
    match value
        Some x => x
        None => 0
"#;

        let resolved = compile_source_to_resolved_hir_for_test(source);
        let (_, read) = resolved.program.test_function_by_name("read").unwrap();
        let (enum_id, enum_def) = resolved.program.test_enum_by_name("Maybe").unwrap();
        let some_variant = enum_def
            .variants
            .iter()
            .find(|variant| variant.name == "Some")
            .unwrap();

        let pattern = find_first_enum_pattern(&read.body).unwrap();
        let location = pattern.2.as_ref().expect("enum pattern must be resolved");
        assert_eq!(location.owner, enum_id);
        assert_eq!(location.variant_id, some_variant.id);
        assert_eq!(location.name, "Some");
    }

    #[test]
    fn qualified_named_enum_field_pattern_records_variant_location() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_enumeration(named_field_enum());
        lowerer
            .resolver
            .item_paths
            .insert("Record".to_string(), def_id(40));
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(40), "Record".to_string());
        let expected_ty = Type::Enum {
            id: def_id(40),
            args: Vec::new(),
        };

        let lowered = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_pattern(&qualified_named_enum_field_pattern(), &expected_ty)
        });

        match lowered {
            HirPattern::Enum(enum_name, variant_name, Some(location), patterns) => {
                assert_eq!(enum_name, "Record");
                assert_eq!(variant_name, "Pair");
                assert_eq!(location.owner, def_id(40));
                assert_eq!(location.variant_id, VariantId(0));
                assert_eq!(location.name, "Pair");
                assert_eq!(patterns.len(), 1);
            }
            other => panic!("expected resolved enum field pattern, got {other:?}"),
        }
    }

    #[test]
    fn qualified_named_enum_field_pattern_preserves_omitted_field_positions() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_enumeration(two_field_named_enum());
        lowerer
            .resolver
            .item_paths
            .insert("Record2".to_string(), def_id(41));
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(41), "Record2".to_string());
        let expected_ty = Type::Enum {
            id: def_id(41),
            args: Vec::new(),
        };

        let lowered = lowerer.with_test_body_context(|lowerer| {
            lowerer.lower_pattern(&qualified_second_named_enum_field_pattern(), &expected_ty)
        });

        match lowered {
            HirPattern::Enum(_, _, Some(location), patterns) => {
                assert_eq!(location.owner, def_id(41));
                assert_eq!(patterns.len(), 2);
                assert!(matches!(patterns[0], HirPattern::Wildcard));
                match &patterns[1] {
                    HirPattern::Binding { name, .. } => assert_eq!(name, "second"),
                    other => panic!("expected second field binding, got {other:?}"),
                }
            }
            other => panic!("expected resolved enum field pattern, got {other:?}"),
        }
    }

    #[test]
    fn qualified_named_enum_field_pattern_rejects_unknown_field() {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_enumeration(two_field_named_enum());
        lowerer
            .resolver
            .item_paths
            .insert("Record2".to_string(), def_id(41));
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(41), "Record2".to_string());
        let expected_ty = Type::Enum {
            id: def_id(41),
            args: Vec::new(),
        };

        let lowered =
            lowerer.lower_pattern(&qualified_unknown_named_enum_field_pattern(), &expected_ty);

        assert!(matches!(lowered, HirPattern::Wildcard));
        assert!(
            lowerer
                .diagnostics
                .errors()
                .iter()
                .any(|error| error.message.contains("has no field 'typo'")),
            "expected unknown field diagnostic, got {:?}",
            lowerer.diagnostics.errors()
        );
    }
}
