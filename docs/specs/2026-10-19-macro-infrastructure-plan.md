# Macro infrastructure: staged implementation plan

Date: 2026-10-19

Status: execution sequence. Test families below are implementation contracts,
not claims that every slice exists or passes; acceptance requires actual test
receipts for the implemented slice and its prerequisites.

Contract: [macro infrastructure specification](2026-10-19-macro-infrastructure.md).
The specification owns language/API requirements; this document breaks delivery
into dependent, independently testable changes. These identifiers describe design
slices, not issue status. Beads remains the issue tracker; `bd` is unavailable in
the planning environment, so no implementation issues were created here.

## Delivery strategy

Build the preservation layer first, then macro semantics, then a usable Rock
worker/build workflow, and finally derives. Do not implement the entire frontend,
SDK, or syntax library in one change.

| Spec stage | Slices | Usable result at the stage gate |
| --- | --- | --- |
| A0: lossless frontend | A0.01–A0.10 | Existing Rock parses through lossless preservation and a capture-capable Rock interpretation boundary |
| A: expansion foundation | A.01–A.15 | Hygienic declarative expressions and bounded block invocations work across source modules; generated syntax is checked normally |
| B: host proc macros | B.01–B.22 | A Rock-authored dependency builds/runs through the SDK; indented DSL and interpolation proofs work without compiler callbacks |
| C: derive attachment | C.01–C.12 | `%derive` invokes external Rock macros using a reusable Rock syntax library; all spec acceptance families have final evidence |

The default dependency of a slice is the preceding slice, with each stage
depending on the previous stage's gate. This deliberately gives one safe serial
execution order, not a requirement to introduce parallel implementations.
Private implementation tests may exercise an unfinished component, but public
syntax must either work through normal checking or fail with a structured
unsupported-feature diagnostic. Never let an unexpanded node reach lowering.

For every functional slice:

1. Add the smallest test expressing the new behavior and at least one relevant
   rejection; observe the expected failure before implementation.
2. Change only the owning component and its necessary integration call sites.
3. Run that slice's focused test family and adjacent existing regressions.
4. Preserve a compiling tree and passing previously delivered behavior. Stop
   before starting the next slice if its prerequisites are not established.

Characterization slice A0.01 is different: its tests must pass on the starting
compiler and describe language behavior, not accidentally freeze bugs or hangs.
Do not leave intentionally failing tests or production compatibility paths as
the result of a slice. Evolving the implementation is allowed; keeping two
shipping lexers, token ABIs, or macro engines is not.

## Test naming and commands

Use a discoverable prefix for each slice, such as `macro_a0_02_`, `macro_a_12_`,
or `macro_b_08_`. Each test name adds a behavior suffix. The prefix is a test
selection contract, not a new test framework. Reuse existing test helpers; keep
compiler execution in `lib/tests/integration.rs` unless a concrete need justifies
a dedicated target. Pure unit tests stay beside their owning modules.

The runner codes used in the tables mean:

| Runner | Focused command template |
| --- | --- |
| U | `cargo test -p rock-lib --lib PREFIX -- --nocapture` |
| I | `cargo test -p rock-lib --test integration PREFIX -- --nocapture` |
| S | `cargo test -p rock-shared --lib PREFIX -- --nocapture` |
| R | `cargo test -p rock --lib PREFIX -- --nocapture` |
| K | `cargo test -p rockc --bin rockc PREFIX -- --nocapture` |

Replace `PREFIX` with the slice prefix; execute every runner listed for a slice,
serially. Verify a nonzero test count, not only exit status. To isolate one test,
use its fully qualified name with `--exact`. Test names may move with module
boundaries; update the actual command reference rather than claiming a zero-test
run passed.

For example, once its proposed tests exist:

```bash
cargo test -p rock-lib --lib macro_a0_02_ -- --nocapture
```

Rock SDK/syntax-library tests run as ordinary Rock programs compiled by the Rust
integration harness; they are not Cargo packages. Build their ordinary providers
first and supply dependencies as artifacts. Use `rockc`/compiler test helpers
until the package workflow is delivered; afterward also test real `rock build`
and `rock run`. Never add compiler-owned SDK/std discovery to make tests pass.

## A0. Lossless frontend foundation

Primary owners: `lib/src/lexer/`, `lib/src/parser/engine/`,
`lib/src/parser/mod.rs`, and adjacent lexer/parser tests.

### A0a. Establish the data model without changing Rock syntax

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| A0.01 | Add a compact source/expected-result corpus for current lexical and parser behavior. Include tokens/spans, spacing-sensitive operators/dots, literals, comments, layout, and ordinary parse failures. | Existing valid cases retain AST meaning and byte spans; malformed ordinary strings/indentation produce structured errors. Existing fixtures and frozen observers remain untouched. | U |
| A0.02 | Introduce immutable source-backed lossless units/runs and a deterministic neutral scanner. Do not replace the parser yet. | Exact coverage/reconstruction for UTF-8, whitespace, CRLF, comments, backticks, emoji, NUL, unterminated quotes/comments, and unmatched brackets; bounded progress and source-ingestion rejection of invalid UTF-8. | U |
| A0.03 | Add checked slices, joins, and origin/context segments over that representation. Reserve root/source contexts without implementing expansion hygiene yet. | Slice within/between runs, join and duplicate slices without metadata loss; reject out-of-range/non-UTF-8-boundary cuts and conflicting binding-context merges. Adjacency is not guessed from printed text. | U |
| A0.04 | Implement the Rock interpretation of identifiers, keywords/types, numbers, punctuation, special macro/intrinsic tokens, arrows, and spacing-sensitive operators. | Relevant A0.01 token/span expectations pass over neutral units; classification works across run boundaries. An unfamiliar unit remains preservable but fails when requested as ordinary Rock syntax. | U |
| A0.05 | Move Rock string/character/comment recognition into that interpretation layer, retaining original units and source mappings. | Escapes/Unicode and comment handling match the existing language; failed recognition is bounded and does not destroy the source. A slice ending inside a quote cannot read beyond its supplied limit. | U |
| A0.06 | Supply the parser's layout/trivia/EOF view with checked indentation widths and explicit source extents. | Comment-line/newline behavior matches ordinary Rock; original trivia remains retrievable. Large widths cannot wrap through `u8`; NUL is not mistaken for EOF. Parser-added terminal layout is not misreported as original source. | U |
| A0.07 | Add explicitly requested balanced-group views and structural group constructors, retaining underlying units. | Valid nested `()`, `[]`, `{}` round-trip with exact origins; malformed Rock group requests diagnose. The same unmatched characters remain traversable as neutral leaves; group creation never globally validates arbitrary DSL text. | U |

The old lexer is not a second public input model. These slices introduce and
test the replacement components; A0.10 retires the old destructive scanning
implementation after integration. Existing internal Rock `TokenType` values can
remain the parser's interpreted vocabulary, not the SDK's lossless substrate.

### A0b. Replace the eager parser boundary

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| A0.08 | Encapsulate parser input access behind cursor/range operations before changing its backing storage. Port direct slice users in arrays, expressions, macro declarations, function shorthand, traits, and combinators. | Existing parser/combinator tests pass. Peek, bounded subparse, rollback, remaining-input comparison, and EOF spans retain their semantics; no caller bypasses capture boundaries by indexing `ParseCtx.tokens`. | U |
| A0.09 | Back the cursor with lossless data and a bounded, capture-capable Rock view. Make lookahead, backtracking, and indentation discovery respect opaque ranges. | A cursor can suspend Rock interpretation, capture a physical range, and resume after it. Malformed quotes/comments and odd body indentation in that range cannot poison later tokens, `indent_step`, or `invalid_indent`; repeated peeks have bounded work. | U |
| A0.10 | Switch source/generated-source parser entry points to the new pipeline; remove the old whole-file strict `lexer.collect()` path and destructive scanner. Keep internal interpreted-token entry points only where they are real compiler APIs. | A0.01 and existing parser tests pass through the replacement. Source and generated-source entry points share the boundary/EOF rules; ordinary compilation/diagnostic smoke cases pass without a legacy fallback. | U, I |

**A0 gate:** run all `macro_a0_` unit/integration families, adjacent lexer/parser
regressions, and `cargo test -p rock-lib` serially. Record actual failures and
warnings. The public block-invocation grammar arrives in A.03; A0 proves the
capture mechanism, not a usable HTML macro or self-hosted compiler.

### A0 delivery receipt (2026-10-19)

A0.01–A0.09 were delivered in separate commits, from `c4abe1b` through
`86b9669`; A0.10 completes the production source-entry switch and scanner removal.
Source and generated-source text now use the same lossless parser boundary.
Existing interpreted-token compiler APIs remain for macro matching/output until
their storage migration in A.01; the token fixture adapter is test-only and uses
the replacement views, not another scanner.

- A0.09: initial RED for missing cursor APIs, then 12 focused tests and all 2,479
  library unit tests passed; capture/backtracking/cache and indentation checks pass.
- A0.10: initial RED demonstrated NUL being accepted as forged EOF. Wiring the new
  entry point exposed three lost lexical-error diagnostics, repaired by source-error
  prioritization and capture-scoped speculative-error cleanup. Final focused
  `macro_a0_10_` runners passed six unit and two compiler integration tests.
- Serialized A0 gate: `cargo test -p rock-lib --lib macro_a0_ -- --nocapture`
  passed 112 tests; the corresponding `--test integration` runner passed two.
  Adjacent `--lib lexer::` and `--lib parser::` runners passed 86 and 540 tests.
- Final `cargo test -p rock-lib` passed 2,485 unit tests, 789 main integration
  tests, and one parser integration test; one doctest remains ignored.
- Final gate warnings: 13 library dead-code warnings for optional group interfaces
  not yet used in production, plus two group-error-field warnings in library tests.
  No gate failures remain. Formatting is still explicitly deferred; no formatter,
  formatting check, Clippy, or separate build gate was run for these slices.

This closes the lossless frontend foundation only; public block invocation syntax,
macro storage/protocol migration, hygiene, authoring workflows, and library policies
remain in the following stages.

## A. Common invocation, expansion, and hygiene

Primary owners: `lib/src/ast/`, parser macro/expression items,
`lib/src/macro_expansion/`, `lib/src/source_loader/`, `lib/src/lib.rs`, and
binding/resolution owners in `collect`/`lower`.

### Aa. Capture and expand one invocation correctly

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| A.01 | Migrate macro invocation/capture/template storage to the shared lossless stream and add invocation/node IDs plus requested output categories. Port existing proc fixture payloads and bump the active wire version when changing them. | Current item macro fixtures still work; equal spans have distinct invocation identities and explicit parent edges. Inline/item/block-shaped synthetic inputs all use the same stream type; old payloads reject rather than silently losing trivia. | U |
| A.02 | Parse qualified expression macro calls using ordinary comma/grouping/postfix rules, preserving exact argument ranges. | `%m 1, (f 2, 3)` and `%m!` have the expected boundaries; argument names need not resolve. Ambiguous cases follow ordinary calls; malformed inline Rock input rejects, and nested macro input is not executed during parsing. | U |
| A.03 | Add bare-header indented-block capture in item/expression positions, using A0's physical-range boundary. | Dedent/EOF, leading/internal blank lines, comment-looking lines, tabs/CRLF under source indentation policy, and exact untrimmed ranges work. Quotes/comments/brackets cannot swallow following Rock. `%m!`, `%derive`, and multiline ordinary calls do not compete for block ownership; no false header inside a Rock string/comment. | U |
| A.04 | Parse generated results by requested category with full consumption and capture-aware nested-block recognition. | One expression or an item list succeeds in the proper category; empty expression, trailing junk, item/expression mismatch, forged EOF, and invalid direct DSL-as-Rock output reject. Arbitrary input inside a generated nested block is not prematurely Rock-lexed. | U |
| A.05 | Port declarative matching to lossless captures with explicit Rock-view trivia policy, preserving current first-arm and flat-repetition semantics. | Existing item matchers pass; each arm consumes all input. Equal-spelled captures retain separate origins; repetition progresses, and unsupported nested repetition/mismatched lengths diagnose instead of flattening. Non-Rock block input is not silently treated as an expression. | U |
| A.06 | Expand declarative invocations into expressions, forwarding opaque expression captures and checked relative layout. | Precedence/grouping survive nested forwarding without print/re-lex; bindings/tail expressions parse normally. Moving generated layout preserves a nested DSL payload; `return`/`break` are not changed by a synthetic lambda wrapper. | U, I |

The parsing/storage slices do not declare expression macros finished. Before the
stage gate, unfinished execution/resolution paths must diagnose explicitly; do
not fall back to a global macro name or spelling-based capture to get a demo.

### A.01 delivery receipt (2026-10-19)

Invocation inputs, declarative captures, and retained templates now use the shared
lossless stream. Interpreted matcher tokens remain a temporary Rock view with
exact logical capture ranges, not a second stored or wire payload. Template
source retains original margins/trivia while the existing item expansion applies
its relative margin. Optional groups use the shared group view rather than the
retired compiler-token tree wrappers.

Invocations carry distinct node-backed IDs, an explicit parent expansion edge,
and an item/expression output request. Generated parsing records the parent on
each child node; the span-keyed parent queues are removed. Worker protocol 3
transports checked snapshot-table streams and invocation/category metadata;
legacy token payloads and requests reject without a compatibility reader. The
obsolete interpreted-token module parser and token wire helpers are removed.

- Initial `cargo test -p rock-lib --lib macro_a_01_ -- --nocapture` was RED because
  lossless streams lacked serialization. Snapshot-table transport subsequently
  passed preservation and independent-snapshot adjacency rejection controls.
- Adjacent `--lib macro_expansion:: -- --nocapture` initially passed 48/51;
  `--lib parser:: -- --nocapture` initially passed 538/540. The migration exposed
  incomplete-head diagnostics, odd-indent error ordering, and repetition layout;
  repaired adjacent runs passed 58 macro and 540 parser tests.
- An intermediate `cargo test -p rock-lib --lib` passed 2,482/2,483; adapting the
  invocation observer to its explicit Rock view removed duplicated source trivia
  without changing the observer fixture. The first full-suite attempt did not
  compile because its fixture referenced the private lexer module; exporting the
  shared scan-limit type through the macro API repaired that boundary.
- The first complete `cargo test -p rock-lib` passed 2,492 units, 789 main
  integrations, and one parser integration; one doctest was ignored (archive
  `e696fe998537f7dc`). A subsequent dedicated RED test exposed a recaptured
  generated-source location reverting to its definition file. Effective run and
  empty-slice locations now retain the captured virtual source.
- Final focused `cargo test -p rock-lib --lib macro_a_01_ -- --nocapture` passes
  12 tests: distinct equal-span IDs/parents, shape-independent exact UTF-8/trivia
  transport, retained template/capture ranges, actual worker metadata, provenance,
  and malformed/old payload rejection.
- Final `cargo test -p rock-lib` passes 2,493 unit tests, 789 main integrations,
  and one parser integration; one doctest remains ignored (archive
  `18cb8d2b745d7d20`). Library warnings remain the 13 unused optional-group
  interfaces and two group-error-field warnings in tests. No functional edits
  follow that gate.

This slice does not add expression/block grammar, change declarative matching
policy, or authorize worker-created hygiene/provenance; those remain in their
own slices. `bd ready --json` still fails because `bd` is unavailable. Formatting
remains deferred; no formatter/check, Clippy, or separate build gate was run.

### A.02 delivery receipt (2026-10-19)

Expression operands now retain qualified macro paths, requested expression output,
and exact logical argument ranges into the same lossless invocation stream. Both
ordinary and macro calls use the same comma/grouping/postfix argument parser;
the temporary parsed argument expressions validate syntax without resolving names
or expanding nested input. `%m!` retains empty input. AST traversal, source spans,
observers, and the lowering rejection boundary recognize the new operand.
Expression execution still returns a structured macro diagnostic before lowering.

- Initial `cargo test -p rock-lib --lib macro_a_02_ -- --nocapture` was RED
  because the expression operand and path/range fields did not exist. The first
  implemented run passed eight tests.
- The added lexical-diagnostic control first failed to compile due to a moved
  test binding, then observed RED: an unterminated argument string was replaced
  by a generic macro-header error. Required macro-input diagnostics now preserve
  lexical/layout errors and their original spans; the focused runner passes all
  nine parser tests.
- The first full `cargo test -p rock-lib` passed 2,502 units, 789 main
  integrations, and one parser integration; one doctest was ignored (archive
  `8a436a98ec22b5cd`). A subsequent ancestry control observed RED because the
  new expression invocation inside generated items had no parent edge. Generated
  parsing now carries the explicit parent in its parse context and records it
  when constructing either invocation category, rather than patching only root
  item invocations afterward.
- Final focused `cargo test -p rock-lib --lib macro_a_02_ -- --nocapture`
  passes ten tests. Adjacent `cargo test -p rock-lib --lib parser::` and
  `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` pass 549 and
  60 tests.
- Final `cargo test -p rock-lib` passes 2,503 units, 789 main integrations,
  and one parser integration; one doctest remains ignored (archive
  `9cef8f9ac3d43fd9`). The ten focused tests pass again after moving the new
  ancestry test below the imports. No functional edits follow the final full
  gate. Warnings remain the 13 unused optional-group library interfaces and
  two group-error-field warnings in library tests.

Block capture, generated expression parsing/expansion, and scoped macro resolution
remain in A.03 and later slices. `bd ready --json` still reports command not found.
Formatting remains deferred; no formatter/check, Clippy, or separate build gate
was run.

### A.03 delivery receipt (2026-10-19)

Actual parsed macro paths can now introduce indented item/expression input blocks.
The optional capture view compares physical margins before interpreting the body,
retains everything after the header LF through dedent/EOF, and records header and
first-body indentation separately from the requested output category. Arbitrary
UTF-8, odd internal margins, wide margins, blank lines, comment-looking lines,
CRLF/tabs, quotes, delimiters, and apparent nested calls remain lossless data.
Expression blocks expose a source-aware synthetic separator; item parsing consumes
that separator. Parentheses can close on the terminating dedented line, and outer
argument ranges may span capture branches of the same retained source only.

- Initial `cargo test -p rock-lib --lib macro_a_03_ -- --nocapture` was RED
  because block metadata did not exist. The first implementation passed three of
  eight tests; item parsing incorrectly retained the expression separator. After
  separating item newline ownership, all eight passed.
- Adjacent `cargo test -p rock-lib --lib parser:: -- --nocapture` initially
  passed 556/557: its old invocation assertion included the header newline in
  input. Updated that assertion to the specified after-header range without
  changing the source fixture.
- Added nested-parenthesis and inline-module parser controls. The former observed
  RED until closing-layout handling and cross-branch retained ranges were wired.
  The latter exposed existing top-level `mod` dispatch selecting a source-backed
  shell before its inline alternative; this slice leaves that unrelated dispatch
  unchanged and tests the inline-module parser directly.
- A dedicated array-lookahead control observed RED: speculative repeat-array
  scanning had already interpreted an opaque quote across the dedent. That scan
  now yields to the ordinary array/expression parser when it encounters a block
  header, before requesting body tokens. Literal-array capture passes without
  opaque payload tokens appearing in the cached-token observer.
- Focused `cargo test -p rock-lib --lib macro_a_03_ -- --nocapture` passes
  13 tests, including synthetic separator origin and rejection of unrelated-source,
  backwards, and bounded-token-prefix capture ranges.
- Final `cargo test -p rock-lib` passes 2,516 units, 789 main integrations,
  and one parser integration; one doctest remains ignored (archive
  `cf4e52eb9f1e5015`). Adjacent final `--lib parser:: -- --nocapture` and
  `--lib macro_expansion:: -- --nocapture` runners pass 562 and 60 tests.
  Warnings remain the 13 unused optional-group library interfaces and two
  group-error-field warnings in tests. No functional edits follow these gates.

`%derive` is reserved with a structured unsupported-attachment diagnostic; `%m!`
does not acquire block input. Unresolved item-block paths diagnose explicitly,
while existing declared item macros keep their current matching behavior.
Expression execution, generated-result categories, repeated-array values containing
opaque blocks, and library DSL policies are not certified by this parser slice.
Formatting remains deferred; no formatter/check, Clippy, or separate build gate
was run. Beads remains unavailable.

### A.04 delivery receipt (2026-10-19)

Generated parsing now dispatches explicitly to an item list or one ordinary Rock
expression. Both use the same lossless/capture-aware source boundary, expansion
parent, parser configuration, resource limits, and complete-input check. Empty
item output is accepted; no category fallback or synthetic lambda is introduced.

- Initial `cargo test -p rock-lib --lib macro_a_04_ -- --nocapture` was RED:
  the requested-category/result API did not exist (and the initial assertion
  referenced an unavailable test formatter helper, subsequently removed).
- The first implemented run passed five of six tests; an empty item stream
  exposed terminal blank layout being treated as an item. The module parser now
  consumes empty lines before probing the next item. All six focused tests pass.
- Controls cover comma/grouping structure, empty/multiple items, empty expressions,
  category mismatches, trailing input, NUL/forged EOF, direct non-Rock DSL output,
  bounded-source offsets/errors, ordinary `do`/`loop` syntax, and opaque generated
  nested blocks with explicit parent edges.
- Adjacent `cargo test -p rock-lib --lib parser:: -- --nocapture` and
  `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` pass 568 and
  66 tests, respectively.
- The subsequent full `cargo test -p rock-lib` passes 2,522 units, 789 main
  integrations, and one parser integration; one doctest is ignored (archive
  `ce87e6befc02ff1d`). No A.04 functional changes followed its focused gates.

Expression execution and repeated-array values containing opaque blocks remain
outside this slice; the latter must parse correctly or reject structurally before
A.06 enables execution. Formatting remains deferred; no formatter/check, Clippy,
or separate build gate was run. Beads remains unavailable.

### A.05 delivery receipt (2026-10-20)

Declarative matching now explicitly ignores Rock whitespace, physical line
endings, and comments in its temporary view while retaining original internal
trivia, snapshot segments, contexts, and independent capture occurrences. Literal
matching keeps Rock's spacing-sensitive token kinds; `ty` remains a single type
name. Indentation is not synthesized or narrowed before the flat matcher ignores
it. Non-Rock block input still produces structured diagnostics, not an arbitrary
text/DSL match.

Repetition submatches must finish the whole group before succeeding; complete
arms must consume all significant input. Ambiguous complete splits diagnose and
cannot fall through to another arm. Definition validation rejects unsupported
nested repetitions, empty-progress matcher repetitions, and empty definitions,
including unused declarations. Repeated key groups are stable across hash-map
order, duplicated captures are not deduplicated, and template length mismatches
reject. A selected arm emitting zero items now succeeds without arm fallthrough.

- `cargo test -p rock-lib --lib macro_a_05_ -- --nocapture` initially observed
  RED at three passes/seven failures. Strengthening the duplicate-origin control
  and adding ambiguity produced two passes/nine failures. Implementation then
  reached nine/eleven and ten/eleven; fixture corrections made repetition closes
  explicit (`)*`) and retained the spacing needed for a literal operator at EOF.
- The focused diagnostic command
  `cargo test -p rock-lib --lib macro_a_05_flat_repetition -- --nocapture`
  observed the remaining literal-token spacing mismatch. Its temporary debug
  print was removed after correcting that input, without relaxing token matching.
- The focused runner subsequently passed eleven tests; the added empty-definition
  control brings final acceptance to twelve. Final adjacent
  `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` and
  `cargo test -p rock-lib --lib parser:: -- --nocapture` pass 78 and 568 tests.
- Final `cargo test -p rock-lib` passes 2,534 units, 789 main integrations, and
  one parser integration; one doctest is ignored (archive `971354cdeddd19a8`).
  Warnings remain the 13 unused optional-group library interfaces and two
  group-error-field warnings in tests. No functional edits follow these gates.

Expression execution/opaque expression forwarding and capture-aware relative
layout remain A.06. Formatting is still deferred; no formatter/check, Clippy, or
separate build gate ran. Beads remains unavailable.

### A.06 delivery receipt (2026-10-20)

Root function-body declarative calls now use the same matcher, template engine,
and requested-category parser as item calls. Captured expressions retain opaque
range boundaries over their original stream bytes, not printed substitutions or
stored compiler tokens/ASTs. Slicing, splicing, origin tagging, and wire transport
carry those boundaries; bounded ordinary parsing revalidates their contents.
Nested emitted calls expand outside-in, and discarded input is never executed.

Capture parsing retains the original header margin and rebases Rock layout with
checked subtraction; ordinary dedented closing wrappers remain legal. Physical
block scans skip captured-expression internals without imposing Rock layout on
DSL payload bytes. Moving a capture under generated layout preserves its nested
DSL input exactly. Invalid relative margins reject instead of clamping to zero.
Opaque repeated-array values currently reject structurally rather than silently
turning the repeat separator into an ordinary operator. Expression substitution
introduces grouping, not a lambda, so caller-owned `return` and `break` retain
their behavior under normal compilation.

The stream format is now `RKLOSS02` and worker protocol 4; previous streams reject
without a compatibility reader. Transport rejects empty, non-UTF-8-boundary,
crossing, duplicate, unordered, and excessively nested expression ranges.

- Initial `cargo test -p rock-lib --lib macro_a_06_ -- --nocapture` was RED at
  compilation because expression boundaries/shared category dispatch did not
  exist. After those interfaces landed it observed two passes/four failures:
  execution was unsupported and opaque repeated arrays parsed incorrectly.
- The same runner reached six passes. Added multiline/physical-layout controls
  observed seven passes/three failures before capture-aware scanning and bounded
  source-prefix matching were repaired, then ten passes. Transport/literal
  matcher controls brought acceptance to twelve passes.
- Initial `cargo test -p rock-lib --test integration macro_a_06_ -- --nocapture`
  exposed parser stack growth; boxing the interpreted fragment view repaired it.
  The exact exploratory command
  `cargo test -p rock-lib --test integration macro_a_06_generated_do_keeps_break_in_the_callers_loop -- --exact --nocapture`
  then confirmed ordinary `do` forbids escaping control flow. Fixtures now use
  ordinary `if` blocks and unit-valued loops rather than assuming different
  language semantics. The focused runner subsequently passed three tests.
- Adjacent `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture`
  initially passed 82/84; expected expression grouping and the worker-version
  assertion were updated. The parser runner passed 568 tests.
- An intermediate `cargo test -p rock-lib` passed 2,546 units before cancellation
  during integration tests to add stronger layout/precedence controls (archive
  `e032111936f6a137`); this is not a completed full-suite receipt.
- The added checked-margin control observed thirteen passes/one failure with
  the focused unit runner, proving an invalid dedented `else` had been clamped.
  Checked rebasing repairs it; final focused units pass fourteen tests, adjacent
  macro/parser runners pass 91/568 tests.
- Final `cargo test -p rock-lib` passes 2,548 units, 792 main integrations, and
  one parser integration; one doctest is ignored (archive `94ba1fdbf1c55086`).
  The final focused integration runner also passes all three tests, including
  unparenthesized caller-expression forwarding. No functional edits follow
  these gates. Existing optional-group warnings remain unchanged.

Scoped identities/resolution, all-module/all-field traversal, aggregate budgets,
and binding hygiene remain the subsequent A slices. Procedural expression calls
still reject explicitly. Formatting remains deferred; no formatter/check, Clippy,
or separate build gate ran. Beads remains unavailable.

### Ab. Scope and schedule the same engine across modules

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| A.07 | Replace short-name registry lookup with canonical macro identities and module-scoped explicit imports/exports. Reuse existing path/visibility machinery without requiring full ordinary declaration collection first. | Local forward references, aliases, qualified direct proc imports from synthetic artifacts, and same-spelled value/macro names work. Conflicts/privacy failures diagnose; no auto-injection, last-writer-wins, facade/glob fallback, or cross-crate declarative publication. | U |
| A.08 | Traverse loaded root/inline/file modules and every expression-bearing AST field with one expansion visitor. | Invocations in function/lambda bodies, defaults, initializers, conditions, arrays, and nested arguments are visited. Outer input stays opaque until emitted; discarded nested invocations never run. | U |
| A.09 | Interleave generated item/module/import discovery to a deterministic fixed point, then expand expressions outside-in before ordinary collection. | Generated modules and macro imports become available before bodies are diagnosed. Each invocation executes once; forward definitions and no-progress/import-cycle failures are stable independent of map registration order. | U, I |
| A.10 | Enforce aggregate expansion/source-discovery/matching budgets and ID-based ancestry through those rounds. | Direct/mutual recursion, many siblings, repeated spans, non-progress matching, wide generated modules, and spelling/trivia-heavy streams terminate with the right invocation trace; no unexpanded success reaches collection/lowering. | U |

### A.07 delivery receipt (2026-10-20)

Macro discovery now builds a separate, module-scoped namespace from parsed root,
inline, and already loaded file modules, without ordinary declaration collection
or worker execution. Source definitions have node-backed canonical identities;
proc identities combine the supplying crate identity and export identity.
Dependency aliases select a declared surface without changing that identity.
The frontend supplies the loader's source-module set and the artifact's declared
dependency name; filesystem paths and export registration order are not binding
authorities. Existing AST path extraction and explicit export collection are
reused rather than running the ordinary collector early.

Local forward references, explicit macro imports, imported module aliases,
qualified source/proc calls, and nested direct proc surfaces resolve through
those identities. Same-spelled values do not compete in this namespace. Duplicate
declarations, conflicting explicit imports/providers, duplicate proc surfaces,
private macros, and private nested modules diagnose instead of overwriting a
binding or choosing an implementation kind. Module alias discovery is bounded
by the number of explicit imports and accepts forward alias references.

No loaded proc export is injected unqualified. Native worker fixtures now name
their supplying dependency explicitly. Macro globs, facade/re-export uses, and
external declarative publication diagnose without implementing those deferred
features. Ordinary value-only imports/re-exports/globs remain inert during macro
discovery; deferred macro bindings diagnose only when a macro use requests them.

- Initial `cargo test -p rock-lib --lib macro_a_07_ -- --nocapture` observed RED
  at compilation because scoped resolution/canonical identity APIs did not exist
  (archive `a6615e57d22e5c7d`). Once implemented it passed six/eight tests; two
  item fixtures were corrected to use existing item invocation syntax rather
  than expression-only `!` parsing. The runner then passed eight tests.
- Added conflict/facade/descriptor/native-worker controls observed twelve
  passes/two failures, exposing an undiagnosed facade import and conflicting
  duplicate proc surface. Repairs brought the runner to fourteen passes.
- A forward module-alias control observed fourteen passes/one failure; bounded
  alias discovery repaired it and the runner passed fifteen tests.
- An intermediate `cargo test -p rock-lib` was cancelled during units to add the
  value-only facade-import control (archive `411e7c8a709dda9d`); this is not a
  completed gate. The first fixture passed without exercising sorted-scope
  order; moving its consumer after the facade exposed fifteen passes/one failure.
  Deferring facade lookup until macro use repairs that ordering dependency.
- Final focused units pass sixteen tests. Adjacent
  `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` and
  `cargo test -p rock-lib --lib parser:: -- --nocapture` pass 105/568 tests.
- Final `cargo test -p rock-lib` passes 2,562 units, 792 main integrations,
  and one parser integration; one doctest is ignored (archive
  `41ee2ccd271bc724`). No functional edits follow this gate.

Expansion traversal remains root function/item scoped until A.08; discovery of a
loaded module is not a claim that its bodies are expanded. Fixed-point generated
module discovery, aggregate budgets, binding hygiene, and Rock package authoring
remain later slices. Formatting is deferred; no formatter/check, Clippy, or
separate build gate ran. Beads remains unavailable.

### A.08 delivery receipt (2026-10-20)

One scope-aware expansion visitor now walks root, inline, and already loaded
file-module items, then all semantic expression-bearing fields. It handles
function/lambda parameters and bodies, struct/enum field defaults, trait/impl
methods, assignment targets/initializers, conditions/guards, loops, ranges,
casts, arrays/repeats, tuples, instances, secondary arguments/indices, and
pattern-owned literal containers. Method and instance maps are visited in stable
source/name order. Macro definitions and invocation inputs remain opaque data;
only emitted syntax is recursively executed, outside-in.

`expand_macros_in_graph` publishes expanded root and loaded bodies back to the
source graph used by normal collection. Registry rebuilds use that same current
forest. Later source discovery reuses graph bodies rather than appending a raw
parse-cache duplicate that would hide expanded expressions. Loader identity,
paths, and source bytes remain unchanged. Root-only expansion/inspection uses
the same visitor and its explicitly provided source-module environment.

Inline module parsing now tries a nonempty indented body before committing to a
file-module header. Nested macro templates rebase from their declaration margin
with checked arithmetic rather than subtracting a fixed root-level margin.
No formatter or source-printing round trip is involved.

- Initial `cargo test -p rock-lib --lib macro_a_08_ -- --nocapture` observed RED
  at compilation because the graph expansion entry point did not exist. After
  the visitor landed it passed three/six tests; inline parsing failed and one
  container fixture used unsupported brace-instance syntax. The fixture now
  uses ordinary named fields and a parenthesized lambda.
- The same runner again passed three/six: prioritizing inline parsing exposed
  empty inline bodies stealing file declarations and fixed-root macro margins
  rejecting nested definitions. Requiring a nonempty body and checking the
  actual declaration margin brought it to six passes.
- A normal-frontend/rediscovery control observed five passes/two failures from
  duplicate raw graph entries and unexpanded macros reaching lowering. Reusing
  graph bodies repaired both; the runner passed seven tests.
- Added guard/range/cast, pattern/enum-default, loaded native-worker, and local
  parser controls observed ten passes/one failure from an incorrect expected
  input count (seven, not eight). The final focused runner passes eleven tests.
  The native worker control records exactly two executed expansions: one loaded
  item call and one discard call; its nested procedural input never executes.
- Adjacent `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture`,
  `cargo test -p rock-lib --lib parser:: -- --nocapture`, and
  `cargo test -p rock-lib --lib source_loader:: -- --nocapture` pass 115/569/16
  tests respectively.
- Final `cargo test -p rock-lib` passes 2,573 units, 792 main integrations,
  and one parser integration; one doctest is ignored (archive
  `0a95921d27639ee9`). No functional edits follow this gate.

Interleaved generated file discovery/import scheduling remains A.09; aggregate
budgets and complete ID-based progress/ancestry enforcement remain A.10.
Procedural expression calls still reject explicitly. Formatting remains deferred;
no formatter/check, Clippy, or separate build gate ran. Beads remains unavailable.

### A.09 delivery receipt (2026-10-20)

The same expansion visitor now runs against a deterministic discovery frontier:
publish current bodies, discover generated file declarations through the ordinary
source loader, rebuild the macro-only namespace, and expand available item calls.
Loaded modules execute in canonical qualified-name order. Generated inline
modules/definitions/imports/exports and transitive generated file declarations
become available on subsequent rounds before expression bodies are expanded or
ordinary collection begins. Successful item invocations are replaced immediately
and are not executed again while another invocation waits for resolution.

Provisional namespace discovery postpones import access failures that generated
exports/modules can repair; declaration/import identity conflicts remain errors.
The final frontier uses strict import validation. A frontier with neither a new
module nor an executed invocation diagnoses unresolved macro paths rather than
spending the recursion limit. Unresolved explicit module-alias cycles diagnose
at the macro use without adding facade/glob resolution. Root-only inspection
shares the scheduler but cannot discover files without a supplied loader.

The frontend refreshes diagnostic source snapshots after expansion/discovery,
including on failure, so errors in newly discovered files retain their text and
origin. The old final one-shot source-discovery pass is removed; collection sees
the fixed-point graph, not a second independently rebuilt graph.

- Initial `cargo test -p rock-lib --lib macro_a_09_ -- --nocapture` observed two
  passes/four failures: generated file discovery was absent, a generated export
  could not repair premature privacy rejection, unknown calls consumed the
  depth budget, and loaded registration order changed worker execution order.
- Interleaved discovery, provisional access validation, stable module ordering,
  and no-progress/import-cycle diagnostics brought the same runner to six
  passes. Added newly discovered diagnostic-source and native one-time execution
  controls bring the final focused runner to eight passes.
- `cargo test -p rock-lib --test integration macro_a_09_ -- --nocapture` passes
  two runtime tests: generated inline imports print `42`, and generated file
  modules with transitive generated children return status `42` without std.
- Adjacent `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` and
  `cargo test -p rock-lib --lib source_loader:: -- --nocapture` pass 123/16 tests.
- Initial `cargo test -p rock-lib` observed 2,580 unit passes/one failure
  (archive `f5187116b5fd2a0c`): the A.03 unresolved-block control still expected
  the pre-scheduler unsupported-execution wording. Its assertion now requires
  the stalled-resolution diagnostic and macro path; parser input/opacity
  expectations are unchanged. The adjacent parser runner passes all 569 tests.
- Final `cargo test -p rock-lib` passes 2,581 units, 794 main integrations,
  and one parser integration; one doctest is ignored (archive
  `e2cb1028a3c217d9`). No functional edits follow this gate.

Aggregate matching/expansion/source-discovery budgets and complete ID-based
ancestry enforcement remain A.10. Procedural expression execution, binding
hygiene, authoring workflows, and library policies remain later slices.
Formatting is deferred; no formatter/check, Clippy, or separate build gate ran.
Beads remains unavailable.

### A.10 delivery receipt (2026-10-20)

Expansion now shares compiler-controlled aggregate limits across siblings,
modules, matcher arms/repetition submatches, and discovery rounds: 10,000
invocations, 64 MiB output spelling/trivia, 4,194,304 stored output units,
33,554,432 matching-work units, 100,000 source-discovery visits, and 64 MiB newly
discovered source bytes. Tests override these limits without changing production
defaults. Stored-run and full-byte accounting cannot be bypassed by exposing an
opaque expression/DSL as one interpreted atom. Output is charged before generated
parsing; matcher-budget exhaustion is fatal, not ordinary arm fallthrough.

Execution records are checked against invocation IDs before a worker or
declarative arm runs. Reusing an executed ID rejects; distinct IDs at identical
spans remain independent. Explicit parent chains must exist and be acyclic;
their depth, not total scheduler rounds or the latest sibling, enforces the
recursion limit. Direct/mutual recursion and sibling-budget failures carry the
correct trace. Generated source declarations record their producing expansion
by invocation ID and structural module scope, so source-budget diagnostics keep
that ancestry without looking it up by source position.

The loader accepts a resource callback for file/inline visits and source-byte
preflight, stops discovery after exhaustion, charges file growth after preflight,
and caps physical reads at the existing ordinary-parser source limit. This does
not introduce an alternate loader or std-specific discovery. A strict semantic
AST check rejects any remaining invocation before collection.

The new executed-ID check exposed a pre-existing duplicate-root seam: named
graphs index the entry module both as root and as a loaded-module record. The
scheduler now executes one root authority and publishes its result to both graph
views instead of executing the same invocation twice.

- Initial `cargo test -p rock-lib --lib macro_a_10_ -- --nocapture` observed RED
  at compilation because limits/state did not exist. A subsequent compile caught
  a resource-error conversion arm returning the wrong type; it was repaired.
- The focused runner then observed five passes/one failure (twice while exposing
  its diagnostic): the named graph root executed the same invocation ID twice
  before source-budget exhaustion. Deduplicating root execution brought it to
  six passes, then source-byte and forged/cyclic-parent controls to eight passes.
- Initial adjacent `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture`
  observed 128 passes/one failure: the A.08 synthetic-container fixture cloned
  one invocation ID into multiple executable positions. It now creates distinct
  parsed occurrences while retaining all nine field/container expectations;
  the adjacent runner subsequently passed 131 tests.
- Added strict-boundary and single-root controls bring final focused acceptance
  to ten tests. Adjacent `cargo test -p rock-lib --lib parser:: -- --nocapture`
  and `cargo test -p rock-lib --lib source_loader:: -- --nocapture` pass 569/16
  tests respectively.
- Final `cargo test -p rock-lib` passes 2,591 units, 794 main integrations,
  and one parser integration; one doctest is ignored (archive
  `fc62516a22dbf398`). No functional edits follow this gate.

Binding contexts/resolution remain A.11–A.14; diagnostic propagation through
normal checking remains A.15. Procedural expression calls still reject explicitly.
Formatting remains deferred; no formatter/check, Clippy, or separate build gate
ran. Beads remains unavailable.

### Ac. Carry binding identity all the way to normal checking

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| A.11 | Carry syntax contexts through interpreted tokens, AST identifiers/paths, and capture/group operations. Update equality, hashing, and ordering together wherever they represent binding identity. | Same spelling/different contexts remain distinct; ordinary source root contexts preserve existing programs. Classify/slice/forward operations cannot drop or mint contexts, and source spans do not determine identity. | U |
| A.12 | Make local binding/reference and pattern resolution context-aware through lowering. | Caller locals collide with generated locals safely; destructuring, shadowing, lambdas, duplicated captures, and captured mutable references bind to the intended owners. Borrow-check negatives remain negatives. | U, I |
| A.13 | Carry contexts through module/type/generic/associated paths and definition-side environment lookup. | Captured type names and generic binders survive impl/type application; definition helpers resolve under aliasing/shadowing. Private helpers and undeclared dependencies remain inaccessible; no hardcoded std/operator meaning enters resolution. | U, I |
| A.14 | Apply template fresh contexts and proc construction permissions: fresh identifiers, explicit call-site names, and authorized definition-side dependency paths. | Template free names use definition scope, captures use caller scope, and generated public names require explicit exposure. Forged/cross-invocation contexts and conflicting-context merges reject. Exercise existing native fixture workers; Rock authoring comes later. | U, I |
| A.15 | Preserve expansion/source mappings through inference, borrow checking, codegen diagnostics, and expanded-output inspection. Close the normal-compilation integration seam. | Caller/generated failures show the correct ancestry and locations; `return`/`break`, unsafe rules, borrowing, and drop timing behave like handwritten output. Normal compilation and equivalent expansion inspection use the same engine. | I, R |

### A.11 delivery receipt (2026-10-20)

Binding-bearing interpreted tokens now carry their agreed retained syntax
context into ordinary parser cursors instead of leaving it only on the optional
lossless view. Identifiers, macro-path segments, operator names, type names,
generic binders, and associated type members retain that context in the AST.
Identifier equality, hashing, and ordering use the same spelling/context pair;
moving a diagnostic span does not change binding identity. Operator/token
equality also distinguishes contexts. Ordinary source and explicitly synthetic
root names continue to use `SyntaxContext::ROOT` through its default value.

Existing expected-AST constructors now state that root context; their Rock
source fixtures and semantic assertions are unchanged. Operator reclassification
and shorthand token conversion copy contexts, while capture slices, neutral
groups, duplicated declarative captures, and opaque expression forwarding retain
the original lossless authority. A type-constructor projection in collection
copies its existing context rather than silently replacing it with root.
No production allocator for fresh contexts is introduced in this slice.

- `cargo test -p rock-lib --lib macro_a_11_ -- --nocapture` initially observed
  RED because token/identifier/operator context fields were absent; that compile
  also caught two incorrect fixture helper names. Adding context fields required
  root-context updates to existing expected-AST constructors; a subsequent RED
  compile identified the remaining 52 constructor/helper errors.
- The first compiled five-test runner observed three passes/two failures: one
  assertion assumed a top-level binary expression rather than visiting the
  ordinary parsed tree, and another included retained leading trivia from a
  different context in its identifier range. Correcting those fixture assertions
  brought the runner to five passes.
- Extending acceptance to eight tests first caught an incorrect source-cursor
  import path at compilation, then observed six passes/two failures from fixture
  setup (initial indentation and an overly broad multi-item type fixture). After
  the indentation correction, seven passed/one failed; the type fixture now uses
  locally declared generic fields and associated paths without unrelated record
  construction. Focused acceptance then passed all eight tests.
- Adjacent `cargo test -p rock-lib --lib lexer:: -- --nocapture`,
  `cargo test -p rock-lib --lib parser:: -- --nocapture`, and
  `cargo test -p rock-lib --lib macro_expansion:: -- --nocapture` pass
  99/569/133 tests, respectively.
- Initial `cargo test -p rock-lib` aborted with stack overflow in
  `crate_artifact::tests::std_index_selection_persists_exact_impl_and_trait_authority`
  (archive `8ec8cee26c947225`);
  `cargo test -p rock-lib --lib crate_artifact::tests::std_index_selection_persists_exact_impl_and_trait_authority -- --exact --nocapture`
  reproduced the abort.
  Boxing the retained `MacroInvoc` operand payload keeps it out of every ordinary
  recursive expression frame without changing its data/serialization or raising
  the test stack size. The same exact test now passes; a new size guard brings
  focused A.11 acceptance to nine passes. The post-repair
  `cargo test -p rock-lib --lib macro_a_ -- --nocapture` passes all 121 tests.
- Final `cargo test -p rock-lib` passes 2,600 units, 794 main integrations,
  and one parser integration; one doctest is ignored (archive
  `ca76208b2cf20122`). No functional edits follow this gate.

This is context delivery, not completed hygiene: local lowering still needs
context-aware binding/reference keys (A.12), module/type/definition environments
remain A.13, and template freshness/procedural construction permissions remain
A.14. Worker transport continues rejecting unauthorized non-root wire contexts;
this slice does not grant new authoring permissions. Formatting remains deferred;
no formatter/check, Clippy, or separate build gate ran. Beads remains unavailable.

### A.12 delivery receipt (2026-10-20)

Local scope keys now pair spelling with syntax context. Source bindings,
reassignments, tuple destructuring, match patterns, array-loop bindings, ordinary
and curried lambda parameters, function parameters, and method parameters retain
that authority until lowering selects a canonical `HirLocalId`. A context mismatch
cannot fall back to a same-spelled caller local. Binary-operator flattening keeps
the operator context for local-callee lookup; function-dependency collection uses
contextual local shadowing without changing module lookup policy.

Implicit `self` binds in the receiver marker's context, independently of a
captured/public method name. The parser retains that context on function
declarations/signatures; existing synthetic expected-AST constructors explicitly
use root without changing their Rock fixtures or semantic assertions.

Closure capture discovery, deduplication, and capture-kind joining now use local
IDs, not spellings. Scope metadata retains hidden IDs until their lexical frame
ends. End-to-end ownership acceptance exposed an additional MIR name-lookup
defect: a caller-local write was redirected to a same-spelled generated local,
incorrectly accepting a conflicting loan. MIR now maps resolved reads, places,
move sources, and captures through canonical local IDs, registering parameters,
capture slots, bindings, match patterns, and loop variables. Existing unresolved
synthetic `Var` handling is unchanged; resolved locals never fall back to it.

- Initial focused compilation caught test-helper API mistakes; adding receiver
  context also required root-context fields in existing expected-AST constructors.
  The first seven focused cases passed. Extended acceptance initially passed
  eight/failed five: four fixture issues (curried arrow spelling, an omitted
  receiver marker, an expected implicit dereference, and an immutable `FnMut`
  binding) plus the real MIR loan bypass described above. After repairs, the
  fifteen-case runner passed; two additional ownership controls passed in the
  final macro-family gate.
- `cargo test -p rock-lib --lib lower::` passes 319 tests; serial
  `cargo test -p rock-lib --lib mir::builder::` passes 87 tests.
- `cargo test -p rock-lib --test integration macro_a_12_ -- --nocapture`
  passes three tests: caller-expression/closure forwarding with destructuring,
  a captured mutable-reference update, and rejection of a conflicting caller loan.
  Its first run passed one/failed two because the new fixtures used a stuck `*$`
  operator and an assignment where an expression result was required; the final
  fixtures use ordinary `*($loan)` syntax and an identifier capture inside `if`.
- `cargo test -p rock-lib --lib macro_a_ -- --nocapture` passes all 138 tests,
  including all seventeen A.12 synthetic-context cases. They cover independent
  assignments, shadows/destructuring, ordinary/curried/nested lambdas, duplicate
  references and same-spelled distinct captures, mutable-reference ownership,
  match/loop bindings, receiver markers, operators, function-dependency ordering,
  and both conflicting and independent-loan controls through finalized HIR/MIR.
- Final `cargo test -p rock-lib` passes 2,617 units, 797 main integrations,
  and one parser integration; one doctest is ignored (archive
  `bc89866781371c0d`). No functional edits follow this gate.

This completes local-context/ID plumbing, not fresh macro-name construction.
Non-root authorities remain test-only; module/type/generic/definition environments
remain A.13 and production freshness/procedural permissions remain A.14. Native
runtime fixtures exercise ordinary forwarding, not yet fresh template allocation.
Formatting remains deferred; no formatter/check, Clippy, or separate build gate
ran. Beads remains unavailable.

### A.13 delivery receipt (2026-10-20)

Module, nominal/type-alias, generic, associated-type, enum-variant, and static
member paths retain spelling/context pairs until canonical identity selection.
The item index distinguishes contextual module segments and declarations;
collection payloads are owned once by `DefId`, with spelling indexes used only as
lookup surfaces. Signature/body pairing, generic defaults and type lambdas,
constructor applications, impl generic discovery, where/static bounds, receiver
`Self`, and generic-owner remapping retain their binding authority. Cyclic generic
remapping snapshots source contexts before assigning destination slots.

Glob import/export AST paths now retain identifiers rather than strings. A
compilation-local identity surface resolves ordinary imports and re-exports to a
fixed point, including contextual modules and definition-scope import aliases.
Glob imports retain member contexts, exclude private members, and do not publish
generated names in the root context. Explicit aliases take precedence over glob
aliases; contextual conflicts and unresolved paths reject. Existing spelling-only
export surfaces expose only root-context names. Macro globs remain deferred;
this is ordinary value/type import plumbing, not a second macro glob resolver.

Expansion carries compiler-owned definition environments into collection and
lowering. Source helpers resolve in their recorded module environment rather
than a same-spelled caller scope; exact generated-item bindings take precedence.
Dependency environments select explicitly supplied providers by
`ProductCrateIdentity` and public export IDs, not caller aliases. Undeclared,
missing/transitive-only, private, conflicting, or mixed-context authorities reject.
All new resolver context/environment tables are compilation-local (`serde(skip)`),
not serialized binding authority, and no std/operator meaning was introduced.

- Focused `cargo test -p rock-lib macro_a_13_ --lib` grew from the generic
  controls to 43 acceptance cases. RED runs exposed constructor/enum path heads
  lost during application/projection, contextual import aliases and containing
  modules falling back to root siblings, source-backed declarations lacking their
  module identity, `Self` projection owners selected by spelling, and generated
  functions leaking into the root value scope. Each has a regression control.
- Enum/static-member negative controls finalize inference, since unresolved
  qualified paths can remain inference variables after partial lowering.
  Contextual enum-pattern controls cover both owner and member identity.
- The glob/definition-import runner initially passed 40/failed two because the
  legacy loader attempted to load indexed inline modules as files. Indexed
  identity surfaces now handle those globs without filesystem reconstruction;
  the next 42-case runner passed. An additional conflict/unknown-glob control
  passes in the final macro-family gate.
- Final `cargo test -p rock-lib macro_a_ --lib` passes all 181 tests, including
  all 43 A.13 cases (`/tmp/opencode/macro-a13-family-final.log`).
- Integration acceptance adds
  `macro_a_13_captured_nominal_and_generic_binder_survive_generated_impl`;
  existing enum/string-match, std-frontend, artifact, and ordinary-module controls
  remain part of the full library gate.
- Final `cargo test -p rock-lib` passes 2,660 units, 798 main integrations,
  and one parser integration; one doctest is ignored. The retained log is
  `/tmp/opencode/macro-a13-acceptance-final.log` (SHA-256
  `2c4e5f48dcb11aa313377f54af71a53a31703538e2700e33f4422e424a3d655a`).
  No functional edits follow this gate.

Production fresh-template allocation and procedural construction permissions
remain A.14; the A.13 environment-recording APIs are exercised by synthetic-context
tests, not yet by production context minting. Formatting remains deferred; no
formatter/check, Clippy, or separate build/CLI gate ran. Beads remains unavailable.

### A.14 delivery receipt (2026-10-20)

Declarative templates now allocate compiler-owned fresh contexts per expansion
and inherited template context; captures retain their input contexts. Recorded
definition scopes include contextual module paths and inherited environments,
including nested generated declarations and private source-file modules. Macro
lookup, explicit imports/exports, module aliases, ordinary item/member lookup,
prelude metadata aliases, and synthesized `do` sequencing use those identities.
Generated public names require captured names or explicit call-site construction;
same-spelled helpers from separate expansions remain private and distinct.

Procedural requests expose invocation-owned symbolic `Fresh`, `CallSite`, and
declared `Dependency` construction handles. The compiler resolves them before
generated parsing and rejects cross-invocation requests, raw forged contexts,
modified/unauthorized forwarded ranges or provenance, and conflicting-context
identifier merges. Snapshot identities distinguish forwarding from equal printed
text; validated snapshots are rebound to compiler-owned storage and anchors are
rebuilt from validated data. Declared dependency handles resolve only through
supplied product identities and public export IDs, independently of caller aliases.
The lossless wire is `RKLOSS03`, worker protocol is 5, and both artifact version
owners are 51; dependencies must be rebuilt, with no compatibility reader.

Acceptance on the final functional snapshot:

- `cargo test -p rock-lib macro_a_`: 204 units and 13 integrations pass;
  `/tmp/opencode/macro-a14-family-final.log`, SHA-256
  `bc62586009a9403a868ec3eb57c78b1e563affdc71318adce0154a043dbf9cd9`.
- `cargo test -p rock-lib`: 2,683 units, 802 main integrations, and one parser
  integration pass; one doctest remains ignored. Log
  `/tmp/opencode/macro-a14-acceptance-complete.log`, SHA-256
  `44a812180640e5f3b9ae2b0cd4ca22457b77405467e3d5f1f498a8eb67ae9c0d`.
- `cargo test -p rockc -p rock -p rock-shared`: rock 86, rock-shared 26,
  rockc 48, and the shared doctest pass. The first attempt selected the cached
  schema-50 sysroot and failed its freshness check; the passing run uses std
  rebuilt by `rockup dev std package` into `/tmp/opencode/macro-a14-sysroot`,
  with explicit `ROCK_SYSROOT` and the current `ROCKC`. Log
  `/tmp/opencode/macro-a14-cli-current-sysroot.log`, SHA-256
  `778f7d6553b39d92fcabca7e55fb41992cc472db6aa6e581c082b9746e615ab4`.

No functional edits follow these gates.

Earlier gates exposed stale native-worker/source-exposure fixtures and missing
definition-side prelude/member lookups, including the unchanged `do` integration;
those are repaired, not waived. An earlier broad run timed out during integrations
and is not acceptance. Native tests exercise compiler construction permissions,
not Rock-authored proc packages; that authoring workflow remains B. Formatting
remains deferred and Beads is unavailable.

**A gate:** run all `macro_a_` families and existing declarative/proc/module
regressions, then `cargo test -p rock-lib` and affected CLI tests serially.
Synthetic/native worker tests prove compiler semantics only; they are not evidence
that a user can author/build a Rock proc package. Context delivery is not complete
if only AST equality changed while lowerer lookups still discard it.

## B. Usable Rock-authored host proc macros

Primary owners: `lib/src/macro_expansion/proc_macro.rs`, compiler config/products/
artifact owners, new ordinary Rock package `rock_macro/`, `rock-shared` manifests,
and `rock` dependency/artifact/build drivers. Keep `rockc` thin and artifact-only;
keep compiler semantics and dispatcher type checking in `lib`.

### Ba. Harden the existing one-shot worker contract

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.01 | Specify/implement the enriched version-locked Rust request/result schema and golden messages: lossless units/segments, categories, authorized handles, optional views, diagnostics, and fragment descriptors. | Full round trips and byte-exact goldens; reject truncated/trailing/oversized data, invalid tags/UTF-8/ranges/coverage, and forged handles before allocation/execution as applicable. Unknown source characters are data, not invalid wire variants; no raw-string input variant. | U |
| B.02 | Bound the whole subprocess IO deadline and handle success/error streams without unbounded joins. | Native fixtures cover partial IO, blocked stdin/stdout, oversized stdout/stderr, malformed response, nonzero exit after valid output, crash, and missing executable; success requires a valid response plus successful termination. | U, I |
| B.03 | Complete timeout/cancellation cleanup including descendants retaining pipes. Reuse the platform/process support appropriate to the execution host. | Controlled child/descendant fixtures are reaped and IO resources close within a bounded watchdog deadline on cancellation, timeout, broken pipe, and decode failure; no leaked child survives the test. Unsupported host cleanup must diagnose, not hang. | I |
| B.04 | Add no-proc-execution policy to compiler configuration and both CLI paths; enforce it immediately before any spawn. | Metadata load/inspection and declarative expansion spawn nothing; a proc use fails clearly when disabled. Test with a marker executable and wrong-host/protocol metadata; preserve OS permission errors and never launch through a shell. | U, K, R |

Do not run unbounded failure fixtures merely to discover they hang; every process
test needs its own bounded cleanup/watchdog. One fresh worker per invocation,
protocol-only stdout, bounded diagnostic stderr, and no execution on import are
requirements from the first runnable path, not deferred hardening.

### Bb. Bootstrap the SDK without macros or compiler callbacks

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.05 | Add ordinary Rock SDK data types for streams, segments, spans, contexts, results, and diagnostics matching B.01. Use explicitly supplied host dependencies. | A plain Rock program constructs/inspects owned input and result data without a proc worker, quotation macro, compiler AST, or implicitly discovered SDK/std package. Empty and Unicode cases compile and run. | I |
| B.06 | Add local cursor/slice/splice/group operations and symbolic authorized identifier/path/diagnostic constructors. | Rock programs preserve exact spelling/trivia and context handles through copies/joins; fresh versus forwarded/call-site identifiers differ correctly. Invalid ranges yield structured failures; unmatched DSL punctuation remains accessible. | I |
| B.07 | Exercise binary stdin/stdout transport with ordinary Rock IO before implementing serialization. Reuse `std::io::traits::{Read, Write}` and existing byte collections. | A plain executable reads fragmented input, handles EOF/limits, and writes exact bytes including NUL/non-ASCII. Short IO/error paths work. Any missing reusable IO helper gets its own focused upstream test, not compiler magic or a second IO framework. | I |
| B.08 | Implement bounded request decoding in Rock for the exact schema/options from B.01. | Rust-produced goldens decode in Rock with matching units, handles, categories, views, and diagnostics metadata. Invalid lengths/tags/truncation/UTF-8 fail before unchecked allocation; do not build a general serializer framework. | I |
| B.09 | Implement Rock result/diagnostic/fragment encoding interoperable with Rust validation. | Rock-produced bytes match goldens and Rust decoding for every required result variant. Test errors and empty output as well as success; field order, integer representation, and framing are explicit rather than Rust-memory-layout assumptions. | I |
| B.10 | Add the generated-runner support function: read one request, call a supplied ordinary entry, write one final response, exit. | A plain Rock identity worker processes real compiler requests using the same ABI for inline/block-shaped input. Two requests are not silently accepted, errors do not contaminate stdout, and process-global state is not reused. | I |

The first worker is intentionally built from ordinary Rock code and test-provided
entry functions. This proves the SDK/runtime seam before the source-declared
export workflow; it is not permission to require authors to write a dispatcher.

### Bc. Build proc packages through the ordinary dependency workflow

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.11 | Add `[lib].proc_macro = true` and target-role metadata to the existing manifest model. | Ordinary libraries remain unchanged; reject nonboolean flags and forbidden proc-library/bin combinations. No macro export list, new target section, or macro-dependency table appears. | S, R |
| B.12 | Parse source `macro name = ...` proc declarations alongside existing declarative definitions; discover ordinary exports after source macro expansion. Reserve derive declarations for C.02. | Exported/private helper distinction, duplicate names, reserved `derive`, and wrong-target declarations diagnose. Metadata discovery executes no provider macro; ordinary functions are not implicit macro exports. | U |
| B.13 | Generate/check the compiler-owned host dispatcher and SDK wrapper from discovered source declarations, then compile it through the ordinary pipeline. | A source-declared identity implementation needs no handwritten table/main. Wrong entry signatures fail normal type checking against the explicitly supplied SDK contract, not a compiler-known package spelling; no self-expansion bootstrap is needed. | U, I |
| B.14 | Publish/validate proc-only products with host executable identity/required files/exports, distinct from runtime object artifacts. Adapt `rockc` artifact emission/inspection by explicit product role. | Proc artifact loads with no dummy object/runtime exports. Ordinary artifact object validation is not weakened; wrong kinds/host/protocol/content identities reject before spawn. Host implementation functions cannot be imported as runtime values. | U, K |
| B.15 | Make dependency planning and artifact identities host/target-role aware using ordinary dependency declarations. | A proc's SDK/std/ordinary libraries build for the host; program libraries build for the target. Distinct simulated triples, transitive proc dependencies, and cycles are tested; host objects/SDK exports never enter the target linker. Runtime helper requirements resolve to consumer-side dependency identities. | R |
| B.16 | Wire host SDK/dispatcher build, packaging, and direct proc artifact inputs through real `rock build`/`rock run`. | Build/run an ordinary consumer importing the Rock identity macro from a direct dependency. Check root/qualified/aliased imports, invocation-only execution, missing runtime helper dependencies, and no source/bare-executable dependency injection into `rockc`. | R, I |
| B.17 | Complete role-sensitive freshness, relocation, and source-free package acceptance. | Rebuild on provider/SDK/dependency/executable changes without sharing host/target objects. Move the package and consume artifacts after removing fixture provider sources; relative paths and content checks work, provider bytes stay unchanged, obsolete/corrupt versions reject, and no worker runs on inspection. | R, K, I |

Do not widen `rock`'s dependency on compiler internals to generate semantic data:
`rock` orchestrates `rockc`, which delegates compilation/discovery/checking to
`lib`. Proc-only artifact emission must stop assuming every emitted artifact has
a normal target object, while retaining that invariant for ordinary libraries.

### Bd. Prove the actual consumers, not only an identity macro

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.18 | Extract/reuse canonical literal decoding and supply decoded-byte-to-source mappings for already Rock-recognized input literals. | Escapes/multibyte subranges map precisely. Do not scan arbitrary DSL quotes as Rock literals to populate views; no duplicate full lexer or compiler callback enters the SDK. | U, I |
| B.19 | Materialize authorized literal/generated-source fragments after worker return into lossless units, then normal capture-aware Rock parsing. | Literal holes and generated text keep contexts/subspans; invalid ranges/handles/Rock output and token-gluing attempts reject. Fragment nesting/bytes/interpretation share budgets, and copied captures are never printed/re-lexed. | U, I |
| B.20 | Add an external Rock block-DSL fixture using the generic SDK, without a production HTML implementation. | Exact HTML-shaped input includes apostrophes, unmatched quotes/brackets, comments, backticks, Unicode, whitespace, and a following dedented binding. Prove library-defined interpretation/subspan errors, expression/item results, nested forwarding, and normal rejection of DSL text emitted directly as Rock. | I, R |
| B.21 | Add a minimal external interpolation macro/helper fixture using literal views and deferred fragments. | `Hello {name}` and a `count + 1` hole resolve at caller scope; escaped/Unicode error ranges are exact. Missing names, formatting support, or runtime helper dependencies fail normally; provider/trait names are not compiler-recognized. | I, R |
| B.22 | Close interpolation's evaluation/ownership proof and host artifact acceptance for the nontrivial consumers. | Holes execute once left-to-right, named values are borrowed, temporary destruction is normal, helper/local collisions are hygienic, and calls retain consuming behavior. Repeat source-free packaged consumer tests for DSL/interpolation, not only identity. | I |

**B gate:** all `macro_b_` families, the real SDK cross-language goldens, existing
process/build/artifact tests, and the formatting/DSL proofs pass. Run the broader
serialized gates below. A Rust-only worker, protocol round trip, or manifest flag
alone is not Stage B completion. Runnable foreign-target acceptance is required
only where supported by the existing toolchain; simulated host/target identity
tests are unconditional, and any real cross-target limitation is recorded.

## C. Derives and the reusable Rock syntax library

Primary owners: parser/AST declaration token retention and attachment items,
the shared expansion engine, and new ordinary Rock package `rock_syntax/`.
The SDK must not depend on the syntax library. Compiler AST projection, full
expression/type coverage, production derives, and self-hosting stay out of scope.

### Ca. Retain declarations and define the attachment boundary

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| C.01 | Retain original declaration streams/ranges during parsing rather than reconstructing syntax from AST printing. | Struct/enum names, fields/variants, visibility, generic bounds/defaults, trivia, and context segments recover exactly from original input. Multiple consumers see one immutable original declaration; generated origins remain intact. | U |
| C.02 | Parse `%derive path, path` attachment and `macro derive Name = ...` source declarations/exports; validate target/kind metadata without running a derive. | Qualified/comma-separated lists, comments/blank lines, and consecutive directives attach correctly. Empty lists, intervening items, missing/unsupported targets, duplicate canonical derives, and Rust-style/arbitrary attributes diagnose; block capture does not steal the directive. | U |

### Cb. Grow the syntax library one grammar family at a time

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| C.03 | Add standalone Rock syntax cursor, bounded lexical/group interpretation, and parse-error data over SDK lossless streams. | Parse supplied units without `MacroContext`/worker/RPC; compare shared positive/negative lexical/group fixtures with the Rust interpretation. Preserve trivia/contexts and diagnose malformed requested Rock syntax without preventing neutral inspection. | I |
| C.04 | Parse/emission-support the initial derive type syntax needed by fixtures: names/paths, applications, references, and other explicitly enumerated forms. | Shared compiler/library type fixtures and unchanged-node round trips pass; unsupported forms and trailing tokens diagnose at the right spans. No alias resolution, inference, or operator meanings are implemented in the library. | I |
| C.05 | Parse generic binders, supported bounds/kinds/defaults, and distinguish binder syntax from type-application arguments. | Earlier-parameter/default cases and nested comma/group boundaries match the Rust parser. Emitting impl binders/type arguments is correct; unsupported grammar does not get guessed into another node. | I |
| C.06 | Add `DeriveInput` struct parsing with ordered fields, visibility, and opaque default-expression slices where boundaries are established. | Named fields, generics, defaults, exact source spans/trivia, full input consumption, malformed fields, and unsupported boundaries agree with shared fixtures; no compiler-prepared declaration structure is used. | I |
| C.07 | Extend `DeriveInput` to enums and named/positional/nullary variants supported by Rock. | Variant/field order and shape, nested types, and generic cases match the compiler; malformed/unsupported variants report structured errors. Struct handling continues to pass. | I |
| C.08 | Complete token emission/builders for transformed supported nodes and the impl syntax required by the derive proofs. Keep quote-style conveniences ordinary library helpers. | Parse/emit/parse and edited-node tests preserve unchanged slices/contexts/trivia and grouping. Captured input names bind correctly in emitted impls; construction does not stringify captured subtrees or require a quotation proc macro to build itself. | I |

Each grammar slice publishes its supported forms in library docs and adds shared
conformance fixtures. A concrete missing form needed by the next proof becomes a
small parser extension with tests; do not silently grow this into all of Syn or
add type signatures to unrelated annotation-free inference regressions.

### Cc. Dispatch and validate real derives

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| C.09 | Dispatch attachments through the same proc engine with `DeriveLike` kind and item output, retaining the declaration once and appending outputs in order. | Every derive sees identical original input; empty output is valid. Generated derives re-enter bounded discovery, consume-once is node-ID based, wrong kinds fail, and a derive cannot replace/delete the declaration. | U, I |
| C.10 | Add an external Rock derive for a small ordinary test trait on a struct using `rock_syntax`, including a generic/bound case. | The full source export/build/import/attachment/parse/emission path compiles and runs. Capture/helper collisions are hygienic; missing bounds, privacy failures, and duplicate/incoherent impls are ordinary compiler errors. | I, R |
| C.11 | Extend the external proof to enum shapes and packaged/source-free providers; exercise multiple/recursive attachments and invalid categories. | Named/positional/nullary cases and ordered derives run correctly from artifacts alone. Ordinary impl/type rules remain enforced, unsupported attribute transforms diagnose, and generic/default/visibility/token-retention controls survive real transport. | I, R |
| C.12 | Close the acceptance mapping, remove obsolete implementation paths/fixture encodings, and execute final-snapshot gates. | Every spec family maps to named tests and actual receipts; synthetic wrong-version controls remain, while no legacy reader/global registry/destructive macro input path survives. Full passes certify infrastructure, not production formatting/derive libraries or self-hosting. | All affected runners and broader gates |

## Stage gates and final verification

At each stage gate, execute the stage's proposed families plus adjacent existing
tests first. For B and final C, run these broader commands serially on the same
final source snapshot, and include any additional affected workspace target:

```bash
cargo test -p rock-lib
cargo test -p rock-shared
cargo test -p rock
cargo test -p rockc
cargo build --release
cargo clippy --workspace --all-targets
```

LLVM 18/static development dependencies remain required. Build current compiler
binaries before CLI acceptance so `ROCKC`/adjacent-binary selection cannot use a
stale compiler; use existing child-local test environment support. Rebuild
explicit std/SDK dependencies after schema or semantic changes. A dependency
rebuild is not permission for compiler-owned std discovery or compatibility
readers.

Keep test execution serialized; retain each long run's output once and inspect
that receipt rather than rerunning to recover it. Record exact commands, test
counts, warnings/failures, and final source/artifact identity in the associated
issue/evidence. Respect current log-location/access instructions; this plan does
not authorize `.agent-work/` or `.sisyphus/` inspection, formatting, or VCS changes.
Do not run formatters or formatting checks. Do not stage, commit, push, reset,
or alter unrelated existing work as part of executing this plan without current
user authorization.

### Acceptance-family mapping

This table locates where requirements are established and subsequently exercised;
it is not a completion checklist or a claim of test execution.

| Spec family | Establishing/proof slices |
| --- | --- |
| Lossless frontend | A0.01–A0.10; A.01; B.05–B.09 |
| Syntax/category | A.02–A.06; B.20; C.02/C.09 |
| Block boundaries | A0.09; A.03/A.04/A.06; B.20 |
| Declarative behavior | A.05/A.06; A.10; A.14 |
| Modules/resolution | A.07–A.10; A.13; B.12–B.17 |
| Hygiene | A0.03; A.11–A.15; B.06/B.19/B.22; C.08–C.11 |
| SDK/protocol | A.01; B.01; B.05–B.10; B.18/B.19 |
| Process failure | B.02–B.04; B.10/B.14 |
| Build/artifacts | B.11–B.17; B.22; C.11 |
| Host separation | B.14–B.17 |
| Recursion/budgets | A0.02/A0.09; A.01/A.09/A.10; B.01–B.03/B.19; C.03/C.09/C.11 |
| Formatting proof | B.18/B.19/B.21/B.22 |
| DSL proof | A.03/A.04; B.20/B.22 |
| Syntax library | C.03–C.08; C.10/C.11 |
| Derive proof | C.01/C.02; C.09–C.11 |
| Semantic integration | A.12–A.15; B.21/B.22; C.10/C.11 |
| Trust/inspection | A.07; B.02–B.04; B.14–B.17 |

### Stop and split rules

- If a slice requires a new abstraction spanning another subsystem, first split
  out the prerequisite with its own focused test; retain the stage dependency.
- If lexical/parser behavior conflicts with the spec, resolve that design
  explicitly before changing acceptance expectations. Do not secretly revert to
  Rust-token-only blocks, raw-string input variants, or a global regex prepass.
- If SDK/Rock syntax work exposes a compiler or std prerequisite, reproduce and
  track the smallest blocker in beads; fix its actual owning phase/library rather
  than adding compiler-known SDK names, inferred-type callbacks, or fixture-only
  language bypasses.
- If a gate fails, the stage is not complete. Preserve existing semantic negatives,
  source fixtures, and evidence; report the blocker instead of weakening tests.
- Keep full raw-source scanning in Rock, full Syn-scale grammar, macro facades,
  persistent workers/caching, general attributes, production HTML/formatting/
  derives, and the self-hosted compiler rewrite outside these slices.

## First implementation boundary

Start with **A0.01 only**: establish the small lexical/parser conformance corpus
and its focused passing baseline. Then A0.02 introduces neutral preservation
behind unit tests. Neither first slice changes invocation syntax, the macro SDK,
artifact schemas, resolution, or production compiler selection.
