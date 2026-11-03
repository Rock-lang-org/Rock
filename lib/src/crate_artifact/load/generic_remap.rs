use super::nominal_validator::ProductNominalTypeValidator;
use super::ProductIdentityRemap;
use crate::products::ProductDefId;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::TypeNormalizer;
use crate::types::{GenericParamId, Type};

pub(super) fn remap_generic_bounds(
    bounds: &mut crate::hir::HirGenericBounds,
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut original = std::mem::take(bounds);
    let predicates = std::mem::take(&mut original.predicates);
    for (mut generic_param, mut trait_bounds) in original {
        generic_param.owner = remap.def_id(ProductDefId::from(generic_param.owner))?;
        for bound in &mut trait_bounds {
            bound.trait_id = remap.def_id(ProductDefId::from(bound.trait_id))?;
            for ty in &mut bound.type_args {
                remap_type_def_ids(ty, remap, type_validator)?;
            }
        }
        bounds.insert(generic_param, trait_bounds);
    }
    bounds.predicates = predicates;
    remap_predicates(&mut bounds.predicates, remap, type_validator)?;
    Ok(())
}

pub(super) fn remap_predicates(
    predicates: &mut [crate::types::Predicate],
    remap: &ProductIdentityRemap,
    type_validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    for predicate in predicates {
        match predicate {
            crate::types::Predicate::Trait {
                subject,
                trait_id,
                args,
            } => {
                remap_type_def_ids(subject, remap, type_validator)?;
                *trait_id = remap.def_id(ProductDefId::from(*trait_id))?;
                for arg in args {
                    remap_type_def_ids(arg, remap, type_validator)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn remap_generic_param_decls(
    decls: &mut [crate::types::GenericParamDecl],
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    let mut env = validator.normalization_env.clone();
    for decl in decls.iter() {
        env.register_generic_kind(decl.id, decl.kind.clone());
    }
    let mut default_seen = false;
    for decl in decls {
        if let Some(default) = &mut decl.default {
            default_seen = true;
            let owner = ProductDefId::from(decl.id.owner);
            let subject_index = validator.trait_generic_counts.get(&owner).copied();
            if let Some(index) = subject_index {
                let subject = GenericParamId {
                    owner: decl.id.owner,
                    index: index as u32,
                };
                if TypeNormalizer::new(&env)
                    .kind_of(&Type::Generic(subject))
                    .is_err()
                {
                    env.register_generic_kind(subject, crate::type_services::kind::Kind::Type);
                }
            }
            let mut invalid_generic = false;
            crate::type_services::visit::visit_type(default, &mut |ty: &Type| {
                if let Type::Generic(param) = ty {
                    if param.owner != decl.id.owner
                        || (param.index >= decl.id.index
                            && Some(param.index as usize) != subject_index)
                    {
                        invalid_generic = true;
                    }
                }
            });
            if invalid_generic {
                return Err(
                    "Product artifact generic default references an out-of-scope parameter".into(),
                );
            }
            struct BoundScope {
                groups: Vec<Vec<Kind>>,
                valid: bool,
            }
            impl crate::type_services::visit::TypeVisitor for BoundScope {
                fn enter_binders(&mut self, params: &[Kind]) {
                    self.groups.push(params.to_vec());
                }
                fn exit_binders(&mut self) {
                    self.groups.pop();
                }
                fn visit_type(&mut self, ty: &Type) {
                    if let Type::BoundVar { depth, index, kind } = ty {
                        self.valid &= self
                            .groups
                            .iter()
                            .rev()
                            .nth(*depth as usize)
                            .and_then(|group| group.get(*index as usize))
                            == Some(kind);
                    }
                    crate::type_services::visit::visit_type_children(ty, self);
                }
            }
            let mut scope = BoundScope {
                groups: Vec::new(),
                valid: true,
            };
            crate::type_services::visit::visit_type(default, &mut scope);
            if !scope.valid {
                return Err(
                    "Product artifact generic default contains an escaping type binder".into(),
                );
            }
            let kind = TypeNormalizer::new(&env)
                .kind_of(default)
                .map_err(|error| error.to_string())?;
            if kind != decl.kind {
                return Err(format!(
                    "Product artifact generic default has kind {kind}, expected {}",
                    decl.kind
                ));
            }
            remap_type_def_ids(default, remap, validator)?;
        } else if default_seen {
            return Err("Product artifact required parameter follows a defaulted parameter".into());
        }
        decl.id.owner = remap.def_id(ProductDefId::from(decl.id.owner))?;
    }
    Ok(())
}

pub(super) fn remap_type_def_ids(
    ty: &mut crate::types::Type,
    remap: &ProductIdentityRemap,
    validator: &ProductNominalTypeValidator,
) -> Result<(), String> {
    struct ProductTypeRemapper<'a> {
        remap: &'a ProductIdentityRemap,
        validator: &'a ProductNominalTypeValidator,
    }

    impl crate::type_services::visit::TryTypeFolder for ProductTypeRemapper<'_> {
        type Error = String;

        fn try_fold_type(
            &mut self,
            mut ty: crate::types::Type,
        ) -> Result<crate::types::Type, Self::Error> {
            match &mut ty {
                crate::types::Type::Struct { id, .. } => {
                    let product_id = ProductDefId::from(*id);
                    self.validator.validate_struct(product_id)?;
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Enum { id, .. } => {
                    let product_id = ProductDefId::from(*id);
                    self.validator.validate_enum(product_id)?;
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Constructor { id, flavor } => {
                    let product_id = ProductDefId::from(*id);
                    match flavor {
                        crate::types::NominalTypeKind::Struct => {
                            self.validator.validate_struct(product_id)?
                        }
                        crate::types::NominalTypeKind::Enum => {
                            self.validator.validate_enum(product_id)?
                        }
                        crate::types::NominalTypeKind::Alias => {
                            self.validator.validate_type_alias(product_id)?
                        }
                    }
                    *id = self.remap.def_id(product_id)?;
                }
                crate::types::Type::Projection {
                    trait_id,
                    assoc_type,
                    ..
                } => {
                    let product_trait_id = ProductDefId::from(*trait_id);
                    let product_assoc_owner = ProductDefId::from(assoc_type.owner);
                    self.validator.validate_projection(
                        product_trait_id,
                        product_assoc_owner,
                        assoc_type.assoc_type_id,
                    )?;
                    *trait_id = self.remap.def_id(product_trait_id)?;
                    assoc_type.owner = self.remap.def_id(product_assoc_owner)?;
                }
                crate::types::Type::TypeVar(_) => {
                    return Err(format!(
                        "Product artifact contains unresolved type Type::TypeVar in executable metadata: {ty:?}"
                    ));
                }
                crate::types::Type::Error => {
                    return Err(format!(
                        "Product artifact contains unresolved type Type::Error in executable metadata: {ty:?}"
                    ));
                }
                crate::types::Type::Generic(param) => {
                    let product_id = ProductDefId::from(param.owner);
                    param.owner = self.remap.def_id(product_id)?;
                }
                _ => {}
            }

            crate::type_services::visit::try_fold_type_children(ty, self)
        }
    }

    let remapped = crate::type_services::visit::try_fold_type(
        ty.clone(),
        &mut ProductTypeRemapper { remap, validator },
    )?;
    *ty = remapped;
    Ok(())
}
