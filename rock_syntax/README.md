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
- `cursor.all_groups!` performs the same checked scan from the start of the
  original input, also without changing the current cursor position.

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
        offset: 0
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
panicking. No captured subtree is stringified to emit it. Generated type
construction and range-based edits are described in C.08 below; unchanged
emission never falls back to an AST printer.

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

General edited-node and impl construction use the C.08 output helpers below.

## C.06 struct derive input

`rock_syntax::derive_input::DeriveInput::parse input` parses one complete struct
or enum declaration, independently of any compiler/worker. This section covers
structs; enum shapes are described below. It supports ordinary/private
and `<` exported declarations, C.05 generic parameters, ordered named fields,
field visibility, C.04 types and bounded opaque defaults. For example, this is a
complete library declaration using its own generic parameters:

```rock
struct Packet T, {U = T}
    < value: T
    fallback: U
    attempts: I64 = 3
```

The initial layout subset uses LF/CRLF, space-indented declaration/field margins
with even widths up to 254 bytes, and a consistent field margin deeper than the
declaration margin.
Original nested-module margins are retained, not stripped. Blank/comment-only
lines are preserved. Headers and field type annotations occupy one physical
line; multiline annotation continuations, indentation tabs, code after a leading
inline block comment in a header/field margin, and unsupported declaration kinds
diagnose instead of being
guessed into this subset. A second declaration or an unconsumed malformed field
is an error, not a successful prefix parse. Struct `where` headers are not added:
they are not part of the compiler's struct grammar.

`syntax.decl_kind!`, `decl_name`, `is_public`, `params`, `fields` and `nodes` expose the
parsed data. All generic/type indices refer to one flat arena, and all ranges
are logical byte ranges in the original declaration input. `Field` contains an
original name token, a visibility flag, a type-node index, an optional
`default_range`, a significant-syntax `range` and a complete physical
`layout_range`. `syntax.header!` and the ordered field layout ranges partition
every original input byte, including leading/trailing comments and blank lines.
Comments before a field stay in the preceding layout slice (or the header).

A default starts with a token on the same line as `=` and may have indented
continuation/body lines. Delimiter groups, quoted/comment text and SDK opaque
expression captures cannot create false sibling fields. Bodies are retained as
original ranges, not decoded, formatted or converted to expression ASTs. This is
**boundary detection, not expression validation**: ordinary compiler parsing and
typing still decide whether a preserved body is a valid expression. Newline-only
operands after `=`, non-default type continuations and ambiguous margins diagnose.
The C.03 lexical subset still applies except inside SDK opaque captures.

`syntax.emit!` clones the unchanged original stream. `syntax.field_tokens index`
returns a checked significant field slice; an invalid index diagnoses. Materialize
other parts through the SDK's checked `syntax.original!.slice range` API.
Snapshots, capture metadata, binding contexts and provenance are never rebuilt
from spelling. Parsing stores ranges rather than a whole snapshot per field.

`DeriveInput::parse_with_limits input, lexical_limits, type_limits, declaration_limits`
uses the existing lexical/group ceilings, one shared type-node budget for the
entire declaration, and `DeclarationLimits::standard!` (16,384 fields, also the
hard ceiling). Lower/zero limits and invalid configurations are checked. The
shared grammar engine is a private library module, not a public compiler API.
Edited declaration/impl construction uses the C.08 helpers below.

## C.07 enum derive input

The same `DeriveInput` entry point supports enum headers, generic parameters,
visibility and ordered nullary, named or positional variants. This is a complete
library declaration with each generic parameter declared in its header:

```rock
enum Signal T, {U = T}
    Empty
    Pair T, U
    Named
        < value: U
        retries: I64 = 3
```

`syntax.decl_kind!` distinguishes `Struct` from `Enum`. For enums,
`syntax.variants!` returns ordered `Variant` records, while `syntax.fields!`
remains the struct-field list (empty on enums). Each variant has an original
name token, `VariantShape` (`Nullary`, `Named`, `Positional`), `named_fields`,
positional `types` indices, and significant, physical-layout and header ranges.
All generic, positional and named-field type indices use the declaration's one
shared arena. A nullary variant has no fields; a variant with `()` has one
positional unit field, not nullary shape.

Positional variants mirror the compiler's declaration-argument grammar, not a
new whitespace-separated list. An ungrouped argument uses the complete type
parser, including its comma ownership: `One Result T, E` has one application
field. `Pair (Result T, E), U` has two fields. Adjacent `(T) U` are two fields,
whereas ungrouped `T U` is one application. Trailing positional commas are
retained. Explicit declaration-kind binders are not type arguments; type lambdas
remain outside C.04's supported type subset.

Named fields reuse the struct parser, including field visibility and opaque
defaults. Their margin is one further declaration indentation step beyond the
variant margin. Layout/default limitations from C.06 still apply. Variant
visibility markers, discriminants, compiler-internal markers and mixed inline
positional/named bodies are unsupported. Mixed bodies diagnose instead of
silently discarding one field family; no variant shape is guessed from malformed
syntax. A following declaration or malformed sibling is not ignored.

The declaration header plus variant layout ranges partition the original input;
each named variant's header plus named-field layout ranges also partition its
layout. `syntax.variant_tokens index` returns a checked original significant
variant slice and diagnoses invalid indices. Use checked SDK slicing for nested
field/type ranges. Unchanged `emit` preserves original snapshots, contexts,
provenance, trivia and opaque captures without an AST-print/re-lex pass.

`DeclarationLimits` now includes `max_variants` (16,384 standard/hard ceiling).
`max_fields` is a **single total** across all named and positional variant
fields; it is not reset per variant. The common lexical/group and type-node
limits also cover the whole enum. Empty enums and lower/zero limits are handled
explicitly; no resolution, kind/arity inference or expression validation is added.

## C.08 token edits and output construction

`rock_syntax::emission` builds `MacroOutput` through the SDK's ordinary
`syntax`, `forward`, and `quote` helpers. No quotation proc macro, worker callback,
compiler AST, or eager materialization is needed. Unchanged syntax remains
`OutputPart::Stream`, not generated text. Generated identifiers still come from
the SDK builder: forward an original name, request a fresh helper, or explicitly
request call-site/dependency authority. Copying a spelling does not grant that
authority.

`TokenEdits::new original` creates an ordered edit script. `replace context,
range, output` replaces a range in the **original** stream, including zero-width
insertion or deletion with empty output. Use ranges from type nodes, binders,
constraints, fields, variants, or declaration tokens. Replacements must be
ordered and disjoint; invalid ranges, UTF-8 cuts and partial opaque-expression
cuts diagnose. A failed replacement leaves the script unchanged. Consuming
`emit context` forwards the remaining suffix. With no edits, the output contains
one unchanged stream, including its anchor, snapshots, contexts and captures.
No edit reparses or reformats untouched text. Authors choose the supported node
range to replace; this is not a visitor/folder or a mutable compiler AST.

`TypeEmitter::emit context, kind, children` constructs the C.04 `TypeKind` forms
from output children. Arity is checked: names/grouping/references/pointers/slices
take one child; paths/arrays take two; applications/functions need at least two;
tuples need at least one; unit, holes and array-length values take none. Arrays'
second child is a length fragment; `ArrayLength U64` emits the full unsigned
decimal range without signed narrowing. Application arguments, function operands
and unary pointees are grouped; singleton tuples keep their comma. Paths expect
name/path children, not arbitrary expressions. Children may retain parsed nodes
or contain unresolved SDK construction requests; final Rock validity remains
the compiler's responsibility. Captured nodes are never printed and re-lexed.

This complete ordinary helper changes the first struct field to a shared
reference while preserving the declaration's other bytes and token authority:

```rock
> rock_macro::*
> rock_syntax::derive_input::DeriveInput
> rock_syntax::emission::TokenEdits
> rock_syntax::emission::TypeEmitter
> rock_syntax::types::TypeKind

reference_first_field: &MacroContext -> DeriveInput -> Result MacroOutput, Diagnostic
reference_first_field = context, syntax ->
    fields = syntax.fields!
    if fields.len! == 0
        return context.output_error "expected a struct with a field"
    nodes = syntax.nodes!
    bounds = nodes[fields[0].ty].range.clone!
    tokens = match syntax.original!.slice (bounds.clone!)
        Result::Ok tokens => tokens
        Result::Err _ => return context.output_error "invalid field type slice"
    children = [context.forward tokens ?]
    replacement = TypeEmitter::emit context, TypeKind::SharedReference, &children ?
    mut edits = TokenEdits::new syntax.emit! ?
    edits.replace context, bounds, replacement ?
    edits.emit context
```

`ImplBuilder::new target, trait_type` accepts output for the target and an optional
trait (`None` for inherent impls). `constrain context, subject, optional_bound`
adds a grouped WHERE subject and optional grouped trait bound, avoiding comma
capture by adjacent applications. For generic declarations, use C.05's
`GenericSyntax.type_arguments` for target arguments: declaration defaults are not
impl syntax, and constructor sections must retain their holes/grouping. Existing
unsupported explicit constructor-kind emission still diagnoses; this helper
does not guess a conversion.

`member context, name, signature, definition` adds a member with a signature and
a complete opaque definition such as `-> 7` or `other -> expression`. The name
can include a receiver marker (for example generated `@show`). `emit context`
generates the impl header, WHERE clauses, and four-space outer member layout;
an empty impl diagnoses. Definitions retain supplied internal layout: multiline
bodies need valid continuation indentation at their emitted location, or SDK
opaque expression fragments. This is not an expression parser/reindenter.

`OutputBuffer::new!`, `append context, output`, and consuming `output!` support
larger custom emitters. Appends move owned parts without repeatedly deep-copying
the accumulated prefix and preflight the complete append before mutation.
All helpers bound aggregate output at 1 MiB spelling, 16,384 parts and a
conservative 16 MiB retained-storage charge (including full copied snapshots).
Arity, edit and output failures use structured SDK diagnostics; ordinary syntax
parse failures still use `ParseError.diagnostic!`.

`LayoutEmitter::unindent context, input, margin` translates physical leading
space margins when forwarding a block-captured declaration into item output.
Use the SDK's `block_base` fact for a complete block input, then parse the
returned stream to obtain its new logical ranges before applying node edits.
This helper consumes ordinary lexical pieces: it never scans through multiline
quotes, block comments or opaque captures to remove their interior whitespace.
It joins original SDK slices, preserving contexts, provenance and capture
metadata, without generating or printing captured names. A zero margin returns
the exact stream. Margins above 254, non-space prefixes and excessive retained
copy work diagnose; this bounded layout operation is not a formatter.

The native C.08 group checks every supported type constructor against the actual
Rust type parser, plus changed types/declarations, unchanged contexts/provenance,
UTF-8/capture/range errors, output limits and failed-append atomicity. Its separate
function-like proc provider emits inherent, trait and generic impls through the
real engine, with captured target/field/parameter names and same-spelling helpers
in caller and nested scopes. This is emission acceptance, not derive dispatch.

```sh
cargo test -p rock-lib --lib macro_c_08_
cargo test -p rock-lib --lib macro_acceptance_syntax_emission -- --ignored --nocapture
```

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
defaults (using the partial section `Result T, _`); an explicit `Type`-kind impl
also typechecks. Macro-free compiler regressions cover inferred construction and
explicit constructor overrides. Other explicit-kind
emission diagnoses at the whole-binder range instead of producing invalid impls.
This spelling-only C.05 check does not establish hygiene; C.08 tests function-like
impl transport, while C.09–C.11 cover derive dispatch and its proofs.

```sh
cargo test -p rock-lib --lib macro_c_05_
cargo test -p rock-lib --lib macro_acceptance_syntax_generics -- --ignored --nocapture
```

`lib/tests/fixtures/syntax/structs.rs` supplies the C.06 struct fixtures. The
native client compares the actual compiler's field/parameter structure and
visibility, checks exact name/type/default ranges and source spelling, and
reconstructs the complete input from its header and field layout slices.
Supplementary controls cover shared budgets, group limits, captures, independent
name contexts, invalid SDK data and checked field access. The cursor program and
struct declaration example are compiled by the native syntax groups. No declaration structure is
requested from the compiler, and expression bodies stay opaque in the library.

```sh
cargo test -p rock-lib --lib macro_c_06_
cargo test -p rock-lib --lib macro_acceptance_syntax_structs -- --ignored --nocapture
```

`lib/tests/fixtures/syntax/enums.rs` shares C.07 enum fixtures with the actual
Rust parser. Native acceptance compares variant/field shape and order, nested
type structure, visibility and exact name/type/default spelling/ranges. It also
checks whole-input and nested layout reconstruction, unchanged round trips,
common resource budgets, independent name contexts and checked variant slicing.
The complete enum example above is compiled in this ordinary in-memory proof.

```sh
cargo test -p rock-lib --lib macro_c_07_
cargo test -p rock-lib --lib macro_acceptance_syntax_enums -- --ignored --nocapture
```
