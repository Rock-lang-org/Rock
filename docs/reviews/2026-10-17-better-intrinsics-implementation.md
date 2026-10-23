# Implementation of the `better_intrinsics` review findings

Date: 2026-10-17

This records the fixes for R1–R7 in the [branch review](2026-10-17-better-intrinsics-branch-review.md), based on commit `e4af46b31cc738f2149977fda3021f18b942ffab`.

## Changes

### R1–R3: raw-pointer loans and reborrow authority

- Resolve the pointer-bearing prefix of a projected place using the MIR backend type contract; recover origins through fields and tuple projections.
- Collect return loans from every argument selected by the return-origin summary, combining reference arguments with raw-pointer arguments.
- Retain the exact reborrow access path on each recovered loan. Conflict checking and loan activation use that source rather than the first pointer argument.
- Transfer statement loan ownership through the same helper in the dataflow analysis and conflict checker.
- Raw-pointer arguments no longer receive the ordinary first-reference-argument forwarding rule. Pointer intrinsics retain their explicit forwarding behavior.

Native artifact calls without a precise body summary continue to conservatively retain all eligible argument origins. The artifact regressions verify alias protection and valid reborrow formation; they do not assert precise exclusion of unused native arguments.

### R4–R6: generic defaults

- Publish completed struct/enum parameter descriptors to every predeclared name with the same identity before lowering recursive fields.
- Apply the existing dependency-ordered header traversal to file-backed qualified declarations.
- Share owner-default registration between selected constructor members and ordinary static methods, preserving deferred fallback after ordinary inference.

### R7: source-free constant arithmetic

- Build bounded summaries through selected scalar method calls, including calls to imported methods whose source is absent.
- Substitute arguments into identity-free summaries, preserving evaluation of unused arguments and discarded computations.
- Use the same summaries for local method evaluation and artifact publication; carry them from inference to product construction and remap method keys through product identities.
- Retain the existing 512-node/32-depth validation bounds. Recursive, unsupported, and excessive expressions do not receive summaries.
- Reuse the existing serialized summary representation; no artifact schema change is required. Dependencies must be rebuilt to acquire newly supported summaries.

## Regression coverage

The following tests were added to `lib/tests/integration.rs`:

| Finding | Regression |
| --- | --- |
| R1 | `test_raw_pointer_projected_borrows_retain_storage_loans` |
| R2 | `test_raw_pointer_mixed_return_arguments_retain_all_loans` |
| R3 | `test_raw_pointer_return_reborrow_uses_selected_argument` |
| R1–R3, artifacts | `test_raw_pointer_return_loans_survive_deleted_source_artifacts` |
| R4 | `test_default_type_arguments_recursive_nominal_fields` |
| R5 | `test_default_type_arguments_forward_declarations_in_file_modules` |
| R6 | `test_default_type_arguments_static_factory_fallback_and_inference` |
| R7 | `test_constant_arithmetic_composes_selected_methods_across_artifacts` |

Controls cover accepted projected borrows, rejected mutations of retained origins, both raw argument positions, permitted mutation of an unrelated local origin, source-deleted dependencies, recursive structs and enums, explicit/inferred overrides of defaults, discarded arithmetic, ignored arguments, a second source-free wrapper dependency, and std `wrapping_div` by zero.

## Verification

The initial focused runs reproduced failures for all seven findings before their fixes:

- `cargo test -p rock-lib --release --test integration test_raw_pointer_` — three added tests failed (`borrow-red.log`).
- `cargo test -p rock-lib --release --test integration test_default_type_arguments_` — three added tests failed (`defaults-red.log`).
- `cargo test -p rock-lib --release --test integration test_constant_arithmetic_composes_selected_methods_across_artifacts` — failed on the imported wrapper (`constants-red.log`).

Final checks:

| Command | Result | Log |
| --- | --- | --- |
| `cargo test -p rock-lib --release` | 2,373 unit tests, 776 main integrations, and one parser integration passed; one doctest ignored | `implementation-library-pass.log` |
| `cargo test -p rock -p rockc --release` | 72 Rock CLI and 48 compiler CLI tests passed | `implementation-cli-tests.log` |
| `cargo build --release` | Passed; existing `rockup` dead-code warning | `implementation-build.log` |
| `cargo clippy --workspace --all-targets --release` | Passed with warnings, including argument-count warnings on the expanded inference constructor and reference-borrow helper | `implementation-clippy.log` |

Logs are retained under `.agent-work/branch-review-20261017/`. Earlier intermediate failures are preserved there as well; the final checks above supersede those runs.

`bd` was unavailable (`command not found`), so issue status could not be recorded. No associated issue or PR URL was identified. Changes remain uncommitted.
