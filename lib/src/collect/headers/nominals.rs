use super::functions::build_function_sig_with_id;
use crate::ast;
use crate::collect::context::CollectContext;
use crate::hir::*;
use crate::ids::{DefId, FieldId, VariantId};
use crate::types::{GenericParamDecl, Type};

pub(crate) fn build_nominal_generic_params(
    context: &mut CollectContext,
    syntax: &[ast::GenericParamDecl],
    id: DefId,
) -> Vec<GenericParamDecl> {
    let mut generic_params = crate::type_lowering::lower_generic_param_decls(id, syntax);
    let spans = syntax
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let previous = context.push_generic_context_with_kinds_at(
        id,
        GenericParamDecl::names(&generic_params)
            .map(str::to_string)
            .collect(),
        generic_params
            .iter()
            .map(|param| param.kind.clone())
            .collect(),
        &spans,
    );
    crate::type_lowering::lower_param_defaults(context, &mut generic_params, syntax, None);
    context.pop_generic_context_with_kinds(previous.0, previous.1, previous.2);
    // Publish through every qualified/imported alias before any fields are lowered.
    for declaration in context.structs.values_mut().filter(|item| item.id == id) {
        declaration.generic_params.clone_from(&generic_params);
    }
    for declaration in context.enums.values_mut().filter(|item| item.id == id) {
        declaration.generic_params.clone_from(&generic_params);
    }
    generic_params
}

pub(crate) fn build_struct_with_id(
    context: &mut CollectContext,
    sd: &ast::StructDecl,
    id: DefId,
    generic_params: Vec<GenericParamDecl>,
) -> HirStruct {
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_param_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = sd
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
        id,
        generic_param_names,
        generic_param_kinds,
        &generic_spans,
    );
    let fields = sd
        .fields
        .iter()
        .enumerate()
        .map(|(index, f)| HirField {
            id: FieldId(index as u32),
            name: f.name.name.clone(),
            ty: context.lower_parse_type(&f.ty),
            public: f.public,
        })
        .collect();
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);

    HirStruct {
        id,
        name: sd.name.name.clone(),
        generic_params,
        fields,
    }
}

pub(crate) fn build_type_alias_with_id(
    context: &mut CollectContext,
    name: &ast::ParseTypeInner,
    params: &[ast::GenericParamDecl],
    target: &ast::ParseType,
    id: DefId,
) -> crate::hir::HirTypeAlias {
    let mut generic_params = crate::type_lowering::lower_generic_param_decls(id, params);
    let generic_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (previous_owner, previous_params, previous_kinds) = context
        .push_generic_context_with_kinds_at(id, generic_names, generic_kinds, &generic_spans);
    crate::type_lowering::lower_param_defaults(context, &mut generic_params, params, None);
    let mut ty = crate::type_lowering::TypeLowerer::lower_parse_type_term(context, target);
    context.pop_generic_context_with_kinds(previous_owner, previous_params, previous_kinds);
    if !generic_params.is_empty() {
        struct AliasBinder<'a> {
            owner: DefId,
            params: &'a [GenericParamDecl],
            depth: u32,
        }
        impl crate::type_services::visit::TypeFolder for AliasBinder<'_> {
            fn enter_binders(&mut self, _params: &[crate::type_services::kind::Kind]) {
                self.depth += 1;
            }

            fn exit_binders(&mut self) {
                self.depth -= 1;
            }

            fn fold_type(&mut self, ty: Type) -> Type {
                match ty {
                    Type::Generic(param) if param.owner == self.owner => Type::BoundVar {
                        depth: self.depth,
                        index: param.index,
                        kind: self.params[param.index as usize].kind.clone(),
                    },
                    other => crate::type_services::visit::fold_type_children(other, self),
                }
            }
        }
        ty = crate::type_services::visit::fold_type(
            ty,
            &mut AliasBinder {
                owner: id,
                params: &generic_params,
                depth: 0,
            },
        );
        ty = Type::Lambda {
            params: generic_params
                .iter()
                .map(|param| param.kind.clone())
                .collect(),
            body: Box::new(ty),
        };
    }
    crate::hir::HirTypeAlias {
        id,
        name: name.name.clone(),
        generic_params,
        ty,
    }
}

pub(crate) fn build_enum_with_id(
    context: &mut CollectContext,
    ed: &ast::EnumDecl,
    id: DefId,
    generic_params: Vec<GenericParamDecl>,
) -> HirEnum {
    let generic_param_names = GenericParamDecl::names(&generic_params)
        .map(str::to_string)
        .collect();
    let generic_param_kinds = generic_params
        .iter()
        .map(|param| param.kind.clone())
        .collect();
    let generic_spans = ed
        .generic_params
        .iter()
        .map(|param| param.span.clone())
        .collect::<Vec<_>>();
    let (prev_owner, prev_params, prev_kinds) = context.push_generic_context_with_kinds_at(
        id,
        generic_param_names,
        generic_param_kinds,
        &generic_spans,
    );
    let mut next_field_id = 0;
    let variants = ed
        .variants
        .iter()
        .enumerate()
        .map(|(variant_index, v)| {
            let fields = match &v.fields {
                ast::NamedFieldsOrTypesList::NamedFields(fields) => HirVariantFields::Named(
                    fields
                        .iter()
                        .map(|f| {
                            let id = FieldId(next_field_id);
                            next_field_id += 1;
                            HirField {
                                id,
                                name: f.name.name.clone(),
                                ty: context.lower_parse_type(&f.ty),
                                public: f.public,
                            }
                        })
                        .collect(),
                ),
                ast::NamedFieldsOrTypesList::TypesList(types) => {
                    if types.is_empty() {
                        HirVariantFields::Unit
                    } else {
                        HirVariantFields::Positional(
                            types.iter().map(|t| context.lower_parse_type(t)).collect(),
                        )
                    }
                }
            };
            HirVariant {
                id: VariantId(variant_index as u32),
                name: v.name.name.clone(),
                fields,
            }
        })
        .collect();
    context.pop_generic_context_with_kinds(prev_owner, prev_params, prev_kinds);

    HirEnum {
        id,
        name: ed.name.name.clone(),
        generic_params,
        variants,
    }
}

pub(crate) fn build_extern_with_id(
    context: &mut CollectContext,
    sig: &ast::FunctionSig,
    name: String,
    id: DefId,
) -> HirExtern {
    let hir_sig = build_function_sig_with_id(context, sig, id);
    let params = hir_sig.params.clone();
    let ret = hir_sig.ret.clone();

    let func_type = Type::function_with_safety(
        params.clone(),
        ret.clone(),
        crate::types::FunctionSafety::from_is_unsafe(sig.is_unsafe),
    );
    context.scope.define(name.clone(), func_type, false);

    HirExtern {
        id,
        name,
        params,
        ret,
        variadic: false,
        is_unsafe: sig.is_unsafe,
    }
}
