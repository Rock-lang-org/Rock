# Macro infrastructure acceptance mapping

This maps the requirements in the [specification](2026-10-19-macro-infrastructure.md)
to current tests. The [implementation plan](2026-10-19-macro-infrastructure-plan.md)
contains the per-step receipts and C.12's final verification results. A test prefix
below denotes the named tests in that module, not an unexecuted proposed suite.

## Runners and receipts

All commands are manual and serialized; no GitHub CI job is required or added.
Logs are retained under `/tmp/opencode/` using the `macro-c12-` prefix:

| Receipt suffix | Command |
| --- | --- |
| `focused.log` | `cargo test -p rock-lib --lib macro_ -- --test-threads=1` |
| `debug-build.log` | `cargo build -p rock -p rockc` |
| `release.log` | `cargo build --release` |
| `acceptance.log` | `ROCK_SYSROOT=/home/champii/prog/rust/Rock/target cargo macro-acceptance` |
| `library.log` | `cargo test -p rock-lib -- --test-threads=1` |
| `cli-shared.log` | `ROCK_SYSROOT=/home/champii/prog/rust/Rock/target cargo test -p rock -p rockc -p rock-shared -- --test-threads=1` |
| `clippy.log` | `cargo clippy --workspace --all-targets` |
| `format-demo.log` | From `test_projects/proc_macro/app`, with the same explicit sysroot: `cargo run -p rock --release -- run` |
| `cross-tools.log` | `type -a aarch64-linux-gnu-gcc aarch64-linux-gnu-clang qemu-aarch64 x86_64-w64-mingw32-gcc wine` (expected missing-tool exit status) |
| `identities.log` | SHA-256 identities of the shared std, debug/release compiler binaries and tested code diff from C.11 |

The alias explicitly runs ignored `macro_` tests in `rock-lib` and `rock`, one
test at a time. Default suites intentionally skip expensive native/cold-package
acceptance; their passing result alone does not establish these proofs. CLI
acceptance uses current debug binaries and an explicit shared development sysroot.

## Specification families

Paths below are relative to the repository root. Native test names are in
`lib/tests/macro_sdk.rs` unless a different registration file is stated.

| Spec family | Current named tests and observations | Receipt coverage |
| --- | --- | --- |
| Lossless frontend | `lib/src/lexer/lossless/tests.rs::macro_a0_02_runs_cover_original_utf8_bytes_and_layout`, `macro_a0_02_arbitrary_dsl_and_incomplete_rock_are_preserved`; lossless splice/wire tests, interpreter/layout tests and `lib/src/parser/source_pipeline_tests.rs` cover metadata, bounded scans, ordinary tokens, NUL/errors and EOF. | focused, library |
| Syntax/category | `lib/src/macro_expansion/expression_tests.rs::macro_a_06_expression_category_failures_keep_invocation_diagnostics`, `macro_a_06_nested_opaque_blocks_are_forwarded_without_early_interpretation`; parser macro-invocation and generated-output tests cover commas/grouping/zero calls, postfix, exact consumption and inline DSL rejection. | focused, library, acceptance |
| Block boundaries | Parser source-cursor `macro_a0_09_` and source-pipeline `macro_a0_10_` tests; `macro_a_03_nested_capture_ranges_remain_bound_to_one_source_input`; `macro_acceptance_sdk_block_dsl` checks exact external arbitrary block input, margins, forwarding and diagnostics. | focused, library, acceptance |
| Declarative behavior | `lib/src/macro_expansion/matcher_contract_tests.rs::macro_a_05_each_arm_consumes_all_significant_input`, `macro_a_05_equal_spelled_and_duplicated_capture_occurrences_stay_independent`, `macro_a_05_flat_repetition_keeps_each_iteration_and_stable_key_groups`, `macro_a_05_nested_repetitions_reject_during_definition_validation_even_unused`; expression tests cover precedence. | focused, library |
| Modules/resolution | Registry/module/scheduling `macro_a_07_`–`macro_a_09_` tests and integration `macro_a_09_generated_file_modules_and_transitive_generated_children_run`; `macro_b_16_real_rock_build_run_imports_only_execute_invocations` in `rock/src/tests/macro_packages.rs` covers direct external imports, privacy and no export injection. Explicit facade/glob/external-declarative rejection tests remain. | focused, library, cli-shared, acceptance |
| Hygiene | `lib/src/macro_expansion/binding_environment.rs::macro_a_13_definition_dependency_uses_identity_not_alias_or_caller_shadow`; contextual lowering/integration `macro_a_11_`–`macro_a_15_`; `macro_acceptance_syntax_emission`, `macro_acceptance_derive_struct` and `macro_acceptance_derive_enums_packaged` in `lib/src/lexer/syntax_library_tests.rs` prove real-worker captures, fresh/call-site identifiers and independent same-spelling owners. | focused, library, acceptance |
| SDK/protocol | `macro_acceptance_sdk_data_and_operations`, `macro_acceptance_sdk_request_codec`, `macro_acceptance_sdk_result_codec`, `macro_acceptance_sdk_worker_lifecycle`, `macro_acceptance_sdk_literals_and_fragments`, `macro_acceptance_sdk_authoring`; Rust schema/literal/fragment tests cover handles, bounds, literal subspans and deferred fragments. All input categories share the lossless stream ABI. | focused, library, acceptance |
| Process failure | All fourteen named tests in `lib/tests/macro_process.rs`, including `macro_b_02_io_and_shutdown_stalls_share_the_invocation_deadline`, `macro_b_03_cancellation_reaps_worker_and_descendants_during_io`, and `macro_b_03_repeated_cleanup_does_not_leak_descriptors_or_threads`; artifact preflight tests reject obsolete/corrupt metadata before spawn. | library |
| Build/artifacts | `macro_acceptance_sdk_host_publication`, `macro_acceptance_derive_dispatch`; CLI `macro_b_17_freshness_and_relocated_source_free_proc_bundle` and `macro_b_22_nontrivial_consumers_execute_from_relocated_source_free_bundle`; packaged enum acceptance relocates a two-file artifact/worker package, invalidates original host/syntax locations, and verifies provider bytes/content identity stay unchanged after success and failures. | focused, library, cli-shared, acceptance |
| Host separation | `rock/src/dependency_plan.rs::macro_b_15_host_closure_is_separate_from_target_inputs`, `macro_b_15_cycles_report_host_dependency_chain_without_compiling`; `rock/src/rockc.rs::macro_b_16_proc_host_invocation_supplies_only_artifacts_and_explicit_sdk`. Distinct host/target planning identities are tested; no runnable foreign-target toolchain is claimed. | cli-shared, acceptance |
| Recursion/budgets | `lib/src/macro_expansion/budget_tests.rs::macro_a_10_equal_spans_have_independent_ids_and_reusing_one_id_rejects`, `macro_a_10_mutual_recursion_uses_parent_depth_not_rounds_or_unrelated_siblings`, `macro_a_10_wide_generated_modules_and_source_discovery_are_bounded`; C.09 generated-derive/depth/output tests and packaged native `Loop` negative exercise the same bounded engine. | focused, library, acceptance |
| Formatting proof | `macro_acceptance_sdk_interpolation_ownership`; CLI `macro_b_21_external_interpolation_resolves_caller_holes_and_runtime_authority` and `macro_b_22_nontrivial_consumers_execute_from_relocated_source_free_bundle`; provider/runtime fixtures in `lib/tests/fixtures/macros/interpolation/` cover caller holes, exact escape/Unicode errors, normal bounds, once/left-to-right/borrowing/drop and consuming-call negatives. | library, acceptance |
| DSL proof | `macro_acceptance_sdk_block_dsl`; CLI `macro_b_20_external_block_dsl_runs_and_diagnoses_through_rock`; `lib/tests/fixtures/macros/block_dsl/` supplies library-defined expression/item output, exact arbitrary input, nested forwarding, subspan diagnostics and invalid direct DSL-as-Rock output. | library, acceptance |
| Syntax library | All six `macro_acceptance_syntax_` tests in `lib/src/lexer/syntax_library_tests.rs`, plus the two native derives there. Shared Rust/Rock fixtures cover standalone cursors/groups, type/binder/constraint/struct/enum grammar, unsupported inputs, limits, byte/context fidelity and edited/constructed emission. The supported subset is documented in `rock_syntax/README.md`. | focused, library, acceptance |
| Derive proof | Parser `macro_c_02_`, original-declaration `macro_c_01_`, all ten dispatch `macro_c_09_` tests; native `macro_acceptance_derive_dispatch`, `macro_acceptance_derive_struct`, `macro_acceptance_derive_enums_packaged`. These cover attachment grammar, exact original input, node-ID consumption, retained declaration, generated ancestry/order, explicit caller trait authority and ordinary impl/privacy failures. | focused, library, acceptance |
| Semantic integration | `lib/tests/macro_diagnostics.rs::macro_a_15_borrow_failures_map_primary_and_ownership_labels`, `macro_a_15_unsafe_rules_are_not_bypassed_by_generated_calls`; contextual inference/control-flow/drop integrations, interpolation ownership and derive coherence negatives. Generated code passes ordinary lowering, inference, specialization and MIR checks. | library, acceptance |
| Trust/inspection | `lib/tests/macro_process.rs::macro_b_04_metadata_and_declarative_expansion_do_not_execute_registered_workers`; `macro_c_09_no_execution_flag_rejects_selected_derives_without_starting_a_worker`; CLI `macro_b_04_expand_forwards_execution_policy_to_rockc`. SDK trust documentation distinguishes arbitrary native worker execution from metadata inspection; no expansion-result cache or sandbox guarantee is claimed. | focused, library, cli-shared, acceptance |

## Residue audit and scope

- Production source recognition is `LosslessStream` → Rock interpretation/layout
  → source cursor/parser. `lib/src/lexer/test_support.rs` is only a test adapter
  over these views, not the retired independent destructive scanner.
- `lib/src/macro_expansion/token_tree.rs` aliases the shared lossless stream;
  `ProcMacroInputShape` has only `TokenStream`. Block margins/category are metadata,
  not a second raw-string input ABI. Captures are streamed/sliced, not printed and
  re-lexed; generated source requests are interpreted only at the explicit output
  boundary.
- The Rust wire reader accepts only `RKLOSS04`; both SDK codecs implement protocol
  7/current stream schema, and products require format 55. Rejection controls use
  fabricated invalid magic/version values. `protocol-7/*.hex` are current interoperability
  fixtures, not compatibility readers or preserved historical artifact snapshots.
- Registries are built for each expansion forest/context. Derive consumption is a
  bounded context-local node-ID set; ancestry and aggregate budgets are shared with
  ordinary macros. There is no global derive registry or span/name execution cache.
- Current hosts check identity plus exact function/derive kind through
  `entry_kind_matches`. The old function-only bootstrap, unimplemented-derive
  rejection path and old stream encodings were already removed by earlier slices;
  closure removes stale documentation, not valid semantic negative fixtures.
- The final packaged derive check compares artifact bytes and worker SHA-256/size
  before and after relocation/consumption. Shared development std remains explicit;
  compiler-owned std discovery and SDK/syntax package special cases are not added.

Infrastructure completion does not ship production formatting/derive/HTML
libraries, arbitrary attribute transforms, a full Syn grammar, persistent workers,
or a self-hosted compiler. The independent C.05/C.08/C.10/C.11 compiler failures
recorded in the original receipts are addressed by the follow-up repairs below.

## Compiler repair follow-up

- C.05 retains ambiguous constructor-application equations until caller context
  or declaration-default fallback determines the constructor. The native generic
  fixture again uses the partial `Result T, _` default; macro-free integrations
  also cover inference and explicit constructor overrides.
- C.08 cleans RHS temporaries inside the evaluated short-circuit branch. The
  native emission fixture again uses its combined owned-temporary assertion;
  macro-free tests cover skipped/taken `&&` and `||`, nesting and exact drops.
- C.10 lowers field initializers into declaration-scoped functions and publishes
  resolved initializer identities in product format 55. Explicit fields evaluate
  in source order, then omitted defaults in declaration order. Generic owner
  witnesses preserve body-only type parameters. Tests cover declaration scope,
  source-free artifacts at O0/O3, invalid fields and malformed initializer metadata.
  Rebuild dependencies; there is no compatibility reader. Protocol 7 and RKLOSS04
  are unchanged.
- C.11 uses sparse initialization states, shared local-root maps, sparse
  loan-location entries and early release of unused initialization exit states.
  The original unfactored enum emitter is restored. The measured packaged run
  fell from 10,457,392 kB to 3,347,252 kB peak RSS (about 68% lower); this is a
  workload-specific improvement, not a general memory bound. Profiling hooks
  were removed after measurement.

The original C.12 receipts above describe the earlier snapshot. Follow-up command
receipts are recorded separately in the implementation plan.
