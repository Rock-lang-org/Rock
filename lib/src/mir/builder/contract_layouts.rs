use std::collections::BTreeSet;

use crate::hir::HirVariantFields;
use crate::ids::{DefId, TypeId};
use crate::type_context::TypeContext;
use crate::type_services::projection::{ProjectionNormalizer, ProjectionProvider};
use crate::types::Type;

use crate::mir::{MirFunction, MirFunctionId};

use super::projection_provider::MirProjectionResolutionProvider;
use super::*;

impl<'a> MirBuilder<'a> {
    pub(super) fn populate_backend_contract_externs(
        contract: &mut crate::mir::MirBackendContract,
        program: &HirProgram,
        type_context: &mut TypeContext,
    ) {
        for (_, ext) in program.externs_in_order() {
            let key = crate::mir::MirCallableKey::Extern(ext.id);
            let params = ext
                .params
                .iter()
                .map(|ty| type_context.intern_type(ty))
                .collect::<Vec<_>>();
            let ret = type_context.intern_type(&ext.ret);
            assert!(
                contract
                    .callables
                    .insert(
                        key.clone(),
                        crate::mir::MirCallableDecl {
                            key,
                            source_def_id: Some(ext.id),
                            kind: crate::mir::MirCallableKind::Extern {
                                link_name: ext.name.clone(),
                                variadic: ext.variadic,
                            },
                            llvm_symbol: ext.name.clone(),
                            linkage: crate::mir::MirLinkage::External,
                            signature: crate::mir::MirCallableSignature::from_type_ids(
                                &params,
                                ret,
                                crate::mir::MirPassMode::Direct,
                            ),
                        },
                    )
                    .is_none(),
                "duplicate extern callable authority for {:?}",
                ext.id,
            );
        }
    }

    pub(super) fn populate_backend_contract_nominal_layouts(
        contract: &mut crate::mir::MirBackendContract,
        program: &HirProgram,
        type_context: &mut TypeContext,
    ) {
        for (id, _, structure) in program.structs_by_id() {
            assert!(
                contract
                    .nominal_layouts
                    .insert(
                        id,
                        crate::mir::MirNominalLayout::Struct {
                            id,
                            fields: structure
                                .fields
                                .iter()
                                .map(|field| {
                                    (
                                        field.name.clone(),
                                        type_context.intern_canonical_type(&field.ty),
                                    )
                                })
                                .collect(),
                            generic_params: Self::mir_generic_param_ids(&structure.generic_params),
                        },
                    )
                    .is_none(),
                "duplicate struct layout authority for {id:?}",
            );
        }
        for (id, _, enum_def) in program.enums_by_id() {
            assert!(
                contract
                    .nominal_layouts
                    .insert(
                        id,
                        crate::mir::MirNominalLayout::Enum {
                            id,
                            variants: Self::mir_enum_variants(type_context, &enum_def.variants),
                            generic_params: Self::mir_generic_param_ids(&enum_def.generic_params),
                        },
                    )
                    .is_none(),
                "duplicate enum layout authority for {id:?}",
            );
        }
    }

    pub(super) fn populate_backend_contract_drop_glue(
        contract: &mut crate::mir::MirBackendContract,
        generated: &std::collections::BTreeMap<TypeId, crate::mono::GeneratedMethodInstance>,
    ) {
        for entry in generated.values() {
            assert!(
                contract
                    .drop_glue
                    .insert(
                        entry.receiver_ty,
                        crate::mir::MirCallableKey::Instance(entry.instance_id),
                    )
                    .is_none(),
                "duplicate generated drop glue authority for {:?}",
                entry.receiver_ty,
            );
        }
    }

    pub(super) fn populate_backend_contract_projection_outputs(
        contract: &mut crate::mir::MirBackendContract,
        program: &HirProgram,
        functions: &std::collections::BTreeMap<MirFunctionId, MirFunction>,
        type_context: &mut TypeContext,
    ) {
        let mut used_type_ids = BTreeSet::new();

        for function in functions.values() {
            used_type_ids.insert(function.ret_type);
            for local in &function.local_decls {
                used_type_ids.insert(local.ty);
            }
            for block in &function.basic_blocks {
                for statement in &block.statements {
                    if let crate::mir::StatementKind::Assign(_, crate::mir::Rvalue::Cast(_, ty)) =
                        &statement.kind
                    {
                        used_type_ids.insert(*ty);
                    }
                }
            }
        }

        for callable in contract.callables.values() {
            for param in &callable.signature.params {
                used_type_ids.insert(param.semantic_ty);
            }
            used_type_ids.insert(callable.signature.ret.semantic_ty);
            used_type_ids.insert(callable.signature.ret.abi_ty);
        }

        for layout in contract.nominal_layouts.values() {
            match layout {
                crate::mir::MirNominalLayout::Struct { fields, .. } => {
                    for (_, ty) in fields {
                        used_type_ids.insert(*ty);
                    }
                }
                crate::mir::MirNominalLayout::Enum { variants, .. } => {
                    for variant in variants {
                        match &variant.fields {
                            crate::mir::MirVariantLayoutFields::Unit => {}
                            crate::mir::MirVariantLayoutFields::Positional(fields) => {
                                for ty in fields {
                                    used_type_ids.insert(*ty);
                                }
                            }
                            crate::mir::MirVariantLayoutFields::Named(fields) => {
                                for (_, ty) in fields {
                                    used_type_ids.insert(*ty);
                                }
                            }
                        }
                    }
                }
            }
        }

        let provider = MirProjectionResolutionProvider { program };
        for ty in used_type_ids {
            Self::collect_projection_output_for_type_id(ty, type_context, &provider, contract);
        }
    }

    fn collect_projection_output_for_type_id(
        ty: TypeId,
        type_context: &mut TypeContext,
        provider: &MirProjectionResolutionProvider<'_>,
        contract: &mut crate::mir::MirBackendContract,
    ) {
        let ty = type_context.type_for(ty);
        Self::collect_projection_output_for_type(&ty, type_context, provider, contract);
    }

    fn collect_projection_output_for_type(
        ty: &Type,
        type_context: &mut TypeContext,
        provider: &MirProjectionResolutionProvider<'_>,
        contract: &mut crate::mir::MirBackendContract,
    ) {
        match ty {
            Type::Projection {
                ty: base,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                Self::collect_projection_output_for_type(base, type_context, provider, contract);
                for arg in trait_args {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }

                if assoc_type.owner == *trait_id {
                    let resolved_base = ProjectionNormalizer::normalize(provider, base);
                    let resolved_trait_args = trait_args
                        .iter()
                        .map(|arg| ProjectionNormalizer::normalize(provider, arg))
                        .collect::<Vec<_>>();
                    let output = provider.resolve_projection_output(
                        &resolved_base,
                        *trait_id,
                        assoc_type.assoc_type_id,
                        &resolved_trait_args,
                    );

                    if let Some(output) = output {
                        let output = ProjectionNormalizer::normalize(provider, &output);
                        let base = type_context.intern_type(&resolved_base);
                        let trait_args = resolved_trait_args
                            .iter()
                            .map(|arg| type_context.intern_type(arg))
                            .collect();
                        let output = type_context.intern_type(&output);
                        let key = crate::mir::MirProjectionKey {
                            base,
                            trait_id: *trait_id,
                            assoc_type_id: assoc_type.assoc_type_id,
                            trait_args,
                        };
                        if let Some(existing) = contract.projection_outputs.get(&key) {
                            assert_eq!(
                                *existing, output,
                                "conflicting projection output for canonical key {key:?}"
                            );
                        } else {
                            contract.projection_outputs.insert(key, output);
                        }
                    }
                }
            }
            Type::Reference { inner, .. } | Type::Pointer(inner) | Type::Slice(inner) => {
                Self::collect_projection_output_for_type(inner, type_context, provider, contract);
            }
            Type::Array(inner, _) => {
                Self::collect_projection_output_for_type(inner, type_context, provider, contract);
            }
            Type::Tuple(elems) => {
                for elem in elems {
                    Self::collect_projection_output_for_type(
                        elem,
                        type_context,
                        provider,
                        contract,
                    );
                }
            }
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                for arg in params {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }
                Self::collect_projection_output_for_type(ret, type_context, provider, contract);
                for capture in captures {
                    Self::collect_projection_output_for_type(
                        &capture.ty,
                        type_context,
                        provider,
                        contract,
                    );
                }
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                for arg in args {
                    Self::collect_projection_output_for_type(arg, type_context, provider, contract);
                }
            }
            _ => {}
        }
    }

    pub(super) fn instance_origin_callable_def_id(origin: &crate::mono::InstanceOrigin) -> DefId {
        match origin {
            crate::mono::InstanceOrigin::Function(id) => *id,
            crate::mono::InstanceOrigin::ImplMethod { method, .. }
            | crate::mono::InstanceOrigin::TraitDefault { method, .. } => *method,
        }
    }

    fn mir_enum_variants(
        type_context: &mut TypeContext,
        variants: &[crate::hir::HirVariant],
    ) -> Vec<crate::mir::MirEnumVariantLayout> {
        variants
            .iter()
            .map(|variant| crate::mir::MirEnumVariantLayout {
                name: variant.name.clone(),
                fields: match &variant.fields {
                    HirVariantFields::Unit => crate::mir::MirVariantLayoutFields::Unit,
                    HirVariantFields::Positional(fields) => {
                        crate::mir::MirVariantLayoutFields::Positional(
                            fields
                                .iter()
                                .map(|ty| type_context.intern_canonical_type(ty))
                                .collect(),
                        )
                    }
                    HirVariantFields::Named(fields) => crate::mir::MirVariantLayoutFields::Named(
                        fields
                            .iter()
                            .map(|field| {
                                (
                                    field.name.clone(),
                                    type_context.intern_canonical_type(&field.ty),
                                )
                            })
                            .collect(),
                    ),
                },
            })
            .collect()
    }
}
