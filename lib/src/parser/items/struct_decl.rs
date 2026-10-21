use crate::parser::{engine::*, StructDecl, StructDeclField, TokenType};

use super::{
    empty_lines, expression, ident, indent, parse_generic_param_list, parse_type, parse_type_name,
};

pub fn struct_decl(stream: Input) -> IResult<StructDecl> {
    let start = stream.clone();
    let (rest, mut declaration) = (
        TokenType::Keyword("struct".to_string()),
        parse_type_name,
        parse_generic_param_list,
        TokenType::Eol.followed_by(empty_lines),
        indented(many(struct_decl_field.followed_by(empty_lines))),
    )
        .map(|(_, name, generic_params, _, fields)| StructDecl {
            original: None,
            name,
            generic_params,
            fields,
            exported: false,
        })
        .process(stream)?;
    declaration.original = start.declaration_until(rest.clone(), start.header_indent()?)?;
    Ok((rest, declaration))
}

pub fn struct_decl_field(stream: Input) -> IResult<StructDeclField> {
    (
        indent,
        TokenType::Operator("<".to_string()).opt(),
        ident,
        TokenType::Colon,
        parse_type,
        (TokenType::Equal, expression).opt(),
        TokenType::Eol,
    )
        .map(|(_, public, name, _, ty, expr_opt, _)| StructDeclField {
            name,
            ty,
            public: public.is_some(),
            default: expr_opt.map(|(_, expr)| expr),
        })
        .process(stream)
}
