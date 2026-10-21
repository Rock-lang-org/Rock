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

After B.22, native SDK conformance and cold CLI package/freshness/relocation
acceptance are opt-in via `cargo macro-acceptance`, run manually and serially.
The former B.13-named SDK monolith is split into `macro_acceptance_sdk_*` groups;
their commands and prerequisite reuse are documented in [compiler testing](../testing.md).
Historical receipts below record the commands used at the time; add `--ignored`
when selecting the now-ignored CLI acceptance tests. Default suites retain
focused semantic/process regressions and a native one-shot SDK smoke test.
This test-orchestration change does not begin Stage C or change worker isolation.

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

### A.15 delivery receipt (2026-10-20)

The shared frontend now owns dependency loading, generated-file discovery and
expansion for both normal compilation and `rock_lib::expand`; `rockc --expand`
passes its actual dependency/name/permission configuration into that API instead
of rebuilding a dependency-free context. Inspection stops before collection and
typing, includes loaded file modules, and never reparses printed expansion text.
The existing `rock expand` launcher uses this compiler path.

Expanded parsing projects spans into per-expansion logical output snapshots while
retaining physical source snapshots, original provenance, contexts, join boundaries
and opaque capture ranges. Projection addresses are namespaced by compilation
source/name so an imported body's expansion number cannot alias a consumer's.
Worker forwarding validates projection metadata alongside the original provenance.
The retained map follows the AST through analysis, lowering, inference,
monomorphization, borrow checking and codegen error conversion: caller captures
map back to caller ranges; generated primary errors point at their invocation,
with nested ancestry, definition ranges and inspectable output labels. Mixed
source extents remain explicit output snapshots rather than guessed source ranges.
Diagnostic codes and non-source backend/toolchain failures remain unchanged.
`TokenOrigin` separates provenance from projection; lossless wire `RKLOSS04`,
worker protocol 6 and product format 52 require rebuilt dependencies, with no
compatibility decoder.

Acceptance on the final functional snapshot:

- `cargo test -p rock-lib`: 2,688 units, 803 main integrations, six macro
  diagnostic integrations, and one parser integration pass; one doctest ignored.
  `/tmp/opencode/macro-a15-acceptance-terminal.log`, SHA-256
  `f9a42d07679c832588f134c86c5d090dc4d98a49a92731c95fe207e2c0a448a3`.
- `cargo test -p rock-lib macro_a_`: 209 units, 14 main integrations and six
  diagnostic integrations pass. `/tmp/opencode/macro-a15-family-terminal.log`,
  SHA-256 `91d89f418f9efe83d62445ca8524ac97e489ac4f44bb040d1568b100ce8f27fe`.
- `cargo test -p rockc -p rock-shared`: 50 compiler CLI units, 26 shared units,
  and one shared doctest pass. `/tmp/opencode/macro-a15-compiler-cli.log`, SHA-256
  `088c3322c2c6d1d89ba65224241d0520c4d33884763a6fc4d7425f22cbe0fc72`.
- `cargo test -p rock -p rock-lsp`: 86 application CLI and five LSP units
  pass, using current `ROCKC` and std rebuilt with `rockup dev std package` in
  `/tmp/opencode/macro-a15-sysroot` via explicit `ROCK_SYSROOT`.
  `/tmp/opencode/macro-a15-user-cli.log`, SHA-256
  `8891aa4af7dd8ff8954ea64713a731ebf6c5b283cf616a7d4239d31560626d2d`.

No functional edits follow these gates.

The new runtime control compares handwritten and expanded `return`/`break`
with user destructors; unsafe calls and conflicting loans still reject. Nested
type/resolution/parse errors and secondary ownership labels have retained source
snapshots. Codegen diagnostic remapping is exercised directly with a source-coded
backend diagnostic; no deliberate compiler backend defect is claimed as an
end-to-end codegen failure. Early candidate runs exposed a missing test-source
snapshot, a stale equal-diagnostic-span assertion, and a fixture incorrectly
assuming loops return break values; final gates above supersede those attempts.
Formatting remains deferred and Beads remains unavailable.

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

### B.01 delivery receipt (2026-10-20)

The enriched one-shot schema is specified in
[`2026-10-19-macro-worker-wire.md`](2026-10-19-macro-worker-wire.md).
Requests carry kind/category, invocation/layout/dependency handles, shared
lossless input and optional argument/group/literal views. Results carry ordered
lossless/deferred-fragment parts and structured diagnostics together, including
successful warnings. There is no raw-string input alternative, AST projection,
worker-supplied diagnostic path, or callback channel. Output-part splices do not
silently glue even contiguous forwarded spelling; explicit adjacency within a
lossless stream remains retained.

Protocol 7 and product format 53 replace their predecessors without compatibility
readers. Rust codecs retain bincode's existing limited/default-options encoding,
validate version headers before payload decoding, bound declared sequence counts
before element allocation, and reject malformed UTF-8, tags, trailing/truncated
data, invalid view/mapping coverage, zero/duplicate snapshot identities, invalid
layout and forged/cross-invocation/dependency/literal/diagnostic handles. Returned
streams are reauthorized against the original compiler input before generated
parsing. Warning reports retain severity/labels/notes/help through expansion and
analysis APIs, compiler/expansion CLIs, and successful LSP analysis.

Three byte-exact shared hex goldens under `lib/tests/fixtures/macros/protocol-7/`
cover the enriched request, success-with-warning/fragment result, and failure
result. They fix only fixture handle values, not runtime identities. Canonical
literal preparation remains B.18 and deferred fragment materialization B.19;
using those output descriptors before that implementation fails explicitly.
The schema tests do not establish the later Rock SDK or package workflow, and
whole-process deadline/descendant hardening remains B.02/B.03.

Final functional snapshot acceptance:

- `cargo test -p rock-lib macro_b_01_`: ten units pass. Log
  `/tmp/opencode/macro-b01-focused-terminal.log`, SHA-256
  `7f6b9e2a17874c0bd35d0b21e124c3d0c6a1189453caeecdb336e7cf2188b6a2`.
- `cargo test -p rock-lib`: 2,698 units, 803 main integrations, six macro
  diagnostic integrations and one parser integration pass; one doctest ignored.
  `/tmp/opencode/macro-b01-acceptance-terminal.log`, SHA-256
  `384ec631653d62295de66dae413af620750c533f52f2bd7a9151bc3029371911`.
- `cargo test -p rockc -p rock -p rock-shared -p rock-lsp`: 50 compiler CLI,
  86 application CLI, 26 shared, and five LSP units plus one shared doctest pass.
  Current `ROCKC` and explicit `ROCK_SYSROOT=/tmp/opencode/macro-b01-sysroot`
  use std rebuilt by `rockup dev std package`. Log
  `/tmp/opencode/macro-b01-cli-terminal.log`, SHA-256
  `b1848887bc344362bf852a5548422f75f96f9ca51fceed213fa443428aa00e78`.

No functional edits follow these gates.

Earlier candidates exposed an unchanged A.14 negative's diagnostic wording and
a synthetic fixed-handle test accidentally choosing the same fresh invocation;
both are repaired without weakening authority negatives. Final review added the
explicit output-part non-gluing regression, and the terminal gates supersede the
preceding passing candidate. Formatting remains deferred and Beads unavailable.

### B.02 delivery receipt (2026-10-20)

Worker transport now pumps nonblocking Unix stdin/stdout/stderr directly under
one invocation deadline, including request encoding/writes, output reads,
response validation and process termination. There are no transport IO threads
or blocking joins. Only a valid response plus successful exit is success;
malformed output, broken request IO, stream limits, crashes, nonzero exit after
valid output and unfinished shutdown fail with invocation/export identity and
bounded stderr context. Native OS permission/spawn errors remain visible.

Each invocation gets a private process group and an ownership guard. Exit is
observed with `waitid(WNOWAIT)` so group cleanup precedes leader reaping and
cannot signal a reused group ID. All error paths close pipes, terminate the
group and reap the leader. The already installed libc dependency is now a Unix
runtime dependency for these platform primitives; unsupported execution hosts
diagnose before spawning rather than retaining an unbounded thread fallback.
Explicit cancellation and dedicated descendant/resource acceptance remain B.03;
this is not a native-code sandbox or permission to escape the worker group.
Protocol 7/product format 53 and shared golden bytes are unchanged.

Six focused units include a compiled native C fixture for fragmented request/
response IO, blocked stdin/stdout/shutdown, broken pipe, oversized stdout/stderr,
malformed response, valid response followed by exit 7, crash, missing executable,
and an expired pre-spawn deadline. Every process case has a separate three-second
test watchdog and worker alarm; return checks prove the direct worker is gone
and reaped. Stalls are exercised only after the fixture's PID marker exists.

Final functional snapshot acceptance:

- `cargo test -p rock-lib macro_b_02_`: six units pass.
  `/tmp/opencode/macro-b02-focused-final.log`, SHA-256
  `a0cfb85e5451fb3e0a3626737dd56e176c5441ffd1ce0eafebf5ce2927148d15`.
- `cargo test -p rock-lib`: 2,703 units, 803 main integrations, six macro
  diagnostic integrations and one parser integration pass; one doctest ignored.
  `/tmp/opencode/macro-b02-acceptance.log`, SHA-256
  `3f8a9e3c66779abb640a1db516ee0b7a22ec4822d73ab27570d4f06920bb04da`.
- `cargo test -p rockc -p rock -p rock-shared -p rock-lsp`: 50 compiler CLI,
  86 application CLI, 26 shared and five LSP units plus one shared doctest pass,
  with rebuilt current `ROCKC` and the unchanged format-53 std at
  `/tmp/opencode/macro-b01-sysroot` selected by `ROCK_SYSROOT`.
  `/tmp/opencode/macro-b02-cli.log`, SHA-256
  `884f9098789df3da5990ce67bdac3cfd178cae4f49bfdec4289a8013fa89ed19`.

The earlier 29-unit process/schema regression pass precedes the final large
fragmented-IO fixture and error-identity wording; terminal gates above cover
those changes. No functional edits follow the terminal gates. Formatting remains
deferred and Beads unavailable.

### B.03 delivery receipt (2026-10-20)

Native execution accepts an optional host-owned atomic cancellation flag;
`MacroExpansionContext::with_cancellation` carries it to the runner. Checks before
encoding/spawn, throughout the nonblocking IO loop and after validation use the
same ownership guard as deadline/IO/decode failures. Pre-cancelled expansion
spawns no worker. No wire metadata, package discovery, compiler-global signal
handler, detached IO thread or cancellation-specific process framework is added.
The B.02 private process-group cleanup still precedes leader reaping; unsupported
execution hosts reject before spawn. Protocol 7/product format 53 are unchanged.

Native process tests now live in `lib/tests/macro_process.rs`, exercising the
public API rather than a compiler-only unit seam. Alongside the five existing
B.02 native cases, six B.03 integrations cover descendants retaining stdout/stderr
on success, malformed response, timeout, broken request and cancellation;
pre-cancelled expansion; cancellation during blocked input, response and shutdown;
and isolated repeated cleanup with stable Linux descriptor/thread counts. Every
recorded direct worker must be gone and reaped, and every recorded descendant
must disappear within a bounded check. Independent three-second per-run
watchdogs, worker alarms and a 20-second isolated-resource watchdog bound regressions.
Native code remains trusted host code; intentionally escaping the process group
is outside the supported worker lifecycle.

Final functional snapshot acceptance:

- `cargo test -p rock-lib --test macro_process`: all 11 integrations pass,
  including the isolated repeated-run child. Log
  `/tmp/opencode/macro-b03-process-candidate.log`, SHA-256
  `87518cfb492b1ac3abacb3896547f13b92e11804677c2444e18f277b7de2f43d`.
- `cargo test -p rock-lib`: 2,698 units, 803 main integrations, six diagnostic
  integrations, 11 native-process integrations and one parser integration pass;
  one doctest ignored. Five B.02 native units moved to the integration target,
  accounting for the lower unit count. Log
  `/tmp/opencode/macro-b03-acceptance.log`, SHA-256
  `21b02767100a7a51b8bfa480ba0c391b97cb27f568be701dca0c1a9a20227743`.
- `cargo test -p rockc -p rock -p rock-shared -p rock-lsp`: 50 compiler CLI,
  86 application CLI, 26 shared and five LSP units plus one shared doctest pass,
  using rebuilt current `ROCKC` and unchanged format-53 std through explicit
  `ROCK_SYSROOT=/tmp/opencode/macro-b01-sysroot`. Log
  `/tmp/opencode/macro-b03-cli.log`, SHA-256
  `6b7850aff5e936574a5ab08332f07c787d72329fc9ea8dc5ff4b53769e79c90e`.

No functional edits follow these gates. Formatting remains deferred and Beads
unavailable; unrelated untracked work remains untouched.

### B.04 delivery receipt (2026-10-20)

`Config::no_proc_execution` and `--no-proc-execution` on `rockc` and `rock`
leave metadata/declarative processing available while disabling native workers.
The application CLI accepts the global flag for build/run/expand, propagates
it through root/library/transitive dependency invocations, and preserves program
arguments after `rock run --`. Native spawn checks the immutable host policy
independently of protocol/host/executable metadata; expansion also diagnoses
disabled item/expression uses at their invocation. No worker path launches
through a shell, and enabled execution retains OS permission/format errors.

Artifact inspection and declarative expansion are tested with registered native
marker workers under both policy settings. Protected use, wrong-protocol and
wrong-host metadata, missing executables and native format/permission errors
leave markers absent. Compiler CLI tests consume a current-format synthetic
artifact through both compile/expand paths; the fixture includes its required
local product crate ID. Application CLI tests verify global parsing, root and
dependency argument propagation, and actual expand-launcher forwarding.

Protected builds can reuse prebuilt artifacts and sysroots, but never silently
invoke the separate developer std packager without this policy; if automatic
packaging is needed, they explicitly request a prebuilt `ROCK_SYSROOT` instead.
This policy does not block the compiler/linker or the requested runtime program,
and is not a native-code sandbox. Source-declared proc packaging and complete
host-role validation remain B.11–B.17. Protocol 7/product format 53 are unchanged.

Final functional snapshot acceptance:

- `cargo test -p rock-lib -p rockc -p rock macro_b_04_`: four application CLI
  units, two compiler CLI units and three native-process integrations pass.
  `/tmp/opencode/macro-b04-focused-certified.log`, SHA-256
  `100e783e48135294e945ae941f7b4d3f81cf24f778640d9ebed9a671468a26b3`.
- `cargo test -p rock-lib`: 2,698 units, 803 main integrations, six diagnostic
  integrations, 14 native-process integrations and one parser integration pass;
  one doctest ignored. `/tmp/opencode/macro-b04-acceptance.log`, SHA-256
  `950a082a8b8b40050a1f84424f133672cf7e2af3be23f7b8492c1ce45760823b`.
- `cargo test -p rockc -p rock -p rock-shared -p rock-lsp`: 52 compiler CLI,
  90 application CLI, 26 shared and five LSP units plus one shared doctest pass.
  Rebuilt current `ROCKC` uses unchanged format-53 std through explicit
  `ROCK_SYSROOT=/tmp/opencode/macro-b01-sysroot`. Log
  `/tmp/opencode/macro-b04-cli.log`, SHA-256
  `bed2d6fbd39b4286dd04fd1f700cf577b13dc875d2b238dc065f758c5ef1285d`.
- `cargo check --workspace --all-targets` passes with existing warnings. Log
  `/tmp/opencode/macro-b04-check-final.log`, SHA-256
  `512006935ed4ca13295292d65a1044b5d6b044290aee44b0e4129caddc718f06`.

Earlier focused attempts caught missing Rust configuration-field initializers,
a missing fixture import, and the synthetic provider's missing local crate ID;
the final gates supersede them. No functional edits follow these gates.
Formatting remains deferred, Beads unavailable, and unrelated work untouched.

### Bb. Bootstrap the SDK without macros or compiler callbacks

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.05 | Add ordinary Rock SDK data types for streams, segments, spans, contexts, results, and diagnostics matching B.01. Use explicitly supplied host dependencies. | A plain Rock program constructs/inspects owned input and result data without a proc worker, quotation macro, compiler AST, or implicitly discovered SDK/std package. Empty and Unicode cases compile and run. | I |
| B.06 | Add local cursor/slice/splice/group operations and symbolic authorized identifier/path/diagnostic constructors. | Rock programs preserve exact spelling/trivia and context handles through copies/joins; fresh versus forwarded/call-site identifiers differ correctly. Invalid ranges yield structured failures; unmatched DSL punctuation remains accessible. | I |
| B.07 | Exercise binary stdin/stdout transport with ordinary Rock IO before implementing serialization. Reuse `std::io::traits::{Read, Write}` and existing byte collections. | A plain executable reads fragmented input, handles EOF/limits, and writes exact bytes including NUL/non-ASCII. Short IO/error paths work. Any missing reusable IO helper gets its own focused upstream test, not compiler magic or a second IO framework. | I |
| B.08 | Implement bounded request decoding in Rock for the exact schema/options from B.01. | Rust-produced goldens decode in Rock with matching units, handles, categories, views, and diagnostics metadata. Invalid lengths/tags/truncation/UTF-8 fail before unchecked allocation; do not build a general serializer framework. | I |
| B.09 | Implement Rock result/diagnostic/fragment encoding interoperable with Rust validation. | Rock-produced bytes match goldens and Rust decoding for every required result variant. Test errors and empty output as well as success; field order, integer representation, and framing are explicit rather than Rust-memory-layout assumptions. | I |
| B.10 | Add the generated-runner support function: read one request, call a supplied ordinary entry, write one final response, exit. | A plain Rock identity worker processes real compiler requests using the same ABI for inline/block-shaped input. Two requests are not silently accepted, errors do not contaminate stdout, and process-global state is not reused. | I |

### B.05 delivery receipt (2026-10-20)

`rock_macro/` is an ordinary Rock library with an explicit path dependency on
`std`, not a compiler-registered SDK or proc target. Its exported owned data
models the B.01 contract: snapshot-indexed lossless segments/anchor/opaque ranges,
physical provenance and diagnostic projections, symbolic construction contexts,
invocation/category/layout/dependency facts, optional group/argument/literal
views, output-only fragment descriptors, and structured diagnostics/results.
`MacroResult::Expand`/`Error` and the warning-capable `Report` are one authoring
result contract, to be normalized by the B.09 encoder into B.01's single result
record. Transport indices/ranges are explicitly `U64`; narrower handles retain
their declared widths. There is no raw-string input variant, AST projection,
quotation facility, worker dispatcher, implicit package discovery or binary codec.

`lib/tests/macro_sdk.rs` builds std and SDK host artifacts explicitly, asserts
both are ordinary libraries with no proc exports, then compiles/runs a plain
Rock client through those artifacts with procedural execution disabled. The
client constructs and inspects empty/Unicode streams, contexts, all provenance
forms, projections, literal mappings, dependencies, output fragments, warnings
and errors. Separate negative configurations omit SDK or std rather than relying
on a sysroot/discovered source package. The fixture uses the ordinary entry
contract `MacroContext, TokenStream -> MacroResult`, not a macro invocation.

Local operations remain B.06, binary IO/codecs/generated runner support B.07–B.10,
host-role planning B.15 and source-free package acceptance B.17. This receipt
does not certify any of those later features or Rust/Rock wire interoperability.

Final functional snapshot acceptance:

- `cargo test -p rock-lib`: 2,698 units, 803 main integrations, six diagnostic
  integrations, 14 native-process integrations, the new SDK integration and one
  parser integration pass; one doctest ignored. This includes independently
  omitting each explicit dependency. `/tmp/opencode/macro-b05-acceptance.log`,
  SHA-256 `b6a6fd6d1b859ab0cfa560df591f743d109ac0e533e85d6164f509e5568de947`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b05-check.log`, SHA-256
  `e97f001b2ec210ad4da248147e0938f47945268d3fb13abf8be32c137de19d27`.

The earlier focused `cargo test -p rock-lib --test macro_sdk` passed before
strengthening the independent missing-std negative; its log
`/tmp/opencode/macro-b05-sdk-final.log` has SHA-256
`31d7487f2951b99ba29d7cbe52dfa9d9bb1bfd70d8124abb7b02da24851fbcc9`.
Initial fixture attempts exposed the lack of unsigned `Show` impls and contextual
generic integer payloads defaulting to signed widths (one reached LLVM verifier
rejection). The fixture now uses explicit protocol widths and signed display
casts; no compiler numeric-coercion repair is claimed. A standalone probe built
successfully, but its direct executable launch was blocked by shell policy and
was not retried or counted as runtime acceptance. Runtime acceptance is the
standard Cargo integration, including its exact UTF-8 output comparison.

No compiler/CLI production code or artifact/wire format changes belong to this
slice; B.04 CLI gates remain unchanged, not rerun evidence for new SDK workflows.
No functional edits follow the final gates. Formatting remains deferred, Beads
unavailable, and unrelated untracked work untouched.

### B.06 delivery receipt (2026-10-20)

`rock_macro` now supplies explicit deep copies, checked byte positions/slices,
identity-remapped stream joins, and an owned cursor over UTF-8 scalars or opaque
expression captures. These operations preserve spelling/trivia, retained source
identities, context handles, provenance, diagnostic projections and splice
boundaries. Invalid ranges, UTF-8/capture cuts, conflicting snapshots and invalid
adjacency return `operations::SdkError`. Local validation retains the transport
bounds: 16 MiB source/logical bytes, 1,048,576 entries and capture depth 128.

One `MacroBuilder` per invocation emits symbolic fresh/call-site identifiers,
definition-dependency paths and delimiter groups. Cloned identifiers preserve
their symbolic fresh identity; separate fresh requests receive separate handles.
Forwarded identifiers retain their input context and reject mixed contexts,
separate boundaries or construction requests. The deliberately small name/path
helpers accept ASCII names; arbitrary Unicode/DSL input remains available through
preserved stream slices. Group views check supplied delimiter endpoints without
certifying nested syntax. Diagnostic builders carry checked input ranges or
invocation locations, never additional binding authority. The compiler remains
the authority for returned handles and dependency exports.

The runtime fixture covers copies, Unicode/trivia slices, source identity joins,
origin/projection preservation, empty streams, unmatched DSL punctuation, opaque
captures, rejected ranges/contexts/adjacency, symbolic identities, dependency
paths, groups, diagnostics and exhausted source identities. Its native execution
is bounded to 15 seconds; the unchanged B.05 data/output and independent missing
SDK/std controls remain in the same explicit-artifact integration.

Runtime acceptance exposed an existing MIR initialization bug: `if`/`match`
merges marked destinations initialized without a value write. A discarded match
with a unit arm consequently dropped uninitialized aggregate storage and hung.
Branch assignments now own destination initialization; the dedicated MIR
regression requires exactly the two branch writes, with no initialization at the
merge. The original SDK mixed-context/separated-name negatives remain intact.
The rebuilt focused runtime probe passes; temporary probe/tracing code was
removed. The exact MIR regression passes as well.

Verification evidence:

- `cargo test -p rock-lib --lib mir::builder::tests::branch_destinations_are_initialized_only_by_value_writes -- --exact`
  passes (one test). `/tmp/opencode/macro-b06-branch-regression.log`, SHA-256
  `7321da84fe91471ed0af3d6a65e4bf4bd4734ca79e3bbbbacba7d77d46d04dbf`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b06-check-final.log`, SHA-256
  `59bb5bfb1de4af5155a569ebfc4ff50c94589a286efd2c316f86538206ab99a0`.
- `cargo test -p rock-lib` passes: 2,699 units, 803 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK integration
  (113.27 seconds) and one parser integration; one doctest ignored.
  `/tmp/opencode/macro-b06-acceptance.log`, SHA-256
  `91cbf058c0474355cc556165d60c16ec36b158d1000b3330f8f1bdf8383d3a44`.

No wire/artifact format, serialization, worker registration or procedural package
workflow is added here. Canonical literal preparation and deferred fragment
materialization remain B.18/B.19. Formatting remains deferred and Beads unavailable.
No functional edits follow these final gates; unrelated work remains untouched.

### B.07 delivery receipt (2026-10-20)

The ordinary IO seam uses existing `Read`, `Write`, `Stdin`, `Stdout` and `Vec U8`.
The missing reusable operation is now `std::io::traits::read_to_end_limited`,
with its own upstream integration test. It reads through EOF with a fixed 4 KiB
scratch buffer and checks counts before copying into the result. A one-byte probe
beyond the cap distinguishes exact-limit EOF from oversized input without
unchecked length arithmetic or allocation based on untrusted input lengths.
Negative limits, excess input and invalid read counts yield
`IoError::InvalidInput`; underlying read errors propagate unchanged, including
after partial progress. Errors do not expose a partially accumulated vector.

The plain `sdk-io.rk` executable depends only on an explicitly supplied std
artifact. It caps each underlying stdin read at three bytes, then writes through
the existing `write_all` helper with writes capped at two bytes. The Rust harness
feeds two-byte fragments and compares exact output for empty input, embedded NUL,
multibyte UTF-8, a non-UTF-8 byte and an exact 32-byte limit. A 33-byte request must
exit with status 2 and no partial stdout. The upstream read controls also cover
short reads, zero/negative/extreme limits, EOF, overreported/negative counts and
IO failures; separate write controls preserve errors after short-write progress
and retain the existing zero-write test. No string conversion handles transport.

Verification evidence:

- Focused `cargo test -p rock-lib --test macro_sdk -- --nocapture` passes, including
  all B.05/B.06 controls and the new binary IO executable (114.72 seconds).
  `/tmp/opencode/macro-b07-sdk-io.log`, SHA-256
  `d0f22ab180188758bed324f4c69f1cad37f9949497320dce1ec94567917f5d30`.
- `cargo test -p rock-lib --test integration test_write_ -- --test-threads=1`
  passes both partial-progress error and existing short/zero-write controls.
  `/tmp/opencode/macro-b07-write-controls.log`, SHA-256
  `f2a5f8acde2e131408fa598273c0c86de7db4b20cfe7c7d734b5c89e97776536`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b07-check-final.log`, SHA-256
  `f2175bdd73825ae60e1914c665e75ed9322dc06bc952065898bd0e900de0f848`.
- `cargo test -p rock-lib` passes: 2,699 units, 805 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK/IO integration
  (114.68 seconds) and one parser integration; one doctest ignored. This includes
  the strengthened partial-read-error control and synchronous-input harness bound.
  `/tmp/opencode/macro-b07-acceptance.log`, SHA-256
  `f2c4cb1a0a9a079f7dd0563232b83e5de615c3e563f097cb9d19f58a69a7d735`.

No codec, extra IO framework, compiler callback, proc-worker dispatcher or wire-format change is
introduced. B.08/B.09 remain the request/result codec steps, followed by the
one-shot runner in B.10. Formatting remains deferred and Beads unavailable.
No functional edits follow these final gates; unrelated work remains untouched.

### B.08 delivery receipt (2026-10-20)

The ordinary SDK now exports `decode::decode_request`, taking owned bytes and an
explicit expected compiler version. The decoder implements only protocol 7's
declaration-order fields, unsigned bincode varints/options and nested `RKLOSS04`;
it is not a general serialization framework or a compiler callback. Both the
envelope and nested payload are capped at 1 MiB, counts are checked against schema
caps and remaining bytes before allocation, and string conversion validates UTF-8.
Invalid tags, integer narrowing, truncation, versions and trailing data produce a
typed `DecodeErrorKind` plus byte offset.

Local validation reuses `TokenStream::len_bytes!` for snapshot/range/adjacency and
capture invariants, then checks neutral run classification, diagnostic projection
arithmetic, no input construction requests, invocation/layout handles, duplicate
dependency identities and input views. View ranges are checked against logical
UTF-8 boundaries, not capture slicing: a literal/argument view may lie inside an
opaque captured expression. The Unicode whitespace table follows the Rust
transport; CR/LF/CRLF remain physical line-ending runs. No binding authority is
allocated by this library operation.

The explicit-artifact native acceptance now feeds the exact Rust request golden,
every truncated prefix, enriched source/captured/generated provenance, projection
and dependency records, all integer widths, all macro kinds/categories, empty
input, and malformed schema/semantic controls to the Rock decoder. The fixture
checks decoded fields and typed failure offsets. Test-owned wire mirrors only
construct malformed lossless messages; they introduce no production alternative
representation. The harness writes stdin concurrently with its 15-second process
deadline, including messages above pipe capacity and early worker rejection;
B.07's two-byte write fragments and exact binary controls remain in that gate.

Verification evidence:

- The first focused native run passes 195 Rust/Rock request controls, preserving
  B.05/B.06/B.07 acceptance (145.81 seconds).
  `/tmp/opencode/macro-b08-sdk-test.log`, SHA-256
  `d5d0ca42f2c5b5ebb434c2ef541e6e0eb74d4029b1d7cfefb4474364f18d7d3a`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b08-check.log`, SHA-256
  `a03e9a3774d51a028827e7830871c91b8e1af68df89719041d4d68184baea571`.
- `cargo test -p rock-lib` passes: 2,699 units, 805 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK integration
  (149.13 seconds) and one parser integration; one doctest ignored. This includes
  the additional tag/count/narrowing/optional/error-kind controls.
  `/tmp/opencode/macro-b08-acceptance.log`, SHA-256
  `c1eb492f4358803dd87d5ca815cd0bba5e78c976c8d895a1722e214cdd72fd42`.
- `cargo clippy --workspace --all-targets` passes with existing warnings and no
  warning in the new SDK test harness/support.
  `/tmp/opencode/macro-b08-clippy.log`, SHA-256
  `023d463f7697484faf80526ced52eed8885fc5c52636230b4b2942eff290f8d5`.
- Final `cargo test -p rock-lib --test macro_sdk -- --nocapture` passes all 206
  Rust/Rock request controls and preserved SDK/IO controls (147.00 seconds).
  Only the fixture's oversized-envelope control changed after the full gate:
  its IO cap retains one extra byte, then asserts the SDK decoder itself returns
  `LengthLimit` at offset zero. The decoder and all other functional code are
  unchanged from the full gate.
  `/tmp/opencode/macro-b08-sdk-final.log`, SHA-256
  `bd8534511f35fd16e2fcfa44fb0bb134575644a0583cec1ff3a58f64b93254cb`.

No worker runner, result encoder, literal preparation or fragment materialization
is introduced: those remain B.09/B.10/B.18/B.19. Protocol 7, `RKLOSS04` and product
artifact 53 are unchanged. Formatting remains deferred and Beads unavailable.
No functional edits follow the final focused gate; unrelated work remains untouched.

### B.09 delivery receipt (2026-10-20)

`encode::encode_result` now normalizes all three authoring variants into the
single protocol-7 optional-output/diagnostics record and explicitly writes its
declaration-order fields, varints, options and enum tags. Stream parts encode the
same `RKLOSS04` snapshot/run/anchor/capture representation; source and literal
parts retain their typed descriptors. No Rust memory layout, generic serializer,
compiler callback or fragment scanner participates.

The writer checks the 1 MiB envelope before byte-buffer growth and returns only
complete owned responses. Shared `validation.rk` preserves B.08 request checks
and permits symbolic output construction while retaining stream invariants.
Output validation checks invocation/dependency contexts, literal handles and
UTF-8 ranges, expanded spelling, metadata counts and diagnostic locations.
`None` output requires an Error diagnostic; `Some` with zero parts is an empty
success. Error/warning reports may accompany output without changing the wire
shape. Only the compiler can reauthorize forwarded snapshots/contexts/origins:
the native forged-context control encodes ordinary data, then Rust's
request-specific validator rejects it.

The native fixture compares exact `response.hex` and `error.hex` bytes, including
warning labels/notes/help, all construction kinds and literal fragments. Its 25
result controls cover Expand/Error/Report normalization, empty parts/streams,
all severities/locations, NUL/Unicode, forwarded enriched provenance, wide
integers/65,536-byte messages, constructed streams, and invalid locations,
contexts, literal handles/cuts, streams, missing errors and spelling/metadata/
encoded-message limits. Failed encoders emit no partial stdout. All 206 B.08
request controls and earlier SDK/IO controls remain intact. Capped concurrent
stdout/stderr drains now accompany stdin pumping, so responses above pipe
capacity cannot deadlock the fixture's 15-second execution deadline.

The first combined result-dispatch fixture overflowed the compiler's Rust test
thread stack; splitting the same cases into small success/error helper functions
preserves coverage and passes without changing compiler stack configuration.
This is not a general compiler stack-safety claim. Initial CLI probes also
exposed a stale pre-B.07 std artifact and fixture-only borrow/signature mistakes;
the acceptance rebuilds explicit std/SDK artifacts from current sources.

Verification evidence:

- `cargo test -p rock-lib --test macro_sdk -- --nocapture` passes 206 request
  controls and 25 result controls plus all preserved SDK/IO controls (159.68
  seconds). `/tmp/opencode/macro-b09-sdk-test.log`, SHA-256
  `973146ed5fc256f02aff8684be8ad9f37a4cc5c864f49557879d8de5eaee16c2`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b09-check.log`, SHA-256
  `753a137f56c732b42d7b561c1a37f2ed88bafdcf200ca5645fce1b0f448ea23b`.
- `cargo clippy --workspace --all-targets` passes with existing warnings and no
  warning in the SDK Rust harness/support.
  `/tmp/opencode/macro-b09-clippy.log`, SHA-256
  `971c16d7596c0f9e9ad7a3184cb7da42e36b8f77ad0da1e5902ebb8738ffe40b`.
- `cargo test -p rock-lib` passes: 2,699 units, 805 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK integration
  (162.31 seconds) and one parser integration; one doctest ignored.
  `/tmp/opencode/macro-b09-acceptance.log`, SHA-256
  `d158392ba55d63d817cf7b43d229671f4305c4d1d08a9a705d351c6aaa4bb54b`.

The one-shot runner remains B.10; canonical literal preparation/materialization
remain B.18/B.19. Protocol 7, `RKLOSS04` and artifact 53 are unchanged. Formatting
remains deferred, Beads unavailable, and unrelated work untouched.
No functional edits follow these final gates.

### B.10 delivery receipt (2026-10-20)

`runtime::run_worker` now reads one bounded request through ordinary std IO,
decodes against an explicitly supplied compiler version, invokes one typed
`MacroContext -> TokenStream -> MacroResult` entry, encodes/writes one final
response, and exits the process. There is no persistent dispatch loop, compiler
callback or handwritten export table in the SDK. The private `run_once` propagates
typed IO/decode/encode failures; the outer function reports only static phase
messages on stderr and exits nonzero. A valid structured macro error remains a
normal final protocol response. Input/result codecs run before entry/output IO
respectively, so invalid requests never invoke the entry and invalid results
emit no partial protocol response.

The runtime retains the original request for output checks and supplies deep-owned
context/input copies to the entry. Explicit Clone implementations now cover the
context's enums, optional facts, dependency fingerprints and input views, using
the existing owned-copy style. The native mutation control changes nested view
ranges, dependency names and input source text in author-owned data; the original
request still validates. Missing stderr IO is added upstream as ordinary
`std::io::traits::{Stderr, stderr}` implementing existing Write helpers, with its
own native integration proving NUL/Unicode stderr bytes never mix into stdout.

The native SDK worker's 14 direct controls preserve empty/Unicode/opaque/DSL input,
inline-expression and block-shaped records, wide metadata and structured errors.
Repeated identical requests produce distinct PIDs, each entry logs exactly once,
and a trap after `run_worker` never executes. Empty/truncated/wrong-version,
concatenated and oversized requests fail before the entry marker. Invalid result
encoding fails after exactly one marker with clean stdout. Three additional
controls run through the real Rust compiler host: one golden request and exact
parser-produced inline-expression/block invocation metadata, checked against the
returned lossless streams and requested categories. Existing 206 request and 25
result controls remain intact.

These are worker/ABI proofs, not premature language-dispatch or source-package
acceptance. Initial expansion probes confirmed the explicit procedural-expression
placeholder in `expression.rs::operand` and legacy item argument capture of `!`
in `macro_decl.rs::macro_invoc`. The runner deliberately does not normalize block
margins or reinterpret identity output as valid Rock; procedural expression/DSL
consumer integration and obsolete-path removal remain required in B.20/C.12.
Generated source export wrappers/build roles remain B.11–B.17. No such compiler
path was changed or silently bypassed to claim this runtime step.

Verification evidence:

- `cargo test -p rock-lib --test integration test_stderr_uses_ordinary_write_helpers_without_stdout_contamination -- --exact`
  passes (one test). `/tmp/opencode/macro-b10-stderr-test.log`, SHA-256
  `34473cbd98c85140a82e2ab3a3c3230d61ddab34789ecbc22ab2f48a8786b100`.
- `cargo test -p rock-lib --test macro_sdk -- --nocapture` passes all prior controls
  plus 14 direct one-shot and three compiler-host ABI controls (168.92 seconds).
  `/tmp/opencode/macro-b10-sdk-test.log`, SHA-256
  `2425d9ec819bae87783294e05ebbbd4eda31b43f8ada08fa4496fd771e394bf3`.
- `cargo check --workspace --all-targets` passes with existing warnings.
  `/tmp/opencode/macro-b10-check.log`, SHA-256
  `5bc3f2a55990292104b9ac0b53e18b10a4f14ba17d746457f96c189005f82e9e`.
- `cargo clippy --workspace --all-targets` passes with existing warnings and no
  warning in the SDK Rust harness/support.
  `/tmp/opencode/macro-b10-clippy.log`, SHA-256
  `20ec2e692f5393b09bae3e50bc4053b43d2e19019cefe2a47ce88c2750f52f32`.
- `cargo test -p rock-lib` passes: 2,699 units, 806 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK integration
  (171.11 seconds) and one parser integration; one doctest ignored.
  `/tmp/opencode/macro-b10-acceptance.log`, SHA-256
  `be1c739ac564545673d2078e06c1530045a436b16d852a0869217479940b0547`.

Protocol 7, `RKLOSS04` and artifact 53 are unchanged. Formatting remains deferred,
Beads unavailable, and unrelated work untouched.
No functional edits follow these final gates.

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

### B.11 delivery receipt (2026-10-20)

The existing `[lib]` parser accepts a boolean `proc_macro` flag and records true
as `CrateTarget::ProcMacro { path }`, with explicit `TargetRole` metadata for
binary, ordinary library and proc library products. Library/primary-target
selection includes proc libraries; binary selection does not. Default/custom
entry paths and ordinary dependencies are unchanged. Omitted/false flags produce
the same ordinary target model as before. No export list, new manifest section,
macro dependency table or SDK-name special case is introduced.

Four new shared manifest tests cover default/custom proc entries with explicit
ordinary dependencies, unchanged ordinary library/binary combinations, eight
nonboolean flag shapes, misplaced binary flags, and forbidden proc/bin mixes in
both section orders and multiple-bin packages. An empty binary array declares no
binary and remains valid. The compiler re-exports the same shared target roles.
The appended target variant leaves existing ordinary target serialization shapes
unchanged; no worker/protocol/product schema change is needed for this metadata.

Until B.15/B.16 connect host products, build/run/dependency-artifact/analysis entry
points explicitly reject proc libraries instead of compiling them as runtime
objects. One new CLI test uses deliberately invalid source to prove role rejection
precedes compiler execution and creates no build output for root/dependency cases
in both profiles. This fail-closed boundary is not host-building acceptance.

The initial complete CLI run passed 90 tests and failed the existing missing-rockc
override fixture: a changed compiler stamp required development std provisioning,
so the override failed there before the root process spawn the test intended to
check. The fixture now explicitly uses no_std and retains both original root-spawn
assertions, adding error text on assertion failure. The exact regression and
complete rerun pass; no production spawn/sysroot behavior was changed.

Verification evidence (all logs under `/tmp/opencode/`):

- Focused manifest 15/15: `cargo test -p rock-shared manifest::tests`,
  `macro-b11-manifest-test.log`, SHA-256
  `b2153ce9bcaf1dd0873e73cb2455ef6014eda5d6e0e9d9495b38eecc26506839`.
- Focused role test passes: `cargo test -p rock proc_library_role_never_falls_through_to_runtime_builds`,
  `macro-b11-role-test.log`, SHA-256
  `9017e3b9fbeea95f66a9a7e4e92b0a6d2bc8e56239c02ce37715e97dc01a0772`.
- `cargo test -p rock-shared --all-features`: 30 units and one doctest pass,
  `macro-b11-shared-test.log`, SHA-256
  `cda39461e66d91feeabcc48220fa24c3f86ab82b9385cd30030c13bc5be0ec45`.
- Initial CLI failure retained in `macro-b11-rock-test.log`, SHA-256
  `a726b599cde4e7211c758ed59222ac80b04c6f6206e36b0fc20bb9651673c07a`.
  Exact root-spawn regression passes via `cargo test -p rock --lib tests::build::test_build_project_reports_missing_rockc_override -- --exact`,
  `macro-b11-spawn-test.log`, SHA-256
  `f0b7e1ce57f1993fc660a2a8569acb1f048a29fc209ecf52fe945d3346a4458a`.
- `cargo test -p rock`: 91 units pass; zero binary/doctests,
  `macro-b11-rock-final.log`, SHA-256
  `603addb47526cf4e7f2426db6ac1e36d8ac6eef8fe9269786053aa46e2eac4a3`.
- `cargo test -p rockc`: 52 tests pass, `macro-b11-rockc-test.log`, SHA-256
  `7c977b452e663bb3eda4aa471b2599fb2c35d2d9a6a9709f29718df58fde672c`.
- Workspace/all-targets check and Clippy pass with existing warnings and no warning
  in changed files. `macro-b11-check.log`, SHA-256
  `caa01893c32f9e0eeaadebe6ccec33bb8c3fe1b009acff9d287fcedc5438a082`;
  `macro-b11-clippy.log`, SHA-256
  `52c3e9f59ee48f6b8555787a1970d48a7891e5b713f9df37d4aa1e8349e666e3`.
- `cargo test -p rock-lib` passes: 2,699 units, 806 main integrations, six
  diagnostic integrations, 14 native-process integrations, one SDK integration
  (169.30 seconds) and one parser integration; one doctest ignored.
  `macro-b11-acceptance.log`, SHA-256
  `d47264d7fb234b399e41eab98b805e789e0582b8aa0e8694d99919416af96837`.

Formatting remains deferred. `bd ready --json` still fails because bd is absent;
unrelated work is untouched. No source export/dispatcher/build-role work is claimed.
No functional edits follow these final gates.

### B.12 delivery receipt (2026-10-20)

The parser recognizes `macro name = context, input -> ...` and inline exported
`< macro name = ...` alongside unchanged declarative definitions. A distinct AST
`ProcMacroDecl` owns its source NodeId and ordinary function implementation; proc
definitions occupy the existing macro namespace rather than becoming implicit
runtime functions. `derive` names/forms receive a reserved-syntax diagnostic.
AST walking/debugging and the existing exhaustive display/collection boundaries
recognize the new node; no formatting pass or formatter checks were run.

The existing macro registry indexes source proc identities, rejects proc/proc and
proc/declarative name collisions, honors ordinary standalone/inline exports, and
enumerates only direct source entries reachable through public module/member
paths. Ordinary functions are never inferred as macro exports, and same-spelled
ordinary functions remain separate bindings. Facade publication remains rejected.
Export metadata retains NodeId/module contexts and supplies public name, canonical
crate/export identity, function-like kind and TokenStream input shape. Missing
explicit crate names use compilation's same effective source identity, not an
invented SDK/provider spelling.

`Config::target_role` is explicit (ordinary default Binary), with ordinary CLI
artifact configurations selecting Library. Source proc declarations on runtime
targets diagnose before collection, including declarations generated by source
macros. Source entries cannot self-execute during provider building, in either
item or expression position. The ordinary expansion visitor expands declarative
syntax in implementation bodies and emitted source declarations. The public
`discover_proc_macros` API shares compilation's loader/expansion engine, disables
workers and returns expanded syntax plus `Expansion::proc_exports`, without
collecting/inferencing or generating a host wrapper. Until B.13, attempting a
proc-target runtime compile explicitly diagnoses instead of dropping its entries.

Thirteen new unit tests cover parsing, reserved derive forms, helper/export
distinctions, NodeId/export authority, generated definitions/exports, file-module
privacy, duplicate declarations in both orders, ordinary namespaces, effective
crate identity, runtime/generated wrong-target declarations, self-invocation,
facade rejection, and worker-free dependency metadata resolution. An unresolved
native executable is registered during the metadata proof; successful discovery
cannot depend on executing it. Initial focused failures were test-only use of a
nonexistent source-map iterator and its reserved zero ID; the final controls use
the actual one-based record API. Explicit Config literals were updated locally
with ordinary target roles; no source/artifact fixture semantics were relaxed.

Verification evidence (all logs under `/tmp/opencode/`):

- `cargo test -p rock-lib --lib macro_`: 401 units pass, including all 13 B.12
  controls; `macro-b12-macros.log`, SHA-256
  `ed4308c6e55452a3ab57d7160b09fe85534320af1d0323f8b3d82114baf81bc4`.
- `cargo test -p rock-lib`: 2,712 units, 806 main integrations, six diagnostic
  integrations, 14 native-process integrations, one SDK integration (169.10
  seconds), and one parser integration pass; one doctest ignored.
  `macro-b12-acceptance.log`, SHA-256
  `bfa06646757386be9275ad87cc6654dfb2d0e7a588e479b68a4224b8be2be382`.
- `cargo test -p rock-shared --all-features`: 30 units and one doctest pass;
  `macro-b12-shared.log`, SHA-256
  `9c801bd6833bc4c59cdf4cc562431aeb309b279efd6047e41747c053584a22c7`.
- `cargo test -p rockc`: 52 tests pass; `macro-b12-rockc.log`, SHA-256
  `4a83c968ec46d9fd9598308e32d9355288ab7c0b8cbbb973ec84c78f2719aa43`.
- `cargo test -p rock`: 91 units pass; binary/doctests zero;
  `macro-b12-rock.log`, SHA-256
  `5fd2878900d065333855943f1153bd02c6b1b6149b08916b139baafbb04aefc5`.
- Workspace/all-targets check and Clippy pass with existing warnings and no new
  source-proc implementation warning. `macro-b12-check.log`, SHA-256
  `319aafaefe4c8f6595d6019bde19df4340f8201a097040653ba7880a35a992b8`;
  `macro-b12-clippy.log`, SHA-256
  `535abc94cfeff64c22dc9941afb2b319c485fc6f47e26749b187983fc92c7fe6`.

Protocol 7, `RKLOSS04` and artifact 53 remain unchanged: this is in-memory source
discovery, not proc-only product publication. Formatting remains deferred, Beads
unavailable, unrelated work untouched. No functional edits follow the final gates.

### B.13 delivery receipt (2026-10-20)

`compile_proc_macro(config, sdk)` requires an explicit ProcMacro target role and
`ProcMacroSdk::dependency` naming a supplied ordinary SDK artifact. `rockc
--proc-macro-sdk dependency_name` selects that path and rejects runtime artifact
emission/formatting combinations. No SDK/std is discovered or injected; ordinary
artifact crate-name validation is unchanged. Ordinary compilation of a proc role
without this SDK contract now diagnoses that missing requirement rather than
claiming wrapper generation is unimplemented.

The compiler parses only bounded compiler-owned bootstrap syntax, installs a
fresh-context private module plus main in both AST and module graph, and maps
SDK imports through supplied product identity/public export authority. It inserts
retained source implementation ASTs as private ordinary functions in their
original scopes, with SDK signatures whose type contexts bridge to the isolated
wrapper imports. Author bodies/parameters/locations are not printed, reparsed or
rewritten. Separate contexts bridge dispatcher calls to the private entries and
main to its bootstrap; user-spelled wrapper/function/type names cannot capture
those bindings. Export identity/version values replace AST literal placeholders,
never interpolate user strings into generated syntax. Template and identity
bytes are bounded by the existing source budget.

The ordinary collector, inference, MIR and codegen check the generated program.
The SDK's root facade supplies the same one-shot runner plus function-like-kind
checking and structured unknown-entry errors. Only discovered public identities
dispatch; helpers/private declarations are not table entries. Proc-only macro
imports/exports remain separate from runtime binding surfaces. This step builds
the native executable and returns discovered export metadata, not runtime
CompilerProducts or a fabricated object artifact. Proc-only publication remains
B.14; host-aware package integration remains B.15–B.17.

Three new unit controls check bounded/ordinary bootstrap syntax, unchanged
implementation/parameter ASTs and signature type contexts, and fresh contexts
with explicit source/dependency authority. Fourteen native SDK controls cover
root/file-module/source-generated entries, original-scope private helpers,
same-spelled ordinary functions and wrapper/type/helper collisions, unknown
identities/private/ordinary entries and wrong kind, arity/result/ownership
signature failures, missing/incompatible SDKs, wrong target, and an SDK rebuilt
under a different package name. No source provider writes a table or main. Two
compiler-CLI units check explicit host selection without auto-loading and
incompatible runtime-emission/format flags. The existing SDK artifact harness is
reused rather than rebuilding std for another integration target.

Initial native attempts exposed normal Rock move/pattern constraints in the SDK
kind helper; it now matches an ordinary clone. Contextual bootstrap imports then
exposed qualified-export rather than root-facade names; the SDK now supplies
actual ordinary root facade functions without changing dependency resolution.
The first differently-named dependency probe correctly failed the existing
artifact identity check; final acceptance rebuilds the SDK with the requested
package identity rather than weakening that invariant. The arity negative uses
the ordinary Resolve-category non-callable diagnostic; result/ownership failures
also run through ordinary checking, never a host-specific signature whitelist.

Verification evidence (logs under `/tmp/opencode/`):

- `cargo test -p rock-lib --lib macro_b_13`: three units pass;
  final post-import-check rerun `macro-b13-units-final.log`, SHA-256
  `6d77a5f3f0a6f876d8698bd0d2c626f6a7fdc410aae2aa3d362cb1810be00951`.
- `cargo test -p rock-lib --test macro_sdk -- --nocapture`: one integration passes
  in 234.56 seconds, including all 14 generated-host controls and prior SDK/worker
  controls; `macro-b13-sdk-test.log`, SHA-256
  `1377a5f501706492e9a5069befa89c5f8520136797acbc09076981ac9695a62b`.
- `cargo test -p rock-lib`: 2,715 units, 806 main integrations, six diagnostic
  integrations, 14 native-process integrations, one SDK integration (244.92
  seconds), and one parser integration pass; one doctest ignored.
  `macro-b13-acceptance.log`, SHA-256
  `f32021595928a087df9bf99e939d99857e410cdc43f79ee0013bfd605f752121`.
- `cargo test -p rockc`: 54 tests pass; `macro-b13-rockc-final.log`, SHA-256
  `729a5820fff4c90f40f1dbb165aa4c95dc919a66d56b750fbb2ba87fe59d805a`.
  The initial run passed 53 and failed the new host-role fixture because it omitted
  the ordinary required entry path; only that fixture argument changed.
  Initial `macro-b13-rockc.log`, SHA-256
  `72a7326d4392a062dee8b8756c4129da4a568043bf8fa977dd3531044594af93`.
- `cargo check --workspace --all-targets` passes with warnings;
  `macro-b13-check.log`, SHA-256
  `9c2443f0730b5fef68e758ac39e7f530b17f009e1ee7b9ff77d76c346e08050b`.
- `cargo clippy --workspace --all-targets` passes with warnings, including the
  new eight-argument host installation boundary; `macro-b13-clippy.log`, SHA-256
  `18fec5f3b42397711d698e58de441ea238f9e77470528d1548924dc831ebc7ec`.

Protocol 7, `RKLOSS04` and artifact 53 are unchanged. Formatting remains deferred;
Beads remains unavailable and unrelated work is untouched. No functional edits
follow the final library/SDK gates; the CLI fixture argument rerun is recorded above.

### B.14 delivery receipt (2026-10-20)

Artifact format 54 adds an explicit `ProductRole` to the bounded header. The
semantic payload's canonical role follows presence of a proc host descriptor;
the header must agree. A proc product contains exactly one descriptor and rejects
all runtime interfaces/bodies, identity exports/aliases, language items, link
records/objects, precedence and consumer SDK/std dependencies. Its constructor
retains only provider/source identity and host/export metadata. Ordinary object
validation is unchanged; no dummy object is used. There is no old-format reader.

`compile_proc_macro_with_products` runs the existing B.13 host compilation and
publishes this minimal product only after linking the executable. It rejects
no-link/object publication. `rockc --proc-macro-sdk dependency --emit-artifact path`
uses that API, records colocated/descendant host paths relative to the artifact,
and accepts no no-link/object flags. `--validate-artifact` branches on product
role: proc metadata/files validate without spawning, while ordinary products
still require their object. Dependency inspection marks proc roots metadata-only
and rejects requests to obtain runtime bodies/objects from them.

Proc descriptors now carry compiler version, SHA-256/byte executable identity,
and bounded required host-file identities in addition to host/protocol/kind/export
metadata. The small sha2 dependency supplies cryptographic content hashing rather
than introducing a handwritten codec/hash or relying on a shell utility. Host
files must be regular, have matching identities, and total at most 512 MiB with
at most 128 required files; Unix opens are nonblocking before regular-file checks.
Relative paths anchor only at the supplying artifact, never cwd/sibling fallback.
Loading resolves/checks files before changing crate context. Invocation rechecks
host/compiler/protocol/format, selected identity/kind, permissions and file
content before spawn. Public identities remain opaque; dependency aliases do
not rewrite them. Hashes provide content integrity, not author trust/sandboxing.

The consumer receives an empty runtime extern surface and metadata-only linkage;
proc imports are seeded into the separate macro binding namespace. Native SDK
acceptance publishes a real source-generated provider artifact, loads/analyzes it
without SDK/std/object dependencies, and rejects an implementation-name runtime
import. Unit controls cover header/payload and unknown roles, mixed runtime
exports, relative file resolution, required-file changes/removal, and seven
independent pre-spawn compatibility/content/kind rejections plus unknown identity.
The CLI's existing no-execution observer now proves object-free validation and
metadata-only inspection, with unchanged no-worker markers. A CLI role test
checks proc publication without --no-link.

The initial macro-focused run exposed an overstrict assumption that a canonical
export identity must equal its crate/name spelling; that check was removed, and
the original opaque-identity dispatch observer remains unchanged. Process tests
now capture current host identities. Their deliberate native-format replacement
recaptures its identity so it still reaches the unchanged OS-format/no-shell
assertions; separate controls reject stale content. Permission preflight retains
the OS EACCES cause. Invalid protocol metadata now fails publication, and wrong
hosts fail loading before the execution-disabled boundary; valid disabled
invocations retain the original no-spawn assertion. An initial workspace check
also caught a test-only missing qualification of the dependency inspector.

Verification evidence (logs under `/tmp/opencode/`):

- Initial API RED: `cargo test -p rock-lib --lib macro_b_14` fails before the new
  host identity/product APIs exist; `macro-b14-red.log`, SHA-256
  `0b7a097a0d8e41ed36dfa47952b238ccf0f14fd88c11af61a2fbea1aad6e34a6`.
- Final focused macro units: `cargo test -p rock-lib --lib macro_`, 406 pass;
  `macro-b14-macros-final.log`, SHA-256
  `f2b3bfb09e94bb2f21d81f358a2105a05f47b546ef62a9f2efd31232b73003dc`.
- Process controls: `cargo test -p rock-lib --test macro_process`, 14 pass;
  `macro-b14-process-final.log`, SHA-256
  `ea08b34320d4089f2922ce7723a060b7b2ee037ecef19067f8cd488da052f708`.
- SDK/publication controls: `cargo test -p rock-lib --test macro_sdk -- --nocapture`,
  one integration passes in 234.76 seconds; `macro-b14-sdk.log`, SHA-256
  `b2e7bb0a3030ab61044915c387414c2cf08f803a612066dbfba0f863e1db19bd`.
- Complete library: `cargo test -p rock-lib`, 2,717 units, 806 main integrations,
  six diagnostic, 14 process, one SDK (241.58 seconds), and one parser integration
  pass; one doctest ignored. `macro-b14-acceptance.log`, SHA-256
  `e41bd7f2e01e61497bdb51c30bab9d570765f55286f2a774d116f6239e3a58ec`.
- `cargo test -p rockc`: 55 pass; `macro-b14-rockc-verified.log`, SHA-256
  `6440945ed96ab346f7b942c3f22d19851c3bb24d5a96de666a70209c1c6e5a4a`.
- `cargo test -p rock-shared --all-features`: 30 units and one doctest pass;
  `macro-b14-shared.log`, SHA-256
  `dda8e84f176c0d18cf990da314b71e54f73fa3ff00695568277a7a81fd09de19`.
- `cargo test -p rock`: 91 pass; binary/doctests zero; `macro-b14-rock.log`, SHA-256
  `f32502704e6ce65a693fc4ec571d19a848e42de9f79006a9204c980655587ecf`.
- Workspace/all-targets check and Clippy pass with warnings; no new warning in
  proc artifact/publication code. `macro-b14-check-final.log`, SHA-256
  `926c28a337630119310d66a3ad571966b1bb6ad120e0b484e368f74db999ca14`;
  `macro-b14-clippy.log`, SHA-256
  `acd534ed848ccf3e11ce68a98d68d77d91bd05faec54d6b0949b581733d27bcb`.
- Real `cargo run -p rockc -- ... --proc-macro-sdk rock_macro --emit-artifact ...`
  builds `lib/tests/fixtures/macros/sdk-host.rk` against freshly CLI-built format-54
  std/SDK artifacts in `macro-b14-cli/`. The provider directory contains only
  `sdk-host` and `cli_provider.rkca`. `macro-b14-cli-provider.log`, SHA-256
  `7f27790347c5eab375de56ba3d63346e215d7a50f0baf080f1422c5f3fa31ab1`.
  `--validate-artifact ... --no-proc-execution` passes (`macro-b14-cli-inspect.log`,
  SHA-256 `426993027bafb56a50506400963ca8afa840b01011cb90e458a1ac1f01db2755`),
  and `--print artifact-deps --extern-artifact cli_provider=...` reports
  `metadata=true bodies=false link=metadata-only` (`macro-b14-cli-deps.log`, SHA-256
  `387e88463db6d92534e94b78143811440c83a117f7085fbe153ef9a884cfb676`).

Protocol 7 and `RKLOSS04` remain unchanged; only product artifacts advance to 54.
Host/target planning, package building and role-sensitive freshness/relocation
remain B.15–B.17. Formatting remains deferred, Beads unavailable, unrelated work
untouched. No functional edits follow the final gates.

### B.15 delivery receipt (2026-10-20)

`rock::dependency_plan` models source target/role and execution platform separately.
Build units are keyed by canonical root, selected target, platform and triple;
existing profile/compiler/source/dependency checks remain in the artifact build
path. Ordinary dependency edges inherit host/target, proc edges switch to host.
Topological postorder builds nested proc dependencies before their users, and
cycles name the involved root/platform/role/triple chain. Compiler-input closures
include ordinary-library dependencies but stop at proc boundaries; runtime input
sets exclude proc units. No manifest dependency table or SDK package spelling was
added. The existing artifact builder consumes these planned inputs and keys its
in-session cache by build unit rather than path alone.

Package artifact/object directories distinguish platform, triple and role (and
retain profile/selected source target/root identity). Native target layout stays
unchanged; native host libraries still have separate paths, not shared objects.
Compiler-produced product identities now record the native triple, using the
existing identity field without a schema change. Runtime products mark proc
dependencies metadata-only, with no bodies/target objects. Proc package execution
continues to fail closed until the B.16 SDK/dispatcher orchestration is wired.

Definition-side runtime helper handles resolve their logical crate name/version/
identity-format requirement to exactly one explicitly supplied consumer ordinary
identity, then reuse public-export-only binding environments. A host fingerprint
or triple is not binding authority in the consumer; missing/wrong-version or
ambiguous target identities diagnose with the required crate/version. The index
still must have been declared by the macro descriptor, and neither aliases nor
private exports gain authority. Only requested helper handles require consumer
artifacts; an unused host dependency does not become a target dependency.
Proc publication records the provider's explicitly supplied ordinary identities
as the deterministic set of available handles, including SDK/std; these are
requirements only when used, not automatic target artifact dependencies. Native
acceptance checks that SDK/std handles are published with their host triple while
remaining absent from the consumer dependency/object list.

Two planner controls cover distinct simulated triples, one package needed in both
platforms, host SDK/std libraries, nested proc dependencies, topological order,
proc-boundary closure isolation, native same-triple host/object isolation,
ordinary/proc role changes for the same source, and host-cycle rejection without
reading compiler source or producing outputs. A compiler unit covers consumer
fingerprint/triple rebinding, alias independence, public/private authority, missing,
wrong-version and ambiguous consumer identities. Extended native SDK acceptance
publishes an ordinary consumer with a metadata-only proc dependency, then links
and executes a downstream program (exit 42) with only consumer/proc artifacts,
without provider host/SDK/std object inputs.

Initial planner and runtime-identity API RED commands failed before the new APIs
existed. The first full CLI run passed 92 tests but exposed a regression in the
unchanged in-session artifact reuse observer after deleting a dependency manifest;
the planner now reuses successfully built package metadata from the same build
state, preserving source-free reuse without reading deleted sources. Its complete
rerun passes. No original observer assertion was removed or narrowed.

The existing compiler/toolchain is native-only. Foreign triples are simulated in
the planner, and the execution adapter explicitly rejects unsupported triples;
no runnable cross-target acceptance is claimed. B.16 remains real proc package
SDK/dispatcher execution, B.17 role-sensitive freshness/relocation/source-free
acceptance. Product format 54, protocol 7 and `RKLOSS04` remain unchanged.

Verification evidence (logs under `/tmp/opencode/`):

- Planner API RED: `cargo test -p rock --lib macro_b_15`, missing planner APIs;
  `macro-b15-red.log`, SHA-256
  `f4bc79ea5a7b1c20288b8932c210f34fc0169d4c4a68d9ed56abaaaf285ae940`.
- Initial planner GREEN: the same command passes two controls;
  `macro-b15-plan.log`, SHA-256
  `fe48bd1447699587f7cfe9f5b988cc41d30dad2a3fdf77b49f06323c181b28fd`.
- Runtime API RED: `cargo test -p rock-lib --lib macro_b_15`, missing consumer
  dependency API; `macro-b15-runtime-red.log`, SHA-256
  `03275a8ca14f024e6bdee0fc7450273fe2be8ade7ed9ec67468bb908101dd7a6`.
- `cargo test -p rock-lib --lib macro_`: 407 pass; `macro-b15-macros.log`, SHA-256
  `c27974c6a4ba99d7c313408a91e939dc144a4ac7a30360e678948607ac42415f`.
- `cargo test -p rock`: initial 92 pass/one failed source-deletion reuse control;
  `macro-b15-rock.log`, SHA-256
  `a4af946a17a59284245e8457b7aece31a3f54634fea1693e6444bff17eb2f7d8`.
  After retaining known package metadata, 93 pass, binary/doctests zero;
  `macro-b15-rock-final.log`, SHA-256
  `3bb6dfd4d972cb28b1941fa38641674bf424ecfa071c92c4d18b795178ee5aa4`.
- Native consumer publication/linkage: `cargo test -p rock-lib --test macro_sdk
  -- --nocapture`, one integration passes in 234.75 seconds;
  `macro-b15-sdk.log`, SHA-256
  `60fe817b613d9c44bd12e179d941569e933c82980ef7854cf445c54aca71f515`.
  This focused run precedes the final automatic publication of the host's
  available dependency handles; the final SDK rerun below covers those assertions.
- Workspace/all-targets check and Clippy pass with warnings, with no new
  planner/runtime-identity warning. `macro-b15-check-final.log`, SHA-256
  `1e0dc46a29d84b46b29b14f3a7791fe760ca7c341cb575761362e6cffdce76dd`;
  `macro-b15-clippy-final.log`, SHA-256
  `3720bd836ce3f9c9abe9500252c6c8722aa92fd15f955a7e755f9f7f089c9cdd`.
- Full library: `cargo test -p rock-lib`, 2,718 units, 806 main integrations,
  six diagnostic integrations, 14 process integrations, one SDK (240.01 seconds)
  and one parser integration pass; one doctest ignored. The run was started
  before the final descriptor handle publication change; final focused SDK and
  macro reruns follow below. `macro-b15-acceptance.log`, SHA-256
  `2297322e33739e9f4a76cf8f8ef79f6c69c9d03d7d16e2a0790944f8d6e87c6d`.
- Final SDK: `cargo test -p rock-lib --test macro_sdk -- --nocapture`, one
  integration passes in 233.53 seconds, including the added host handle list
  assertions and downstream native linkage. `macro-b15-sdk-final.log`, SHA-256
  `5d6f6d2b3aad81a55c5251f4d2009f2ab66a5579f706673e990cddd593d10764`.
- Final macro units: `cargo test -p rock-lib --lib macro_`, 407 pass;
  `macro-b15-macros-final.log`, SHA-256
  `3bf1a4d41dfb198e39fa4589d5e92c221c11db304482e775ddf72a86016cd0bd`.
- `cargo test -p rockc`: 55 pass; `macro-b15-rockc.log`, SHA-256
  `d73aeaa7fa7623476afead1d955a93e234a9bd5bc7ff2c1c0334cef54fbe459c`.
- `cargo test -p rock-shared --all-features`: 30 units and one doctest pass;
  `macro-b15-shared.log`, SHA-256
  `7726bc32dff531ffd5afd5fb172828d6866a78d7e053fdf93e9d9c4bf84b55e7`.
- `cargo build --workspace`: pass with warnings; `macro-b15-build.log`, SHA-256
  `8da56ffd5411b1abaa96d5ed8fc55d779cb23273e8a9584ad932a7e6a98583bf`.

No functional edits follow the final focused/check/Clippy gates. Formatting
remains deferred, Beads unavailable, and unrelated work untouched.

### B.16 delivery receipt (2026-10-20)

Real `rock build`/`rock run` now consume the B.15 plan to build host SDK/ordinary
libraries, discover the explicit SDK contract, compile the generated dispatcher,
and publish proc-only artifacts for root providers and direct consumer inputs.
`rockc --print proc-macro-sdks` inspects supplied ordinary public interfaces without
executing workers; the orchestrator requires exactly one candidate and passes it
through `--proc-macro-sdk`. Contract names are shared with B.13's wrapper check;
there is no SDK package-spelling special case. Actual signature/type checking
remains in the normal compiler pipeline. Native SDK acceptance checks original,
renamed and two-candidate discovery against real artifacts.

Host invocation construction reuses ordinary compiler flags/dependency artifact
inputs but emits a linked worker and proc-only product, not no-link/runtime object
outputs. No source or bare-executable dependency option is passed to `rockc`.
Proc roots select the host layout at package load, and their basic freshness check
requires the worker rather than a dummy object. Proc artifacts do not propagate
host std requirements into target compilation. An explicit ordinary std already
in the compiler-input closure is reused rather than adding a second implicit
identity. Role-sensitive freshness and relocation completion remain B.17.

The existing expression-proc placeholder blocked the real identity example.
Item and expression calls now share the same execution/output reconstruction
function and category-specific lossless parser. An identity result forwarding
the inline call-separator space also exposed root-expression indentation handling:
the ordinary expression-fragment entry now treats its first line as a fragment,
preserving retained token offsets and subsequent layout, without changing ordinary
item indentation or printing/re-lexing captured syntax. A local regression keeps
the retained literal's exact original range and the ordinary item-indent negative.
The parser's existing unresolved-outer/no-inner-execution observer now expects an
unresolved-path diagnostic instead of the retired unimplemented-placeholder text;
all its source and semantic assertions remain unchanged.

The new real CLI test builds/imports the source identity macro via short, qualified
and module-import alias paths, runs each consumer to exit 42, proves unused imports
build with execution disabled and actual invocations diagnose, then emits a runtime
helper path. Missing consumer runtime dependencies diagnose with `runtime@0.1.0`;
adding that ordinary dependency runs successfully. It verifies that actual host
and target helper artifact/object files both exist at separate paths, and that
the provider has a worker but no ordinary runtime object. An invocation unit
checks exact SDK/artifact flags and absence of source/bare-worker/object options.
The original invalid-source proc-role observer still proves no runtime fallthrough;
its build boundary now diagnoses missing SDK before source compilation, while
runtime-only provider analysis points to `rock build` rather than claiming host
building is unimplemented.

The first focused RED hits the old host-only build guard. Follow-up fixture runs
exposed unsupported inline file-module indentation, the actual expression-trivia
bug above, a diagnostic assertion reading stderr instead of compiler stdout, and
an attempted import of std's private prelude module. The fixture now uses a real
module file and ordinary explicit std artifacts with normal prelude injection.
No compiler-owned std loading or observer weakening was introduced. A macro-only
run also caught the stale unimplemented-diagnostic observer before the final gates.

Verification evidence (logs under `/tmp/opencode/`):

- `cargo test -p rock --lib macro_b_16` initial RED: `macro-b16-red.log`, SHA-256
  `325d13e4c7909023151488e005ce4e084e60f9d89c5aeb36275791177c991200`.
  The first two-control GREEN is `macro-b16-package-verified.log`, SHA-256
  `08f5f66e0d59d45126f84c66f3a923c176cd5c4b040b21b45612111b47475965`;
  final broader gates include the subsequent closure-std/path assertions.
- `cargo test -p rock-lib`: 2,719 units, 806 main integrations, six diagnostic,
  14 process, one SDK (236.03 seconds) and one parser integration pass; one doctest
  ignored. `macro-b16-acceptance.log`, SHA-256
  `6d63541a26987bbdf959d003b0937837edc4955b1cabbe3f82bae1a7a7308908`.
- `cargo test -p rock`: 95 pass including the real CLI native controls; zero
  binary/doctests. `macro-b16-rock.log`, SHA-256
  `ee761c49582471dffa7411e55ae066024bf067f5f6607a889ead51d77478c8df`.
- `cargo test -p rockc`: 56 pass; `macro-b16-rockc.log`, SHA-256
  `03d7cde52786997fae265616c82e72e6bf473c9050802d1778b99428144b4f71`.
- `cargo test -p rock-shared --all-features`: 30 units and one doctest pass;
  `macro-b16-shared.log`, SHA-256
  `b2421faa67e816f07aa007fb158757302f2ffc26624c0938f0eb6abd27de2282`.
- Workspace/all-targets check and Clippy pass with warnings;
  `macro-b16-check.log`, SHA-256
  `b9c7dbdc870a904cab1c03f61b70c89d72f19cba95a5a764146f5aacf2246e2c`;
  `macro-b16-clippy.log`, SHA-256
  `807b8a3a814ef4ba9af75b268146c793a07de4b4062371dbd67a1e0ec8e3c3d1`.
- The complete README provider/consumer source examples were exercised in
  `macro-b16-doc/`, with only the SDK locator made absolute. Root `rock build
  --debug --no-proc-execution` passes (`macro-b16-doc-provider.log`, SHA-256
  `b4f61b44a5af90e41564a0a60961bcdb2998ae0ebff298d870950c83aed36041`),
  and consumer `rock run --debug` prints `42` with exit 0
  (`macro-b16-doc-consumer.log`, SHA-256
  `06ed9e39f1297245d263c8d7ff405e7c222a711afe4775e965b57a1cd64e0e39`).

Product format 54, protocol 7 and `RKLOSS04` remain unchanged. No functional edits
follow the final gates. Formatting remains deferred, Beads unavailable, unrelated
work untouched. B.17 and the nontrivial consumer/derive stages remain unfinished.

### B.17 delivery receipt (2026-10-20)

The existing freshness path now validates the expected runtime/proc product role,
requires the appropriate object/worker output, includes compiler modification
time, and rejects outputs changed after artifact publication. Existing profile,
platform/role/source/dependency inputs remain in use; no new cache is introduced.
Proc executable content checks continue to run before reuse, so a changed worker
causes a source rebuild instead of accepting stale metadata. Ordinary runtime
object validation is not weakened. `rockc --expected-artifact-role runtime|proc-macro`
adds a no-worker role constraint for orchestrator validation.

Proc executable/required-file paths are normalized against the actual artifact
directory, including explicit `..` components to sibling worker directories.
There is no fallback sibling guessing. An isolated CLI unit proves the exact
`../worker/host` result and accepts/rejects ordinary artifacts by expected role.
The initial unit RED fails before this normalization API exists; a follow-up
compile caught a missing test-local std::fs import, fixed before final gates.

The new real CLI acceptance fixture copies the ordinary SDK source into its own
package, builds the provider through `rock`, proves an unchanged host is reused,
and independently changes provider, helper and SDK sources. Each invalidates the
provider artifact. It then changes the executable bytes and verifies a successful
build restores the exact worker bytes. Host/target object isolation continues to
be covered by the unchanged B.15/B.16 controls.

The fixture copies only the proc artifact and worker into a bundle, relocates
that bundle, removes fixture provider/SDK source and manifests, and deletes all
host-library artifacts/objects (including SDK/std/helper). Direct `rockc`
inspection succeeds with execution disabled; a source-free consumer supplied
only the relocated proc artifact compiles/runs to exit 42. Provider metadata
bytes remain unchanged. Synthetic obsolete versions, corrupt products and
changed worker contents reject on inspection; restoring the original files
recovers inspection without rewriting metadata. The existing no-worker-marker
inspection observer is included in the full compiler-CLI gate. This is explicit
artifact consumption, not a new source-free manifest locator or cross compiler.

Verification evidence (logs under `/tmp/opencode/`):

- `cargo test -p rockc macro_b_17`: initial missing-API RED;
  `macro-b17-red.log`, SHA-256
  `cae948962e82b1636d1e250971a34349e3178ccf5b34ca2bcbcc37b9fdc74fd8`.
  Final one-unit GREEN: `macro-b17-role-final.log`, SHA-256
  `faa066705427b4b1c7d040b3fbbcf1d767a10d94ddf723b9567b96a1a548fb84`.
- `cargo test -p rock --lib macro_b_17`: final focused native GREEN, including
  deletion of all host dependency artifact/object files;
  `macro-b17-package-final.log`, SHA-256
  `4a3046937b1f8727ecbdc865ceb0dad3816a0c1c8e4b76c62e9d0e27d63ddda9`.
- `cargo test -p rock`: 96 pass; binary/doctests zero. This full run precedes
  only the strengthened fixture cleanup of host-library files; the focused rerun
  above covers that final assertion boundary. `macro-b17-rock.log`, SHA-256
  `8b8344db2c544c42dc8b10bab938ec8cef3098efe8d96b9aadb4fb68a9b5348c`.
- `cargo test -p rockc`: 57 pass; `macro-b17-rockc.log`, SHA-256
  `3164095cbb607365bbb99ca8045ff2e867191be494e26df433398de5baccfbd8`.
- Workspace/all-targets check and Clippy pass with warnings;
  `macro-b17-check.log`, SHA-256
  `60987dfba53a511471197c3b5cccef13a683d9b9303e3685956854565e434698`;
  `macro-b17-clippy.log`, SHA-256
  `000b9aedfa836cd431c205996400168fe8781083ef575f13799265ed60816e5a`.

B.17 changes only CLI/orchestration and their controls; compiler-library sources
are unchanged from B.16's complete library acceptance. Product format 54,
protocol 7 and `RKLOSS04` remain unchanged. No functional edits follow final gates;
the subsequent test-only source-free cleanup strengthening passes its focused
rerun above. Formatting remains deferred, Beads unavailable, unrelated work
untouched. B.18–B.22 and Stage C remain unfinished.

Do not widen `rock`'s dependency on compiler internals to generate semantic data:
`rock` orchestrates `rockc`, which delegates compilation/discovery/checking to
`lib`. Proc-only artifact emission now uses host metadata rather than a target
object; ordinary library artifact validation retains its object invariant.

### Bd. Prove the actual consumers, not only an identity macro

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| B.18 | Extract/reuse canonical literal decoding and supply decoded-byte-to-source mappings for already Rock-recognized input literals. | Escapes/multibyte subranges map precisely. Do not scan arbitrary DSL quotes as Rock literals to populate views; no duplicate full lexer or compiler callback enters the SDK. | U, I |
| B.19 | Materialize authorized literal/generated-source fragments after worker return into lossless units, then normal capture-aware Rock parsing. | Literal holes and generated text keep contexts/subspans; invalid ranges/handles/Rock output and token-gluing attempts reject. Fragment nesting/bytes/interpretation share budgets, and copied captures are never printed/re-lexed. | U, I |
| B.20 | Add an external Rock block-DSL fixture using the generic SDK, without a production HTML implementation. | Exact HTML-shaped input includes apostrophes, unmatched quotes/brackets, comments, backticks, Unicode, whitespace, and a following dedented binding. Prove library-defined interpretation/subspan errors, expression/item results, nested forwarding, and normal rejection of DSL text emitted directly as Rock. | I, R |
| B.21 | Add a minimal external interpolation macro/helper fixture using literal views and deferred fragments. | `Hello {name}` and a `count + 1` hole resolve at caller scope; escaped/Unicode error ranges are exact. Missing names, formatting support, or runtime helper dependencies fail normally; provider/trait names are not compiler-recognized. | I, R |
| B.22 | Close interpolation's evaluation/ownership proof and host artifact acceptance for the nontrivial consumers. | Holes execute once left-to-right, named values are borrowed, temporary destruction is normal, helper/local collisions are hygienic, and calls retain consuming behavior. Repeat source-free packaged consumer tests for DSL/interpolation, not only identity. | I |

### B.18 delivery receipt — canonical literal views

Canonical quoted-literal decoding now lives in `lib/src/lexer/literal.rs` and
is reused by character lowering, string codegen and procedural literal-view
preparation. Existing supported escapes and unknown-string-escape behavior are
unchanged. Each decoded Unicode scalar retains its exact original UTF-8 or
escape-spelling range; empty strings have empty mappings.

The parser retains input-relative ranges from existing cached interpretations,
following only the selected capture lineage and its consumed token prefixes.
No additional token reads or opaque block scanning populate these views;
speculatively interpreted DSL quotes cannot leak from an abandoned branch.
Both item and expression procedural requests use the same bounded preparation.
The SDK receives plain protocol-7 literal records, with no decoder or compiler
callback added to Rock. Product format 54 and `RKLOSS04` are unchanged.

Five focused `macro_b_18` units pass, covering canonical decoding, precise
escape/multibyte subranges, empty/character values, metadata bounds, item and
expression opaque blocks, and branch-lineage/non-reading controls. Workspace
all-targets check and Clippy pass with existing warnings. The first SDK run was
interrupted by its two-minute command limit after the data client passed; it
does not establish SDK acceptance. The longer SDK run passed all existing
controls, then exposed untyped integer comparisons in the new host fixture.
Explicit U64 comparisons and borrowed Option inspection fix the fixture, which
now compiles through `rockc` against the already built ordinary SDK/std artifacts.
The final complete library acceptance passes, including native execution of the
corrected fixture. No compiler or fixture edits follow that acceptance.

Completed focused commands (logs under `/tmp/opencode/`):

- `cargo test -p rock-lib --lib macro_b_18`: five pass in
  `macro-b18-focused-final.log`, SHA-256
  `71431c0fae467f4e58dfaa6d4fc73c081d8aaab8b418b85ead76a5c15a3115d6`.
  The first three-test attempt failed on the test's incorrect expected closing
  quote offset (`1..13` instead of `1..12`); it was not a missing-feature RED.
- `cargo check --workspace --all-targets`: pass in `macro-b18-check.log`, SHA-256
  `5c2d6aba40781eeb0c8e6f84d5e5195d70a5d46a492115fb923353722fcd4008`.
- `cargo clippy --workspace --all-targets`: pass with existing warnings in
  `macro-b18-clippy-final.log`, SHA-256
  `08723b516de2e0861500974d25e0b39dec1a37b61d15b6e7343dbcf3bfae17fb`.
  The final complete suite includes the subsequent fixture-only U64
  and borrowed Option corrections; the compiler implementation is unchanged.
- `cargo build --release`: pass with existing warnings in
  `macro-b18-release-build.log`, SHA-256
  `cbea0d41dc56cec66c827b496234b56e0d8698e53a497d0f673de636f3367777`.
- Manual `cargo run -p rockc -- ...` provider publication and consumer compilation
  pass using the corrected `sdk-literal-host.rk`, its explicit SDK/std artifacts,
  and `/tmp/opencode/macro-b18-consumer.rk`. The consumer supplies only the
  provider artifact, exercises real workers for the escaped/Unicode/character/
  empty-string views and opaque malformed DSL, and links successfully. Program
  execution is established by the final integration gate, not this CLI check.
  `macro-b18-host-artifact.log`, SHA-256
  `fd37dfc28bd68a5b689b4e64e676fba79b6522f424bd48620d3071ebfd5447d4`;
  `macro-b18-consumer-build.log`, SHA-256
  `128d346a5d89e3b2eb77104f3d201da9a9ba0df7f101924e08ff782ce0f36194`.
- `cargo test -p rock-lib`: 2,724 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, one SDK integration and one parser
  integration pass; one doctest is ignored. The SDK integration runs the
  existing cross-language request/result, one-shot, generated-host and renamed-
  SDK controls, then checks compiler-prepared escape/Unicode/character/empty
  literal views in a real Rock worker and the absence of views for an opaque
  malformed block. Its consumer executes with exit status 42, without target
  SDK/std dependencies. `macro-b18-acceptance.log`, SHA-256
  `f945eeded80a9bd45531d4809b3b47cc056f920a165edb11889c3c5d8d5f232e`.

B.18 is implemented and verified. Deferred fragment materialization remains
B.19; interpolation/DSL policy remains external library work in B.20–B.22.
Stage C is unfinished. Formatting remains deferred, Beads unavailable, and
unrelated work untouched. No protocol, lossless transport or product schema
change was needed for these parser-side ranges and existing literal records.

### B.19 delivery receipt — authorized deferred fragments

The common procedural item/expression path now materializes validated Stream,
Source and Literal parts after worker exit. Source fragments use the existing
neutral scanner and construction authorization. Literal fragments select only
compiler-owned canonical view ranges, retain per-scalar escape/source extents,
and use invocation-site context. Captured streams retain snapshot identities,
contexts and opaque expression ranges without printing/re-lexing; emission
boundaries never grant adjacency. Ordinary generated parsing and expansion
ancestry remain authoritative, and aggregate work/output limits cover fragments.

Literal snapshots use existing Generated provenance with exact source extents;
only the compiler can register the fresh snapshot identities for literal
diagnostic projection. Registration grants neither binding nor adjacency and
is not serialized worker data. Source-map projection supports indivisible escaped
scalars and proved contiguous mixed-width extents, including later forwarding.
Protocol 7, `RKLOSS04` and product format 54 remain unchanged.

Two initial `macro_b_19` tests reproduced the unsupported-fragment diagnostic
before implementation (`macro-b19-red-runtime.log`; the preceding attempt had
a missing test import and is not a behavioral RED). Seven focused units now
pass, covering generated items, literal holes after escape/Unicode prefixes,
nonlinear escaped spans and diagnostics, nested declarative forwarding,
capture snapshot/context/opacity preservation, anti-gluing, bad handles/ranges/
construction authority/Rock, and shared work/output budgets. Real `rockc`
provider publication and consumer compilation also pass with only the provider
artifact supplied to the consumer; an unknown literal-derived name is diagnosed
at its exact caller substring. The complete library/native SDK gate passes.

Workspace/all-targets check and Clippy pass; the full library run precedes only
four Clippy-only removals of needless closure borrows, stronger negative-message
assertions and documentation updates.
No functional edits follow that gate's source snapshot. The final seven-test
focused rerun and terminal workspace/all-targets Clippy pass cover those
cleanups/assertions before committing B.19.

Commands and logs under `/tmp/opencode/`:

- RED: `cargo test -p rock-lib --lib macro_b_19`, two unsupported-fragment
  failures in `macro-b19-red-runtime.log`, SHA-256
  `231de8852dbc006423b7891a475967d42f6f4995f58edbd1fb537cd304a44dde`.
- Final focused GREEN: same command, seven pass in
  `macro-b19-final-snapshot.log`, SHA-256
  `20ff740c648457956f36fee19b685dfd39d4cb5d9355cae74d717a1c2adf43af`.
- `cargo test -p rock-lib`: 2,731 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, one SDK integration and one parser
  integration pass; one doctest is ignored. The native SDK gate retains all
  existing cross-language/host/worker controls and now emits deferred source
  items and literal expressions from a real Rock provider, executes the caller-
  local consumer with exit status 42, and diagnoses an unknown literal-derived
  name at its precise escaped/Unicode caller substring.
  `macro-b19-acceptance.log`, SHA-256
  `818d13c33dcfaaf70449b1da03f3834e094e71b5dac8ddbc0aa37e819510e659`.
- `cargo check --workspace --all-targets`: pass, `macro-b19-check.log`, SHA-256
  `023a0321bae9e6d690632e28b12d1f818de85e294f64faf94efe7b5416495f6e`.
- `cargo clippy --workspace --all-targets`: pass with existing warnings,
  `macro-b19-clippy-terminal.log`, SHA-256
  `05085e6914da41a0079cee13f2c6e4247fe3ae9bef1a5640ff4128501bce86f5`.
- `cargo build --release`: pass with existing warnings,
  `macro-b19-release-build.log`, SHA-256
  `434c5a4af62a7ed79ecf31f418a5c5228432e5f75281541f054308063c1564c8`.
- Manual real-worker CLI positives and exact unknown-name rejection use
  `/tmp/opencode/macro-b19-{consumer,missing}.rk` and the explicitly published
  `macro-b19-host/literal_provider.rkca`. `macro-b19-consumer.log`, SHA-256
  `f8d1155bec9a28f90e27ca559d35def4a5a86e1f974293c094d81fea59cc7e09`;
  expected exit-1 `macro-b19-missing.log`, SHA-256
  `4aa8fb5ae0254904689b4db091b7aa231cc73ae6d77a3ac71efa447ee7775675`.

B.19 is implemented and verified. B.20–B.22 and Stage C remain unfinished;
formatting remains deferred, Beads unavailable, and unrelated work untouched.

### B.20 delivery receipt — external opaque block DSL

`lib/tests/fixtures/macros/block_dsl` is an ordinary external Rock procedural
package using only the generic SDK. Its deliberately tiny library-owned grammar
reads a decimal attribute and otherwise treats HTML-shaped contents as opaque;
it is not a production HTML parser. The three exported entries emit expression
or item source, or deliberately return uninterpreted DSL for normal Rock output
rejection. A reserved byte produces a library diagnostic on the exact input
subrange, including after multibyte text. No compiler policy or schema changes
are part of this step.

The complete consumers include apostrophes, an unmatched quote and brackets,
HTML/Rock-comment-looking text, backticks, Unicode, an odd margin, a tab and a
following dedented function. Native SDK acceptance checks exact retained bytes,
no block literal views, the following function, both result categories,
declarative forwarding of a nested opaque block, real execution, exact input
error ranges and normal rejection of raw DSL output. CLI acceptance builds an
ordinary provider dependency and runs the same consumers through `rock run`,
then checks library errors and raw-output rejection through `rock build`.

The initial CLI RED forwards the unprocessed DSL and fails in normal generated
Rock parsing at `=` (`macro-b20-red.log`). Provider compilation then exposed a
nonexistent String mutation helper and underconstrained generic early-error
returns; the fixture now uses ordinary `String::concat` and a concretely typed
error constructor. These failed authoring attempts are not compiler regressions.
The corrected native provider publication, final native SDK acceptance and final
CLI gate pass. Native acceptance also proves an alternate numeric result and
library-defined missing/invalid/oversized-attribute diagnostics. Its first run
passed the success cases before a test compared an unprefixed error string to
the compiler's correctly prefixed message; the final assertion requires the
exact `Proc macro 'render': ...` message. The provider did not change for that
assertion correction. B.21–B.22 and Stage C remain unfinished.

Commands/logs under `/tmp/opencode/`:

- `cargo test -p rock macro_b_20 -- --nocapture`: initial forwarding RED,
  `macro-b20-red.log`, SHA-256
  `ee75572950e261719ffe2b212f387a419f4e899636f35e45dfdd2c1cd68396af`.
- Direct provider publication uses `cargo run -p rockc -- --entry-file
  lib/tests/fixtures/macros/block_dsl/src/lib.rk --crate-name block_dsl ...`
  with explicit SDK/std artifacts and `--proc-macro-sdk rock_macro`.
  Failed authoring attempts: `macro-b20-host.log`, SHA-256
  `b5b6fd7e4ef2e446cbfb84825d4c471e394852e5d22ca37c444e4c7ab10ad1e4`;
  `macro-b20-host-green.log` (despite its name, failed), SHA-256
  `56790b9a04819ba5f503e3e52720c9e7b3f17e7c8d5c58ba39aa5d7ddff8bb85`.
  Corrected publication: `macro-b20-host-final.log`, SHA-256
  `3acfce5b56cb639ee36b45782a0b8e0da933ba74336e51ca6a8d5d65c5def35d`.
- `cargo check --workspace --all-targets`: pass, `macro-b20-check.log`, SHA-256
  `1578104d07807df7878c63b381103a52dc676b3883c9488c39bf2c36606d012e`.
- `cargo test -p rock-lib --test macro_sdk -- --nocapture`: final one-test native
  SDK gate passes all existing request/result/worker/host/literal/fragment
  controls and the new DSL controls. `macro-b20-sdk-final.log`, SHA-256
  `4fcde34432fe61a88b0e193d692c1a22434dbb77b88fc1c50f7f9fa6072fe799`.
  Earlier failed assertion run: `macro-b20-sdk.log`, SHA-256
  `3a73a792945241d598af4c8f38ae01b31bd68f9250c4b14827165e2fd2d27916`.
- `cargo test -p rock macro_b_20 -- --nocapture`: final one-test CLI gate passes,
  `macro-b20-cli-final.log`, SHA-256
  `25ef2257cba51f23639756e7ca997ddfb297c89f43ec8b5d358a79e7ec505594`.
  Initial GREEN preceded stronger raw-output error assertions:
  `macro-b20-cli-green.log`, SHA-256
  `aaeb60ec2991744d10cb7ada9261bed29c0b04cdb37a7ea66b19db4c20a9c0bc`.
- `cargo clippy --workspace --all-targets`: final pass with existing warnings,
  `macro-b20-clippy-final.log`, SHA-256
  `290111134f8321319e063c2a56b34a21885c1199dd1d7dbb24acd36a6a1cf04d`.
- `cargo build --release`: pass, `macro-b20-release-build.log`, SHA-256
  `d9326b638deda9351a53ae262003b5089b041523ef741ba4af806428a2dae40a`.
- Actual checked-in package manifest: `cargo run --manifest-path
  /home/champii/prog/rust/Rock/Cargo.toml -p rock -- build --debug`, from
  `lib/tests/fixtures/macros/block_dsl`, passes. This separately validates the
  fixture's relative SDK dependency and proc-only library role.
  `macro-b20-fixture-package-final.log`, SHA-256
  `03295549695e9dec4363f4162f6613eceeab5df5abbac24158ccb7cf82e91b26`.
  The first manual attempt timed out at 120 seconds during host compilation;
  it is not acceptance (`macro-b20-fixture-package.log`, SHA-256
  `9485fec8a9f45a7e905aded1a5e39ed51bec6820e5811c2c0db8115ad80456fd`).

B.20 is implemented and verified; no functional edits follow the final gates.
Compiler/SDK/schema bytes remain unchanged, so the B.19 complete library receipt
remains the compiler regression baseline; this step refreshes the affected
native SDK/CLI suites. Formatting remains deferred, Beads unavailable, and
unrelated work untouched. B.21–B.22 and Stage C remain unfinished.

### B.21 delivery receipt — external interpolation and runtime formatting

`lib/tests/fixtures/macros/interpolation` contains three ordinary packages: the
`weave` procedural provider, the `ink_runtime` target helper, and a complete
consumer. The provider reads exactly one canonical literal view and emits new
literal text as generated source plus deferred Literal fragments for holes.
It neither decodes Rock quoting nor prints captured expressions. The compiler's
normal generated parser and caller context resolve `Hello {name}` and a
`{count + 1}` expression. Runtime formatting is selected by the helper library's
ordinary `Paint` trait, not a compiler-known formatting/provider name.

Definition-side helper paths carry the declared dependency handle; the consumer
supplies the matching runtime library explicitly, without a target SDK dependency.
Library grammar errors use the supplied decoded-to-input scalar mapping. The
fixture deliberately supports only non-nested braces; nested/empty/unmatched
holes diagnose rather than guessing a complete Rock-expression grammar.

Native SDK acceptance executes the full existing cross-language and B.18–B.20
controls, then proves caller bindings, numeric expressions, exact missing-name
and library-grammar spans after escaped/Unicode prefixes, and correctly quoted
literal output. Unsupported values fail the ordinary Paint bound; omitting the
runtime library fails the existing dependency-authority check. CLI acceptance
builds/runs the same ordinary packages and repeats caller/missing-name/formatting/
runtime checks. The checked-in package manifests also run directly and print
`Hello Ada: 3` with exit status zero.

Final commands/logs under `/tmp/opencode/`:

- `cargo test -p rock-lib --test macro_sdk -- --nocapture`: one native integration
  passes, `macro-b21-sdk-final.log`, SHA-256
  `058bee83bf490af470b78711d70098772ad6074c66fb85a0ab02457824ab5632`.
- `cargo test -p rock macro_b_21 -- --nocapture`: one CLI test passes,
  `macro-b21-cli-final.log`, SHA-256
  `c4337b9983c820bf86a46682dab1907ddd709441feda1bff8857cf0b0be175ce`.
- `cargo check --workspace --all-targets`: pass, `macro-b21-check.log`, SHA-256
  `bdccac4e8ff01264dfeb301ff82c79b32ded734787c9bf2642f60379e813f2c6`.
- `cargo clippy --workspace --all-targets`: pass with existing warnings,
  `macro-b21-clippy.log`, SHA-256
  `f44fc05c030cca445d5f562822e3fa7b62ce7dc113e464a97f9268706d436a78`.
- `cargo build --release`: pass, `macro-b21-release-build.log`, SHA-256
  `ec89a08c8aff871a1163521be8184ed52b32a71e074550a1cf801f8eb03448d8`.
- `cargo run --manifest-path /home/champii/prog/rust/Rock/Cargo.toml -p rock --
  run --debug` from the checked-in interpolation fixture passes,
  `macro-b21-checked-in-example.log`, SHA-256
  `90eba82a828e4ae3d4cb6d1f6030204c018c96c71436586f35f02f64f3abd880`.

Failed attempts are retained, not counted as acceptance:

- Initial CLI setup probes (`cargo test -p rock macro_b_21 -- --nocapture`):
  `macro-b21-red.log` (missing explicit std), SHA-256
  `b81ac959451e229902b22437762a9b70a6b895001c709d19c6f7ae35763def21`;
  `macro-b21-red-behavior.log` (invalid export marker on a signature), SHA-256
  `5f6508217de87f97b4859b53c31bf5431d3b74559f151d14bb0f83fd0fd9fd6e`;
  `macro-b21-red-interpolation.log` (noncanonical package alias in CLI input),
  SHA-256 `d8e7a479b58dd6218ba472c9803759a6be481b07e0fd2bcef6674fccbf76a70a`.
- Behavioral RED is `macro-b21-red-policy.log`: identity output prints literal
  braces instead of interpolating, SHA-256
  `d8103d9316a734228bdb65e07535e3d71b5735b88a1b6ab6c0c29c7971f001c7`.
- Direct provider `rock build` initially rejected a temporary receiver borrow:
  `macro-b21-provider-build.log`, SHA-256
  `f27fa504137296d5d73ba3228da00d5547842a69f1f71460b79280ee1c4b3ba1`.
  Binding the new prefix String before concatenation compiles:
  `macro-b21-provider-build-final.log`, SHA-256
  `caf6d4cbd7c33f0a59c6461be5e07b00e5c236ae27ba79d53cdf3bd4c50eef6d`.
- Direct example `rock run` exposed an extra generated closing parenthesis:
  `macro-b21-example.log`, SHA-256
  `1d3010627c037713c78e3544efc001b32d906baf385e36767f10e9f2e8833dce`;
  its correction passes in `macro-b21-example-final.log`, SHA-256
  `3664ff38c4f1040832c66c7482a8b10e3cbf0ca929a534c3ebf7a7cebc0536a5`.
- An intermediate CLI gate reached the unsupported-value control but used an
  invalid nullary constructor call, corrected to a value: `macro-b21-cli-green.log`
  (despite its name, failed), SHA-256
  `923a4ec98edd0a8c486807a9a7c8119bc18d6b58c84912b59c0a493fba030685`.
- Initial native SDK attempt used an alias forbidden by the product loader's
  existing name-equality contract. Final fixtures use canonical artifact names;
  no general dependency-renaming support is claimed or added.
  `macro-b21-sdk.log`, SHA-256
  `1564176b189f6c9425178b59b1d9316b8c30c3c83aacde9973ab02972a4815f5`.

B.21 is implemented and verified. No functional edits follow the final gates.
Compiler/SDK/schema bytes remain unchanged; the B.19 full library receipt remains
the compiler regression baseline. B.22 evaluation/ownership/source-free proofs
and Stage C remain unfinished. Formatting remains deferred, Beads unavailable,
and unrelated work untouched.

### B.22 delivery receipt — evaluation, ownership and source-free consumers

The interpolation fixture now compares an effectful macro consumer against
equivalent ordinary helper calls. Both execute each hole once left-to-right,
paint borrowed named values repeatedly while retaining their later uses, and
destroy each tracked temporary exactly once with identical observable traces.
Caller locals named `ink_runtime`, `value`, `text`, and `concat` resolve inside
holes without capturing definition-side helper paths. A call inside a hole still
consumes its String argument; the ordinary Borrow diagnostic maps its move label
to the original literal substring and rejects the later use.

The new CLI packaged consumer combines raw HTML-shaped block DSL input with an
interpolated arithmetic hole and a real runtime helper. Acceptance first runs
the source-backed packages, copies exactly two proc artifacts/workers and two
target artifacts/objects (std and ink_runtime), relocates the bundle, deletes all
copied provider/DSL/SDK/runtime/std source trees/manifests and the original build
tree, then compiles/runs the consumer with only explicit bundled artifacts.
The result is `source-free 42`; all bundled metadata, workers and objects retain
identical bytes. No SDK or host dependency objects are supplied to the target.

Final serialized gates and logs under `/tmp/opencode/`:

- `cargo test -p rock-lib`: 2,731 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, one native SDK integration and one
  parser integration pass; one doctest is ignored. This includes all B.18–B.22
  consumers and the new ownership/consuming-call controls. `macro-b22-library.log`,
  SHA-256 `921ed82b6994dceedc889cf7e1e8c36028693ef86259ceb0e762990e67ca5746`.
- `cargo test -p rock -p rockc -p rock-shared`: 99 application CLI, 57 compiler
  CLI and 30 shared units plus one shared doctest pass. This includes existing
  worker/process/build/freshness/host-target identity controls and all new DSL,
  interpolation and source-free package tests. `macro-b22-cli-shared.log`, SHA-256
  `f43fbeb530056b5aefbf7587005293f3c1042155a31761f55d09d16c044c50f1`.
- Focused `cargo test -p rock macro_b_22 -- --nocapture`: one passes,
  `macro-b22-cli.log`, SHA-256
  `de75e43ce3895d92404f0c55f49dd4daa9d4456be7f968ac040e4dd520ece706`.
- `cargo check --workspace --all-targets`: pass, `macro-b22-check.log`, SHA-256
  `ebd2f7740b762de18bb6c7697add38ad45273e47d3ab07f3b7d3e57488b4c178`.
- `cargo clippy --workspace --all-targets`: pass with existing warnings,
  `macro-b22-clippy.log`, SHA-256
  `e593409e9998ab50dba82a8a647f9fdf5c828ee2c7d9f933652917ade387d8d2`.
- `cargo build --release`: pass, `macro-b22-release-build.log`, SHA-256
  `7e4b9810c8909b8e34e0be5aff57552a57bb1fe531998d4c0a0c873892f36322`.
- Direct `cargo run -p rockc -- --entry-file .../consuming.rk ...` rejects the
  later use with `borrow of moved value owned`, including the original hole's
  exact move range. Expected exit-1 `macro-b22-consuming.log`, SHA-256
  `f24c09ed9d2ca51ac446d46bd92b391ec43368a04db5315d8599c9c9f73f22db`.

Authoring/verification corrections are retained, not acceptance failures hidden
by narrowed observers. Direct ownership compilation first found a private field
in the fixture (`macro-b22-ownership-build.log`, SHA-256
`e095d2b6a6b0808f9af0dd8ed39ec4272af24537767eb7911e4e027b01f0f081`),
then a reference-to-temporary error in direct comparison expressions that stored
intermediate results unlike the immediate-print macro expressions
(`macro-b22-ownership-build-final.log`, despite its name failed, SHA-256
`a00234171d631d260ecb97e9ba727cb93bc51f92102602a1cd375040eb4bf76a`).
Matching the immediate-use scopes compiles (`macro-b22-ownership-build-3.log`,
SHA-256 `6407befa5529fad9560b18fbc0ec4ef81e4ed97d8a477c9f29a2ebbe718c69a4`).
The first native attempt passed the execution/trace checks but incorrectly used
frontend-only `analyze` to expect a MIR Borrow failure. The final gate uses
`compile` and checks the moved-value diagnostic and precise move label. Failed
`macro-b22-sdk.log`, SHA-256
`ddb16c228f1c579120ec9432410ed7efd99bd1d9ca1bb1d8dcd74af3568b63ca`.

Runnable foreign-target acceptance was not exercised. `command -v` probes found
neither an AArch64 linker/emulator nor a MinGW cross compiler (Wine is present);
`macro-b22-cross-tools.log`, SHA-256
`339e78ad215a963fe262af2b9774a8d2b98a21a0a0a5892cabd0a7d379a0a5b6`.
Unconditional simulated host/target identity tests pass in the shared/CLI gate.
No cross toolchain installation or unsupported target execution is claimed.

B.22 and the native Stage B gate are implemented and verified. No functional
edits follow the final gates; compiler/SDK/schema bytes remain unchanged in
B.20–B.22. Stage C remains unimplemented. Formatting remains deferred, Beads
unavailable, and unrelated work untouched.

**B gate (native evidence above):** all `macro_b_` families, the real SDK cross-language goldens, existing
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

#### C.01 completion receipt

Struct/enum AST nodes now retain `DeclarationInput`: an immutable logical range
and physical header margin over shared original lossless backing. Parser input
clones that backing once on demand, not once per declaration. The direct grammar
parsers retain their input; the top-level boundary includes inline visibility,
owned indentation/trivia and language markers. Token-only/synthetic AST fixtures
explicitly have no original input; there is no AST-printing fallback.

Six `macro_c_01_` unit tests cover exact struct kind binders/defaults, field
visibility/defaults, CRLF/comments, enum variant shapes/order, nested margins,
EOF/bounded input, shared backing across consumers and AST mutation, mixed
generated/captured contexts and diagnostic projections, retained expression
captures, serialization, and checked invalid-range rejection. Materialization
uses the existing lossless slice contract, preserving runs and their origins.

Final verification (suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_c_01_ -- --nocapture`: 6 pass.
- `cargo test -p rock-lib`: 2,737 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass; ten native SDK acceptance groups and one doctest
  remain explicitly ignored under the manual-acceptance split.
- `cargo test -p rock -p rockc -p rock-shared`: 94/57/30 units and one shared
  doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Receipts are `/tmp/opencode/macro-c01-{focused-final,library-final,cli-shared,clippy,release-build}.log`.
The first library attempt hit `Text file busy` while spawning an existing native
worker fixture; its fully qualified exact retry and the complete library rerun
both passed without source changes to that fixture or transport.

This receipt completes declaration retention only; C.02 attachments, derive
dispatch and `rock_syntax` were still pending at that snapshot. Worker protocol 7
and product format 54 are unchanged. No formatter or formatting check was run.

#### C.02 completion receipt

The parser now retains ordered `%derive path, path` attachments in a typed
`DerivedDecl` wrapper whose target can only be a struct or enum. Consecutive
directives, qualified paths, trailing commas, comments/blank lines, visibility,
and nested scopes are supported. Each attachment has independent invocation
identity, syntax contexts and expansion parent; the declaration has stable node
identity. Parsing the target independently preserves its original C.01 input
without any attached directive. Reserved derive prefixes commit before ordinary
macro/block alternatives, preserving requested lexical failures.

`macro derive Name = ...` declarations use the existing source proc-macro target,
namespace and ordinary export rules, publishing `DeriveLike` metadata. At the
stable item-expansion frontier, metadata-only validation resolves attachments
through the shared registry and rejects missing/private paths, wrong kinds and
canonical duplicates without starting workers. Artifact metadata accepts derive
exports but still rejects attribute kinds. Host bootstrap callable selection
remains function-like only; derive bodies retain the ordinary implementation
signature-checking path, including providers with no callable exports.

Fourteen `macro_c_02_` tests cover these paths, malformed/empty lists,
missing/intervening/unsupported targets, scope escapes, Rust-style/arbitrary
attributes, generated declarations/attachments, retained input, serialization,
source-file visibility, invalid synthetic metadata, and wrong-kind item and
expression calls. The existing bootstrap parser test also covers zero callable
entries. The complete `macro_` unit group passes all 440 tests with one test
thread; its initial parallel run hit the previously observed `Text file busy`
race in two existing native-worker fixtures, without changes to those fixtures.

Final verification (test suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 440 pass, including
  all 14 C.02 tests and adjacent declaration-retention, source-export, bootstrap,
  artifact, hygiene and expansion regressions.
- `cargo test -p rock-lib`: 2,751 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass; ten native SDK acceptance groups and one doctest
  remain explicitly ignored under the manual-acceptance split.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` pointing to
  the existing workspace `target/` development bundle: 94/57/30 units and one
  shared doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Receipts are `/tmp/opencode/macro-c02-{macro-regressions-serial,library,cli-shared-final,clippy,release-build}.log`.
The initial CLI run resolved Cargo's redirected cache target as its explicit
sysroot, which had no bundled std. The fully qualified sysroot control and the
complete CLI/shared rerun passed with the existing development bundle supplied
through `ROCK_SYSROOT`; its artifact/object hashes remain unchanged, with no std
rebuild or compiler-loading change.

This completes attachment syntax and metadata validation only. Compilation
explicitly rejects unexecuted attachments before ordinary collection; C.09 owns
derive dispatch/consumption and `rock_syntax` begins at C.03. Worker protocol 7,
`RKLOSS04` and product format 54 are unchanged. No formatter, formatting check or
CI workflow was added or run.

### Cb. Grow the syntax library one grammar family at a time

| Slice | Implement only | Focused acceptance | Runner |
| --- | --- | --- | --- |
| C.03 | Add standalone Rock syntax cursor, bounded lexical/group interpretation, and parse-error data over SDK lossless streams. | Parse supplied units without `MacroContext`/worker/RPC; compare shared positive/negative lexical/group fixtures with the Rust interpretation. Preserve trivia/contexts and diagnose malformed requested Rock syntax without preventing neutral inspection. | I |
| C.04 | Parse/emission-support the initial derive type syntax needed by fixtures: names/paths, applications, references, and other explicitly enumerated forms. | Shared compiler/library type fixtures and unchanged-node round trips pass; unsupported forms and trailing tokens diagnose at the right spans. No alias resolution, inference, or operator meanings are implemented in the library. | I |
| C.05 | Parse generic binders, supported bounds/kinds/defaults, and distinguish binder syntax from type-application arguments. | Earlier-parameter/default cases and nested comma/group boundaries match the Rust parser. Emitting impl binders/type arguments is correct; unsupported grammar does not get guessed into another node. | I |
| C.06 | Add `DeriveInput` struct parsing with ordered fields, visibility, and opaque default-expression slices where boundaries are established. | Named fields, generics, defaults, exact source spans/trivia, full input consumption, malformed fields, and unsupported boundaries agree with shared fixtures; no compiler-prepared declaration structure is used. | I |
| C.07 | Extend `DeriveInput` to enums and named/positional/nullary variants supported by Rock. | Variant/field order and shape, nested types, and generic cases match the compiler; malformed/unsupported variants report structured errors. Struct handling continues to pass. | I |
| C.08 | Complete token emission/builders for transformed supported nodes and the impl syntax required by the derive proofs. Keep quote-style conveniences ordinary library helpers. | Parse/emit/parse and edited-node tests preserve unchanged slices/contexts/trivia and grouping. Captured input names bind correctly in emitted impls; construction does not stringify captured subtrees or require a quotation proc macro to build itself. | I |

#### C.03 completion receipt

New ordinary package `rock_syntax/` depends on the SDK, never the reverse. Its
`cursor::Cursor` validates SDK preservation data and indexes UTF-8 scalars without
requiring valid Rock syntax. Explicit peek/consume operations interpret supported
lexical pieces lazily, preserve trivia and opaque captures, and leave a failing
piece unconsumed. Tokens retain logical ranges and agreed binding contexts;
materialization uses checked original SDK slices, not reconstructed spelling.
An explicit group request validates the remaining input iteratively and returns
flat preorder delimiter/range/parent records. Typed parse errors carry logical
input ranges, optional related opening ranges, and standalone SDK diagnostics;
rejected caller slice ranges cannot produce invalid diagnostic labels.

The documented initial lexical subset covers ASCII names/keywords, current
numeric/prefix/symbol spellings, spaces/tabs/CR/LF, quoted boundaries, comments,
and explicit groups including repetition delimiters. Unicode/NUL within quoted
or comment text and opaque captures stay intact. Non-ASCII identifier/whitespace
classification is explicitly unsupported, not approximated; there is no Unicode
classification API in the current std. This slice does not synthesize layout
tokens, decode literal values, parse types/declarations, resolve operators or
emit syntax. Defaults/hard ceilings are 1 MiB logical input, 65,536 pieces,
128 group levels and 16,384 groups, with checked lower caller limits.

Twenty-five shared positive/negative fixtures compare the actual Rust and Rock
lexical/group interpretations, including classifications/ranges, binding-context
presence, group ranges/parents and error categories/ranges. Supplementary native
controls cover join/context boundaries, preserved snapshot/context/origin data,
opaque/nested/interrupted captures, unsupported forms, invalid SDK data/slices,
limits, EOF and failure rollback. The native proof compiles the ordinary library
artifact and an ordinary in-memory client with proc execution disabled; it also
compiles/runs the complete README example. No `MacroContext`, worker or RPC is
needed at runtime. Rust oracle coverage stays in default regressions; native
conformance is an explicit ignored group included by `cargo macro-acceptance`.
The test-only Rust wiring keeps private compiler lexical views private and
reuses the SDK prerequisite cache rather than adding a new transport encoding.

Final verification (test suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_acceptance_syntax_cursor_and_groups -- --ignored --nocapture`:
  native conformance, supplementary controls and README example pass.
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 442 pass; the new
  native syntax group is ignored here and was executed explicitly above.
- `cargo test -p rock-lib`: 2,753 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass. The native syntax group, ten SDK acceptance groups
  and one doctest remain explicitly ignored in this default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` pointing to
  the existing workspace `target/` development bundle: 94/57/30 units and one
  shared doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.
  Clippy also reports the intentional repeated inclusion of the shared native
  prerequisite cache-key helper inside compiler tests; no lint is suppressed.

Receipts are `/tmp/opencode/macro-c03-{native-final,macro-regressions,library,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes remain unchanged. No formatter,
formatting check, CI workflow, SDK wire layout or compiler-owned package loading
was added. Worker protocol 7, `RKLOSS04` and product format 54 remain unchanged.
C.04 type syntax, later declaration parsing and C.09 derive dispatch are not
implemented by this receipt.

#### C.04 completion receipt

`rock_syntax::types` now owns an immutable `TypeSyntax` tree over its original
SDK stream, with a flat postorder `TypeNode` arena, explicit grouping, earlier
child indices, source ranges and original name contexts. Complete-input parsing
supports ASCII names, the compiler's two-name associated paths, applications and
comma ownership, shared/mutable references, tight raw pointers, unit/grouping/
tuples, slices, checked U64 decimal array lengths, holes and function arrows.
Flat arrow chains are parsed iteratively; recursive grammar nesting and stored
nodes have checked limits (128 / 65,536 hard ceilings). No name/alias resolution,
kind/arity inference, semantic operator lookup or declaration parsing is added.

Whole-input `emit` and checked `node_tokens` preserve original SDK data rather
than printing ASTs, including raw leading-zero array lengths, comments/CRLF,
snapshots and independently captured name contexts. Invalid node indices diagnose.
Cursor spelling inspection now validates range/UTF-8 boundaries against its
existing text index without cloning/revalidating a complete SDK snapshot for
each lexeme. Type parse errors cover missing operands/names/closers/lengths,
overflow, trailing tokens and explicitly unsupported syntax at input ranges,
including EOF and related opening-delimiter labels.

The supported grammar and limits are documented in `rock_syntax/README.md`.
Type lambdas, non-ASCII names, binder/default/bound grammar and constant-expression
array lengths are not guessed into supported nodes. Lowercase module-qualified
annotations and longer associated paths are rejected consistently with the
current compiler type grammar. The enclosing declaration parser still owns
physical layout and field/default boundaries; this standalone token entry point
treats lexical trivia as trivia. Transformed type/impl builders remain C.08 work.

Sixty-one shared type fixtures cover 39 accepted forms, 20 malformed/trailing
forms and two compiler-valid forms explicitly outside this initial library subset.
The native client compares structural shapes against the actual Rust type parser
(not its formatter), validates error codes/ranges, and checks whole-input and
root-node parse/emit/parse round trips. Supplemental controls cover mixed name
contexts, snapshot retention, invalid node/spelling access, opaque captures,
lower/invalid resource limits, and a 161-argument flat arrow chain under a
one-level recursive grammar limit. The existing 25-case C.03 native proof and the
expanded, complete README example also pass; no runtime worker/RPC is involved.

Final verification (test suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_c_04_`: shared Rust type oracle passes.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  both native syntax groups pass, including type controls and the README example.
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 443 pass; two native
  syntax groups remain opt-in and were executed explicitly above.
- `cargo test -p rock-lib`: 2,754 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass. Two native syntax groups, ten SDK acceptance groups
  and one doctest remain ignored in this default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` pointing to
  the existing workspace `target/` development bundle: 94/57/30 units and one
  shared doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Receipts are `/tmp/opencode/macro-c04-{oracle-initial,native-verified,macro-regressions,library,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes remain unchanged. No formatter,
formatting check, CI job, SDK wire-layout change or compiler package special case
was added. Worker protocol 7, `RKLOSS04` and product format 54 are unchanged.
C.05 binders/bounds/defaults and C.06–C.12 remain separate, unimplemented slices.

#### C.05 completion receipt

`rock_syntax::types` now provides standalone `GenericSyntax` and `BoundsSyntax`
entry points over the same bounded type parser/flat node arena. Declaration
parameters retain ordered names, constructor-hole applications or explicit kind
roots, optional default-type roots, whole-binder ranges and original name
contexts. Empty/trailing-comma binder lists, brace defaults, grouped constructor
binders, and explicit `Type`/arrow kinds match the Rust parser. Separate `where`
constraints retain subject/bound roots and ranges, with the compiler's nested
application/comma ownership. These entry points consume the complete input;
inline declaration bounds, unsupported default types, malformed kinds and
trailing tokens diagnose instead of becoming another node. No declaration-scope
lookup, earlier-name validation, kind inference or compiler AST projection is
implemented in the library. Type/default/constraint nodes share one input budget.

Unchanged `emit` retains original SDK data. The supported transformed
`type_arguments` and `impl_binders` helpers remove defaults and preserve copied
names/constructor sections, contexts, inner trivia and provenance. Impl binders
mean unbounded `where` subjects, not a new declaration-style impl header. Both
helpers group constructor sections through a caller-supplied ordinary SDK
`MacroBuilder`: bare `F` would declare kind `Type` in an impl target, while
ungrouped `F _, _` can absorb later comma-separated parameters. Plain and
explicit `Type` parameters emit names. Other explicit kinds remain parseable and
unchanged-emittable, but transformed impl/type-argument emission explicitly
returns `binder-emission` at the whole-binder range; it does not invent an impl
kind binder, guess a higher-order arity, or erase that kind. General transformed
builders remain C.08 work, and the README documents this initial emission subset.

Sixty-four shared fixtures (45 binder / 19 constraint cases) cover 37 accepted
parses, 26 malformed/trailing cases and one compiler-valid default type lambda
outside C.04's subset. They compare actual Rust parser structure, nested comma
ownership, explicit/grouped kinds, earlier/chained/forward/default names and
error ranges. Six accepted explicit-kind forms additionally require the
documented transformed-emission error. Supported emitted fragments are checked
against the Rust type/where parsers, then the actual native library output is
used in an ordinary compiler/runtime proof: earlier-parameter defaults and an
`Option` constructor default run, and an explicit `Type`-kind impl typechecks.
The latter check uses spelling only inside the test harness, not library
emission; real hygienic impl transport remains C.08–C.11 acceptance work.

Native supplementary controls cover whole-input and individual-binder round
trips, original snapshots and independent name contexts, generated grouping,
source provenance, lexical/type limits, invalid limits, opaque defaults and SDK
construction exhaustion. C.04 still exercises every node's checked slice API
after sharing its structural-shape helper with this group. The C.03 proof and
complete README example continue to pass.

Final verification (test suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 444 pass; three
  native syntax groups remain opt-in here.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  all three native groups pass, including the emitted impl/runtime proof.
- `cargo test -p rock-lib`: 2,755 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass. Three native syntax groups, ten SDK acceptance groups
  and one doctest remain ignored in this default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` set to the
  existing workspace `target/` development bundle: 94/57/30 units and one shared
  doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Final receipts are `/tmp/opencode/macro-c05-{macro-regressions,native-complete,library,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes remain unchanged. No compiler
semantics, SDK wire layout, package-loading special case, CI job or formatter was
added; worker protocol 7, `RKLOSS04` and product format 54 are unchanged.

The initial runtime experiment also exposed a separate existing compiler limit:
`Holder T, {F _ = Result T, _}` with a `Holder I64` construction reports
`Type mismatch: _ vs I64`. A direct existing release `rockc` compilation of
`/tmp/opencode/c05-partial-constructor-default.rk`, without any macro, syntax
library or emitted impl, reproduces it in
`/tmp/opencode/macro-c05-partial-default-baseline.log`. The shared parser fixture
still retains that default form; this receipt certifies its syntax, not runtime
default inference. The runtime emission proof instead uses `Option`. Compiler
default-inference repair is outside this grammar slice; `bd` remains unavailable
in this environment (`command not found`), so no issue was created for it.
C.06–C.12 remain separate, unimplemented slices.

#### C.06 completion receipt

`rock_syntax::derive_input::DeriveInput` now parses one complete struct from its
original SDK stream, with declaration visibility/name, ordered generic
parameters, ordered named fields, field visibility, type roots, exact logical
ranges and opaque default-expression ranges. Header and physical field slices
partition every original input byte, including comments, blank lines and CRLF.
Unchanged `emit` and checked `field_tokens` use retained SDK data; invalid field
indices diagnose without panicking. No compiler-prepared declaration structure,
semantic name lookup, inference, expression AST or worker/RPC is involved.

The type/binder engine is extracted into a private ordinary library module and
reused through bounded token windows. All declaration parameters, kinds,
defaults and field types share one flat type-node arena/budget and one retained
stream, rather than cloning a source snapshot into each field. Lexical/group
limits remain checked; `Cursor::all_groups` adds a non-consuming whole-input
group scan after lexical indexing. `DeclarationLimits` separately bounds fields
(16,384 standard/hard ceiling), including lower/zero and invalid settings.

The documented initial layout subset supports space-indented even declaration
and field margins through 254 bytes, original nested margins, single-physical-
line headers/type annotations, and defaults beginning on their '=' line with
optional indented bodies/continuations. Delimiter groups, quoted/comment text
and opaque expression captures cannot introduce accepted false sibling fields;
multiline token tails remain on their actual closing physical line. Default
bodies are preserved, not expression-validated or printed. Non-default type
continuations, newline-only operands after '=', unsupported margins/types,
non-struct kinds, malformed fields and trailing declarations diagnose. No struct
`where` grammar or compiler layout/typing semantics are added.

Forty-five shared struct fixtures cover 22 accepted declarations, 20 malformed/
trailing cases and three compiler-valid forms outside this slice (enum input,
type-lambda annotations and tab-indented margins). The actual Rust source
parser is the structural/visibility oracle; independent fixture spellings check
exact name/type/default ranges and retained text. Coverage includes generic
defaults/kinds, nested types, duplicate/unknown names as syntax, EOF, visibility,
CRLF/comments, macro defaults, indented instance/lambda defaults, multiline
quoted field lookalikes and postfix code after a multiline token. Native checks
also reconstruct the complete input from layout slices and compare unchanged
parse/emit/parse field structure/ranges.

Supplementary controls cover a shared header/field node budget, field limits,
byte/piece/group limits, invalid limits/SDK data/access, independent declaration
and field contexts, conflicting name contexts, and opaque captures containing
otherwise invalid delimiters and field lookalikes. The new complete README
declaration example typechecks as part of the native client. The original README
program, C.03 cursor/groups, C.04 types and C.05 binder/emission/runtime proofs
continue to pass after the private parser extraction.

Final verification (test suites executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 445 pass; four native
  syntax groups remain opt-in here.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  all four native groups pass (195.05 seconds including cold prerequisites).
- `cargo test -p rock-lib`: 2,756 units, 806 main integrations, six diagnostic
  integrations, 14 process integrations, four SDK smoke/helper tests and one
  parser integration pass. Four native syntax groups, ten SDK acceptance groups
  and one doctest remain ignored in the default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` set to the
  existing workspace `target/` development bundle: 94/57/30 units and one shared
  doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Final receipts are `/tmp/opencode/macro-c06-{macro-regressions,native-final,library,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes remain unchanged. No compiler
semantics, SDK wire layout, package special case, CI job or formatter was changed;
worker protocol 7, `RKLOSS04` and product format 54 remain unchanged. `bd` is still
unavailable (`command not found`). C.07 enums, C.08 edited builders and C.09–C.12
derive dispatch/proofs/final closure remain separate, unimplemented slices.

#### C.07 completion receipt

`DeriveInput` now accepts complete enum declarations as well as structs.
`decl_kind` distinguishes the two; ordered `variants` retain original name
tokens, explicit `VariantShape` (`Nullary`, `Named`, `Positional`), named fields,
positional type roots, significant ranges, physical layout ranges and variant
header ranges. All generic and field types remain in the declaration's shared
flat arena/stream. Struct `fields` and enum `variants` are separate views, not
an ambiguous flattened list. Checked `variant_tokens` retains original SDK data
and diagnoses invalid indices; unchanged `emit` still copies the original input.

Positional headers mirror the actual compiler declaration-argument grammar:
grouped types are individual fields, while ungrouped types own nested
application/arrow commas. Applications, tuples, unit payloads, references,
arrays, constructor sections, grouped functions and trailing commas retain
their structure and spelling. Named fields reuse the struct parser and opaque
default boundaries, with one additional declaration indentation step. Generic
defaults/kinds, visibility, original nested margins and source order survive
both paths. No resolver, inference, expression AST or compiler-prepared
declaration data is queried.

The private parser now owns the shared declaration-field/variant routines.
`DeclarationLimits::max_variants` has a 16,384 standard/hard ceiling;
`max_fields` is a cumulative named-plus-positional budget across the entire enum,
not a per-variant reset. The existing lexical/group/type budgets remain shared.
Zero/invalid limits, malformed siblings/fields, unsupported payload types,
discriminants, variant visibility and trailing declarations diagnose.

Mixed inline positional/named bodies are explicitly unsupported: the current
Rust parser can consume such a body while retaining only its positional fields,
so the library returns `variant-shape` rather than dropping the named family.
The supported subset and this boundary are documented in the README. Type
lambdas remain outside C.04's type subset. Edited enum/impl construction remains
C.08 work, not a new quotation macro or an AST-print/re-lex fallback.

Forty-one shared enum fixtures cover 24 accepted forms, 15 malformed/trailing
cases and two compiler-valid forms explicitly outside the subset (mixed bodies
and type-lambda payloads). The actual Rust parser checks variant/field order,
shape and nested type structure; independent raw spellings check exact
name/type/default ranges and trivia. Native acceptance reconstructs complete
inputs from header/variant layout slices and named variants from header/field
slices, and checks unchanged round trips, snapshots, source contexts, shared
budgets, zero/invalid limits, checked access and opaque defaults containing fake
variants/invalid delimiters. The complete README enum example also typechecks.
All 45 struct fixtures/controls continue to pass; their former enum-rejection
control is intentionally replaced by a trait-declaration rejection as enum
parsing becomes supported. The existing cursor, type and generic/emission proofs
also pass, with their expected behavior unchanged.

Final verification (suite commands executed serially; warnings remain):

- `cargo test -p rock-lib --lib macro_c_07_`: shared Rust enum oracle passes.
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 446 pass; five native
  syntax groups remain opt-in here.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  all five native groups pass in 100.73 seconds.
- `cargo test -p rock-lib -- --test-threads=1`: 2,757 units, 806 main integrations,
  six diagnostic integrations, 14 process integrations, four SDK smoke/helper
  tests and one parser integration pass. Five native syntax groups, ten SDK
  acceptance groups and one doctest remain ignored in the default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` set to the
  existing workspace `target/` development bundle: 94/57/30 units and one shared
  doctest pass; five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

The first default-parallel `cargo test -p rock-lib` attempt passed 2,756 units but
failed the existing native-worker raw-context rejection test before request
creation because launching its fixture returned `Text file busy` (`ETXTBSY`).
That fully qualified test passed alone with `--exact --nocapture --test-threads=1`,
and the subsequent complete serialized gate above passed on the same source.
No worker-fixture assertions, worker implementation or retry policy were changed.
The failed run remains `/tmp/opencode/macro-c07-library.log`, with isolated
evidence in `/tmp/opencode/macro-c07-worker-isolated.log`.

Final receipts are `/tmp/opencode/macro-c07-{oracle-initial,macro-regressions,native-final,library-serial,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes remain unchanged. No compiler
semantics, SDK wire layout, compiler package special case, CI job or formatter
was changed; worker protocol 7, `RKLOSS04` and product format 54 remain unchanged.
`bd` is still unavailable (`command not found`). C.08–C.12 remain separate,
unimplemented slices.

#### C.08 completion receipt

The ordinary `rock_syntax::emission` module now supplies bounded output
accumulation, ordered range edits, type constructors, layout translation and impl
construction over the existing SDK output contract. `OutputBuffer` moves owned
parts after complete append validation instead of repeatedly deep-copying the
accumulated prefix. `TokenEdits` checks original logical ranges, ordering, UTF-8
boundaries and opaque captures; unsuccessful replacements leave its state
unchanged. Unchanged emission retains the original stream anchor as well as its
snapshots, contexts, provenance and capture metadata, rather than deriving a new
anchor through a full-range SDK slice.

`TypeEmitter` constructs the supported `TypeKind` forms with checked arity,
grouped application/function operands and reference/pointer pointees, singleton
tuple commas, and full-range unsigned decimal array lengths. Copied nodes and
names remain `OutputPart::Stream`; new syntax uses ordinary SDK construction and
quote helpers. `ImplBuilder` emits inherent/trait headers, grouped WHERE subjects
and bounds, and four-space member layout around opaque signatures and complete
definitions. Internal expression layout remains the author's responsibility;
this does not add an expression parser, general visitor, quotation proc macro or
AST-print/re-lex fallback. C.05's explicitly unsupported constructor-kind
conversion remains diagnosed rather than guessed.

`LayoutEmitter::unindent` translates block-captured physical leading margins
using the existing lexical cursor and original SDK slices/joins. It leaves
multiline quote/comment and opaque-capture interiors untouched and returns a
locally parseable stream with new logical ranges, not generated text. A zero
margin returns the original stream verbatim. Invalid prefixes/margins and
excessive retained copy work diagnose; growing-prefix join work is charged
quadratically. This is a bounded emission operation, not a formatter. Output
accumulation retains the SDK authoring caps of 1 MiB spelling, 16,384 parts and a
conservative 16 MiB retained-storage charge.

The real provider proof exposed one generic compiler authority defect:
`select_inferred_method_candidates` could replace an already concrete receiver
with an unrelated method owner discovered by spelling, causing SDK
`Identifier.tokens` to select `Cursor.tokens`. Candidate inference now normalizes
the actual receiver and requires an unknown nominal head before discovering an
owner. Concrete and unrelated-head rejection checks extend the selection unit
test, and an ordinary `Holder T.tokens` integration with an unrelated method
passes for owned and borrowed receivers. No library API was renamed to hide the
defect, and no compiler-owned syntax-package or operator policy was introduced.

Native acceptance compares fourteen emitted type examples with the actual Rust
type parser, including nested application/function grouping, singleton tuples
and maximum-U64 array lengths. It checks unchanged/edited type, generic, bound,
struct and enum round trips; original trivia, contexts, provenance and anchors;
insertion/removal, invalid/overlapping ranges, UTF-8/capture cuts, arity errors,
failed-append atomicity and all output budgets. Layout controls cover CRLF,
multiline strings, comments, opaque invalid syntax and preserved capture ranges.
The complete README helper also compiles.

A separate function-like provider exercises the existing proc engine without
C.09 derive dispatch: it edits a field type/default into a tuple, emits inherent
and trait impls, emits defaulted scalar and constructor-hole generic targets and
bounds, and operates in independent root/nested scopes with same-spelling types
and traits. A copied default expression still resolves the caller's `value`
function inside a fresh same-spelling helper lambda. The consumer prints
`17 / 53 / 23 / 31 / 41` on separate lines and checks additional tuple/HKT
behavior. Invalid edit and arity requests return ordinary structured macro
diagnostics through real workers.

Final verification (test suites serialized; warnings remain):

- Selection unit and exact field/method integration regression: pass.
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 447 pass; six native
  syntax groups remain ignored in this default group.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  all six native groups pass in 244.15 seconds.
- `cargo test -p rock-lib -- --test-threads=1`: 2,758 units, 807 main integrations,
  six diagnostic integrations, 14 process integrations, four SDK smoke/helper
  tests and one parser integration pass. Six native syntax groups, ten SDK
  acceptance groups and one doctest remain opt-in/ignored in this default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with the existing development
  bundle passed through `ROCK_SYSROOT`: 94/57/30 units and one shared doctest pass;
  five cold macro CLI acceptance scenarios remain ignored.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

During fixture development, a combined short-circuit assertion containing owned
String temporaries exposed an independent MIR drop-flag initialization failure.
Both assertions now run separately with their checks retained; no ownership
compiler repair is claimed here. A macro-free reproduction is
`/tmp/opencode/c08-owned-short-circuit.rk`, with the Cargo/rockc failure retained
in `/tmp/opencode/macro-c08-short-circuit-baseline.log`; the larger isolated
source/MIR evidence remains `c08-client.rk` and `macro-c08-drop-mir.log` in the
same directory. This compiler follow-up is outside C.08's emission contract.

Final receipts are `/tmp/opencode/macro-c08-{selection,field-regression,native-thirteenth,native-final,macro-regressions,library-serial,cli-shared,clippy,release-build}.log`.
The shared development std artifact/object hashes are unchanged. SDK wire
layout, worker protocol 7, `RKLOSS04` and product format 54 remain unchanged.
No CI job, formatter, parser-policy callback or compiler package special case was
added. `bd` remains unavailable (`command not found`). C.09–C.12 remain separate,
unimplemented slices; C.08 does not claim derive dispatch or the final closure.

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

#### C.09 completion receipt

Ready derive declarations now execute in the same deterministic item visitor as
ordinary item invocations, preserving their shared source order. Each attachment
uses the existing proc runner, materializer, diagnostics, output parser and
aggregate budgets with `DeriveLike` kind and `Items` output. The adapter passes
the same immutable original declaration stream to every attachment, together
with its individual invocation ID/location/parent and the original physical
margin. It supplies no compiler declaration AST, literal classification or
block-DSL view. Original default expressions remain unexpanded until the item
fixed point has settled.

The compiler retains the original struct/enum once and appends generated item
lists in attachment order; an empty list is successful, not deletion. A bounded
context-local declaration-node-ID set prevents duplicate representations of one
node from producing another declaration or worker execution. Independently
parsed nodes with identical names, source text and spans remain independent.
There is no span/name cache or persistent global derive registry.

Generated attachments, ordinary item invocations, imports and file modules
return to ordinary bounded discovery before expression expansion. Unresolved
item/derive paths defer while other ready work can publish their imports;
failure is reported at a stable no-progress frontier. The source-order audit
added a mixed derive/item/derive regression before final verification, rather
than retaining a separate derive-after-all-items phase. Canonical duplicates,
wrong kinds and invalid original-input shapes still validate before the
affected declaration's workers start. Generated ancestry, depth, total invocation
and output limits use the shared expansion state.

Metadata-only discovery is now explicit: `discover_proc_macros` validates and
retains attachments without workers. Normal expansion/compilation with proc
execution disabled rejects a selected derive instead of silently leaving it for
collection. A source provider still cannot execute its own implementation during
its build. The strict expansion/collection guard remains, but the obsolete
"derive execution is not implemented" rejection path is removed.

Generated hosts now select both function-like and derive-like public exports.
Their ordinary SDK facade checks the selected identity's exact declared kind
through `entry_kind_matches` and `MacroKind`; unknown/mismatched/attribute kinds
produce structured errors. The obsolete function-only bootstrap helper is
removed, with no compatibility alias or separate worker transport. Provider
implementation ASTs, SDK signature checking, private bootstrap hygiene and
explicit dependency identity remain unchanged.

Ten `macro_c_09_` unit tests cover identical input bytes/contexts and empty views,
retention/output order, original default-macro timing, empty struct/enum results,
cloned versus distinct same-spelling node IDs, generated derives and parent
limits, ordinary non-item rejection, disabled execution, canonical duplicate and
wrong-kind preflight, deferred imports resolving item/derive paths, generated file
modules, aggregate budgets and interleaved source order. The existing C.02
metadata/parser/privacy tests continue to pass with the explicit discovery mode.

The opt-in native `macro_acceptance_derive_dispatch` test builds a real mixed-kind
source provider and consumes its published artifact. Attached `Stamp`/`Empty`
entries retain the struct and emit a callable function, while the ordinary
function-like entry still expands in an expression; the program prints `42 / 7`
on separate lines. Three direct worker requests independently verify that the
generated bootstrap rejects identity/kind mismatches, including an attribute
kind, rather than relying only on compiler-side artifact preflight. This proves
dispatch/publication, not C.10's syntax-library trait derive.

Final source-snapshot verification (test suites serialized; warnings remain):

- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 457 pass; six native
  syntax groups remain ignored in this default group.
- `cargo test -p rock-lib --test macro_sdk macro_acceptance_ -- --ignored --nocapture --test-threads=1`:
  all eleven native SDK/host/derive groups pass in 80.71 seconds, including codec,
  one-shot, publication, literal, DSL, interpolation and authoring regressions.
- `cargo test -p rock-lib --lib macro_acceptance_syntax_ -- --ignored --nocapture --test-threads=1`:
  all six native syntax/emission groups pass in 259.78 seconds.
- `cargo test -p rock-lib -- --test-threads=1`: 2,768 units, 807 main integrations,
  six diagnostic integrations, 14 process integrations, four SDK smoke/helper
  tests and one parser integration pass. Six native syntax groups, eleven SDK
  acceptance groups and one doctest remain ignored in the default gate.
- `cargo test -p rock -p rockc -p rock-shared`, with `ROCK_SYSROOT` pointing to the
  existing development bundle: 94/57/30 units and one shared doctest pass; five
  cold CLI macro acceptance scenarios remain opt-in.
- `cargo clippy --workspace --all-targets` and `cargo build --release`: pass.

Receipts are `/tmp/opencode/macro-c09-{macro-final,sdk-final-snapshot,syntax-final-snapshot,library-final,cli-shared-final,clippy-final,release-final}.log`.
Earlier focused/native development logs are retained separately. The shared
development std artifact/object hashes remain unchanged. Worker protocol 7,
`RKLOSS04` and product format 54 are unchanged; the SDK's ordinary bootstrap
export contract was updated and tested from current source. No CI job,
formatter, std-loading policy or compiler-owned derive meaning was added.
`bd` remains unavailable (`command not found`). C.10–C.12 remain separate,
unimplemented steps: external syntax-library trait proofs and final closure are
not claimed by this dispatch receipt.

#### C.10 completion receipt

The external `metric_derive` fixture now publishes `SumMetric` and the deliberate
negative `UnboundedMetric` as ordinary source derive exports. Its provider is
built against the explicit ordinary SDK and `rock_syntax` artifacts; consumers
receive only explicit std and provider artifacts. The complete build/import/
attachment/worker/`DeriveInput`/`GenericSyntax`/`ImplBuilder` path compiles and runs
without compiler-known trait names, reflection callbacks or syntax-library
package handling.

The fixture-owned `Metric` trait returns `I64`. The derive sums named fields using
`~I64Add`, supplies scalar generic bounds, and emits target arguments without
declaration defaults. A fresh bounded generic helper dispatches the explicitly
caller-owned trait for each field; ordinary unqualified concrete method lookup
would be ambiguous between the intentional root/nested traits. Target and field
names are copied from input streams; trait/member identifiers explicitly request
caller authority. Provider-private `Metric`/`Pair`/`value`, caller functions and
fields, fresh top-level helpers and nested declarations collide intentionally.
All remain independent, including the nested `I64` implementation multiplying
by ten. The executable prints `0 / 18 / 18 / 11 / 19 / 30` on separate lines for
empty, plain, generic, earlier-parameter-defaulted, private-owner and nested cases.

Six negatives are rejected by ordinary compiler rules, not macro errors:
missing concrete impl, missing generic obligations during specialization,
private construction, private access, duplicate impl and overlapping generic/
concrete impl. The enum negative is instead the provider's explicit structured
unsupported-input diagnostic, pending C.11. Field-default ASTs survive expansion;
this does not claim runtime field-default evaluation. A macro-free program with
an omitted defaulted field reproduces an existing MIR panic (reproducer
`/tmp/opencode/c10-qualified-baseline.rk`, log
`/tmp/opencode/macro-c10-default-baseline.log`); executable constructions supply
all fields explicitly. The earlier call-argument failure was fixture precedence,
fixed by grouping `helper (&self.field)`, not a compiler hygiene repair.

Verification on the final code snapshot:

- `cargo test -p rock-lib --lib macro_acceptance_derive_struct -- --ignored --nocapture`:
  one external native group passes in 111.34 seconds, including all six ordinary
  negatives and retained-default checks (`/tmp/opencode/macro-c10-native-final.log`).
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 457 pass, seven native
  groups ignored (`/tmp/opencode/macro-c10-regressions.log`).
- `cargo clippy --workspace --all-targets`: passes with existing warnings
  (`/tmp/opencode/macro-c10-clippy.log`).

Only test registration, provider/consumer fixtures, their driver and docs changed;
no compiler/SDK/syntax production code or schema changed. `bd` remains unavailable
(`command not found`), so the independent field-default limitation could not be
filed there. No formatter, CI job, source std dependency or push was used.
C.11 and C.12 remain unimplemented separate steps.

#### C.11 completion receipt

`EnumMetric` extends the same external ordinary provider to nullary, positional
and named enum variants. Per-variant helpers construct patterns from captured
declaration/variant/field streams, use fresh payload binders, sum measurements
through the explicitly bounded fresh helper, and accumulate arms with ordinary
`OutputBuffer`/`ImplBuilder` APIs. `Cascade` emits a fresh enum carrying another
derive attachment; `After` is a following attachment on the original declaration.
No compiler or syntax-library production extension was needed.

The native packaged test copies only the current proc artifact and its worker
into a two-file package, stores an artifact-relative executable path, relocates
the package, and renames the original host/syntax build directories before any
consumer expansion. Provider input is virtual (there is no provider source file
to reopen). The consumer has only explicit std/provider artifacts, not SDK or
syntax-library dependencies; the published proc products have no runtime bodies
or target dependencies. All ordinary derive construction/parsing code executes
from the relocated native worker.

The generic `Event T, {U = T}` consumer exercises empty, one-field, two-field and
named variants using `Event I64` defaulted applications. It prints
`0 / 3 / 12 / 24 / 0 / 7` on separate lines, including the generated enum and
ordered second attachment. Expansion records are exactly `EnumMetric`,
`Cascade`, `After`, `EnumMetric`, with the last record parented to `Cascade`.
The input is CRLF with Unicode trivia; retained original enum tokens still
contain public visibility, generic defaults, named-field defaults and that
trivia after the real transport. As in C.10, field-default evaluation is not
claimed and every runtime field is supplied explicitly.

Seven negative scenarios assert missing field-trait impls and overlapping enum
impls are ordinary compiler errors, while unsupported targets, non-item output,
wrong macro kinds, canonical duplicate attachments and unbounded recursive
attachments are macro errors. The recursive worker emits another attachment on
the copied declaration until the shared depth limit rejects it. Unsupported
attribute syntax also rejects. C.10's six ordinary negatives, hygiene collisions
and original-default checks remain green with the extended provider.

Development exposed excessive borrow-checker memory use for the original nested
enum-emission function (SIGKILL; phase-isolation logs under
`/tmp/opencode/macro-c11-native-{initial,phases,phases2,borrow-phases}.log`).
The final fixture factors per-variant construction into a smaller ordinary
function and explicitly reborrows its mutable builder; this preserves all
shapes/validation and completes successfully, but is not a compiler resource-use
repair. Temporary compiler phase instrumentation and the one-export isolation
experiment were removed; the final provider publishes all its entries normally.

Final source-snapshot verification, serialized:

- `cargo test -p rock-lib --lib macro_acceptance_derive_ -- --ignored --nocapture --test-threads=1`:
  both native struct/packaged-enum groups pass in 234.82 seconds, including all
  controls (`/tmp/opencode/macro-c11-derives-final.log`).
- `cargo test -p rock-lib --lib macro_ -- --test-threads=1`: 457 pass, eight native
  groups ignored (`/tmp/opencode/macro-c11-regressions.log`).
- `cargo clippy --workspace --all-targets`: passes with existing warnings
  (`/tmp/opencode/macro-c11-clippy.log`).

Only fixture/provider/driver/registration/docs changed. Worker protocol 7,
`RKLOSS04`, product format 54 and production compiler/SDK/syntax code are
unchanged. `bd` is unavailable, so the independent resource-use limitation could
not be filed there. Nothing was formatted, added to CI or pushed. C.12's full
acceptance mapping, residue audit and final-snapshot gates remain outstanding.

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
