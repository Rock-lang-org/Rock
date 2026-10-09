//! Closed native return authority, attached only after producer Borrow/agreement checks.

use super::*;
use crate::mir::{MirCallable, MirProgram};

#[cfg(test)]
#[path = "native_borrowing_tests.rs"]
mod tests;

fn declaration(products: &CompilerProducts, id: ProductDefId) -> Option<&ProductFunctionInterface> {
    products.interface.functions.get(&id).or_else(|| {
        let id = product_def_id_to_def_id(id);
        products
            .interface
            .impls
            .values()
            .flat_map(|implementation| implementation.methods.values())
            .find(|method| method.id == id)
    })
}

fn declaration_mut(
    products: &mut CompilerProducts,
    id: ProductDefId,
) -> Option<&mut ProductFunctionInterface> {
    if products.interface.functions.contains_key(&id) {
        return products.interface.functions.get_mut(&id);
    }
    let id = product_def_id_to_def_id(id);
    products
        .interface
        .impls
        .values_mut()
        .flat_map(|implementation| implementation.methods.values_mut())
        .find(|method| method.id == id)
}

pub(crate) fn validate(products: &CompilerProducts) -> Result<(), String> {
    // A physical layout row cannot replace the nominal declaration's fields.
    // Referenced dependency layouts are checked after canonical loading, when
    // their defining schemas are available to MIR agreement.
    for function in products.interface.functions.values().chain(
        products
            .interface
            .impls
            .values()
            .flat_map(|imp| imp.methods.values()),
    ) {
        let mut invalid = false;
        crate::type_services::visit::visit_type(&function.ret_type, &mut |ty: &Type| {
            if let Type::EnumRepresentation { id, args, variants } = ty {
                let Some(declared) = products.interface.enums.get(&ProductDefId::from(*id)) else {
                    invalid |= id.crate_id.0 == 0;
                    return;
                };
                if declared.generic_params.len() != args.len()
                    || declared.variants.len() != variants.len()
                {
                    invalid = true;
                    return;
                }
                let substitution = declared
                    .generic_params
                    .iter()
                    .map(|parameter| parameter.id)
                    .zip(args.iter().cloned())
                    .collect();
                invalid |= declared
                    .variants
                    .iter()
                    .zip(variants)
                    .any(|(declared, fields)| {
                        let declared = match &declared.fields {
                            crate::hir::HirVariantFields::Unit => vec![],
                            crate::hir::HirVariantFields::Positional(fields) => fields.clone(),
                            crate::hir::HirVariantFields::Named(fields) => {
                                fields.iter().map(|field| field.ty.clone()).collect()
                            }
                        };
                        declared.len() != fields.len()
                            || declared.iter().zip(fields).any(|(declared, actual)| {
                                crate::type_services::callable::value_representation(
                                    &declared.substitute_generics(&substitution),
                                    actual,
                                )
                                .as_ref()
                                    != Some(actual)
                            })
                    });
                return;
            }
            let Type::StructRepresentation { id, args, fields } = ty else {
                return;
            };
            let Some(declared) = products.interface.structs.get(&ProductDefId::from(*id)) else {
                invalid |= id.crate_id.0 == 0;
                return;
            };
            if declared.generic_params.len() != args.len() || declared.fields.len() != fields.len()
            {
                invalid = true;
                return;
            }
            let substitution = declared
                .generic_params
                .iter()
                .map(|parameter| parameter.id)
                .zip(args.iter().cloned())
                .collect();
            invalid |= declared
                .fields
                .iter()
                .zip(fields)
                .any(|(declared, actual)| {
                    crate::type_services::callable::value_representation(
                        &declared.ty.substitute_generics(&substitution),
                        actual,
                    )
                    .as_ref()
                        != Some(actual)
                });
        });
        if invalid {
            return Err(
                "native result representation disagrees with its nominal field schema".into(),
            );
        }
    }
    if !products.link.native_return_origins.is_empty() && products.link.object_path.is_none() {
        return Err("native return authority requires its producer object link".into());
    }
    for (id, origins) in &products.link.native_return_origins {
        let link = products
            .link
            .records
            .get(id)
            .ok_or("native return authority has no canonical link record")?;
        let declaration = declaration(products, *id)
            .ok_or("native return authority has no canonical declaration")?;
        if link.polymorphic_member.is_some()
            || link.backend_symbol.is_empty()
            || !declaration.generic_params.is_empty()
            || !declaration.generic_bounds.is_empty()
            || origins.params != declaration.params
            || origins.ret != declaration.ret_type
            || !origins
                .params
                .iter()
                .chain(std::iter::once(&origins.ret))
                .all(crate::type_services::facts::TypeFacts::is_codegen_concrete)
            || !origins
                .sources_are_valid(|ty| matches!(ty, Type::Reference { .. } | Type::Pointer(_)))
            || origins
                .sources
                .contains(&crate::hir::borrowing::NativeReturnSource::UnknownExternal)
                && !origins.ret.is_numeric()
                && !matches!(origins.ret, Type::Bool | Type::Char)
        {
            return Err(
                "native return authority disagrees with its closed callable signature or sources"
                    .into(),
            );
        }
    }
    Ok(())
}

impl CompilerProducts {
    /// Publish checked physical result layouts without inventing loan sources.
    /// In particular, an empty measured loan set is not rewritten to Static.
    fn attach_native_return_representations(
        &mut self,
        program: &MirProgram,
        remap: &ProductIdRemap,
    ) -> Result<(), String> {
        for (id, body) in program.functions() {
            let storage = program.backend_contract.function_result_storage(body);
            let mut result = program.type_context.type_for(storage);
            if !crate::type_services::visit::type_any(&result, |ty| {
                matches!(
                    ty,
                    Type::StructRepresentation { .. } | Type::EnumRepresentation { .. }
                ) || matches!(ty, Type::Function { captures, .. } if !captures.is_empty())
            }) {
                continue;
            }
            let Some(header) = program
                .backend_contract
                .function_bodies
                .get(id)
                .and_then(|key| program.backend_contract.callable(key))
            else {
                continue;
            };
            let Some(definition) = header.source_def_id else {
                continue;
            };
            let Some(product_id) = unambiguous_product_id(ProductDefId::from(definition), remap)
            else {
                continue;
            };
            if !self.link.records.contains_key(&product_id)
                || self.bodies.functions.contains_key(&product_id)
            {
                continue;
            }
            let Some(declared) = declaration(self, product_id) else {
                continue;
            };
            if !declared.generic_params.is_empty()
                || !declared.generic_bounds.is_empty()
                || !crate::mir::object_runtime_type_is_valid(
                    &program.backend_contract,
                    &program.type_context,
                    storage,
                )
            {
                continue;
            }
            result.try_remap_def_ids(&mut |id| {
                unambiguous_product_id(ProductDefId::from(id), remap)
                    .map(product_def_id_to_def_id)
                    .ok_or_else(|| {
                        "native representation has no canonical product identity".to_string()
                    })
            })?;
            let parameters = body
                .local_decls
                .iter()
                .skip(1)
                .take(body.arg_count)
                .map(|local| {
                    let mut ty = program.type_context.type_for(local.ty);
                    ty.try_remap_def_ids(&mut |id| {
                        unambiguous_product_id(ProductDefId::from(id), remap)
                            .map(product_def_id_to_def_id)
                            .ok_or_else(|| {
                                "native parameter has no canonical product identity".to_string()
                            })
                    })?;
                    Ok(ty)
                })
                .collect::<Result<Vec<_>, String>>()?;
            if parameters != declared.params
                || result == declared.ret_type
                || crate::type_services::callable::value_representation(&declared.ret_type, &result)
                    .as_ref()
                    != Some(&result)
            {
                continue;
            }
            declaration_mut(self, product_id)
                .ok_or("native representation declaration disappeared")?
                .ret_type = result;
        }
        Ok(())
    }

    pub(crate) fn attach_native_return_origins(
        &mut self,
        program: &MirProgram,
        remap: &ProductIdRemap,
    ) -> Result<(), String> {
        self.attach_native_return_representations(program, remap)?;
        let checked = crate::mir::borrowck::BorrowChecker::checked_native_return_origins(program);
        let returned =
            crate::mir::borrowck::BorrowChecker::checked_returned_callable_borrowing(program);
        for (id, origins) in checked {
            let Some(key) = program.backend_contract.function_bodies.get(&id) else {
                continue;
            };
            let Some(header) = program.backend_contract.callable(key) else {
                continue;
            };
            if MirCallable::Resolved(key.clone()).function_id().as_ref() != Some(&id) {
                continue;
            }
            let Some(definition) = header.source_def_id else {
                continue;
            };
            let Some(product_id) = unambiguous_product_id(ProductDefId::from(definition), remap)
            else {
                continue;
            };
            if !self.link.records.contains_key(&product_id) {
                continue;
            }
            let Some(declaration) = declaration(self, product_id) else {
                continue;
            };
            if !declaration.generic_params.is_empty()
                || !declaration.generic_bounds.is_empty()
                || !declaration
                    .params
                    .iter()
                    .chain(std::iter::once(&declaration.ret_type))
                    .all(crate::type_services::facts::TypeFacts::is_codegen_concrete)
            {
                continue;
            }
            let portable = origins.map_types(&mut |id| {
                let mut ty = program.type_context.type_for(*id);
                let mut missing = false;
                ty.remap_def_ids(&mut |id| match unambiguous_product_id(
                    ProductDefId::from(id),
                    remap,
                ) {
                    Some(id) => product_def_id_to_def_id(id),
                    None => {
                        missing = true;
                        id
                    }
                });
                if missing {
                    Err("native return type has no canonical product identity".into())
                } else {
                    Ok(ty)
                }
            });
            // Optional evidence cannot invent a portable identity for an
            // ambiguous/scoped type, or make otherwise valid exports fail.
            // Omitted rows keep the existing conservative caller behavior.
            let Ok(mut portable) = portable else {
                continue;
            };
            // Closed specializations cannot substitute for declaration authority.
            if portable.params != declaration.params || portable.ret != declaration.ret_type {
                continue;
            }
            // Closed, bodyless factories can publish their actual returned
            // callable's invocation envelope inside the existing canonical type
            // row. The native return record binds it to this producer; ordinary
            // function headers and caller-specialized generic bodies do not.
            if !self.bodies.functions.contains_key(&product_id) {
                if let (
                    Some(measured),
                    Type::Function {
                        result_borrowing, ..
                    },
                ) = (returned.get(&id), &mut portable.ret)
                {
                    if result_borrowing.accepts(measured) {
                        *result_borrowing = measured.clone();
                        declaration_mut(self, product_id)
                            .ok_or("native return declaration disappeared")?
                            .ret_type = portable.ret.clone();
                    }
                }
            }
            if self
                .link
                .native_return_origins
                .insert(product_id, portable)
                .is_some()
            {
                return Err(
                    "duplicate native return authority for canonical product callable".into(),
                );
            }
        }
        Ok(())
    }
}
