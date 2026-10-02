use crate::crate_artifact::ArtifactCrateInterface;
use crate::crate_system::CrateContext;
use crate::products::object_abi::ProductObjectSchema;
use crate::type_services::normalize::TypeNormalizationEnv;
use crate::types::{AssociatedTypeKey, NominalTypeKind, Type};
use std::collections::{BTreeMap, HashMap};

pub(super) fn admit(
    interface: &ArtifactCrateInterface,
    ctx: &CrateContext,
    object_backed: bool,
) -> Result<(), String> {
    let interfaces = ctx
        .extern_crates()
        .map(|dep| dep.metadata().interface())
        .chain(std::iter::once(interface))
        .collect::<Vec<_>>();
    let mut traits = HashMap::new();
    let mut orders = BTreeMap::new();
    let mut schemas: BTreeMap<Type, &ProductObjectSchema> = BTreeMap::new();
    let mut env = TypeNormalizationEnv::new();
    let mut aliases = Vec::new();
    let mut nominal_fields = BTreeMap::new();
    let mut member_owners = BTreeMap::new();
    for interface in &interfaces {
        if interface
            .traits
            .keys()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            != interface.object_abi.trait_members.keys().copied().collect()
        {
            return Err(
                "artifact trait declarations are missing their producer member layouts".into(),
            );
        }
        for (id, record) in &interface.structs {
            env.register_constructor(
                *id,
                NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&record.generic_params),
            );
            for param in &record.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            nominal_fields.insert(
                *id,
                (
                    record.generic_params.clone(),
                    record
                        .fields
                        .iter()
                        .map(|field| field.ty.clone())
                        .collect::<Vec<_>>(),
                ),
            );
        }
        for (id, record) in &interface.enums {
            env.register_constructor(
                *id,
                NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&record.generic_params),
            );
            for param in &record.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            let fields = record
                .variants
                .iter()
                .flat_map(|variant| match &variant.fields {
                    crate::hir::HirVariantFields::Unit => Vec::new(),
                    crate::hir::HirVariantFields::Positional(fields) => fields.clone(),
                    crate::hir::HirVariantFields::Named(fields) => {
                        fields.iter().map(|field| field.ty.clone()).collect()
                    }
                })
                .collect::<Vec<_>>();
            nominal_fields.insert(*id, (record.generic_params.clone(), fields));
        }
        aliases.extend(
            interface
                .type_aliases
                .values()
                .map(crate::crate_artifact::types::hir_type_alias_from_interface),
        );
        for (id, record) in &interface.traits {
            let declaration = crate::crate_artifact::types::hir_trait_from_interface(record);
            for signature in declaration.signatures.values() {
                if member_owners
                    .insert(signature.id, *id)
                    .is_some_and(|owner| owner != *id)
                {
                    return Err("object ABI member identity belongs to multiple traits".into());
                }
            }
            for param in declaration
                .generic_params
                .iter()
                .chain(declaration.target.iter())
                .chain(
                    declaration
                        .signatures
                        .values()
                        .flat_map(|sig| sig.generic_params.iter()),
                )
            {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for assoc in &declaration.associated_types {
                env.register_projection_kind(
                    AssociatedTypeKey {
                        owner: *id,
                        assoc_type_id: assoc.id,
                    },
                    assoc.kind.clone(),
                );
            }
            traits.insert(*id, declaration);
        }
        for (id, order) in &interface.object_abi.trait_members {
            if orders
                .insert(*id, order.clone())
                .is_some_and(|old| old != *order)
            {
                return Err("conflicting producer trait member layouts".into());
            }
        }
        for schema in &interface.object_abi.schemas {
            if schemas
                .insert(schema.object.clone(), schema)
                .is_some_and(|old| old != schema)
            {
                return Err("conflicting producer object schemas after importer remapping".into());
            }
        }
    }
    crate::type_lowering::register_type_aliases(&mut env, aliases.iter());
    interface
        .object_abi
        .validate_declarations(&traits, &orders, &env)?;
    // A compiled foreign API can exchange metadata created in the producer.
    // Generic providers are specialized locally and use persisted trait layouts.
    if object_backed {
        for function in interface.functions.values().chain(
            interface
                .impls
                .values()
                .flat_map(|imp| imp.methods.values()),
        ) {
            if super::product_function_requires_object_link_record(function) {
                for ty in function
                    .params
                    .iter()
                    .chain(std::iter::once(&function.ret_type))
                {
                    check_runtime_objects(
                        ty,
                        &nominal_fields,
                        &schemas,
                        &env,
                        &mut Default::default(),
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn check_runtime_objects(
    ty: &Type,
    fields: &BTreeMap<crate::ids::DefId, (Vec<crate::types::GenericParamDecl>, Vec<Type>)>,
    schemas: &BTreeMap<Type, &ProductObjectSchema>,
    env: &TypeNormalizationEnv,
    seen: &mut std::collections::HashSet<Type>,
) -> Result<(), String> {
    let ty = crate::type_services::normalize::TypeNormalizer::new(env)
        .normalize(ty)
        .map_err(|e| e.to_string())?;
    if !seen.insert(ty.clone()) {
        return Ok(());
    }
    if matches!(ty, Type::Object(_)) {
        return if crate::type_services::visit::type_any(&ty, |ty| matches!(ty, Type::Generic(_)))
            || schemas.contains_key(&ty)
        {
            Ok(())
        } else {
            Err("object-backed interface is missing its producer object schema".into())
        };
    }
    let mut children = Vec::new();
    crate::type_services::visit::visit_type_children(&ty, &mut |ty: &Type| {
        children.push(ty.clone())
    });
    for child in children {
        check_runtime_objects(&child, fields, schemas, env, seen)?;
    }
    if let Type::Struct { id, args } | Type::Enum { id, args } = &ty {
        if let Some((params, fields_of_type)) = fields.get(id) {
            if params.len() != args.len() {
                return Err("object ABI nominal type argument arity mismatch".into());
            }
            let substitution = params
                .iter()
                .zip(args)
                .map(|(param, arg)| (param.id, arg.clone()))
                .collect();
            for field in fields_of_type {
                let field = crate::type_services::substitution::instantiate(field, &substitution)
                    .map_err(|e| e.to_string())?;
                check_runtime_objects(&field, fields, schemas, env, seen)?;
            }
        }
    }
    Ok(())
}
