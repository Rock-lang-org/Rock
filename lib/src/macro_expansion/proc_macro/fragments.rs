//! Reconstruct one validated response without printing or re-lexing captures.

use crate::ast::MacroInvoc;
use crate::diagnostic::Diagnostics;
use crate::lexer::lossless::{ConstructionContext, ConstructionKind, JoinBoundary, LiteralSourceRange};
use crate::lexer::Span;
use crate::macro_expansion::{ExpansionId, GeneratedSourceId, MacroExpansionContext, TokenStream};

use super::{ProcMacroPart, ProcMacroRequest, ProcMacroResponse};

pub(in crate::macro_expansion) fn materialize(
    request: &ProcMacroRequest, response: &ProcMacroResponse, invocation: &MacroInvoc,
    context: &MacroExpansionContext<'_>, expansion: ExpansionId, generated_source: GeneratedSourceId,
) -> Result<TokenStream, Diagnostics> {
    let fail = |message: String| super::super::proc_macro_error(message, invocation.span.clone());
    response.validate_for(request).map_err(fail)?;
    let input = &invocation.args;
    let limits = super::super::token_tree::MACRO_LIMITS;
    let resolve = |stream: TokenStream| stream.resolve_proc_output(input, invocation.id, expansion, generated_source,
        |construction| context.construction_context(construction, invocation, &request.definition_dependencies)).map_err(fail);
    let mut output = input.slice(0..0, limits).map_err(|error| fail(format!("macro output anchor: {error:?}")))?;
    if let Some(result) = &response.output {
        for part in &result.parts {
            let stream = match part {
                ProcMacroPart::Stream(bytes) => resolve(super::decode_stream(bytes).map_err(|error| fail(error.to_string()))?)?,
                ProcMacroPart::Source { text, context: construction } => {
                    context.charge_matching(text.len(), &invocation.span)?;
                    resolve(TokenStream::source_fragment(text.clone(), *construction, limits)
                        .map_err(|error| fail(format!("source fragment preservation: {error:?}")))?)?
                }
                ProcMacroPart::Literal { view, range } => {
                    let literal = request.views.literals.get(*view as usize).ok_or_else(|| fail("invalid literal view handle".into()))?;
                    let text = literal.decoded.get(range.clone()).ok_or_else(|| fail("invalid literal fragment range".into()))?;
                    context.charge_matching(text.len() + literal.mapping.len(), &invocation.span)?;
                    let mut mappings = Vec::new();
                    for mapping in literal.mapping.iter().filter(|mapping| mapping.decoded.start < range.end && range.start < mapping.decoded.end) {
                        if mapping.decoded.start < range.start || mapping.decoded.end > range.end {
                            return Err(fail("literal fragment cuts a canonical scalar mapping".into()));
                        }
                        // Account for bounded slice walks before performing them.
                        context.charge_matching(input.runs().len() + 1, &invocation.span)?;
                        let source = input.slice(mapping.source.clone(), limits).map_err(|error| fail(format!("literal source range: {error:?}")))?;
                        let start = source.start_span();
                        let end = source.end_span();
                        if start.file_path != end.file_path || end.end < start.start { return Err(fail("noncontiguous literal source mapping".into())); }
                        mappings.push(LiteralSourceRange {
                            decoded: mapping.decoded.start - range.start..mapping.decoded.end - range.start,
                            source: Span::new(start.file_path, start.start, end.end),
                        });
                    }
                    let call_site = context.construction_context(ConstructionContext { invocation: invocation.id, kind: ConstructionKind::CallSite },
                        invocation, &request.definition_dependencies).map_err(fail)?;
                    let stream = TokenStream::mapped_literal(text.into(), mappings, call_site, expansion, generated_source, limits).map_err(fail)?;
                    context.register_literal_source(&stream);
                    stream
                }
            };
            context.charge_matching(output.runs().len() + stream.runs().len(), &invocation.span)?;
            output = output.join(&stream, JoinBoundary::Separate, limits).map_err(|error| fail(format!("macro output splice: {error:?}")))?;
        }
    }
    Ok(output)
}
