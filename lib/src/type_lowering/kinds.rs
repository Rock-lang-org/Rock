use crate::ast;
use crate::type_services::kind::Kind;

pub(crate) fn constructor_kind(params: &[crate::types::GenericParamDecl]) -> Kind {
    params.iter().rev().fold(Kind::Type, |output, param| {
        Kind::arrow(param.kind.clone(), output)
    })
}

pub(crate) fn constructor_kind_from_arity(arity: usize) -> Kind {
    (0..arity).fold(Kind::Type, |output, _| Kind::arrow(Kind::Type, output))
}

pub(crate) fn lower_generic_param_kind(kind: Option<&ast::TypeApplication>) -> Kind {
    let Some(kind) = kind else {
        return Kind::Type;
    };
    if !kind.args.is_empty() {
        return constructor_kind_from_arity(kind.args.len());
    }
    lower_explicit_kind_syntax(&kind.constructor).unwrap_or(Kind::Type)
}

fn lower_explicit_kind_syntax(kind: &ast::ParseType) -> Option<Kind> {
    match kind {
        ast::ParseType::Type(inner) if inner.name == "Type" && inner.generics.is_empty() => {
            Some(Kind::Type)
        }
        ast::ParseType::Type(inner) if inner.name == "usize" && inner.generics.is_empty() => {
            Some(Kind::Usize)
        }
        ast::ParseType::Function(types) if types.len() >= 2 => {
            let mut types = types.iter().rev();
            let mut output = lower_explicit_kind_syntax(types.next()?)?;
            for input in types {
                output = Kind::arrow(lower_explicit_kind_syntax(input)?, output);
            }
            Some(output)
        }
        _ => None,
    }
}

pub(crate) fn lower_associated_type_kind(kind: Option<&ast::TypeApplication>) -> Kind {
    lower_generic_param_kind(kind)
}
