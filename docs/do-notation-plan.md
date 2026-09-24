# Proposal: `do` Notation for Rock

Status: parser-level expansion and formatter support implemented; validation is recorded below.

## Implementation Notes

`lib/src/parser/items/do_expr.rs` expands the block immediately into ordinary `bind` calls and lambdas. A single wildcard arm of `match true` provides lexical scope, including before the first bind, without introducing another closure. `Match::do_syntax` retains the original statements for formatting only; it is skipped by serialization and semantic visitors operate on the expanded arms. No new language-item protocol, HIR operation, or artifact format is introduced.

Runtime validation covers `Option`, `Result`, nested blocks, short-circuiting, ownership and drops, custom repeated-callback bindings, no-stdlib use, macro expansion, and exported generic artifact bodies. Parser tests cover invalid control flow and formatting round trips. `cargo test -p rock-lib` passes with 2,336 unit tests, 686 integration tests, and one parser integration test; one existing doc-test is ignored. `cargo clippy --workspace --all-targets` completes with warnings. The release compiler has been rebuilt for book validation.

`node docs/checks/compile-examples.cjs` passes with 268 compiled examples, four expected rejections, and no unexpected failures. All eight I/O chapter programs were also linked and run with their expected output and file contents; four file-error scenarios returned exit code `1` without printing success output. `cargo fmt --all` and `git diff --check` complete successfully.

The explicit expansions exposed two existing MIR issues, reproduced without `do`: transitive closure captures borrowed a reference slot instead of its pointee, and block-local shadowing did not restore the outer variable mapping. The accompanying MIR fixes address those ordinary-code regressions rather than adding `do` semantics.

The external tree-sitter grammar now supports `do` expressions, contextual bind arrows, highlighting, lexical scopes, and indentation. The book pins published revision `bce41184c3a5aa9ef3f7ba77d3775637baa38c28`. The grammar's 38 corpus tests and native build pass; all 12 book highlighting tests pass, including nested bindings and restoration of outer parameter colors. The optional compiler-conformance check reports 108 passed, 12 excluded, and three failures also reproduced at the previous grammar pin: `test_projects/expression-problem/src/main.rk`, `test_projects/new_new2/main.rk`, and `test_projects/new_new/main.rk`.

The pinned commit was fetched successfully from GitHub. The book builds with mdBook 0.5.4, and its verifier passes for 37 chapters, 266 Rock fences, and 41 HTML pages, including the rendered `do` keyword and bind arrows. The full example compilation check still reports 268 compiled examples, four expected rejections, and no unexpected failures. A remaining CLI 0.26.9 initializer-color distinction is documented in `docs/checks/README.md`.

## Goal

Provide flat, expression-oriented monadic sequencing by expanding `do` into existing AST calls, closures, and assignments. Preserve the behavior, ownership rules, and static dispatch of ordinary bind calls. The semantic compiler pipeline receives ordinary syntax, not a new monadic operation.

Rock already defines `Monad::bind`, the standalone `bind`, `.bind`, and `>>=` in `stdlib/monad.rk`, and `Applicative::pure` plus standalone `pure` in `stdlib/applicative.rk`. Both traits and functions are exported by the explicitly supplied stdlib prelude. `Option` and `Result _, E` already implement these traits.

## Proposed User Experience

This complete proposed program performs two dependent file operations without nested callbacks:

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write

write_greeting: &Str -> Result I64, IoError
write_greeting = path ->
    file = File::create path?
    file.write_str "Hello from Rock\n"

append_greeting: &Str -> Result I64, IoError
append_greeting = path ->
    file = File::append path?
    file.write_str "Again\n"

greet: &Str -> Result I64, IoError
greet = path -> do
    count <- write_greeting path
    appended <- append_greeting path
    total = count + appended
    pure total

main = ->
    status = greet "rock-greeting.txt" <&> total ->
        total.println!
        0
    status.unwrap_or 1
```

The output is `22`, with exit status `0`; either write error produces exit status `1`. The explicit return type fixes the constructor to `Result _, IoError`, including for `pure`.

## Syntax and Semantics

Introduce an indentation-delimited `do` expression with these statement forms:

| Form | Meaning |
| --- | --- |
| `name <- expression` | Bind the carrier's payload and continue. |
| `_ <- expression` | Bind and explicitly discard the payload. |
| `name = expression` | Ordinary local binding; no monadic operation. |
| Nonfinal bare expression | Bind a carrier and discard its payload. |
| Final expression | Return the carrier unchanged. |

The final expression is unchanged; with stdlib binds it must have their required carrier type. There is no implicit `pure` or independent monad check on a block containing no binds. `pure` remains an ordinary library function, not a keyword. Use existing assignment syntax instead of adding Haskell's `let`. Initially permit identifier and wildcard bind targets; defer destructuring until its failure semantics are designed.

With the stdlib's `bind`, all bind statements use one unary constructor `F`; payload types can vary. For `Result`, the error parameter is fixed unless explicitly converted by ordinary code. `Option` skips the continuation on `None`; `Result` skips it on `Err`. These rules come from the resolved function's existing signature and implementation, not from `do` itself. A custom implementation determines its own continuation behavior, including whether it invokes a callback multiple times.

Rock remains eager. Each bind source is evaluated once per execution of its containing continuation, and the remainder is evaluated only when that bind invokes its callback. Ordinary local bindings after a bind stay inside that callback. Do not hoist later operations or promise global exactly-once execution for arbitrary monads.

With the stdlib's `bind`, nonfinal bare expressions must be carriers: Rock's ordinary `println!` returns an `I32`, not a Haskell `IO` action. Keep printing at the boundary as above, or use an explicit ordinary binding when an eager side effect is intentionally part of a continuation. Do not silently select between plain sequencing and monadic sequencing based on an expression's inferred type.

The block is lexical: bound names are visible in the remainder, not outside it. Nested blocks expand independently and ordinary inference handles their types. Empty blocks and blocks ending in a binding are errors.

Initially reject `return`, `break`, and `continue` that would cross a generated callback boundary, and reject `?` directly in the `do` sequencing region; otherwise generated closures could silently change their targets. Explicit nested functions and loops retain their ordinary rules. Use `<-` for propagation and the final expression for the result. Define and test these boundaries before lowering.

Recognize `<-` as bind syntax in a `do` statement, without installing or globally redefining a stdlib operator. Reserve `do` as syntax and document that language change.

## Compiler Integration

### Ordinary Name Resolution

Expand each sequencing step to an ordinary unqualified call to `bind`. Resolve that identifier exactly as if the programmer had written the expanded call at that lexical position. The stdlib prelude already exports `bind`; a custom library or local binding can supply a different function. Shadowing `bind` deliberately changes the meaning of subsequent generated calls, including when a preceding assignment or bind target introduces that name.

This is rebindable syntax: the parser knows the expansion spelling, but does not know a stdlib path, operator meaning, trait identity, or carrier implementation. The generated calls use the same canonical resolution and selection machinery as user-written calls. Missing or unsuitable `bind` definitions produce ordinary resolution or type errors at source-backed spans.

`pure` remains an explicitly written ordinary function. No new language-item protocol, implicit stdlib dependency, compiler-owned pure operation, or artifact protocol metadata is required.

### Expansion Rules

Treat the following as schematic transformations, with `rest` denoting the recursively expanded remaining statements:

```text
x <- action; rest  =>  bind action, (x -> rest)
_ <- action; rest  =>  bind action, (_ -> rest)
action; rest       =>  bind action, (_ -> rest)
x = value; rest    =>  x = value; rest
final              =>  final
```

The generated lambda uses the ordinary value-returning arrow. Preserve assignments inside the relevant callback and leave the final expression unchanged. Keep the block's outer lexical scope even when assignments precede the first bind or the block has no binds; establish the existing AST representation for that case before implementing expansion. Any required scope wrapper must use existing expression forms and preserve eager evaluation and capture behavior.

### Parsing and AST

Extend lexer keyword handling and parse `do` sequencing statements using a small parser-local representation. Reuse indentation handling and ordinary expression/assignment parsing; do not parse the entire block as an ordinary block and try to recover `<-` from arbitrary binary expressions afterward. Expand into existing AST forms before semantic analysis; a persistent semantic `Do` node is unnecessary.

Primary entry points are `lib/src/lexer/lexer.rs`, `lib/src/ast/tree.rs`, `lib/src/parser/items/expression.rs`, and `lib/src/parser/items/block.rs`; add a dedicated parser module if that keeps expression parsing local. Preserve source spans for the introducer, each bind, its target, and its source expression.

Inspect the formatter's representation before deciding the precise expansion point. If formatting needs the surface form, retain it only in the syntax/formatting layer and expand at that layer's boundary; do not make later semantic phases understand `do`. Check macro expansion and debug printing for the same boundary. Parser tests must cover calls, multiline bodies, nesting, indentation, assignments, wildcards, malformed binds, final-expression requirements, and ordinary operator parsing outside `do`.

### Existing Semantic Pipeline

Reuse name resolution, closure capture analysis, higher-kinded inference, trait selection, MIR construction, borrow checking, and code generation unchanged. Do not add `do`-specific expected-type propagation or selection metadata. If a concrete expanded program reveals an existing inference bug, reproduce it without `do` and address that underlying bug separately.

Test annotation-free local blocks with sufficiently constrained input, generic `F _: Monad` helpers, and return-type-directed `pure`. Ambiguous `pure` without enough context should remain a diagnostic rather than defaulting to `Option` or `Result`.

Attribute generated call and callback errors to the originating bind/tail using real spans. Prefer wildcard parameters for discarded payloads so expansion introduces no hidden textual variable names. User bindings and the intentional `bind` reference obey ordinary lexical scope.

### Ownership Gate

The existing `Monad::bind` accepts `M: FnMut A, (F B)`. A generated continuation that moves a previously captured non-`Copy` owner can require `FnOnce` instead. This affects practical file and string programs, not just theoretical custom monads.

Before implementing syntax, compile explicit bind equivalents for owned strings, file owners borrowed by later operations, nested closures, and consuming captured values. Record which cases already work and which fail because of the callback contract. Reuse the same rules for `do`; do not insert clones, leak references, or special-case `Result` during expansion.

This feature preserves the existing `FnMut` contract. Do not weaken it to `FnOnce`: a general monad may invoke its callback repeatedly. Consuming-capture restrictions remain the same as in explicit bind code; any future callable-trait redesign is separate work. Include a custom repeated-callback carrier in the test matrix to expose accidental single-shot assumptions.

### Artifacts and Tooling

No artifact format change is planned: semantic artifacts contain the existing expanded representations. Test imported generic functions whose source contains `do`, ordinary `bind` functions supplied by dependencies, and missing-function diagnostics.

Update the actual formatter and editor grammar consumers located during implementation. The book uses a separately pinned tree-sitter grammar through `docs/checks/grammar-revision.txt`; coordinate that grammar change before updating the pin. Generated syntax needs highlighting and formatting round-trip coverage.

## Implementation Order and Acceptance Gates

### Phase 1: Semantic and Ownership Baseline

Write focused regression cases against today's explicit bind APIs and confirm final-expression behavior, lexical scope, and control-flow restrictions. Inspect formatting and establish the expansion boundary and outer-scope representation. Acceptance: successful ownership cases and intentional rejections are understood without involving the new parser syntax; the implementation can reuse the existing semantic pipeline.

### Phase 2: Parsing and Expansion

Add keyword handling, the parser-local sequencing representation, and tail-to-head expansion into existing AST nodes. Acceptance: syntax tests pass, outer lexical scope is preserved, formatting retains the surface syntax, and downstream compilation sees ordinary calls and closures.

### Phase 3: Semantic Equivalence and End-to-End Behavior

Verify `Option`, `Result`, generic helpers, nested blocks, and a custom carrier through the unchanged semantic pipeline. Acceptance: behavior matches explicit bind code, failed carriers suppress subsequent effects, and no extra clones or ownership relaxations occur. Test ordinary imports, local shadowing of `bind`, missing or incorrectly typed `bind`, assignments before the first bind, and blocks without binds.

Include negative cases for mixed carriers/error types, noncarrier intermediate expressions, raw-value tails, unresolved `pure`, refutable bind targets, invalid control flow, and use-after-move. Check owner drop counts on success and failure, and borrow lifetimes across binds. Compare representative programs with their explicit bind equivalents for output, exit status, evaluation order, and drops.

### Phase 4: User-Facing Integration

Add a language specification section and a book explanation with self-contained executable examples. Update the file-I/O chapter to use flat sequencing where it improves readability, retaining an operator equivalence example. Complete formatter, highlighting, imported-artifact, and book compilation checks.

Run focused tests first, then `cargo fmt --all`, `cargo test -p rock-lib`, and `cargo clippy --workspace --all-targets` serially. Rebuild `rockc` before running `node docs/checks/compile-examples.cjs`; separately execute the I/O examples because the book script checks object compilation only. Run the grammar and book-highlighting checks after updating those components.

## Scope

This proposal adds expression syntax over existing monadic abstractions. Deferred features include implicit lifting, refutable-pattern failure protocols, recursive bindings, applicative scheduling, automatic carrier/error conversions, and a new deferred `IO` effect system.
