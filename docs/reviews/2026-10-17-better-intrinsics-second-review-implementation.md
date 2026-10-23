# Implementation of second-review findings R8–R10

Date: 2026-10-17

Review baseline: `c3eaddb137d84c4bd09c9cd4b7c9e4c68450382a`.

Related record: [second branch review](2026-10-17-better-intrinsics-second-review.md).

## Repairs

### R8 — Aggregate arguments retain returned-reference loans

`lib/src/mir/borrowck/borrows.rs` now recovers storage origins from selected pointer-containing aggregate arguments as well as direct pointers. It reuses the existing conservative aggregate origin envelope; argument selection still follows the callee's return summary, with conservative eligible-input handling for native artifact calls without a body summary.

`LoanAnalysis` separates explicit recovered-storage reborrow checks from ordinary place access. A selected aggregate input can supply its carried loan authority without inventing a `Deref` projection on the aggregate. Ordinary accesses retain the dereference requirement, and unrelated live sibling loans still conflict. Recovered parameter roots are dereferenced only when the parameter itself is reference- or pointer-typed.

### R9 — Reference-bearing results retain loans

Terminator loan recovery now recognizes reference-bearing aggregate result types. The existing recursive type/layout traversal distinguishes pointer contents, references, and mutable references; results containing mutable references retain an exclusive loan envelope. Return-source activation continues to inherit authority from the selected input rather than the first pointer argument.

The implementation remains conservative within an aggregate: it does not introduce field-specific lifetime summaries or independent per-result-field loan authority. Reference-only argument handling is kept separate from raw-pointer-origin recovery.

### R10 — Defaults precede recursive field lowering

`lib/src/collect/collector.rs` now separates nominal descriptors from fields. Header ordering follows descriptor/default dependencies rather than recursive field edges. Struct and enum descriptors publish completed generic defaults first; field lowering is deferred until all source-module headers have been collected.

Deferred declarations retain their source-module path, are completed in deterministic definition-ID order, and update every registered alias of the nominal identity. This handles cycles spanning different files as well as cycles within one module, while preserving declaration-scope aliases and explicit higher-kinded constructor behavior. Actual default-expansion cycles remain diagnostic errors.

`lib/src/collect/headers.rs` shares nominal-default construction and accepts completed descriptors when lowering struct/enum bodies. No artifact schema change is needed.

## Regression coverage

Eight integration tests were added in `lib/tests/integration.rs`:

| Test | Coverage |
| --- | --- |
| `test_raw_pointer_aggregate_arguments_retain_return_loans` | Struct, tuple, nested tuple inputs; shared and mutable references; shared-array inputs; mutation rejection and valid-use/runtime controls |
| `test_raw_pointer_aggregate_results_retain_return_loans` | Tuple, nested tuple, nominal results; shared and mutable references; shared-array results; exclusive sibling-loan rejection and valid writes through mutable views |
| `test_raw_pointer_enum_arguments_and_results_retain_return_loans` | Enum payload origins and reference extraction from returned enums |
| `test_raw_pointer_aggregate_reborrow_uses_selected_input_authority` | Selected aggregate input, unused first pointer, unrelated storage mutation, and live sibling conflicts |
| `test_raw_pointer_aggregate_return_loans_survive_deleted_source_artifacts` | Native artifact calls after deleting producer source; shared/mutable tuple results and aggregate inputs, with positive and negative controls |
| `test_default_type_arguments_mutually_recursive_nominal_fields` | Struct/struct and struct/enum field cycles in both declaration orders |
| `test_default_type_arguments_mutual_fields_in_deleted_source_artifacts` | File-module recursive defaults, artifact publication/remapping, and deleted producer sources |
| `test_default_type_arguments_mutual_fields_across_file_modules` | Cross-file cycles in both module orders, with different module-local aliases used in defaults and fields |

The existing invalid-default regression also covers a nominal default-expansion cycle. Original R1–R7 regressions remain in the suite.

## Evidence

All logs below are under `.agent-work/branch-rereview-20261017/`.

- `aggregate-red.log`: the initial R8 and R9 regressions both failed because the compiler incorrectly accepted the mutation.
- `defaults-red.log`: the initial R10 regression failed with `MIR/codegen agreement failed`, `invalid_type_ids: 1`.
- `aggregate-green-attempt.log`: both initial borrowing regressions passed after recovery was extended.
- `defaults-green-attempt.log`: the default-argument family passed after the initial within-module phase separation.
- `library-first-pass.log`: intermediate full library suite passed 2,373 unit tests, 779 main integrations, and one parser integration; one doctest ignored.
- `aggregate-expanded.log`: two expanded fixtures had invalid nested tuple access syntax (`.0.0` lexed as a float); corrected to `(value.0).0`.
- `aggregate-expanded-2.log`: expanded tuple/nominal/deleted-source borrowing tests passed.
- `defaults-cross-module-2.log`: a valid cross-file cycle exposed incomplete defaults with the initial module-local phase split.
- `defaults-global-fields.log`: the complete default-argument family passed after deferring fields across all source modules.

Exploratory mutable-array indexing controls rejected on both the implementation and the unchanged baseline compiler (`mutable-array-argument-baseline.log`, `mutable-array-result-baseline.log`). Permanent array positives therefore cover shared references; mutable-loan controls use tuples, structs, and enums. This existing indexing limitation is distinct from the R8–R10 reproducers and remains outside these repairs.

## Final verification

All gates passed on the final compiler/test changes:

| Command | Result | Log |
| --- | --- | --- |
| `cargo test -p rock-lib --release` | 2,373 unit tests, 784 main integrations, one parser integration passed; one doctest ignored | `library-final.log` |
| `cargo test -p rock -p rockc --release` | 72 Rock CLI and 48 compiler CLI tests passed | `cli-final.log` |
| `cargo build --release` | Passed; existing `rockup` dead-code warning remains | `build-final.log` |
| `cargo clippy --workspace --all-targets --release` | Passed with warnings | `clippy-final.log` |

Total: **3,278 tests passed**, no failures. Test suites were serialized. Formatting remains deferred. No compiler/test changes followed these final gates.

## Repository state

R8–R10 are implemented and regression-verified. Six tracked compiler/test files are modified; review documents and scratch evidence remain untracked. No commit, merge, or push was performed.

No associated issue/PR URL was found by `gh pr view`; there was nothing to link to the session. `bd` is unavailable, so issue tracking could not be updated.
