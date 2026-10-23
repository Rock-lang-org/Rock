use std::collections::HashMap;

use crate::ast;
use crate::hir::{HirBlock, HirFunction, HirParam, HirStmt};
use crate::ids::{DefId, HirLocalId, ModuleId};
use crate::types::{GenericParamId, Type};

use super::body_context::{BodyLoweringContext, BodyOwner};
use super::Lowerer;

impl Lowerer {
    /// Publish all initializer headers before any body can construct a struct,
    /// including constructors in another source module or another initializer.
    pub(crate) fn prepare_field_initializers(&mut self, module: &ast::Module, module_id: ModuleId) {
        for (ordinal, item) in module.top_levels.iter().enumerate() {
            match item {
                ast::TopLevel::StructDecl(declaration) => {
                    let Some(owner) = self.item_index.item_at_source(module_id, ordinal).map(|item| item.def_id) else { continue };
                    let Some(mut structure) = self.items.structure(owner).cloned() else { continue };
                    for (field, syntax) in structure.fields.iter_mut().zip(&declaration.fields) {
                        if syntax.default.is_none() || field.initializer.is_some() { continue; }
                        let id = DefId::new(self.root_crate_id, self.local_def_ids.fresh());
                        let substitutions = structure.generic_params.iter().enumerate().map(|(index, param)| {
                            (param.id, Type::Generic(GenericParamId { owner: id, index: index as u32 }))
                        }).collect::<HashMap<_, _>>();
                        let generic_params: Vec<_> = structure.generic_params.iter().enumerate().map(|(index, param)| {
                            let mut param = param.clone();
                            param.id = GenericParamId { owner: id, index: index as u32 };
                            param.default = None;
                            param
                        }).collect();
                        let mut env = self.engine.normalization_env();
                        for param in &generic_params {
                            env.register_generic_kind(param.id, param.kind.clone());
                        }
                        self.engine.set_normalization_env(env);
                        let ret_type = field.ty.substitute_generics(&substitutions);
                        // A null, never-dereferenced owner argument carries all
                        // generic arguments, including ones absent from the field
                        // type but used by its initializer body.
                        let params = if generic_params.is_empty() { Vec::new() } else {
                            vec![HirParam {
                                name: "initializer_owner".into(), local_id: HirLocalId(0),
                                ty: Type::Pointer(Box::new(Type::Struct {
                                    id: owner, args: generic_params.iter().map(|param| Type::Generic(param.id)).collect(),
                                })), mutable: false, is_ref: false,
                            }]
                        };
                        self.items.insert_function(HirFunction {
                            id, name: format!("{}::{}::initializer", structure.name, field.name),
                            generic_params, generic_bounds: Default::default(), params,
                            ret_type: ret_type.clone(), body: HirBlock { stmts: Vec::new(), ty: ret_type },
                            is_curried: false, is_method: false, self_receiver: None, is_unsafe: false,
                        });
                        self.current_def_ids.insert(id);
                        field.initializer = Some(id);
                    }
                    self.items.insert_structure(structure);
                }
                ast::TopLevel::Module(child) => {
                    if let Some(name) = &child.0.name {
                        if let Some(id) = self.item_index.child_module_for_name(module_id, name) {
                            self.prepare_field_initializers(&child.0, id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn lower_field_initializers(&mut self, declaration: &ast::StructDecl, owner: DefId) {
        let Some(structure) = self.items.structure(owner).cloned() else { return };
        for (field, syntax) in structure.fields.iter().zip(&declaration.fields) {
            let (Some(id), Some(expression)) = (field.initializer, &syntax.default) else { continue };
            let Some(mut function) = self.items.function(id).cloned() else { continue };
            let mut context = BodyLoweringContext::new(
                function.name.clone(), BodyOwner::Function(id), Some(id),
                function.generic_params.iter().map(|param| param.name.clone()).collect(),
                Default::default(), false,
            );
            for (param, syntax) in function.generic_params.iter().zip(&declaration.generic_params) {
                context.register_generic_param(param.name.clone(), syntax.name.context, param.id);
            }
            context.seed_after_existing_locals(function.params.iter().map(|param| param.local_id));
            let value = self.with_body_context(context, |lowerer| {
                lowerer.push_scope();
                let value = lowerer.lower_expression(expression);
                let value = lowerer.coerce_argument_to_expected(value, &function.ret_type);
                lowerer.pop_scope();
                value
            });
            if let Err(error) = self.engine.unify(&function.ret_type, &value.ty) {
                self.diagnostics.push_type_with_span(
                    format!("Invalid initializer for '{}.{}': {}", structure.name, field.name, error.render(&self.engine)),
                    value.span.clone(),
                );
            }
            function.body = HirBlock { ty: value.ty.clone(), stmts: vec![HirStmt::Expr(value)] };
            self.resolve_all_types_in_function(&mut function);
            self.items.insert_function(function);
        }
    }
}
