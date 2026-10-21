# rock_syntax

An ordinary Rock library for **explicit, bounded Rock syntax views** over
`rock_macro::TokenStream`. It has no worker, RPC, `MacroContext`, compiler AST,
semantic lookup, or special compiler registration. The SDK does not depend on it.
Use the SDK's `StreamCursor` for neutral unit inspection; this library's cursor
is for explicitly requesting the supported Rock interpretation.

## C.03 supported grammar

This is a lexical/group foundation, **not a complete Rock parser**:

- ASCII identifiers/type names, the current Rock keywords, decimal integers and
  decimal fractions, macro/native prefixes, fixed punctuation and symbolic
  operator spellings. Operators are classified lexically; their existence,
  precedence, traits and meanings are not interpreted here.
- Spaces, tabs, CR/LF, line comments, non-nested block comments, and quoted
  string/character boundaries with escaped quotes. Unicode and NUL inside quotes
  and comments are retained. Literal values/escapes are not decoded or validated
  as expressions; these are lexical views, matching the compiler's lexical layer.
- Parenthesis, bracket and brace groups, including macro repetition delimiters.
  Quotes, comments and opaque expression captures do not contribute delimiters.
- Original trivia, logical UTF-8 byte ranges, join boundaries, contexts,
  provenance and opaque capture ranges remain in the original SDK stream.

Non-ASCII names and whitespace are not supported yet: Unicode classification
tables are not currently available in std. Requested interpretation diagnoses
unsupported characters instead of treating them as ASCII names. Other malformed
Rock characters, unterminated quotes/comments and invalid group structure also
diagnose. The neutral SDK stream remains readable and forwardable afterward.

There is no indentation-token synthesis, expression/type/declaration parser,
name resolution, inference or token emission builder in this slice. Physical
layout stays visible as whitespace/line-ending pieces; type syntax starts in C.04.

## Cursor and errors

- `rock_syntax::cursor::Cursor::new input` validates SDK preservation data and builds a bounded UTF-8
  scalar index. It does **not** require valid Rock syntax or balanced groups.
- `Cursor::with_limits input, limits` uses smaller caller-selected limits.
- `cursor.peek!` requests the next lexical piece without consuming it.
- `cursor.consume!` advances by one piece; a failed read leaves its offset intact.
- `cursor.next_token!` consumes trivia until a significant token or EOF. Trivia
  skipped before a subsequent error remains consumed; the failing piece does not.
- `cursor.byte_offset!` is its logical byte position; repeated EOF reads are valid.
- `cursor.input!` borrows the original stream. `cursor.tokens &token` explicitly
  materializes a checked SDK slice, retaining original metadata rather than
  reconstructing text. Binding spellings spanning conflicting contexts diagnose.
  `cursor.slice_range range` provides the same checked slicing for other logical
  ranges; rejected caller ranges never become invalid diagnostic labels.
- `cursor.groups!` explicitly validates the whole remaining input without moving
  the cursor. It returns flat preorder `Group` records with delimiter, opening and
  closing byte ranges, and optional parent index. `cursor.group_inner &group`
  returns the original inner token slice.

`Token` contains a `TokenKind`, a logical `ByteRange`, and an optional binding
`SyntaxContext`; `token.tag!` provides a stable textual classification and
`token.is_trivia!` identifies trivia. Fixed grammar punctuation uses `Symbol`;
inspect its original spelling rather than inferring an operator meaning.

`ParseError` contains a typed `ParseErrorKind`, the offending logical byte range,
and an optional related opening-delimiter range. `error.code!` is a stable error
category, `error.message!` explains it, and `error.diagnostic!` builds an ordinary
SDK diagnostic with input-relative labels, without needing an invocation.
`InvalidInput` retains the underlying `SdkError` for malformed preservation data.
Unresolved construction requests cannot be interpreted as already-materialized
Rock tokens. Parsing never grants new binding authority.

Default/hard ceilings are 1,048,576 logical bytes, 65,536 lexical pieces, 128 group
levels and 16,384 groups. Every configured limit may be lowered, including to
zero; negative or above-ceiling settings diagnose. Group traversal is iterative.
After SDK validation, scalar indexing and lexical traversal are linear in the
supplied input, with constant-time boundary lookahead. Tokens retain ranges;
source is only sliced/copied when the caller explicitly requests a token/body.

## An ordinary in-memory program

An application can depend on `rock_syntax` and `rock_macro` as ordinary packages
and use the shared toolchain std. No proc-macro provider is involved:

```rock
> rock_macro::*
> rock_syntax::cursor::Cursor

main = !->
    text = String::from "Packet(T)"
    length = text.len! as U64
    mut sources: Vec SourceSnapshot = Vec::new!
    sources.push (SourceSnapshot
        identity: 1
        file_path: String::from "example.rk"
        text: text)
    mut segments: Vec Segment = Vec::new!
    segments.push (Segment
        source: 0
        range: ByteRange
            start: 0
            end: length
        kind: SegmentKind::Text
        context: SyntaxContext
            handle: 0
        boundary: JoinBoundary::Separate
        origin: Option::None
        construction: Option::None)
    input = TokenStream
        sources: sources
        segments: segments
        anchor: StreamAnchor
            source: 0
            offset: 0
            context: SyntaxContext
                handle: 0
            origin: Option::None
        expressions: Vec::new!
    match Cursor::new input
        Result::Err error => error.message!.println!
        Result::Ok cursor =>
            match cursor.peek!
                Result::Err error => error.message!.println!
                Result::Ok token =>
                    match token
                        Option::Some value => value.tag!.println!
                        Option::None => "empty".println!
            match cursor.groups!
                Result::Err error => error.message!.println!
                Result::Ok groups => groups.len!.println!
```

This prints `type` and `1`. A macro uses the same API on its supplied input;
emitting tokens into an invocation still requires the SDK's normal authority.

## Conformance

`lib/tests/fixtures/syntax/lexical.rs` is the shared positive/negative fixture set
for the Rust lexical/group interpretation and this library. The native test
compiles an ordinary Rock client, with proc execution disabled, and compares
piece classifications/ranges, binding-context presence, group ranges/parents,
and error categories/ranges against the Rust implementation. Supplementary
controls cover joins, conflicting contexts, provenance-preserving slices,
opaque/nested captures, limits, rollback and neutral inspection after errors.

Contributor commands (native acceptance is intentionally opt-in, with no CI job):

```sh
cargo test -p rock-lib --lib macro_c_03_
cargo test -p rock-lib --lib macro_acceptance_syntax_cursor_and_groups -- --ignored --nocapture
```

The broader manual `cargo macro-acceptance` runner includes the native proof.
