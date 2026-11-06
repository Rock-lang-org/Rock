use super::functions::{build_function_header_with_id, build_function_sig_with_id};
use super::where_clauses::lower_where_clauses;
use crate::ast;
use crate::collect::context::CollectContext;
use crate::collect::CollectedTraitMemberIds;
use crate::hir::*;
use crate::ids::{AssocTypeId, DefId};
use crate::types::{GenericParamDecl, GenericParamId};
use std::collections::HashMap;

pub(crate) fn build_trait_with_id(
    context: &mut CollectContext,
    td: &ast::TraitDecl,
    id: DefId,
    member_ids: &CollectedTraitMemberIds,
) -> HirTrait {
    let mut generic_params =
        crate::type_lowering::lower_generic_param_decls(id, &td.generic_params);
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect::<Vec<_>>();

    let outer_trait_scope = context
        .generics
        .enter_trait(td.name.name.clone());
    let mut generic_context_params = generic_param_names.clone();
    generic_context_params.push(
        td.for_
            .as_ref()
            .map(|target| target.name.name.clone())
            .unwrap_or_else(|| "Self".to_string()),
    );
    let mut generic_context_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect::<Vec<_>>();
    generic_context_kinds.push(
        td.for_
            .as_ref()
            .map(|target| crate::type_lowering::lower_generic_param_kind(target.kind.as_ref()))
            .unwrap_or(crate::type_services::kind::Kind::Type),
    );
    let mut generic_context_spans = td
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    generic_context_spans.push(
        td.for_
            .as_ref()
            .map(|target| target.name.span.clone())
            .unwrap_or_else(|| td.name.span.clone()),
    );
    let previous_generics = context.push_generic_context_with_kinds_at(
        id,
        generic_context_params,
        generic_context_kinds,
        &generic_context_spans,
    );

    let subject_name = td
        .for_
        .as_ref()
        .map(|param| param.name.name.clone())
        .unwrap_or_else(|| "Self".into());
    let subject = GenericParamDecl::new(
        GenericParamId {
            owner: id,
            index: generic_params.len() as u32,
        },
        subject_name.clone(),
        td.for_
            .as_ref()
            .map(|param| crate::type_lowering::lower_generic_param_kind(param.kind.as_ref()))
            .unwrap_or(crate::type_services::kind::Kind::Type),
    );
    crate::type_lowering::lower_param_defaults(
        context,
        &mut generic_params,
        &td.generic_params,
        Some((subject_name, subject)),
    );
    let associated_types: Vec<HirAssociatedTypeDecl> = td
        .associated_types
        .iter()
        .enumerate()
        .map(|(index, assoc)| HirAssociatedTypeDecl {
            id: AssocTypeId(index as u32),
            name: assoc.name.name.clone(),
            kind: crate::type_lowering::lower_associated_type_kind(assoc.kind.as_ref()),
        })
        .collect();
    let target = td.for_.as_ref().map(|target| {
        GenericParamDecl::new(
            GenericParamId {
                owner: id,
                index: generic_params.len() as u32,
            },
            target.name.name.clone(),
            crate::type_lowering::lower_generic_param_kind(target.kind.as_ref()),
        )
    });
    let predicates = lower_where_clauses(context, &td.where_clauses).predicates;
    let previous_trait = context.traits.insert(
        td.name.name.clone(),
        HirTrait {
            id,
            name: td.name.name.clone(),
            generic_params: generic_params.clone(),
            target: target.clone(),
            predicates: predicates.clone(),
            associated_types: associated_types.clone(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        },
    );

    let mut methods = HashMap::new();
    for (ident, fd) in &td.methods {
        let method_id = *member_ids
            .methods
            .get(&ident.name)
            .expect("trait method IDs must be allocated before header building");
        let func = build_function_header_with_id(context, fd, method_id);
        methods.insert(ident.name.clone(), func);
    }

    let mut signatures = HashMap::new();
    for (ident, sig) in &td.signatures {
        let signature_id = *member_ids
            .signatures
            .get(&ident.name)
            .expect("trait signature IDs must be allocated before header building");
        let hir_sig = build_function_sig_with_id(context, sig, signature_id);
        signatures.insert(ident.name.clone(), hir_sig);
    }

    if let Some(previous_trait) = previous_trait {
        context.traits.insert(td.name.name.clone(), previous_trait);
    } else {
        context.traits.remove(&td.name.name);
    }

    context.generics.leave_trait(outer_trait_scope);
    context.pop_generic_context_with_kinds(previous_generics);

    HirTrait {
        id,
        name: td.name.name.clone(),
        generic_params,
        target,
        predicates,
        associated_types,
        methods,
        signatures,
    }
}
