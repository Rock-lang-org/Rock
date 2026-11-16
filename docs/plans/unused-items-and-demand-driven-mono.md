# Unused items lints and demand-driven monomorphization

Status legend: `[ ]` todo, `[~]` in progress, `[x]` done.

## Goals

- Rust-like `unused` warnings: unused imports, unused variables/parameters, dead items.
- Follow Rust's model: dead code is still name-resolved and type-checked (errors in dead code
  stay errors), but never monomorphized or compiled.
- Respect module/item visibility (`<` exports) and the bin/lib distinction.
- Drop unused work as early as possible to save compute.
- No performance regression: every new pass must be linear in program size and cheaper than
  the work it removes. Measure before/after (see Phase 0 and per-phase perf checks).

## Decisions

- Placement: lints and item reachability run on resolved HIR right after `infer::finalize`;
  dead items are removed before monomorphization.
  - Not on the raw AST: locals shadow items, imports/globs/re-exports and macros must be
    resolved first.
  - After inference (not at lowering): method, operator and trait-dispatched calls resolve
    exactly, and dead code is still type-checked like Rust.
- Suppression: `_` name prefix, plus CLI `-A/-W/-D <lint>` (allow/warn/deny). No attribute syntax.
- Lint names: `unused_imports`, `unused_variables`, `dead_code`, grouped as `unused`.
- Roots:
  - bin: `main` only (plus required lang items). `<` exports in a bin
    are crate-internal visibility only and never make an item live.
  - lib: items publicly reachable from the crate root through `<` export / re-export chains
    (plus lang items and explicit prelude exports). An item exported from a module that is not itself exported
    upward is not public and can be dead.
- Bin/lib kind is an explicit `Config` field, not inferred from the crate name.
- Unused extern declarations are pruned; a library must export an extern to expose it.
- `_` names and allowed lint levels suppress diagnostics, not pruning.
- Tests never live in business files: unit tests go in `tests/` submodules
  (e.g. `lib/src/<module>/tests/`), user-visible behavior in `lib/tests/integration/`.
- Demand-driven mono is a separate step after the unused-items work; the post-mono DCE is
  removed only once demand-driven mono lands.

## Phase 0 — Baseline

1. [x] Record a performance baseline (release build): wall time per phase for std build and a
   few large examples/test projects; per-phase timing via a lightweight timer if none exists.
2. [x] Record baseline binary/LLVM IR sizes for the same inputs.

Timing: `ROCK_TIME_PHASES=1` prints per-phase laps (`lib/src/pipeline/timing.rs`);
`scripts/bench_phases.sh <out-dir>` runs std + examples. Baseline (release rockc, ms, single run):

| input | parse | collect | lower | infer | mono | mir-bodies | inst-dce | borrowck | emit | IR bytes |
|---|---|---|---|---|---|---|---|---|---|---|
| std (lib) | 1163 | 186 | 1453 | 11690 | 769 | 729 | 0.2 | 226 | 187 | 2200590 |
| hello | 139 | 4 | 5 | 42 | 30 | 0.4 | 0.3 | 0.2 | 17 | 4217 |
| showcase | 139 | 3 | 17 | 82 | 33 | 1.8 | 0.3 | 4.5 | 22 | 68124 |
| string_utils | 153 | 4 | 24 | 90 | 51 | 3.8 | 0.4 | 28 | 37 | 213779 |
| matrix | 124 | 3 | 14 | 64 | 26 | 0.8 | 0.3 | 2.2 | 20 | 40552 |

Example `parse` is dominated by loading the std artifact.

## Phase 1 — Warning channel and lint configuration

3. [x] Add `warnings: Diagnostics` to `CompileOutput`; let `compile`/`compile_with_products`
   return warnings on success (and attach them alongside errors on failure).
   Done: `CompileOutput.warnings`, `Analysis.warnings`, `compile_with_warnings`; lints run in
   the frontend after inference (`LintSink`); a denied lint fails before the backend.
4. [x] Add a lint registry (`LintId`, levels Allow/Warn/Deny, `unused` group) and lint levels in
   `Config`; `Deny` turns the warning into an error. Done: `lib/src/lint/`, `DiagnosticCode::Lint`.
5. [x] Add `-A/-W/-D <lint>` to `rockc` and `rock`; print warnings with the existing ariadne
   reporter (`DiagnosticSeverity::Warning`). Order-sensitive like rustc. `rock` forwards them
   to the project's own crate only; dependencies get `-A unused` (cargo-style lint capping).
6. [x] Add the explicit crate kind (bin/lib) to `Config`, set by `rockc`/`rock` from how the
   crate is built. Done: `Config.crate_kind`, `rockc --crate-type bin|lib` (default bin);
   `rock` passes `lib` for library artifacts.
7. [x] Tests: CLI flag parsing, deny-escalation, warnings printed on successful compile.
   Flag parsing/ordering/forwarding, sink levels, and end-to-end warn/allow/deny
   (`lib/tests/integration/unused_lints.rs`).

## Phase 2 — Unused imports

`lib/src/lint/unused_imports.rs` pairs macro-expanded source import sites with semantic
references already recorded by lowering. Explicit function/extern imports are matched by
written path head **and resolved DefId**, not arbitrary name occurrences. Local parameters,
locals, closure parameters, patterns and fields do not use a same-named value import;
fully qualified calls do not use its short import either. A nearer explicit value import
receives the use instead of an outer import of the same target. Callback references,
references in dead bodies and explicit re-exports still count. This reuses the source map;
it does not add resolver hooks or re-type-check source bodies.

Struct, enum, type-alias and trait imports now use the same resolved-reference matching.
Missing references were added first: lowering records the head item of every qualified
expression/literal/pattern path (`Global::dealloc`, `Option::Some`, `shapes::Point x: 1`)
via `Lowerer::record_path_head_reference`, single-segment struct literals and struct
patterns record their struct, and header type references (bounds, supertraits, impl trait
arguments, signatures) resolve through the declaring module's import aliases instead of a
crate-global name lookup. A module's own struct/enum/trait declaration shadows imports of
enclosing modules. Before switching, a transitional probe logged every import the old
name check called used but references called unused: on std and the full test suite only
intended cases remained (value shadowing tests and the five `net.rk` libc imports).
Imports without a resolved item target (variant and module imports) keep the path-head
name check, where only `a` in `a::B` can use an import and declared names never do.
Release validation: 2421 unit + 1045 integration (29 focused lint tests), 89 `rock` +
51 `rockc`; std warnings identical to the head-only commit.

Globs: `> m::*` is used when a reference's head name resolves to `m::name` for the same
item and no nearer module declaration or explicit import binds that name first (explicit
imports shadow globs at the same level). Glob members without `Definition` references
(enum variant exports, modules) fall back to the path-head name check; variant exports are
recognized through the resolver's variant export table. When any glob exists, the lint
indexes all `Definition` references instead of only explicitly imported ones. A
transitional probe logged every glob reported unused across the full test suite: only the
intended test cases and one artifact app whose `> dep::*` really provides nothing
(`root_glob_import_keeps_local_direct_calls_direct_in_artifact_app`) remained; an earlier
variant-only artifact glob false positive was fixed by the variant-export fallback.
Performance: the extra references first cost std lowering ~13% (1.34 → 1.52 s). Cause:
`SemanticSourceMap::record_reference_in_scope` deduplicated by scanning every recorded
reference (quadratic). References are now indexed by span offsets; std lowering is
~1.345 → ~0.90 s versus the previous compiler (two paired runs), collect/infer unchanged
within noise, LLVM IR sizes unchanged.
Named-crate source graphs can also contain the entry file as a module after a
crate-qualified import; exclude it from file-module lint input to avoid duplicate findings.
Value-import milestone release validation: 2418 unit + 1035 integration tests pass
(18 focused unused-lint tests), plus 89 `rock` + 51 `rockc` tests.
The value-reference index includes only imported callable targets; crates without
explicit imports skip the pass, and type-only imports do not build a value index.
Two paired release runs with `scripts/bench_phases.sh`: combined lint median
4.662 → 4.761 ms on std; example lint medians 0.051–0.117 → 0.043–0.069 ms.
LLVM IR sizes match for std and all four examples. Std total varied
16.342 → 16.618 s, mostly from inference/load/lower before the lint pass;
example totals varied -0.6% to +3.2%. No end-to-end speedup is claimed.
The five new unused extern imports in `std/src/io/net.rk` (`bind`, `accept`,
`connect`, `read`, `shutdown`) are confirmed: their calls use fully qualified
paths. Removing those imports belongs to Phase 6 cleanup.

Macro imports (`lib/src/macro_expansion/imports.rs`): expansion registers each explicit
`> path::macro` site (keyed by file + inline-module lineage + span, so re-expansion rounds
reuse one id) and marks it used when a top-level or expression invocation selects that
binding. A later local declaration of the same name drops the import binding. Recording is
skipped entirely when `unused_imports` is allowed. Release validation: 2421 unit + 1042
integration tests pass (25 focused unused-lint tests).

8. [x] ~~Stable id per import binding~~ → import sites collected from the expanded AST
   (span = whole path).
9. [x] Function/extern/struct/enum/alias/trait imports follow resolved semantic references;
   macro provenance and glob selection done. Variant/module imports (no `Definition`
   references) use the path-head name check.
10. [x] Emit `unused_imports` per unused `>` import, including macro and glob imports
   (`TopLevel::GlobImport` now carries the written path's span).
11. [x] Exclude the injected std prelude (it is not in the AST).
12. [x] Tests: used value/type/variant/trait-bound imports, unused imports, file-module span,
    root import used only by a child module; value shadowing, qualified calls, callbacks,
    dead bodies, nearer imports, re-exports, named crates and extern allow/deny levels;
    macro imports; qualified type paths; type imports used by static calls, patterns and
    bounds; nearer type imports; used/unused globs, explicit imports shadowing globs,
    type members through globs.

Language issue found (not fixed here): imports in the root module leak into child file
modules (a child compiles without its own import). Rust keeps imports module-private. The
lint follows actual resolution, so a root import used only by a child counts as used.

## Phase 3 — Unused variables and parameters (done before Phase 2)

13. [x] Single pass per body over HIR collecting `HirLocalId` reads after inference
   (`lib/src/lint/unused_variables.rs`, shared walker `lib/src/lint/walk.rs`).
14. [x] Emit `unused_variables` for unread locals, pattern bindings and parameters; skip
   `_`-prefixed names, `self`, and bindings without a source span (compiler-generated).
   Assigning to a local is not a read. Cost: ~1 ms on std.
15. [x] Tests: let bindings, pattern bindings, closure params/captures, reassignment, `_` prefix,
   allow/deny.

## Phase 4 — Dead items (reachability) and HIR pruning

16. [x] Compute public roots: bin vs lib rules above, walking `<` export chains from the crate root.
17. [x] Build the item graph over accepted HIR in one linear walk: edges from function/method
   bodies (calls, function values, constructors, struct literals, type mentions, statics),
   signatures and field types, resolved method/operator/trait calls.
18. [x] Follow resolved trait dispatch and library impl availability. Trait impls remain whole
   for effective-member records and downstream specialization; generic dispatch conservatively
   retains the trait's candidate impls. Drop impls remain roots while mono still scans all
   interned types (Phase 5 follow-up). Language items remain roots; unused extern declarations
   are NOT roots unless publicly exported from a library (confirmed revision).
19. [x] Emit `dead_code` for unreachable functions, inherent methods, types, traits, aliases,
   externs and unconstructed variants in live bin enums. Variants retain their layout slots;
   library variants are conservatively retained. No top-level statics exist in the current HIR.
20. [x] Remove dead items from HIR before `mono::monomorphize_with_crates`; keep artifact/product
   interface data for libs intact (exports stay).
   Compilation prunes; editor `analyze` retains the complete typed HIR. `_` and `-A` suppress
   diagnostics only, never pruning. Preserve source-item provenance and TypeId identities;
   filter derived HIR indexes only when items were removed (compact declaration positions
   in linear time). Filter type-location owners rather than rewalking/reinterning bodies.
   Annotation source references preserve alias-use edges erased by type normalization.
   Explicit prelude-export IDs are additional library roots, passed by the pipeline without
   adding std-name knowledge to reachability.
21. [x] Tests: unit tests for roots/edges in a `tests/` submodule; pipeline tests asserting
   warnings and that dead items are absent from `--debug-print llvm` output; bin vs lib roots;
   private-module export case.
   `lib/src/dead_code/tests.rs`: bin/lib roots, private-module exports, direct/glob/module
   re-exports, recursive dead cycles, callable values, aliases, inherent method siblings,
   pattern-only variant mentions, suppression versus pruning, deny, dead-code type errors,
   LLVM/product removal, and incremental indexes/type locations versus full recollection.
   Release validation: 2436 library unit tests + 1023 integration tests, 89 rock tests,
   and 51 rockc tests passed.
   Artifact fixtures now explicitly select Lib for producers and Bin for consumers cloned
   from producer configs; declarations tested as library API are explicitly exported.
22. [x] Perf check: small fixed overhead on live inputs, savings on dead-heavy inputs.
   Reachability + filtering on std: ~3.1–3.8 + 1.9–2.0 ms; examples ~0.25–0.50 ms reachability,
   ~0.01–0.29 ms pruning (release phase timers). Avoided a second complete type-ID collection
   and alias-index rebuild. A no-std program with 1000 unused functions (`-A unused`, median
   of 5 runs) spends ~0.10 ms on reachability + ~0.70 ms on pruning; mono falls from ~9.19 ms
   to ~0.23 ms. Existing example LLVM IR sizes are unchanged.
   Compare using a fresh release compiler built from pre-Phase-4 HEAD, not an installed rock.
   Total compile timings fluctuate with machine load; phase timings isolate the added work.
   Lint accumulation now appends directly, avoiding Diagnostics::push's whole-list clone.

Phase 4 intentionally remains conservative for trait impl candidates, implicit Drop roots,
and library variants. It does not remove variant layout slots or eagerly guess which impl
a downstream generic body will select. Phase 5 can narrow runtime instances without changing
this source-level safety policy. Precise resolved import-use tracking remains separate
unfinished Phase 2 work; non-value imports still use a non-Rust-equivalent name-occurrence check.

## Phase 5 — Demand-driven monomorphization

23. [x] Replace eager body passes with an instance worklist (`mono/demand.rs`).
   Production receives the explicit crate kind and root crate id. Bins seed `main`; libraries
   seed surviving concrete current-crate callable API/helpers after Phase 4 pruning.
   Generic impl members are not roots merely because a particular member signature is concrete.
   The standalone HIR-only API compiles a library (it has no source visibility configuration).
24. [x] Follow direct/method/operator calls, callable values, closures and destruction while
   draining a FIFO body queue. Each instance body is enqueued only on first insertion;
   substituted bodies are registered before processing, supporting recursive specializations.
   Removed recursive eager body processing and the repeated whole-registry scan.
   Destruction demand includes nominal fields, arrays, tuples, owned closure captures, and
   explicit `DropInPlace` pointees; no whole-type-table Drop scan remains.
25. [x] Dependency object function/method metadata is indexed separately from instances.
   Referencing a key interns its object declaration, preserving artifact symbols and avoiding
   any local body emission. Unreferenced object symbols allocate no instance ids.
26. [x] Transitional full release run: 2441 unit + 1023 integration tests passed while the
   old MIR DCE was required to report zero removals and zero missing instance edges.
   Existing materialized-body and MIR/backend agreement validation stays enabled.
27. [x] Remove the post-mono DCE, its superseded tests and DCE-only MIR root/retention tables.
   Product linking now tests the emitted instance set directly. New worklist tests exercise
   dead bodies independently of source pruning, recursive substitutions, callable/closure edges,
   selected trait members, lazy object symbols and structural destruction demand.
   Final post-removal release library validation: 2418 unit + 1024 integration tests pass.
   Release CLI validation also passes: 89 `rock` + 51 `rockc` tests.
   One prior run hit a proc-macro launch error (`Text file busy`); the full retry passed.
28. [x] Compare saved Phase 4/5 release compilers with `scripts/bench_phases.sh`
   (two paired runs) and `scripts/bench_mono.py` (five alternating runs).
   Std median mono: 747.93 → 498.91 ms; MIR bodies: 730.83 → 698.36 ms.
   The 256-impl/one-call benchmark: total 344.16 → 300.46 ms, mono
   13.46 → 6.22 ms, MIR bodies 38.76 → 0.51 ms; LLVM IR stays 2902 bytes.
   Example IR sizes are unchanged or slightly smaller; std 2200590 → 2200589 bytes.
   Small-app mono/MIR timings improve or stay effectively unchanged. Matrix total
   varied upward with load/parse noise; no end-to-end speedup is claimed for it.

## Phase 6 — Wrap-up

29. [ ] Fix or suppress new warnings in `std`, examples and test projects.
30. [ ] Document lints and `-A/-W/-D` in the book and CLI help.
31. [ ] Run `cargo test -p rock-lib --release` serialized; update this file's status.
