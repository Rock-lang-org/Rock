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

The lexical layer does not synthesize indentation tokens, parse expressions or
declarations, resolve names or infer types. Physical layout stays visible as
whitespace/line-ending pieces. C.04's separate type entry point is described below.

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
> rock_syntax::types::TypeSyntax

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
            match TypeSyntax::parse (cursor.input!.clone!)
                Result::Err error => error.message!.println!
                Result::Ok syntax =>
                    nodes = syntax.nodes!
                    node = &nodes[syntax.root!]
                    kind = &node.kind
                    kind.tag!.println!
                    emitted = syntax.emit!
                    match emitted.spelling!
                        Result::Err error => "invalid retained spelling".println!
                        Result::Ok text => text.println!
```

This prints `type`, `1`, `apply`, and the unchanged `Packet(T)` spelling.
A macro uses the same API on its supplied input;
emitting tokens into an invocation still requires the SDK's normal authority.

## C.04 type syntax and unchanged emission

`rock_syntax::types::TypeSyntax::parse input` parses one **complete** type from
SDK tokens, with no name/alias lookup, inference, arity/kind checking or compiler
AST projection. An unknown uppercase name is valid syntax. Supported forms are:

- ASCII uppercase type names and the compiler's existing two-name associated
  paths, such as `T::Item` and `Self::Item`. Import nominal names before using
  them: lowercase module-qualified annotations are not part of the current
  compiler type grammar, and longer associated paths are rejected.
- Type applications, comma-separated sibling arguments, nested parenthesized
  applications, and `_` holes/constructor sections. Application binds more
  tightly than `->`. The compiler's comma ownership is preserved: `(F A, B)` is
  one grouped application, while `(A, B)` is a tuple.
- Shared/mutable references (both existing `&mut` and `&^` spellings), tightly
  prefixed raw pointers, unit, grouping, tuples including a trailing comma,
  slices, and fixed arrays with checked unsigned U64 decimal lengths.
- Function arrows, including flat arrow chains and explicitly grouped function
  arguments/returns. Parentheses are retained as `Grouped` syntax nodes even
  where the compiler's semantic AST discards them.

The standalone entry point treats lexical trivia as trivia; an enclosing
declaration parser owns physical layout and field/default boundaries. It does
not accept a valid prefix and silently discard the rest. Missing names, closers,
array lengths, overflow, unsupported forms, and trailing tokens have structured
errors at logical input ranges, including zero-width EOF locations. Type lambdas,
constant-expression array lengths, and the lexical
layer's non-ASCII-name forms remain explicitly unsupported.

`TypeSyntax` owns the original stream and a flat postorder arena of `TypeNode`
records. Read it through `syntax.nodes!` and `syntax.root!`; each node has a
`TypeKind`, source range, earlier child indices, and an optional original name
context. `ArrayLength` retains a U64 value. Grouping stays explicit while flat
arrow chains are parsed iteratively. Traverse from the root through the earlier
child indices. The arena is immutable
through this API and its destruction is nonrecursive.

`syntax.emit!` copies the **unchanged original** SDK stream, including leading
and trailing trivia, snapshots, contexts and provenance. `syntax.node_tokens index`
returns a checked original node slice; an invalid index diagnoses rather than
panicking. No captured subtree is stringified to emit it. Editing and generated
type/impl builders remain C.08 work, not a fake AST-printer fallback.

`TypeSyntax::parse_with_limits input, lexical_limits, type_limits` additionally
accepts the C.03 `Limits` and `TypeLimits` (default/hard ceiling: 128 recursive
grammar levels and 65,536 stored nodes). Lower limits, including zero, are
checked. Lexical spelling is inspected through `cursor.spelling &token`, which
checks UTF-8/range boundaries without copying the entire SDK snapshot per token.
The lexical byte/piece ceilings apply here; C.03's group-view ceilings belong to
explicit `cursor.groups!` requests. Type nesting and storage use `TypeLimits`.

## C.05 generic binders and constraints

`rock_syntax::types::GenericSyntax` parses a complete declaration binder list,
including an empty list or a trailing comma. It supports plain uppercase names,
parenthesized constructor holes, explicit kinds made of `Type`, grouping and
arrows, and brace-wrapped defaults. These **binder-list fragments** illustrate
the grammar (they are not standalone declarations):

```text
A, {B = A}
(F _), (G _, _), T
(H: (Type -> Type) -> Type)
T, {F _ = Result T, _}
{(F: Type -> Type) = Option}
```

`syntax.params!` returns ordered `GenericParam` records with a name-node index,
optional kind/default roots, an `explicit_kind` flag, and a whole-binder range.
`impl_range` identifies the copied name/constructor section before grouping.
`syntax.nodes!` uses the same flat `TypeNode` arena as `TypeSyntax`. Constructor
holes are application nodes; explicit kinds retain their actual type root.
Names retain original syntax contexts. Default types are syntax only: earlier,
forward, duplicate, unknown and `Self` names are not resolved or validated here.
The compiler owns declaration-scope/default ordering and kind correctness.

`BoundsSyntax::parse input` separately parses a complete `where` clause with
comma-separated subjects and optional `: Bound` types. `syntax.constraints!`
returns ordered `Constraint` records with subject/bound node indices and ranges.
Constructor sections, associated subjects and nested type arguments use C.04's
type grammar and comma ownership. Inline bounds on declaration parameters,
Rust-style `+` bound lists and declaration binders in subject position are not
guessed into supported nodes. Enclosing declaration parsers still own layout;
these entry points operate on an already established token boundary.

Both entry points have `parse_with_limits input, lexical_limits, type_limits`,
`original`, and unchanged `emit` APIs. All their types share the existing lexical
ceilings and a single per-input type-node budget; there is no budget reset per
parameter/default/constraint. Unsupported defaults and trailing tokens diagnose
at logical input ranges instead of being skipped.

For emission, `generic.type_arguments &mut builder` returns copied names or
parenthesized constructor sections, separated by original commas, with defaults
removed. A constructor argument must stay `(F _)`, not bare `F`: the compiler's
implicit impl-generic collection would otherwise declare `F` with kind `Type`.
Rock impl parameters
are implicit, not a declaration-style binder header. `generic.impl_binders &mut
builder` returns **unbounded where subjects**, to be placed after `where` by an
enclosing impl builder. Constructor-hole subjects are parenthesized so their
commas cannot swallow later parameters. Plain parameters and explicit `Type`
parameters emit their names. Other explicit kinds remain parseable and round-trip
unchanged, but these transformed emission helpers return `binder-emission` for
them: the current impl grammar has no matching explicit-kind binder spelling,
and this slice does not guess an arity or erase a higher-order kind. Defaults
never enter an impl
subject or type argument. The caller supplies an ordinary SDK `MacroBuilder`
for new parentheses; these carry output construction requests until normal
compiler materialization. Copied names, holes, inner trivia and provenance are
not stringified. Empty parameters emit an empty stream, not an empty `where`.

General edited-node and complete impl builders remain C.08 work.

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

`lib/tests/fixtures/syntax/types.rs` supplies C.04's shared compiler/library type
fixtures. The native type group compares structural shapes, validates error
locations and supported/unsupported boundaries, and checks unchanged parse/emit/
parse round trips, name contexts, caller limits, opaque-capture rejection and
checked spelling. It remains opt-in:

```sh
cargo test -p rock-lib --lib macro_c_04_
cargo test -p rock-lib --lib macro_acceptance_syntax_types -- --ignored --nocapture
```

`lib/tests/fixtures/syntax/generics.rs` supplies the C.05 binder/constraint
fixtures. The native group compares kind/default/constraint structure against
the Rust parser, checks ranges/round trips/limits/contexts, and tests emitted
impl subjects and type arguments. An additional ordinary compiler/runtime check
uses the actual emitted fragments with earlier-parameter defaults and constructor
defaults (using `Option`); an explicit `Type`-kind impl also typechecks. Partial
constructor-section defaults are covered as syntax, not a claim that the compiler
can infer every such default at runtime. Other explicit-kind
emission diagnoses at the whole-binder range instead of producing invalid impls.
This is not yet
the hygienic full derive-transport proof reserved for C.08–C.11.

```sh
cargo test -p rock-lib --lib macro_c_05_
cargo test -p rock-lib --lib macro_acceptance_syntax_generics -- --ignored --nocapture
```
