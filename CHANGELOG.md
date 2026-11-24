# Changelog

## v0.7.4

[Release notes](docs/release-notes/v0.7.4.md) ·
[Compare v0.7.3…v0.7.4](https://github.com/Rock-lang-org/Rock/compare/v0.7.3...v0.7.4)

- Added compound assignment operators (`+=` through `>>=`) backed by
  `std::ops` traits, including on indexed places.
- Added `RightFoldable`, `foldr`, and the `<&<` and `>&>` fold operators.
- Multiline expressions can alternate operator and method stages.
- Renamed monadic bind from `>>=` to `>>-`.
- Made the compiler faster: speed-optimized release builds, lazy std costs
  for small programs, shared inference tables, narrower coherence checks, and
  cached dependency artifact validation.
- Std is always built optimized with overflow checks.
- Fixed inference of fields on imported structs in signatureless functions,
  compact mutable reborrows (`&^*ptr`), and literal coercion for mutable
  operator receivers.

## v0.7.3

[Release notes](docs/release-notes/v0.7.3.md) ·
[Compare v0.7.2…v0.7.3](https://github.com/Rock-lang-org/Rock/compare/v0.7.2...v0.7.3)

- Added pinned GitHub package dependencies, const generic defaults, and
  `T::Assoc` in function signatures and implementation headers.
- Added array and slice rest patterns, structural `Copy` for arrays with
  `Copy` elements, and explicit `Copy` implementations for eligible structs
  and enums.
- Added string and character helpers, `TryFrom` and `TryInto`, and mapping
  methods for arrays and borrowed slices.
- Added inferred local type hints in the language server and isolated
  diagnostics across package targets.
- Improved trait-qualified calls, method inference, literal coercion,
  struct literal diagnostics, and postfix parsing after calls.
- Reduced copying and scanning of compiler inference state.

## v0.7.2 (unreleased)

[Release notes](docs/release-notes/v0.7.2.md) ·
[Development changes since v0.7.1](https://github.com/Rock-lang-org/Rock/compare/v0.7.1...fee9c914)

### Added

- Added `unused_variables`, `unused_imports`, and visibility-aware `dead_code`
  warnings. `rock build`, `rock run`, and `rockc` support `-A`, `-W`, and `-D`
  for individual lints or the `unused` group. Import tracking covers globs and
  resolved references to values, types, traits, and macros.
- Payload enum constructors can be used as function values, including
  constructors with multiple payloads. `where` clauses can constrain concrete
  types, for example `String: From T`.
- Standard functor and monad operators accept borrowed callbacks, including
  mutable borrows when the callable contract permits them.
- `ROCK_TIME_PHASES=1` prints compiler phase timings. `scripts/bench_phases.sh`
  records timings and LLVM IR sizes for std and selected examples.

### Changed

- Functions without signatures can stay generic across calls with different
  types, including across type constructors. Known caller types can also guide
  field and tuple-index inference; result context better guides `.into!`.
- The compiler prunes unreachable items before specializing concrete instances
  on demand. Unused items are still type-checked; exported library items and
  library implementations for foreign receiver types remain available.
- Direct `.rkca` emission requires `--crate-type lib` and `--no-link`.
  Source-built library artifacts are rebuilt when lint levels change.

### Fixed

- Explicit returns are checked against the enclosing function or lambda and
  terminate their branch. Return mismatch diagnostics point to the declaration
  and the value that established the return type when available.
- Unguarded `Bool` matches covering `true` and `false` are exhaustive.
- Owned array loops move elements that need no cleanup. Destructors needed for
  unnamed owned values are retained during specialization.
- Method lookup explains unmet implementation bounds. Argument-bound failures
  point at the argument, and auto-trait failures can be reported even when part
  of a type is unresolved. Terminal diagnostic labels are shown in source
  order, with primary labels colored by severity.
- Fixed pruned enum variant exports and stale language-server diagnostics.
- Standard-library filesystem, networking, and thread operations propagate
  previously ignored POSIX failures where their APIs can report them.
- Fixed standard-library crate type selection in the `rockup` development
  workflow and a proc-macro executable race.

## v0.7.1 development snapshot

Development changes across 99 commits since `master` at `4f571c70` (v0.7.0),
reviewed through `709b25d8`. This is not a published release or a compatibility
guarantee. [Compare the reviewed revisions](https://github.com/Rock-lang-org/Rock/compare/4f571c70...709b25d8).

### Added

- Packages support one `[lib]` and multiple `[[bin]]` targets. `rock build`
  builds all targets; `rock run --bin NAME` selects a binary. Library-only
  packages build without inventing an executable entry point.
- Declarative macros expand in every loaded module and in expression position.
  Exported macros travel through dependency artifacts and explicit imports;
  macro identity includes its originating crate.
- Macro repetitions close with `)*`; templates support inline repetition,
  repeated lines, and direct captures inside repeated templates.
- The std prelude exports `%format`, `%println`, `Some`, `None`, `Ok`, and `Err`.
  Primitive `Show` implementations cover integer and float widths, `Bool`,
  `Char`, `&Str`, and unit.
- `String::from` accepts `U64`; signed and unsigned decimal formatting preserve
  their full ranges. `ThreadError` implements `Show`; `IoError` display includes
  the error variant and OS error number rather than only `IoError`.
- Enum variants support imports and re-exports, construction and matching by
  imported short names, and dependency-artifact transport.
- `usize` const generic parameters describe array lengths in structs, enums,
  traits, aliases, functions, and implementations. Lengths infer through nested
  arrays and work in repeat literals such as `[0; N]`.
- Nested fixed arrays support chained indexing. Owned array loops move elements
  and clean up unvisited elements on early exits.
- `Vec::from` and `.into!` convert fixed arrays to vectors; byte vectors support
  `extend_from_slice`.
- `std::io::fs::read_to_string` reads through EOF, handles short and interrupted
  reads, preserves embedded NUL bytes, and rejects invalid UTF-8.

### Changed and fixed

- Closure ownership follows capture liveness: last-use captures can move into
  escaping closures; body effects determine `Fn`, `FnMut`, or `FnOnce`.
  Owned captures are destroyed even when the closure is never called, including
  environments stored behind erased function types.
- Detached thread closures require the std-owned capture marker alongside the
  callable and thread-safety contracts; last local use alone does not make a
  borrowed capture safe to detach.
- Improved inference of returned numeric literals from caller context, local
  `_` annotation holes, bound-only generic parameters, by-value generic
  receivers, and recursive trait-bound propagation.
- Imports resolve against declared names, including later declarations and
  signature-only items. Unresolved imports are errors. Explicit imports shadow
  glob bindings; local bindings shadow top-level items instead of reassigning
  them. Modules cannot reuse a dependency's name.
- Duplicate operator declarations are errors, including across dependencies.
  Unknown annotation types, undeclared field types, unresolved struct literals,
  kind mismatches, and invalid instantiated array lengths receive diagnostics
  instead of permissive fallbacks or panics.
- Improved move diagnostics, transactional pattern matching, loop cleanup,
  closure drops, and array-length validation and artifact decoding.

### Compiler maintenance

- Split large compiler modules and integration suites into focused modules;
  moved test-only code out of production files and removed source-text audits.
- Consolidated ownership of collection, lowering, MIR/drop, closure, codegen
  layout, and callable-symbol state; removed obsolete fallback paths.

### Migration

- Application manifests using `[lib]` for an executable must use `[[bin]]`;
  reserve `[lib]` for a library entry point.
- Close macro repetitions with `)*`, not a bare `)`; explicitly import exported
  macros rather than relying on glob imports.
- Fix unresolved imports, duplicate operator declarations, dependency/module
  name collisions, and undeclared types rather than relying on old acceptance.
- Exporting an enum does not import its variants as short names; import the
  variant or use `Enum::Variant`.
- Rebuild dependency artifacts and std with the matching compiler; do not mix
  published v0.7.0 artifacts with this development compiler.

## Published releases

- [v0.7.1](docs/release-notes/v0.7.1.md)
- [v0.7.0](docs/release-notes/v0.7.0.md)
- [v0.6.0](docs/release-notes/v0.6.0.md)
