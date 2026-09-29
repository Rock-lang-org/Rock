use std::str::FromStr;

use crate::{
    ast::LanguageItemMarker,
    language_items::LanguageItemRole,
    lexer::TokenType,
    parser::{
        engine::*,
        items::{primitives::indent, utils::empty_lines},
        ModuleDecl, TopLevel,
    },
};

use super::{
    enum_decl, function_decl, function_sig, macro_decl, macro_invoc, module, operator, parse_type,
    parse_type_inner, path, primitives, r#impl, r#trait, struct_decl,
};

fn path_segments(path: &crate::ast::Path) -> Vec<String> {
    let segments = match path {
        crate::ast::Path::Ident(path) => &path.path,
        crate::ast::Path::Type(path) => &path.path,
    };

    segments
        .iter()
        .filter_map(|segment| match segment {
            crate::ast::IdentOrType::Ident(ident) => Some(ident.name.clone()),
            crate::ast::IdentOrType::Type(crate::ast::ParseType::Type(inner)) => {
                Some(inner.name.clone())
            }
            crate::ast::IdentOrType::Type(crate::ast::ParseType::Application(application)) => {
                path_segments_from_type(&application.constructor)
            }
            _ => None,
        })
        .collect()
}

fn path_segments_from_type(ty: &crate::ast::ParseType) -> Option<String> {
    match ty {
        crate::ast::ParseType::Type(inner) => Some(inner.name.clone()),
        crate::ast::ParseType::Application(application) => {
            path_segments_from_type(&application.constructor)
        }
        _ => None,
    }
}

fn import_top_level(path: crate::ast::Path) -> TopLevel {
    let segments = path_segments(&path);
    if segments
        .last()
        .map(|segment| segment == "*")
        .unwrap_or(false)
    {
        TopLevel::GlobImport(segments[..segments.len() - 1].to_vec())
    } else {
        TopLevel::Import(path)
    }
}

fn export_top_level(path: crate::ast::Path) -> TopLevel {
    let segments = path_segments(&path);
    if segments
        .last()
        .map(|segment| segment == "*")
        .unwrap_or(false)
    {
        TopLevel::GlobExport(segments[..segments.len() - 1].to_vec())
    } else {
        TopLevel::Export(path)
    }
}

pub(crate) fn language_item_marker(stream: Input) -> IResult<LanguageItemMarker> {
    let (stream, lang_token) = stream.consume()?;
    if !matches!(lang_token.token_type, TokenType::Ident(ref name) if name == "lang") {
        return Err(ParseError::UnexpectedToken(
            "language item marker 'lang'".to_string(),
            lang_token,
        ));
    }

    let (stream, role_token) = stream.consume()?;
    let role_name = match &role_token.token_type {
        TokenType::Ident(name) | TokenType::Keyword(name) => name,
        TokenType::Eof => return Err(ParseError::UnexpectedEOF(role_token.span.clone())),
        _ => {
            return Err(ParseError::HardError(
                "expected a language item role".to_string(),
                role_token.span,
            ));
        }
    };
    let role = LanguageItemRole::from_str(role_name)
        .map_err(|message| ParseError::HardError(message, role_token.span.clone()))?;

    let (stream, eol) = stream.consume()?;
    match eol.token_type {
        TokenType::Eol => Ok((
            stream,
            LanguageItemMarker {
                role,
                span: lang_token.span,
            },
        )),
        TokenType::Eof => Err(ParseError::UnexpectedEOF(eol.span)),
        _ => Err(ParseError::HardError(
            "expected a newline after a language item marker".to_string(),
            eol.span,
        )),
    }
}

fn language_item_top_level(stream: Input) -> IResult<TopLevel> {
    let (stream, marker) = language_item_marker(stream)?;
    let (stream, _) = empty_lines(stream)?;
    if matches!(stream.seek()?.token_type, TokenType::Eof) {
        return Err(ParseError::UnexpectedEOF(stream.seek()?.span));
    }
    let (stream, _) = indent(stream)?;

    if matches!(
        marker.role,
        LanguageItemRole::Method
            | LanguageItemRole::Output
            | LanguageItemRole::Residual
            | LanguageItemRole::Branch
            | LanguageItemRole::Break
            | LanguageItemRole::Continue
            | LanguageItemRole::RangeFull
            | LanguageItemRole::RangeFrom
            | LanguageItemRole::RangeTo
            | LanguageItemRole::RangeToInclusive
            | LanguageItemRole::RangeExclusive
            | LanguageItemRole::RangeInclusive
    ) {
        return Err(ParseError::HardError(
            format!(
                "language item role '{}' may only mark a language item member",
                marker.role
            ),
            marker.span,
        ));
    }

    let (stream, exported) = match stream.seek()? {
        token if matches!(token.token_type, TokenType::Operator(ref operator) if operator == "<") =>
        {
            let (stream, _) = stream.consume()?;
            (stream, true)
        }
        _ => (stream, false),
    };

    let target = stream.seek()?;
    match target.token_type {
        TokenType::Keyword(ref keyword) if keyword == "trait" => {
            if !matches!(
                marker.role,
                LanguageItemRole::Sized
                    | LanguageItemRole::Drop
                    | LanguageItemRole::Fold
                    | LanguageItemRole::Index
                    | LanguageItemRole::IndexMut
                    | LanguageItemRole::FnOnce
                    | LanguageItemRole::FnMut
                    | LanguageItemRole::Fn
                    | LanguageItemRole::Send
                    | LanguageItemRole::Sync
                    | LanguageItemRole::Try
                    | LanguageItemRole::FromResidual
            ) {
                return Err(ParseError::HardError(
                    format!("language item role '{}' may only mark an enum", marker.role),
                    marker.span,
                ));
            }

            let (stream, mut decl) = r#trait(stream)?;
            decl.exported = exported;
            decl.language_items.root = Some(marker);
            Ok((stream, TopLevel::TraitDecl(decl)))
        }
        TokenType::Keyword(ref keyword) if keyword == "enum" => {
            if !matches!(
                marker.role,
                LanguageItemRole::ControlFlow | LanguageItemRole::Range
            ) {
                return Err(ParseError::HardError(
                    format!("language item role '{}' may only mark a trait", marker.role),
                    marker.span,
                ));
            }

            let (stream, mut decl) = enum_decl(stream)?;
            decl.exported = exported;
            decl.language_items.root = Some(marker);
            Ok((stream, TopLevel::EnumDecl(decl)))
        }
        TokenType::Eof => Err(ParseError::UnexpectedEOF(target.span)),
        _ => Err(ParseError::HardError(
            "language item marker must apply to a trait or enum declaration".to_string(),
            target.span,
        )),
    }
}

pub fn top_level(stream: Input) -> IResult<TopLevel> {
    (
        empty_lines,
        indent,
        language_item_top_level.or(function_decl
            .map(TopLevel::FunctionDecl)
            .or(struct_decl.map(TopLevel::StructDecl))
            .or(macro_decl.map(TopLevel::MacroDecl))
            .or(macro_invoc.map(TopLevel::MacroInvoc))
            .or(enum_decl.map(TopLevel::EnumDecl))
            .or(r#trait.map(TopLevel::TraitDecl))
            .or(r#impl.map(TopLevel::Impl))
            .or(infix_operator_decl
                .map(|(precedence, name)| TopLevel::InfixOperator(precedence, name)))
            .or(
                preceded(TokenType::Keyword("extern".to_string()), function_sig)
                    .map(TopLevel::Extern),
            )
            .or(preceded(
                TokenType::Keyword("mod".to_string()),
                followed(primitives::ident_token, TokenType::Eol),
            )
            .map(|ident| TopLevel::Mod(ident, false)))
            .or(preceded(
                TokenType::Operator("<".to_string()),
                preceded(
                    TokenType::Keyword("mod".to_string()),
                    followed(primitives::ident_token, TokenType::Eol),
                ),
            )
            .map(|ident| TopLevel::Mod(ident, true)))
            .or(module.map(ModuleDecl).map(TopLevel::Module))
            .or((
                TokenType::Keyword("type".to_string()),
                parse_type_inner,
                TokenType::Equal,
                parse_type,
                TokenType::Eol,
            )
                .map(|(_, name, _, ty, _)| TopLevel::NewType(name, ty)))
            .or(preceded(
                TokenType::Operator(">".to_string()),
                followed(path, TokenType::Eol),
            )
            .map(import_top_level))
            // Inline exported declarations: < name = ..., < struct ..., < trait ..., etc.
            // These must be tried before the fallback < path EOL (re-exports).
            .or(preceded(
                TokenType::Operator("<".to_string()),
                function_decl.map(|mut fd| {
                    fd.exported = true;
                    fd
                }),
            )
            .map(TopLevel::FunctionDecl))
            .or(preceded(
                TokenType::Operator("<".to_string()),
                struct_decl.map(|mut sd| {
                    sd.exported = true;
                    sd
                }),
            )
            .map(TopLevel::StructDecl))
            .or(preceded(
                TokenType::Operator("<".to_string()),
                enum_decl.map(|mut ed| {
                    ed.exported = true;
                    ed
                }),
            )
            .map(TopLevel::EnumDecl))
            .or(preceded(
                TokenType::Operator("<".to_string()),
                r#trait.map(|mut td| {
                    td.exported = true;
                    td
                }),
            )
            .map(TopLevel::TraitDecl))
            .or(preceded(
                TokenType::Operator("<".to_string()),
                preceded(
                    TokenType::Keyword("extern".to_string()),
                    function_sig.map(|mut sig| {
                        sig.exported = true;
                        sig
                    }),
                ),
            )
            .map(TopLevel::Extern))
            // Re-exports and legacy standalone exports: < some::path or < single_name
            .or(preceded(
                TokenType::Operator("<".to_string()),
                followed(path, TokenType::Eol),
            )
            .map(export_top_level))
            .or(macro_invoc.map(TopLevel::MacroInvoc))
            .or(function_sig.map(TopLevel::FunctionSig))),
        empty_lines,
    )
        .map(|(_, _, top_level, _)| top_level)
        .process(stream)
        .map_err(|e| e.with_context("top-level declaration"))
}

pub fn infix_operator_decl(stream: Input) -> IResult<(u8, String)> {
    (
        TokenType::Keyword("infix".to_string()),
        primitives::int.assert(|precedence| *precedence <= 255),
        operator,
        TokenType::Eol,
    )
        .map(|(_, precedence, name, _)| (precedence as u8, name.value))
        .process(stream)
}
