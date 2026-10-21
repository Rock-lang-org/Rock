use crate::{
    ast::{LanguageItemMarker, LanguageItemMemberKind, LanguageItemMemberMarker},
    language_items::LanguageItemRole,
    lexer::TokenType,
    parser::{engine::*, EnumDecl, EnumVariant, NamedFieldsOrTypesList, ParseTypeInner},
};

use super::{
    empty_lines_permissive, indent, language_item_marker, parse_generic_param_list,
    parse_type_name, parse_type_inner, struct_decl_field,
};

pub fn enum_decl(stream: Input) -> IResult<EnumDecl> {
    let start = stream.clone();
    let (rest, mut declaration) = (
        TokenType::Keyword("enum".to_string()),
        parse_type_name,
        parse_generic_param_list,
        TokenType::Eol.followed_by(empty_lines_permissive),
        indented(many(enum_item)),
    )
        .map(|(_, name, generic_params, _, items)| {
            let mut language_items = crate::ast::LanguageItemAnnotations::default();
            let variants = items
                .into_iter()
                .map(|(variant, marker)| {
                    if let Some(marker) = marker {
                        language_items.members.push(LanguageItemMemberMarker {
                            marker,
                            kind: LanguageItemMemberKind::Variant,
                            member_name: variant.name.name.clone(),
                        });
                    }
                    variant
                })
                .collect();

            EnumDecl {
                original: None,
                name: ParseTypeInner { name: name.name, context: name.context, span: name.span, generics: Vec::new() },
                generic_params,
                variants,
                exported: false,
                language_items,
            }
        })
        .process(stream)?;
    declaration.original = start.declaration_until(rest.clone(), start.header_indent()?)?;
    Ok((rest, declaration))
}

pub fn enum_variant(stream: Input) -> IResult<EnumVariant> {
    preceded(indent, enum_variant_without_indent).process(stream)
}

fn enum_item(stream: Input) -> IResult<(EnumVariant, Option<LanguageItemMarker>)> {
    let (stream, _) = indent(stream)?;
    let (stream, marker) = if matches!(stream.seek()?.token_type, TokenType::Ident(ref name) if name == "lang")
    {
        let (stream, marker) = language_item_marker(stream)?;
        (stream, Some(marker))
    } else {
        (stream, None)
    };
    if let Some(marker) = &marker {
        if marker.role == LanguageItemRole::IndexMut {
            return Err(ParseError::HardError(
                "language item role 'index_mut' may only mark a trait root".to_string(),
                marker.span.clone(),
            ));
        }
    }
    let stream = if marker.is_some() {
        let (stream, _) = empty_lines_permissive(stream)?;
        let (stream, _) = indent(stream).map_err(|_| {
            ParseError::HardError(
                "language item marker must be followed by a member at the same indentation"
                    .to_string(),
                marker.as_ref().unwrap().span.clone(),
            )
        })?;
        if matches!(stream.seek()?.token_type, TokenType::Ident(ref name) if name == "lang") {
            return Err(ParseError::HardError(
                "language item marker must apply to exactly one member".to_string(),
                stream.seek()?.span,
            ));
        }
        stream
    } else {
        stream
    };

    match enum_variant_without_indent.process(stream) {
        Ok((stream, variant)) => Ok((stream, (variant, marker))),
        Err(_) if marker.is_some() => Err(ParseError::HardError(
            "language item marker must be followed by an enum variant".to_string(),
            marker
                .as_ref()
                .expect("guarded language item marker")
                .span
                .clone(),
        )),
        Err(error) => Err(error),
    }
}

fn enum_variant_without_indent(stream: Input) -> IResult<EnumVariant> {
    (
        parse_type_inner,
        TokenType::Eol.followed_by(empty_lines_permissive).opt(),
        indented(named_fields_or_types_list).opt(),
    )
        .map(|(name, _, fields_opt)| {
            if !name.generics.is_empty() {
                EnumVariant {
        name: ParseTypeInner {
            context: name.context,
                        span: name.span,
                        name: name.name,
                        generics: Vec::new(),
                    },
                    fields: NamedFieldsOrTypesList::TypesList(name.generics),
                }
            } else if let Some(fields) = fields_opt {
                EnumVariant { name, fields }
            } else {
                EnumVariant {
                    name,
                    fields: NamedFieldsOrTypesList::NamedFields(Vec::new()),
                }
            }
        })
        .process(stream)
}

pub fn named_fields_or_types_list(stream: Input) -> IResult<NamedFieldsOrTypesList> {
    many(struct_decl_field)
        .map(NamedFieldsOrTypesList::NamedFields)
        .process(stream)
}
