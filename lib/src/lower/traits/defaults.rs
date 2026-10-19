//! Trait default method body lowering

use crate::ast;

use crate::ids::DefId;
use crate::lower::body_context::{BodyLoweringContext, BodyOwner, GenericLoweringContext};
use crate::lower::Lowerer;
use crate::types::{GenericParamId, TraitBound, Type};

fn type_contains_projection(ty: &Type) -> bool {
    crate::type_services::visit::type_any(ty, |nested| matches!(nested, Type::Projection { .. }))
}

impl Lowerer {
    pub(crate) fn lower_trait_default_bodies(
        &mut self,
        module: &ast::Module,
        module_id: crate::ids::ModuleId,
    ) {
        for (ordinal, top_level) in module.top_levels.iter().enumerate() {
            let ast::TopLevel::TraitDecl(td) = top_level else {
                continue;
            };
            let Some(record) = self.item_index.item_at_source(module_id, ordinal) else {
                self.diagnostics.push_toolchain(
                    "missing indexed trait declaration while lowering default bodies".to_string(),
                );
                continue;
            };
            if record.kind != crate::collect::item_index::ItemKind::Trait {
                self.diagnostics.push_toolchain(
                    "indexed declaration kind does not match trait syntax".to_string(),
                );
                continue;
            }

            self.lower_trait_bodies(td, record.def_id);
        }

        // Recurse into sub-modules
        for tl in &module.top_levels {
            if let ast::TopLevel::Module(module_decl) = tl {
                let Some(module_name) = module_decl.0.name.as_ref() else {
                    continue;
                };
                let Some(child_module_id) = self
                    .item_index
                    .child_module_id(module_id, &module_name.name)
                else {
                    self.diagnostics.push_toolchain(format!(
                        "missing indexed module '{}' while lowering trait default bodies",
                        module_name.name
                    ));
                    continue;
                };
                self.lower_trait_default_bodies(&module_decl.0, child_module_id);
            }
        }
    }

    pub(crate) fn lower_trait_bodies(&mut self, td: &ast::TraitDecl, trait_id: DefId) {
        let trait_name = td.name.name.clone();
        let previous_trait = self.current_trait.clone();
        let previous_trait_id = self.current_trait_id;
        let prev_trait_generics = self.current_trait_generics.clone();
        let previous_generic_context = self.generic_context.clone();
        let Some(trait_def) = self.trait_by_id(trait_id).cloned() else {
            self.diagnostics.push_toolchain(
                "missing indexed trait declaration while lowering default bodies".to_string(),
            );
            return;
        };
        let Some(trait_context_name) = self.canonical_name_for_def_id(trait_id).map(str::to_string)
        else {
            self.diagnostics.push_toolchain(
                "missing canonical trait name while lowering default bodies".to_string(),
            );
            return;
        };

        // Set current trait so method lookups on TypeVar self can find sibling methods
        self.current_trait = Some(trait_context_name.clone());
        self.current_trait_id = Some(trait_id);
        self.current_trait_generics = td
            .generic_params
            .iter()
            .filter(|param| param.kind.is_none())
            .map(|param| param.name.name.clone())
            .collect();
        let mut params = self.current_trait_generics.clone();
        params.push("Self".to_string());
        self.generic_context = Some(GenericLoweringContext::new(trait_def.id, params));

        let mut methods = Vec::with_capacity(td.methods.len());
        for (ident, fd) in &td.methods {
            let method_name = ident.name.clone();
            let Some(method_id) = trait_def.methods.get(&method_name).map(|method| method.id)
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            methods.push((method_id, method_name, fd));
        }
        methods.sort_unstable_by_key(|(method_id, _, _)| *method_id);

        for (_, method_name, fd) in methods {
            let Some(mut func) = self
                .items
                .trait_def(trait_id)
                .and_then(|trait_def| trait_def.methods.get(&method_name))
                .cloned()
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            let signature_ret = trait_def
                .signatures
                .get(&method_name)
                .map(|sig| sig.ret.clone());
            if let Some(sig) = trait_def.signatures.get(&method_name) {
                func = self.lower_function_decl_header_with_sig_and_id(fd, sig, func.id);
            }
            // Body lowering reads declared return types through this canonical trait entry.
            let Some(method) = self
                .items
                .trait_def_mut(trait_id)
                .and_then(|trait_def| trait_def.methods.get_mut(&method_name))
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            *method = func.clone();
            let mut generic_bounds = func.generic_bounds.clone();
            let self_param = GenericParamId {
                owner: trait_def.id,
                index: trait_def.generic_params.len() as u32,
            };
            let trait_args = trait_def
                .generic_params
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    Type::Generic(GenericParamId {
                        owner: trait_def.id,
                        index: index as u32,
                    })
                })
                .collect();
            generic_bounds
                .entry(self_param)
                .or_insert_with(Vec::new)
                .push(TraitBound {
                    trait_id: trait_def.id,
                    type_args: trait_args,
                });

            let mut context = BodyLoweringContext::new(
                format!("{}::{}", trait_context_name, method_name),
                BodyOwner::TraitMethod {
                    trait_id: trait_def.id,
                    method_id: func.id,
                    method_name: method_name.clone(),
                },
                Some(trait_def.id),
                self.current_generic_params().to_vec(),
                generic_bounds,
                fd.is_unsafe,
            );
            // Default bodies share the declaration's generic identities,
            // including method-local parameters used in local annotations.
            for generic in trait_def
                .generic_params
                .iter()
                .chain(trait_def.target.iter())
                .chain(&func.generic_params)
            {
                context.register_generic_param(generic.name.clone(), generic.id);
            }
            context.seed_after_existing_locals(func.params.iter().map(|param| param.local_id));
            let body = self.with_body_context(context, |lowerer| {
                lowerer.push_scope();

                // Register self parameter if present
                if let Some(self_receiver) = func.self_receiver {
                    let self_ty = func.params[0].ty.clone();
                    lowerer.scope.define_local(
                        "self".to_string(),
                        self_ty,
                        matches!(self_receiver, crate::types::ReceiverMode::Mut),
                        func.params[0].local_id,
                    );
                }

                // Register other parameters
                let start = if func.self_receiver.is_some() { 1 } else { 0 };
                for param in &func.params[start..] {
                    lowerer.scope.define_local(
                        param.name.clone(),
                        param.ty.clone(),
                        param.mutable,
                        param.local_id,
                    );
                }

                let body = lowerer.lower_lambda_body(&fd.lambda);
                lowerer.pop_scope();
                body
            });

            let validate_return_now = signature_ret
                .as_ref()
                .map(|ret| !type_contains_projection(ret))
                .unwrap_or(true);
            if validate_return_now {
                if let Err(e) = self.engine.unify(&body.ty, &func.ret_type) {
                    let message = format!(
                        "In trait default method '{}.{}': return type mismatch: {}",
                        trait_name,
                        method_name,
                        e.render(&self.engine)
                    );
                    self.diagnostics
                        .push_type_with_span(message, fd.lambda.span.clone());
                }
            }

            func.body = body;

            // Resolve all TypeVars in the function that were unified during inference.
            // This is critical: when the default body is later injected into a concrete
            // impl, substitute_typevars_in_function replaces ALL remaining TypeVars with
            // the concrete type. If we don't resolve here, types like I32 (from puts)
            // that were inferred via unification would still be TypeVars and get wrongly
            // replaced with the impl type (e.g. I64).
            self.resolve_all_types_in_function(&mut func);

            // Update every alias for the same trait so ID-owned HIR can collapse
            // aliases without losing the lowered default body.
            let Some(method) = self
                .items
                .trait_def_mut(trait_id)
                .and_then(|trait_def| trait_def.methods.get_mut(&method_name))
            else {
                self.diagnostics.push_toolchain(format!(
                    "missing indexed trait method '{}' while lowering default body",
                    method_name
                ));
                continue;
            };
            *method = func;
        }

        self.current_trait = previous_trait;
        self.current_trait_id = previous_trait_id;
        self.current_trait_generics = prev_trait_generics;
        self.generic_context = previous_generic_context;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    use crate::ast::{
        Block, Expression, FunctionDecl, FunctionSig, Ident, IdentOrNumber, IdentOrType,
        IdentifierPath, LambdaArrowKind, LambdaDecl, Literal, LiteralKind, Module, ModuleDecl,
        Operand, ParseType, ParseTypeInner, PrimaryExpr, SecondaryExpr, SelfReceiverMode,
        Statement, TopLevel, TraitDecl, UnaryExpr,
    };
    use crate::collect::item_index::{index_root_module_items, IndexingIds};
    use crate::hir::{HirBlock, HirExprKind, HirFunction, HirFunctionSig, HirParam, HirTrait};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::lexer::Span;
    use crate::types::{
        CallableKind, CaptureKind, FunctionCapture, FunctionSafety, GenericParamDecl,
        GenericParamId, Type,
    };

    fn def_id(local: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(local))
    }

    fn ident(name: &str) -> Ident {
        Ident {
            name: name.to_string(),
            span: Span::test(),
        }
    }

    fn self_method_value_expr(method: &str) -> Expression {
        Expression::UnaryExpr(UnaryExpr::PrimaryExpr(PrimaryExpr {
            operand: crate::ast::Operand::Ident(IdentifierPath {
                path: vec![IdentOrType::Ident(ident("self"))],
            }),
            secondaries: Some(vec![SecondaryExpr::Dot(IdentOrNumber::Ident(ident(
                method,
            )))]),
            type_annotation: None,
        }))
    }

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(ParseTypeInner {
            name: name.to_string(),
            generics: Vec::new(),
            span: Span::test(),
        })
    }

    fn literal_default_decl(name: &str, kind: LiteralKind) -> FunctionDecl {
        FunctionDecl {
            name: ident(name),
            lambda: LambdaDecl {
                parameters: Vec::new(),
                body: Block {
                    statements: vec![Statement::Expression(Expression::UnaryExpr(
                        UnaryExpr::PrimaryExpr(PrimaryExpr {
                            operand: Operand::Literal(Literal {
                                kind,
                                span: Span::test(),
                            }),
                            secondaries: None,
                            type_annotation: None,
                        }),
                    ))],
                },
                arrow_kind: LambdaArrowKind::Normal,
                span: crate::lexer::Span::test(),
            },
            self_receiver: None,
            is_unsafe: false,
            exported: false,
        }
    }

    fn trait_decl(name: &str, method: FunctionDecl) -> TraitDecl {
        TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: name.to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            generic_params: Vec::new(),
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(ident(&method.name.name), method)]),
            signatures: HashMap::new(),
            exported: false,
            language_items: Default::default(),
        }
    }

    fn trait_method(id: DefId, name: &str, ret_type: Type) -> HirFunction {
        HirFunction {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: Vec::new(),
            ret_type,
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: true,
            self_receiver: None,
            is_unsafe: false,
        }
    }

    fn trait_with_method(id: DefId, name: &str, method: HirFunction) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([(method.name.clone(), method)]),
            signatures: HashMap::new(),
        }
    }

    #[test]
    fn trait_default_signature_stub_uses_canonical_params_without_duplicate_self() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(70);
        let signature_id = def_id(71);
        let default_id = def_id(72);
        let self_param = GenericParamId {
            owner: trait_id,
            index: 0,
        };
        let self_ty = Type::Generic(self_param);
        let method_value_ty = Type::function_with_metadata(
            vec![Type::I64],
            Type::Bool,
            FunctionSafety::Safe,
            CallableKind::FnMut,
            vec![FunctionCapture::new(
                CaptureKind::MutableBorrow,
                self_ty.clone(),
            )],
        );
        lowerer
            .resolver
            .item_paths
            .insert("Reader".to_string(), trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Reader".to_string());
        let default_method = HirFunction {
            id: default_id,
            name: "default".to_string(),
            generic_params: Vec::new(),
            generic_bounds: HashMap::new().into(),
            params: vec![HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: self_ty.clone(),
                mutable: true,
                is_ref: false,
            }],
            ret_type: method_value_ty.clone(),
            body: HirBlock {
                stmts: Vec::new(),
                ty: Type::Unit,
            },
            is_curried: false,
            is_method: true,
            self_receiver: Some(crate::types::ReceiverMode::Mut),
            is_unsafe: false,
        };
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Reader".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::from([("default".to_string(), default_method)]),
            signatures: HashMap::from([(
                "set".to_string(),
                HirFunctionSig {
                    id: signature_id,
                    name: "set".to_string(),
                    generic_params: vec![GenericParamDecl::type_param(self_param, "Self")],
                    params: vec![self_ty.clone(), Type::I64],
                    ret: Type::Bool,
                    generic_bounds: HashMap::new().into(),
                    self_receiver: Some(crate::types::ReceiverMode::Mut),
                    is_unsafe: false,
                },
            )]),
        });
        let trait_decl = TraitDecl {
            where_clauses: Vec::new(),
            name: ParseTypeInner {
                name: "Reader".to_string(),
                generics: Vec::new(),
                span: Span::test(),
            },
            generic_params: Vec::new(),
            for_: None,
            associated_types: Vec::new(),
            methods: HashMap::from([(
                ident("default"),
                FunctionDecl {
                    name: ident("default"),
                    lambda: LambdaDecl {
                        parameters: Vec::new(),
                        body: Block {
                            statements: vec![Statement::Expression(self_method_value_expr("set"))],
                        },
                        arrow_kind: LambdaArrowKind::Normal,
                        span: crate::lexer::Span::test(),
                    },
                    self_receiver: Some(SelfReceiverMode::Mut),
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            signatures: HashMap::from([(
                ident("set"),
                FunctionSig {
                    name: ident("set"),
                    sig: named_type("Bool"),
                    where_clauses: Vec::new(),
                    self_receiver: Some(SelfReceiverMode::Mut),
                    is_unsafe: false,
                    exported: false,
                },
            )]),
            exported: false,
            language_items: Default::default(),
        };

        lowerer.lower_trait_bodies(&trait_decl, trait_id);

        let lowered = &lowerer.items.trait_def(trait_id).unwrap().methods["default"];
        let Some(crate::hir::HirStmt::Expr(expr)) = lowered.body.stmts.first() else {
            panic!("expected lowered method value expression");
        };
        assert_eq!(expr.ty, method_value_ty);
        match &expr.kind {
            HirExprKind::Lambda { params, .. } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].ty, Type::I64);
            }
            other => panic!("expected method value lambda, got {other:?}"),
        }
    }

    #[test]
    fn trait_default_bodies_report_missing_canonical_trait_name() {
        let mut lowerer = Lowerer::new_for_test();
        let trait_id = def_id(73);
        let trait_decl = trait_decl(
            "Hidden",
            literal_default_decl("value", LiteralKind::Number(1)),
        );
        lowerer.items.insert_trait_def(trait_with_method(
            trait_id,
            "Hidden",
            trait_method(def_id(74), "value", Type::I64),
        ));

        lowerer.lower_trait_bodies(&trait_decl, trait_id);

        let error = lowerer
            .errors()
            .iter()
            .find(|error| {
                error
                    .message
                    .contains("missing canonical trait name while lowering default bodies")
            })
            .expect("missing canonical trait metadata should be diagnosed");
        assert_eq!(error.span(), None);
        assert_eq!(
            lowerer.items.trait_def(trait_id).unwrap().methods["value"]
                .body
                .ty,
            Type::Unit
        );
    }

    #[test]
    fn trait_default_bodies_restore_caller_trait_context() {
        let mut lowerer = Lowerer::new_for_test();
        let outer_trait_id = def_id(75);
        let trait_id = def_id(76);
        let trait_decl = trait_decl(
            "Inner",
            literal_default_decl("value", LiteralKind::Number(1)),
        );
        lowerer.current_trait = Some("Outer".to_string());
        lowerer.current_trait_id = Some(outer_trait_id);
        lowerer.current_trait_generics = vec!["T".to_string()];
        lowerer.generic_context = Some(GenericLoweringContext::new(
            outer_trait_id,
            vec!["T".to_string()],
        ));
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Inner".to_string());
        lowerer.items.insert_trait_def(trait_with_method(
            trait_id,
            "Inner",
            trait_method(def_id(77), "value", Type::I64),
        ));

        lowerer.lower_trait_bodies(&trait_decl, trait_id);

        assert_eq!(lowerer.current_trait.as_deref(), Some("Outer"));
        assert_eq!(lowerer.current_trait_id, Some(outer_trait_id));
        assert_eq!(lowerer.current_trait_generics, vec!["T".to_string()]);
        assert_eq!(lowerer.current_generic_owner(), Some(outer_trait_id));
        assert_eq!(lowerer.current_generic_params(), ["T".to_string()]);
    }

    #[test]
    fn trait_default_body_diagnostics_follow_method_def_id_order() {
        for _ in 0..16 {
            let mut lowerer = Lowerer::new_for_test();
            let trait_id = def_id(78);
            lowerer
                .resolver
                .item_names_by_id
                .insert(trait_id, "Ordered".to_string());
            lowerer.items.insert_trait_def(HirTrait {
                target: None,
                predicates: Vec::new(),
                id: trait_id,
                name: "Ordered".to_string(),
                generic_params: Vec::new(),
                associated_types: Vec::new(),
                methods: HashMap::from([
                    (
                        "zeta".to_string(),
                        trait_method(def_id(79), "zeta", Type::Bool),
                    ),
                    (
                        "beta".to_string(),
                        trait_method(def_id(80), "beta", Type::Bool),
                    ),
                    (
                        "alpha".to_string(),
                        trait_method(def_id(81), "alpha", Type::Bool),
                    ),
                ]),
                signatures: HashMap::new(),
            });
            let trait_decl = TraitDecl {
                where_clauses: Vec::new(),
                name: ParseTypeInner {
                    name: "Ordered".to_string(),
                    generics: Vec::new(),
                    span: Span::test(),
                },
                generic_params: Vec::new(),
                for_: None,
                associated_types: Vec::new(),
                methods: HashMap::from([
                    (
                        ident("alpha"),
                        literal_default_decl("alpha", LiteralKind::Number(1)),
                    ),
                    (
                        ident("beta"),
                        literal_default_decl("beta", LiteralKind::Number(1)),
                    ),
                    (
                        ident("zeta"),
                        literal_default_decl("zeta", LiteralKind::Number(1)),
                    ),
                ]),
                signatures: HashMap::new(),
                exported: false,
                language_items: Default::default(),
            };

            lowerer.lower_trait_bodies(&trait_decl, trait_id);

            let method_names: Vec<_> = lowerer
                .errors()
                .iter()
                .filter_map(|error| {
                    error
                        .message
                        .strip_prefix("In trait default method 'Ordered.")
                        .and_then(|message| message.split_once("': return type mismatch"))
                        .map(|(method_name, _)| method_name)
                })
                .collect();
            assert_eq!(method_names, ["zeta", "beta", "alpha"]);
        }
    }

    #[test]
    fn trait_default_bodies_use_indexed_inline_trait_owner_ids() {
        let module = Module {
            name: None,
            top_levels: vec![
                TopLevel::Module(ModuleDecl(Module {
                    name: Some(ident("left")),
                    top_levels: vec![TopLevel::TraitDecl(trait_decl(
                        "Shared",
                        literal_default_decl("value", LiteralKind::Number(1)),
                    ))],
                    is_inline: true,
                    filepath: None,
                })),
                TopLevel::Module(ModuleDecl(Module {
                    name: Some(ident("right")),
                    top_levels: vec![TopLevel::TraitDecl(trait_decl(
                        "Shared",
                        literal_default_decl("value", LiteralKind::Bool(true)),
                    ))],
                    is_inline: true,
                    filepath: None,
                })),
            ],
            is_inline: true,
            filepath: None,
        };
        let mut indexing_ids = IndexingIds::new_root();
        let root_module_id = indexing_ids.root_module_id();
        let mut lowerer = Lowerer::new_for_test();
        lowerer.item_index = index_root_module_items(&mut indexing_ids, &module);
        let left_module_id = lowerer
            .item_index
            .child_module_id(root_module_id, "left")
            .unwrap();
        let right_module_id = lowerer
            .item_index
            .child_module_id(root_module_id, "right")
            .unwrap();
        let left_trait_id = lowerer
            .item_index
            .item_at_source(left_module_id, 0)
            .unwrap()
            .def_id;
        let right_trait_id = lowerer
            .item_index
            .item_at_source(right_module_id, 0)
            .unwrap()
            .def_id;
        lowerer.items.insert_trait_def(trait_with_method(
            left_trait_id,
            "left::Shared",
            trait_method(def_id(80), "value", Type::I64),
        ));
        lowerer.items.insert_trait_def(trait_with_method(
            right_trait_id,
            "right::Shared",
            trait_method(def_id(81), "value", Type::Bool),
        ));
        // The old name lookup selects this trait for both inline declarations.
        lowerer
            .resolver
            .item_paths
            .insert("Shared".to_string(), left_trait_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(left_trait_id, "left::Shared".to_string());
        lowerer
            .resolver
            .item_names_by_id
            .insert(right_trait_id, "right::Shared".to_string());

        lowerer.lower_trait_default_bodies(&module, root_module_id);

        assert_eq!(
            lowerer.items.trait_def(left_trait_id).unwrap().methods["value"]
                .body
                .ty,
            Type::I64
        );
        assert_eq!(
            lowerer.items.trait_def(right_trait_id).unwrap().methods["value"]
                .body
                .ty,
            Type::Bool
        );
    }

    #[test]
    fn trait_default_bodies_report_missing_indexed_trait_owner() {
        let module = Module {
            name: None,
            top_levels: vec![TopLevel::TraitDecl(trait_decl(
                "Missing",
                literal_default_decl("value", LiteralKind::Number(1)),
            ))],
            is_inline: true,
            filepath: None,
        };
        let mut indexing_ids = IndexingIds::new_root();
        let root_module_id = indexing_ids.root_module_id();
        let mut lowerer = Lowerer::new_for_test();
        lowerer.item_index = index_root_module_items(&mut indexing_ids, &module);

        lowerer.lower_trait_default_bodies(&module, root_module_id);

        assert!(lowerer.errors().iter().any(|error| error
            .message
            .contains("missing indexed trait declaration while lowering default bodies")));
    }

    #[test]
    fn trait_default_bodies_report_mismatched_indexed_owner_and_missing_member() {
        let trait_module = Module {
            name: None,
            top_levels: vec![TopLevel::TraitDecl(trait_decl(
                "Missing",
                literal_default_decl("value", LiteralKind::Number(1)),
            ))],
            is_inline: true,
            filepath: None,
        };
        let indexed_module = Module {
            name: None,
            top_levels: vec![TopLevel::FunctionDecl(literal_default_decl(
                "not_a_trait",
                LiteralKind::Number(1),
            ))],
            is_inline: true,
            filepath: None,
        };
        let mut indexing_ids = IndexingIds::new_root();
        let root_module_id = indexing_ids.root_module_id();
        let mut lowerer = Lowerer::new_for_test();
        lowerer.item_index = index_root_module_items(&mut indexing_ids, &indexed_module);

        lowerer.lower_trait_default_bodies(&trait_module, root_module_id);

        assert!(lowerer.errors().iter().any(|error| error
            .message
            .contains("indexed declaration kind does not match trait syntax")));

        let mut indexing_ids = IndexingIds::new_root();
        let root_module_id = indexing_ids.root_module_id();
        lowerer.item_index = index_root_module_items(&mut indexing_ids, &trait_module);
        let trait_id = lowerer
            .item_index
            .item_at_source(root_module_id, 0)
            .unwrap()
            .def_id;
        lowerer.items.insert_trait_def(HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: "Missing".to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
        lowerer
            .resolver
            .item_names_by_id
            .insert(trait_id, "Missing".to_string());

        lowerer.lower_trait_default_bodies(&trait_module, root_module_id);

        assert!(lowerer.errors().iter().any(|error| error
            .message
            .contains("missing indexed trait method 'value' while lowering default body")));
    }
}
