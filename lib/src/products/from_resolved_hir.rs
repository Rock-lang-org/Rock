use std::collections::{BTreeMap, BTreeSet};

use super::artifact_metadata::{
    ProductBodies, ProductCrateIdentity, ProductDependencyIdentity, ProductIdentityTable,
    ProductLinkData, ProductSourceFingerprint,
};
use super::id_validation::assert_valid_current_crate_hir_ids;
use super::identity_recording::{
    impl_display_name, product_def_id_to_def_id, qualify_metadata_name, record_display_name,
    record_id_remap, record_name, record_product_aliases, remap_link_data, reserve_product_id,
    PreferredTraitDefaultMethods,
};
use super::interface_types::{
    CompilerProducts, ProductEnumInterface, ProductExternInterface, ProductFunctionInterface,
    ProductImplInterface, ProductInterface, ProductStructInterface, ProductTraitInterface,
    ProductTypeAliasInterface,
};
use super::language_items_build::{is_current_crate_def, product_language_items_from_program};
use super::owned_type_id_remap::{
    remap_enum_owned_type_ids, remap_extern_owned_type_ids, remap_function_owned_type_ids,
    remap_impl_owned_type_ids, remap_struct_owned_type_ids, remap_trait_owned_type_ids,
    remap_type_alias_owned_type_ids,
};
use super::type_id_remap::remap_product_type_def_ids;
use super::{ProductCrateId, ProductDefId, ProductIdRemap};
use crate::hir::{
    function_requires_downstream_specialization, hir_function_is_codegen_concrete,
    impl_requires_downstream_specialization,
};
use crate::infer::ResolvedHirProgram;

impl CompilerProducts {
    pub fn from_resolved_hir(
        crate_identity: ProductCrateIdentity,
        hir: &ResolvedHirProgram,
        dependencies: Vec<ProductDependencyIdentity>,
        dependency_crate_identities: BTreeMap<ProductCrateId, ProductCrateIdentity>,
        source_fingerprint: ProductSourceFingerprint,
        link: ProductLinkData,
    ) -> Result<Self, String> {
        Self::from_resolved_hir_with_remap(
            crate_identity,
            hir,
            dependencies,
            dependency_crate_identities,
            source_fingerprint,
            link,
        )
        .map(|(products, _)| products)
    }

    pub(crate) fn from_resolved_hir_with_remap(
        crate_identity: ProductCrateIdentity,
        hir: &ResolvedHirProgram,
        dependencies: Vec<ProductDependencyIdentity>,
        dependency_crate_identities: BTreeMap<ProductCrateId, ProductCrateIdentity>,
        source_fingerprint: ProductSourceFingerprint,
        link: ProductLinkData,
    ) -> Result<(Self, ProductIdRemap), String> {
        let crate_identity = crate_identity.with_source_fingerprint(source_fingerprint.clone());
        let local_crate = ProductCrateId::from(hir.root_crate_id);
        assert_valid_current_crate_hir_ids(hir);
        let mut identity_table = ProductIdentityTable {
            local_crate: Some(local_crate),
            dependencies: dependency_crate_identities,
            ..ProductIdentityTable::default()
        };
        let mut interface = ProductInterface::default();
        let mut bodies = ProductBodies::default();
        let preferred_default_methods = PreferredTraitDefaultMethods::new(&hir.program);
        let fallback_crate = ProductCrateId::from(hir.root_crate_id);
        let crate_name = crate_identity.name.clone();
        let mut next_fallback_local = hir.local_def_ids.next_raw();
        let mut used_ids = BTreeSet::new();
        let mut id_remap = ProductIdRemap::new();
        let mut trait_member_product_ids = BTreeMap::new();
        let mut functions = hir.program.functions_by_id().collect::<Vec<_>>();
        functions.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, function) in functions {
            if !is_current_crate_def(hir, function.id) {
                continue;
            }
            let requested_id = ProductDefId::from(function.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut function = function.clone();
            let original_id = function.id;
            function.id = product_def_id_to_def_id(id);
            remap_function_owned_type_ids(&mut function, original_id);
            interface
                .functions
                .insert(id, ProductFunctionInterface::from(&function));
            if function_requires_downstream_specialization(&function)
                || !hir_function_is_codegen_concrete(&function)
            {
                bodies.functions.insert(id, function);
            }
        }

        let mut structs = hir.program.structs_by_id().collect::<Vec<_>>();
        structs.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, strukt) in structs {
            if !is_current_crate_def(hir, strukt.id) {
                continue;
            }
            let requested_id = ProductDefId::from(strukt.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut strukt = strukt.clone();
            let original_id = strukt.id;
            strukt.id = product_def_id_to_def_id(id);
            remap_struct_owned_type_ids(&mut strukt, original_id);
            interface
                .structs
                .insert(id, ProductStructInterface::from(&strukt));
        }

        let mut enums = hir.program.enums_by_id().collect::<Vec<_>>();
        enums.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, enm) in enums {
            if !is_current_crate_def(hir, enm.id) {
                continue;
            }
            let requested_id = ProductDefId::from(enm.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut enm = enm.clone();
            let original_id = enm.id;
            enm.id = product_def_id_to_def_id(id);
            remap_enum_owned_type_ids(&mut enm, original_id);
            interface.enums.insert(id, ProductEnumInterface::from(&enm));
        }

        let mut type_aliases = hir.program.type_aliases.iter().collect::<Vec<_>>();
        type_aliases.sort_by_key(|(id, alias)| (**id, alias.name.clone()));
        for (_, alias) in type_aliases {
            if !is_current_crate_def(hir, alias.id) {
                continue;
            }
            let requested_id = ProductDefId::from(alias.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, &alias.name);
            let mut alias = alias.clone();
            let original_id = alias.id;
            alias.id = product_def_id_to_def_id(id);
            remap_type_alias_owned_type_ids(&mut alias, original_id);
            interface
                .type_aliases
                .insert(id, ProductTypeAliasInterface::from(&alias));
        }

        let mut traits = hir.program.traits_by_id().collect::<Vec<_>>();
        traits.sort_by_key(|(id, name, _)| (*id, (*name).to_string()));
        for (_, name, trt) in traits {
            if !is_current_crate_def(hir, trt.id) {
                continue;
            }
            let requested_id = ProductDefId::from(trt.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, name);
            let mut trt = trt.clone();
            let mut signature_backed_method_ids = BTreeMap::new();
            let mut signature_names = trt.signatures.keys().cloned().collect::<Vec<_>>();
            signature_names.sort();
            for signature_name in signature_names {
                let sig = trt
                    .signatures
                    .get_mut(&signature_name)
                    .expect("signature name collected from trait signature map");
                let original_signature_id = sig.id;
                let requested_signature_id = ProductDefId::from(original_signature_id);
                let signature_id = reserve_product_id(
                    requested_signature_id,
                    &mut used_ids,
                    fallback_crate,
                    &mut next_fallback_local,
                    false,
                );
                record_id_remap(&mut id_remap, requested_signature_id, signature_id);
                trait_member_product_ids.insert(original_signature_id, signature_id);
                if trt
                    .methods
                    .get(&signature_name)
                    .is_some_and(|method| method.id == original_signature_id)
                {
                    signature_backed_method_ids.insert(signature_name.clone(), signature_id);
                }
                record_display_name(
                    &mut identity_table,
                    signature_id,
                    &format!("{}::{}", name, signature_name),
                );
                sig.id = product_def_id_to_def_id(signature_id);
            }
            let mut method_names = trt.methods.keys().cloned().collect::<Vec<_>>();
            method_names.sort();
            for method_name in method_names {
                let method = &trt.methods[&method_name];
                let original_method_id = method.id;
                let requested_method_id = ProductDefId::from(original_method_id);
                // A default body and its signature share one canonical member ID.
                // Reserving it twice makes call-target remapping ambiguous.
                let method_id = signature_backed_method_ids
                    .get(&method_name)
                    .copied()
                    .unwrap_or_else(|| {
                        reserve_product_id(
                            requested_method_id,
                            &mut used_ids,
                            fallback_crate,
                            &mut next_fallback_local,
                            false,
                        )
                    });
                record_id_remap(&mut id_remap, requested_method_id, method_id);
                trait_member_product_ids.insert(original_method_id, method_id);
                record_display_name(
                    &mut identity_table,
                    method_id,
                    &format!("{}::{}", name, method_name),
                );
                let mut method =
                    preferred_default_methods.get(original_method_id, &trt.name, &method_name);
                method.id = product_def_id_to_def_id(method_id);
                remap_function_owned_type_ids(&mut method, original_method_id);
                bodies
                    .trait_default_methods
                    .insert(method_id, method.clone());
                trt.methods.insert(method_name, method);
            }
            let original_id = trt.id;
            trt.id = product_def_id_to_def_id(id);
            remap_trait_owned_type_ids(&mut trt, original_id);
            trt.name = qualify_metadata_name(&crate_name, name);
            interface
                .traits
                .insert(id, ProductTraitInterface::from(&trt));
        }

        let mut impls = hir.program.impls_by_id().collect::<Vec<_>>();
        let mut seen_impl_ids = impls.iter().map(|(id, _, _)| *id).collect::<BTreeSet<_>>();
        for (index, (id, imp)) in hir.program.impls_in_order().enumerate() {
            if seen_impl_ids.insert(imp.id) {
                impls.push((id, index, imp));
            }
        }
        impls.sort_by_key(|(id, index, _)| (*id, *index));
        impls.dedup_by_key(|(id, index, _)| (*id, *index));
        for (_, _, imp) in impls {
            if !is_current_crate_def(hir, imp.id) {
                continue;
            }
            let requested_id = ProductDefId::from(imp.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_display_name(&mut identity_table, id, &impl_display_name(imp));
            let mut imp = imp.clone();
            let impl_name = impl_display_name(&imp);
            let original_id = imp.id;
            let mut impl_method_product_ids = BTreeMap::new();
            imp.id = product_def_id_to_def_id(id);
            let mut method_names = imp.methods.keys().cloned().collect::<Vec<_>>();
            method_names.sort();
            for method_name in method_names {
                let method = imp
                    .methods
                    .get_mut(&method_name)
                    .expect("method name collected from impl method map");
                let original_method_id = method.id;
                let requested_method_id = ProductDefId::from(original_method_id);
                let method_id = reserve_product_id(
                    requested_method_id,
                    &mut used_ids,
                    fallback_crate,
                    &mut next_fallback_local,
                    false,
                );
                record_id_remap(&mut id_remap, requested_method_id, method_id);
                impl_method_product_ids.insert(original_method_id, method_id);
                record_display_name(
                    &mut identity_table,
                    method_id,
                    &format!("{}::{}", impl_name, method_name),
                );
                method.id = product_def_id_to_def_id(method_id);
                remap_function_owned_type_ids(method, original_method_id);
            }
            for (&(impl_id, trait_member_id), &impl_method_id) in
                &hir.program.indexes.effective_trait_methods
            {
                if impl_id != original_id {
                    continue;
                }
                let trait_member_id = trait_member_product_ids
                    .get(&trait_member_id)
                    .copied()
                    .unwrap_or_else(|| ProductDefId::from(trait_member_id));
                let impl_method_id =
                    *impl_method_product_ids
                        .get(&impl_method_id)
                        .unwrap_or_else(|| {
                            panic!(
                                "effective impl method {:?} has no product identity on impl {:?}",
                                impl_method_id, original_id
                            )
                        });
                interface
                    .effective_trait_methods
                    .insert((id, trait_member_id), impl_method_id);
            }
            remap_impl_owned_type_ids(&mut imp, original_id);
            if impl_requires_downstream_specialization(&imp) {
                bodies.generic_impls.insert(id, imp.clone());
            }
            interface.impls.insert(id, ProductImplInterface::from(&imp));
        }

        let mut externs = hir.program.externs_by_id().collect::<Vec<_>>();
        let mut seen_extern_ids = externs
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<BTreeSet<_>>();
        for (index, (id, ext)) in hir.program.externs_in_order().enumerate() {
            if seen_extern_ids.insert(ext.id) {
                externs.push((id, index, ext));
            }
        }
        externs.sort_by_key(|(id, _, ext)| (*id, ext.name.clone()));
        for (_, _, ext) in externs {
            if !is_current_crate_def(hir, ext.id) {
                continue;
            }
            let requested_id = ProductDefId::from(ext.id);
            let id = reserve_product_id(
                requested_id,
                &mut used_ids,
                fallback_crate,
                &mut next_fallback_local,
                false,
            );
            record_id_remap(&mut id_remap, requested_id, id);
            record_name(&mut identity_table, id, &ext.name);
            let mut ext = ext.clone();
            let original_id = ext.id;
            ext.id = product_def_id_to_def_id(id);
            remap_extern_owned_type_ids(&mut ext, original_id);
            ext.name = qualify_metadata_name(&crate_name, &ext.name);
            interface
                .externs
                .insert(id, ProductExternInterface::from(&ext));
        }

        remap_product_type_def_ids(&mut interface, &mut bodies, &id_remap);
        let summaries = hir
            .constant_arithmetic
            .iter()
            .flat_map(|(id, summary)| {
                let product_id = ProductDefId::from(*id);
                id_remap
                    .get(&product_id)
                    .into_iter()
                    .flatten()
                    .map(move |id| (*id, summary))
            })
            .collect::<BTreeMap<_, _>>();
        for function in interface
            .functions
            .values_mut()
            .chain(
                interface
                    .impls
                    .values_mut()
                    .flat_map(|imp| imp.methods.values_mut()),
            )
            .chain(
                interface
                    .traits
                    .values_mut()
                    .flat_map(|trt| trt.methods.values_mut()),
            )
        {
            function.constant_arithmetic = summaries
                .get(&ProductDefId::from(function.id))
                .map(|summary| (*summary).clone());
        }
        interface.language_items =
            product_language_items_from_program(&hir.program, &hir.current_def_ids, &id_remap)?;
        record_product_aliases(
            &mut identity_table.import_alias_names,
            &hir.resolver.import_aliases,
            &id_remap,
        );
        record_product_aliases(
            &mut identity_table.export_names,
            &hir.resolver.export_aliases,
            &id_remap,
        );
        record_product_aliases(
            &mut identity_table.module_alias_names,
            &hir.resolver.module_aliases,
            &id_remap,
        );
        for (alias, target) in &hir.resolver.variant_exports {
            let requested = ProductDefId::from(target.enum_id);
            let id = if requested.crate_id == local_crate {
                super::identity_recording::unambiguous_product_id(requested, &id_remap)
                    .ok_or_else(|| format!("variant export '{alias}' has no unique enum identity"))?
            } else {
                requested
            };
            identity_table.variant_export_names.insert(
                alias.clone(),
                (id, target.variant.clone()),
            );
        }

        let link = remap_link_data(link, &id_remap);

        Ok((
            Self {
                crate_identity,
                identity_table,
                interface,
                bodies,
                link,
                dependencies,
                source_fingerprint,
                infix_precedence: BTreeMap::new(),
                proc_macros: Vec::new(),
            },
            id_remap,
        ))
    }
}
