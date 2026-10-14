# Early-return storage and callable/loan composition implementation evidence

Date: 2026-10-09

This record accompanies [the completion contract](early-return-storage-and-callable-loan-composition.md). It is separate from the unchanged initial consolidation review. Implementation evidence and acceptance evidence are deliberately distinguished.

## Implementation

- `lib/src/mono/return_storage.rs` owns reachable callable returns and normal continuations. Nested lambdas have independent return targets; loops consume their own break/continue exits. Evaluated children stop contributing after an unconditional terminator, and a `Never` expression does not supply a normal result.
- Ordinary functions, generic specializations, method specializations and lambdas share atomic return agreement. All-explicit-return bodies obtain storage from their actual returns. A plain candidate cannot hide an earlier owning environment, and failed representation transport cannot suppress a measured-storage diagnostic.
- Normal block/conditional/match values are separate from the enclosing callable's returns. Noncontinuing arms do not compete with continuing storage, and a failed measured normal-result transport reports Mono instead of losing the environment.
- Generic specializations retain declaration-owned refinement restrictions. Declared nominal results preserve producer-owned fields while semantic arguments remain constrained by selected binders. This does not authorize a generic result binder to select new environment storage.
- MIR closure-body construction uses the callable's checked result type, not the normal tail type: a body marked `Never` after all explicit returns still emits the real return ABI and cleanup storage.
- Concrete instance-method calls publish processed producer storage before callers bind or project the result, reusing the existing static-method recursion guard. This repairs a separately observed nominal-result method leak.
- Producer overlays and joins sort complete typed projection paths, not only field indices. Downcasts, dereferences, dynamic indices and field identities remain distinct; shared conflict handling remains conservative.
- Array-repeat Copy checks remain immediate for concrete inference results; generalized results are checked against their actual specialized storage in Mono before emission. A signatureless capturing repeat executes with independent environments, while a real non-Copy array capture rejects with a source-backed Type diagnostic.
- Loops without any break owned by that loop lower as `Never`; nested loop/lambda bodies cannot supply the outer break. MIR emits their unconditional back-edge without a fictitious condition-false unit return.
- Finite argument/projection/native-code return recipes join canonically through branch and loop replacements. Each applicable alternative is evaluated before role-separated must-evidence is retained; missing/opaque/cyclic alternatives still fail closed. Captured environments retain their identity restriction, while capture-free code has no environment identity to confuse with its old binding slot.
- Safe-reference copying of a concrete capturing callable resolves a known containing storage owner to its measured contents without erasing other carried payload loans. Producer dependency measurement and scalar/opaque loads retain their prior rules; address-taking keeps the storage dependency, and Unknown remains Unknown. This repairs an owning callable-array projection that incorrectly borrowed its local array after copying.
- No inference demand, invocation restriction, borrowing-role measurement, native-record admission, runtime prefix, descriptor ABI or artifact format was broadened.

## Observed defects and controls

| Observation | Permanent control / repair |
| --- | --- |
| Captured early return/plain-arrow tail was accepted | `early_return_callable_storage_is_checked_in_both_orders`; reverse order and incompatible scalar/tuple capture environments also reject |
| Overlay order depended on insertion order for different enum downcasts with the same field index | `overlay_order_preserves_downcast_dereference_and_index_identity`; full projection ordering also preserves the existing overlapping-write lattice laws |
| Making all-explicit-return blocks `Never` initially broke the unchanged owned-fold callback | Exact `callable_receiver_transport_preserves_owned_fold_early_returns` passes after the MIR callable-result fix; its source and observer are unchanged |
| A concrete nominal-result instance method produced only two allocations/no releases and observer exit 254 | The nominal-method case in `nested_callable_returns_and_noncontinuing_branches_preserve_storage` passes after publishing producer storage before caller binding |
| Generic nominal-result method refinement rejected compatible field storage | The generic-nominal-method case passes after declared nominal representation transport; generic binder leaves remain restricted |
| Inference rejected a generalized callable-repeat initializer before actual capture storage was known | `early_return_callable_array_repeats_preserve_each_owned_environment` and `generalized_array_repeat_still_rejects_noncopy_capture_storage`; specialization preserves the actual Copy restriction |
| Checked native callable replacements on both/one branches and loop back-edges were discarded | `branch_and_loop_callable_replacements_validate_every_carried_producer`; both flags execute at O0/O3, and opaque replacement/payload mutation reject |
| A non-breaking loop supplied a false unit candidate, then a false MIR return edge | `nonreturning_exits_do_not_compete_with_actual_return_storage` and `loop_without_an_owned_break_has_no_unit_fallthrough`; source and MIR repairs preserve the existing unit-break behavior |
| Returning an owning copy from a callable array retained a local container-storage loan | `callable_array_projection_preserves_owned_environments_and_borrowed_access`, `copied_array_callable_does_not_borrow_the_array_storage`, and `safe_callable_copy_preserves_payload_loans_but_not_containing_array_storage`; payload/address/Unknown roles stay distinct |

New O0/O3 positives cover distinct compatible early/tail producers, all-explicit-return bodies, lambda-local returns (including all-explicit returns), continuing conditional/match branches, returns inside a loop, ordinary/generic nominal-result methods, canonical nominal/enum and nested tuple results, array-repeat results, bodyless native early-return producers, and retained source-free generic value adapters. The unchanged C observer verifies independent live environments and balanced reclamation.

New callable/loan controls cover a stored consuming callable with a real non-Copy nominal capture, rejection of its second use, mutation rejection through its returned payload loan, and signatureless early-return reference alternatives retaining both input loans despite a literal caller flag.

New agreement units distinguish reachable explicit returns from dead tails, lambda return scopes, evaluated-argument termination, unsafe blocks, nonreturning paths, loop targets, failed representation transport, same-width incompatible environments, both return orders, and declaration-owned versus generic-binder storage.

`initializer_guard_and_call_argument_returns_keep_their_own_spans` checks each evaluation context and its original return-value span. `early_return_nominal_storage_checks_each_producer_assignment` independently corrupts each of two emitted producer assignments; neither can inherit validity from the other.

## Intentional native authority boundary

The new nongeneric native producer returns a represented nominal containing an owning callable. It has an exact linked/bodyless storage signature, but no native return-origin record for its untracked non-scalar result. Direct calls and transport of the returned value through a retained generic identity body execute after deleting provider sources.

Passing this producer as a universally certified callback instead rejects with source-backed Borrow diagnostics: physical nominal storage does not supply the missing native producer loan record. The rejected invocation remains a permanent negative, not an executed or weakened positive. Non-scalar native-record expansion remains outside scope.

The first provider fixture borrowed a local scalar implicitly; it correctly rejected and was changed to use an explicit consuming operation, matching existing native ownership controls. An initial loop fixture requested a value-bearing loop, but current loop lowering has a unit result; the supported unit-break/early-return control is separate. A receiver-method fixture initially omitted its `@` receiver marker. These fixture repairs are not recorded as compiler defects.

The nested-tuple fixture required parentheses around its comma-bearing function call. An array consumer initially used indexing without an Index protocol; the array-return control instead observes the complete returned repeated array through C and claims no indexing behavior. A negative array argument likewise required parentheses to distinguish a call argument from indexing syntax. These are fixture boundaries, separate from the generalized Copy-check defect.

The dedicated array-projection fixture now declares its own Index protocol: ArrPtr consumes a borrowed slice, and indexing already dereferences the selected element. Its original observed-array/returned-copy positive is retained after the copy-origin repair; temporary tuple-return experiments were not substituted for it. A returned lambda must actually own its environment: the final independent-return-target control consumes a concrete `Captured` value instead of returning implicit loans to local scalar parameters. A standalone exported adapter cannot infer an unknown nominal receiver from an unconstrained field call; the import-ID control uses a generic callable input and projects the actual nominal field in the consumer. These declaration/syntax repairs are distinct from the three new confirmed defects above. The experimental late capture-refresh change was removed rather than changing accepted capture/signature authority after inference.

## Acceptance evidence

### Family-to-control evidence

This is an evidence index for the contract's case IDs, not an issue tracker. **Runtime** means the registered test executes Rock at O0/O3; **unit** means it checks compiler state or diagnostics and does not establish a runtime observation. The terminal receipts below distinguish the earlier snapshot from the completion follow-up; no passing subset substitutes for the final gates.

File abbreviations:

- **Reclamation**: `lib/tests/copy_environment_reclamation.rs`.
- **Transport**: `lib/tests/callable_result_transport.rs`.
- **Returns**: `lib/src/mono/return_storage.rs`, `tests` module.
- **Bindings**: `lib/src/mir/borrowck/callable_results/bindings/tests.rs`.
- **Forwarding**: `lib/src/mir/borrowck/callable_results/bindings/forwarding.rs`, `tests` module.
- **Callable sources**: `lib/src/mir/borrowck/callable_results/tests.rs`.
- **Representations**: `lib/src/type_services/callable.rs`, `tests` module.

| Case | Executed evidence and its limit |
| --- | --- |
| R01 | Reclamation `compatible_early_return_callable_storage_is_reclaimed`: runtime, two distinct producers, both flags, simultaneous owning copies and balanced cleanup. |
| R02 | Reclamation `early_return_callable_storage_is_checked_in_both_orders`: source-backed Mono rejection at O0/O3 for captured/plain and reverse; Returns `all_reachable_returns_are_checked_atomically_in_both_orders` checks unchanged input after failure. |
| R03 | The same Reclamation control rejects scalar/tuple captures; Returns `failed_transport_and_same_width_environments_reject` also checks outright transport failure. |
| R04 | Reclamation `all_explicit_returns_preserve_callable_storage`: runtime without a lexical tail; nested-return control also covers an all-return lambda. |
| R05 | Reclamation `nested_callable_returns_and_noncontinuing_branches_preserve_storage`, `continuing_conditional` and `continuing_match` fixtures: runtime of returning/continuing alternatives. |
| R06 | Reclamation `nonreturning_exits_do_not_compete_with_actual_return_storage`: runtime explicit return/dead plain tail and noncontinuing loop tails/branches. Returns `explicit_returns_ignore_dead_tails_and_lambda_return_targets` and `evaluated_arguments_stop_after_return_and_never_paths_supply_no_value` additionally check typed never-calls and evaluated-argument termination without inventing producer storage. |
| R07 | Reclamation `nested_source_returns_preserve_storage_and_real_diagnostic_provenance`: O0/O3 runtime and parser-to-source-backed-Mono negatives for nested ordinary/unsafe branches, initializer, guard and evaluated call argument. Returned-value spans identify the original expression or lambda arrow, not the caller. Returns units independently check traversal termination and span preservation. |
| R08 | The same Reclamation control's returned-lambda fixture owns a concrete captured value, independently checks its own early/tail agreement and rejects its incompatible inner return at that return's actual span. Existing nested/all-return lambda fixtures and Transport `concrete_returned_and_nested_callables_compose_at_o0_and_o3` remain unchanged positives. |
| R09 | Reclamation nested-return `loop` fixture: runtime early callable return versus supported unit break; Returns `nested_loops_own_breaks_but_not_callable_returns`: unit nested-loop/continue ownership. No value-bearing loop support is claimed. |
| R10 | Reclamation nested-return `nominal`, `enum`, `nested_tuple` fixtures and `early_return_callable_array_repeats_preserve_each_owned_environment`: runtime recursive storage. `unmatched_callable_branch_storage_cannot_erase_environment_ownership` rejects incompatible conditional/match layouts. Representations `nominal_representations_keep_distinct_fields_without_changing_semantic_arguments` rejects field/storage identity disagreement. |
| R11 | Reclamation nested-return ordinary/generic-method fixtures and source-free early-return producer: runtime. Returns `selected_generic_result_cannot_be_refined_from_an_actual_environment` and Representations `declared_slots_cannot_refine_callable_layouts_introduced_by_generic_binders`: unit selected-binder restriction. |
| R12 | Representations `nominal_representations_keep_distinct_fields_without_changing_semantic_arguments` checks safe-reference mutability; `value_representation_does_not_change_declared_protocols_or_certify_producers` checks raw-pointer invariance. Main integrations `test_array_literal_cast_seed_infers_remaining_integer_elements`, `test_array_literal_mut_slice_use_infers_integer_elements`, and `test_array_integer_literal_does_not_infer_as_bool` preserve existing numeric behavior; these existing helpers are not claimed as new O0/O3 controls. |
| R13 | Transport `early_return_reference_alternatives_keep_both_input_loans`: runtime signatureless compatible references plus mutation negatives for both origins despite a literal flag; distinct loan-role units remain separate. |
| R14 | Forwarding `recursive_and_missing_producer_recipes_cannot_certify_return_alternatives`: direct/indirect recursive producer recipes reject in both alternative orders, clear active recursion state, retain unrelated completed evidence and reject a real body/header whose producer recipe is missing; no budget change or negative execution. |
| R15 | Reclamation `callable_registration_and_branch_order_do_not_change_storage_agreement`: O0/O3 compatible execution and incompatible capture-storage Mono negatives under both branch and registration orders, with an assertion that producer DefIds really changed. Existing R02/Returns and Forwarding full-path/lattice reversals remain positive controls. |
| C01 | Transport `definition_owned_demands_follow_immutable_input_and_result_aliases_at_o0_and_o3` and `composed_definition_inputs_discharge_each_producer_and_keep_payload_loans_at_o0_and_o3`: runtime plus mutation negatives; Bindings `adapter_composes_repeated_definition_owned_requirements` checks repeated obligations. |
| C02 | Bindings `nominal_field_envelope_and_canonical_demand_check_actual_roles_jointly`: full canonical demand versus narrowed/Unknown field matrix, accepted subsets/empty measured dependencies, out-of-narrow-envelope/both-input/Unknown/missing/mismatched-role negatives and absent-enclosing-demand check. Existing retained-binding, malformed-envelope and explicit protocol controls remain intact. |
| C03 | Transport `callable_array_access_retains_the_selected_result_loan`: O0/O3 selected callable execution with payload-mutation rejection. Reclamation `callable_array_projection_preserves_owned_environments_and_borrowed_access`: O0/O3 returned owning copies, both indices, C-observed array/returned environments and balanced cleanup. Existing tuple/record/enum/captured-relay controls remain; no universal array demand was added. |
| C04 | Callable sources `checked_callable_result_includes_every_reachable_return_alternative` checks all possible dependencies and rejects the unsafe alternative; `lib/src/mir/borrowck/mod.rs::tests::early_return_nominal_storage_checks_each_producer_assignment` independently corrupts early and tail MIR assignments. Transport source-free native-alternatives runtime additionally validates actual checked native producers. |
| C05 | Bindings `copy_binding_joins_every_incoming_producer_and_missing_alternatives`: unit good/bad/missing predecessor producers; per-return MIR corruption above rejects either malformed producer assignment. |
| C06 | Transport `branch_and_loop_callable_replacements_validate_every_carried_producer`: source-level both-branch and one-branch replacement positives at O0/O3, repeated returned-carrier transport, plus opaque-replacement and result-loan mutation negatives. Forwarding `one_predecessor_replacement_retains_the_checked_initial_field` checks production borrowing independently; existing missing-predecessor and overlapping-write negatives remain. |
| C07 | Bindings `nested_record_overwrite_transports_only_the_replacement_producer` and `owned_root_loans_preserve_only_disjoint_callable_binding_evidence`: unit parent/child, actual replacement, alias-write/exposure vetoes. Forwarding full-path order controls distinguish downcasts, dereferences and indices; these are lattice/identity evidence, not execution of every write permutation. |
| C08 | Transport `definition_use_demands_transport_aggregate_results_after_incidental_calls_at_o0_and_o3`: runtime supported incidental call. Bindings `exposing_callable_storage_or_crossing_a_call_does_not_supply_empty_effects` and transport units `measured_isolated_callable_survives_an_unrelated_call_without_empty_effects`/`cast_exposure_vetoes_isolation_and_never_supplies_callable_authority` distinguish exposure from proven isolation. |
| C09 | Transport's branch/loop replacement control executes both loop-entry and zero-iteration paths at O0/O3 and rejects an opaque carried producer despite the initial valid producer. Forwarding's direct/indirect recursive-recipe control rejects cyclic proof; existing disjoint-loop/fixpoint caps remain unchanged. |
| C10 | Bindings `callable_input_obligations_do_not_flatten_distinct_measured_loan_roles`; Callable sources `checked_callable_result_sources_distinguish_carried_values_and_environment_storage` and `checked_callable_result_missing_body_summary_or_capture_binding_stays_unknown`: unit separate roles and missing-role/capture evidence. Transport `capturing_factory_does_not_flatten_distinct_raw_lifetime_and_value_roles` checks published raw-result uncertainty. |
| C11 | Transport `source_free_scalar_opaque_dependencies_require_measured_native_evidence_at_o0_and_o3` and `source_free_scalar_identity_requires_real_native_producer_evidence_at_o0_and_o3`: runtime plus missing-record/corruption rejection. Raw factory role check above does not promote scalar evidence to non-scalar native admission. |
| C12 | Bindings `generic_fn_mut_repeated_calls_keep_the_code_obligation_not_empty_effects`, unchanged HKT integrations and Representations protocol/safety negatives pass. Physical representation transport still cannot upgrade FnOnce to explicit Fn/FnMut. |
| C13 | Transport `stored_consuming_callable_keeps_payload_loans_and_rejects_second_use`: runtime real non-Copy nominal capture; source-backed second-use and payload-mutation Borrow negatives are not linked/executed. |
| C14 | Reclamation `nominal_borrowed_parameter_preserves_owning_field_copy`, `nominal_struct_pattern_preserves_measured_callable_environment`, `nominal_enum_payload_preserves_measured_callable_environment`, and `nominal_enum_owned_and_borrowed_parameters_preserve_payload_storage`: runtime measured field/payload transport; bodyless nominal input controls reject incompatible owned/borrowed storage. |
| C15 | Transport `native_capturing_result_keeps_payload_live_after_consuming_the_callable_at_o0_and_o3`, `shared_capturing_result_survives_consumption_of_the_holder_at_o0_and_o3`, and returned/nested callable runtime; Reclamation `consuming_enum_capture_transports_result_before_payload_cleanup`: runtime consuming enum payload transfer. |
| C16 | Transport `capturing_payload_mutation_and_environment_storage_results_still_reject`: pre-link Borrow rejection for environment storage/dead returned reference, including the explicit local-lifetime diagnostic; certified-result transport units retain alias/escape negatives. |
| C17 | Reclamation `early_exit_user_destructors_keep_loan_reads_order_and_field_responsibility`: O0/O3 early/tail exits combine C-observed callables with live destructor loan reads, owner-before-field log order, partial field transfer and immediate unused-result cleanup; paired live-loan mutation and borrowed-move negatives reject. Existing destructor/environment/active-enum and main return-temporary controls remain. |
| C18 | Transport `source_free_callable_joins_survive_actual_import_id_renumbering`: O0/O3 execution and mutation/missing-individual-native-record negatives with both dependency orders, an asserted real crate-ID change, deleted provider sources and frozen product/object bytes. Existing full-path insertion-order and active-variant controls remain. |

Source-free publication evidence is in Reclamation `bodyless_early_return_nominal_producer_preserves_storage_without_sources` and Transport `source_free_generic_value_transport_keeps_an_immediate_lambda_environment_at_o0_and_o3`, in addition to existing factory/record/relay/scalar native fixtures. These execute after provider-source deletion and check retained versus bodyless authority rather than treating local specialization as a bodyless proof. MIR nominal/enum storage corruption controls in `lib/src/mir/borrowck/mod.rs` reject missing rows, forged captures/variants/fields and invalid TypeIds. Exact binding-span units and retained-source fixtures remain evidence for their own actual locations; synthetic-HIR span tests do not replace a missing native producer location. No serialization or ABI encoding change is part of this implementation.

### Source-free/corruption acceptance index

| Boundary | Permanent control |
| --- | --- |
| Checked early-return native layout, bodyless link and retained generic transport | Reclamation `bodyless_early_return_nominal_producer_preserves_storage_without_sources`; Transport `source_free_generic_value_transport_keeps_an_immediate_lambda_environment_at_o0_and_o3` and `source_free_callable_joins_survive_actual_import_id_renumbering` execute after deletion at O0/O3, assert bodyless records/actual remapped IDs and freeze emitted object/product bytes |
| Missing native authority cannot be supplied by another alternative/header | Transport import-ID control removes each real individual native origin record in turn; `source_free_scalar_identity_requires_real_native_producer_evidence_at_o0_and_o3` and Bindings `source_free_native_binding_requires_the_checked_producer_record` retain original negatives |
| Missing/forged/stale/invalid result storage and captures | MIR `nominal_result_storage_is_separate_from_identity_and_checked_against_its_producer`, `enum_result_storage_requires_exact_producer_and_variant_schema`, `early_return_nominal_storage_checks_each_producer_assignment`, and `static_nominal_factory_requires_exact_measured_result_storage_with_canonical_dictionary` reject bad rows/TypeIds/captures/active paths before execution |
| Artifact declaration/schema corruption | Products `early_return_native_storage_schema_corruption_rejects_at_emission_and_decode` publishes virtual-source early-return struct/enum native producers; seven independent missing/extra field, changed callable result, nominal-ID/argument and missing/extra enum variant mutations reject at both emission and untrusted decode, preserving original bytes |
| Closed signature, object path and origin roles | Products `native_return_contract_artifact_rejects_malformed_authority_at_decode` rejects nine independent record/source/arity/duplicate/signature/object/symbol/UnknownExternal mutations at both boundaries; existing native/erased/dictionary agreement units check selected identities |
| Actual owned/borrowed nominal binding | Reclamation bodyless input controls reject incompatible environment storage at their actual source binding; valid bodyless factory and borrowed-field-copy positives remain |
| Definition-owned input obligations | Bindings `retained_closure_record_inputs_are_checked_at_the_actual_binding` removes the retained closure obligation and actual incoming producer independently; Builder `callable_input_demands_require_exact_canonical_instantiation_not_only_source_flags` rejects corrupted canonical instantiation; neither specialized storage nor a destination header replaces that authority |
| Source paths/spans cannot become authority | Products `callable_source_transport_is_exact_remapped_source_free_and_diagnostic_only` rejects eight callable/local/path/range/UTF-8 transport mutations at emission/decode; `callable_body_local_sources_survive_source_free_instantiation_and_missing_location` preserves real producer locations and keeps missing locations absent; Reclamation nested-source negatives retain the actual returned expression span |

### Earlier snapshot terminal gates

The complete debug gate passes on the candidate below (`90999a5424f1ab1b`, exit 0). Earlier scoped passes were not a final-snapshot acceptance claim. The first complete debug attempt (`4570b5f6ed09f227`) failed with 2,988 unit passes / one owned-fold transport failure / nine ignored; later targets were not run. The unchanged exact failure control subsequently passed after the MIR result fix. The next attempt (`2821034ccf0ba106`) passed 2,990 units / nine ignored, then timed out in fresh stdlib compilation for allocator integration tests. The unchanged exact allocator control completed with temporary phase tracing in `661164b6d9a90ef6` (one pass, 187.37 seconds); tracing was removed, and complete gates now use a one-hour timeout. No stalled-compiler defect was established by that timeout, and no source restriction was weakened to resolve it.

The earlier candidate snapshot is identified by SHA256 `c574012ae18e8714e2a120a7748930db2729f1a85effebe69389e11b807d7a82` of `git diff --binary -- lib`, plus untracked `lib/src/mono/return_storage.rs` SHA256 `3ff01f1521314590ced3f3c69db3dfa3eb7058f8c71c240ab1d7b088815f6a55`. These identify functional changes without staging or committing them. These historical receipts are not acceptance evidence for the changed completion snapshot.

Baseline HEAD is `d3c51dbb5ba92e78d101db48f460c109050e49b0`. Environment: Linux x86_64, Cargo `1.101.0-nightly (f3865b2a4 2026-09-29)`, workspace static LLVM 18 configuration. An `llvm-config-18 --version` shell query was denied by the allowlist and was not rerouted; no exact queried LLVM patch-version claim is made.

Complete debug command: `cargo test -p rock-lib`, timeout 3,600,000 ms, no test filtering or formatting. Exact output: `/home/champii/.local/share/lean-ctx/archives/90/90999a5424f1ab1b.txt`, SHA256 `452b16a7a9e586b8f0daba2f829efccaf8589a187f56af1909e2ffcfaf1d515b`.

Complete release command: `cargo test -p rock-lib --release`, timeout 3,600,000 ms, exit 0. Exact output: `/home/champii/.local/share/lean-ctx/archives/d7/d74270e19cce2d6a.txt`, SHA256 `aceca6e73f2df4b07ce5e0e139adeebbad416392a1a8514e729f7ef514e1b571`. No functional source changes occurred between these gates or the subsequent build/Clippy commands; the tracked diff and untracked module hashes were rechecked afterward.

The following terminal counts match independently in **both complete debug and release**, not just in a passing subset or reusable job alias. Every listed target has zero failures, measured tests and filtered-out tests.

| Target | Passed | Ignored |
| --- | ---: | ---: |
| Library units | 2992 | 9 |
| `allocator_owner_execution` | 8 | 0 |
| `allocator_pointer_origins` | 12 | 0 |
| `callable_result_transport` | 29 | 0 |
| `copy_environment_reclamation` | 26 | 0 |
| `dynamic_completion` | 7 | 0 |
| `erased_objects` | 11 | 0 |
| `erased_scope_cleanup` | 4 | 0 |
| `existential_objects` | 16 | 0 |
| `existential_packages` | 19 | 0 |
| `integration` | 718 | 0 |
| `object_artifacts` | 5 | 0 |
| `object_contextual_syntax` | 2 | 0 |
| `object_execution` | 5 | 0 |
| `object_owners` | 13 | 0 |
| `object_types` | 29 | 0 |
| `owned_generic_objects` | 4 | 0 |
| `owned_object_calls` | 23 | 0 |
| `owner_cancellation` | 5 | 0 |
| `owner_constructor_statics` | 5 | 0 |
| `partial_owner_coercion` | 8 | 0 |
| `raw_helper_inference` | 5 | 0 |
| `runtime_dictionary_factories` | 2 | 0 |
| `runtime_nominal_factories` | 3 | 0 |
| `runtime_polymorphic_dictionaries` | 5 | 0 |
| `runtime_static_dictionaries` | 5 | 0 |
| `runtime_tuple_factories` | 2 | 0 |
| `test_parse_struct_with_fields` | 1 | 0 |
| Doctests | 0 | 1 |

Both test builds report seven existing library warnings: one unused import and unused helpers in lowering/borrow analysis. Warnings and ten total ignored tests are not reported as clean lint or executed tests. The main macro diagnostic tests pass in both complete runs; that is not a diagnosis of their historical failure.

`cargo build --release` also exits 0 (`be0b82240b2c4750`, 1m 13s), with seven library warnings and one `rockup` dead-code warning. Exact output: `/home/champii/.local/share/lean-ctx/archives/be/be0b82240b2c4750.txt`, SHA256 `62a8985112e1d8ef1290ad6ce937ed7e10aa7d4495bfc4f3a630a896ee600642`.

`cargo clippy --workspace --all-targets` exits 0 (`dc0c2d7aa8c37993`, 24.52s), **with warnings**. Lossless raw output: `/home/champii/.local/state/lean-ctx/tee/cargo_clippy_--workspace_--all-targets_92caa7b7.log`, SHA256 `d64fae5abedebf6805746459d49ce96088560c3b7939f985514c4a0273be39e4`. Reported target warning counts: library 226; library tests 337 (217 duplicate warnings); main integration 5; allocator-owner integration 1; `rock` library tests 3; `rock-shared` library tests 1; `rock-lsp` binary 2 and binary tests 2 duplicates; `rockc` binary tests 1; `rockup` binary 1. These are per-target counts, not a sum of unique findings. No Clippy auto-fix, formatter or formatting check was run.

Focused earlier-candidate results: reclamation 26/26 at O0/O3, callable-result transport 29/29 at O0/O3, shared-return units 7/7, and exact per-return MIR storage corruption 1/1. The historical complete gate includes these registered tests and all other targets; no test filter substitutes for full acceptance.

Frozen guards were compared against the prior recorded hashes: the initial review, allocator observer, HKT observer, C copy-environment observer, and schema59 stdlib product/object are unchanged. No stdlib source diff was present. No formatter or formatting check was run; `bd` remains unavailable, and no active issue/PR URL is known.

Those four historical gates were terminal and tied to their same functional snapshot, but left dedicated coverage gaps. The updated family/source-free indices above identify the added permanent controls, distinct from historical passes. This record does not resolve the historical macro diagnostic flake or declare the entire dynamic-dispatch consolidation finished.

### Completion follow-up snapshot

Functional snapshot: baseline HEAD unchanged; `git diff --binary -- lib` SHA256 `4ab804288c974b294d73c3c6c8c80c4daaf11a8472ee67f1b5a8d301cfa9d647`, plus untracked return-storage module SHA256 `3ff01f1521314590ced3f3c69db3dfa3eb7058f8c71c240ab1d7b088815f6a55`. Gate results from earlier snapshots do not apply to this one.

The preceding full debug attempt failed in archive `184a9878e974fb74`: 2,994 units passed, four failed, nine ignored; no integration targets ran. The overly broad reference-copy content resolution removed original temporary dependencies from the two frozen scalar-index observers (`HARNESS: actual incoming Temporary dependency must match the original rejection span: {UnknownExternal, Local(Local(1))}`) and bypassed two original missing-demand/retained-closure-obligation negatives. It was narrowed to concrete capturing-callable value copying outside producer dependency measurement, not repaired by changing those tests. The original three scalar-index controls pass again; all 40 binding controls pass, including both original negatives and the new owning-array copy. This failed intermediate gate remains distinct from final acceptance.

That failed command was `cargo test -p rock-lib`, terminal exit 101; exact output SHA256 `de9d580fed71e72b19d1481464acfd0fe0f870280bfb7efd949a79e852d0fc9a`. It predates the final source hash above and the final gates below.

Focused follow-up commands, all terminal exit 0:

| Exact command | Passed / ignored | Archive ID | Output SHA256 |
| --- | --- | --- | --- |
| `cargo test -p rock-lib --test copy_environment_reclamation` | 31 / 0; actual O0/O3 runtime | `bfe300512db192b7` | `7e1026e974bc0527b30226b1a06a32cbf008318dcdb3103e5b13afb3e3a3e671` |
| `cargo test -p rock-lib --test callable_result_transport` | 32 / 0; actual O0/O3 runtime | `90161e2a5096c03f` | `1906d04a260fc275a5589dedaf9e32189cd48e66b7dd47111b0fc79a57a627db` |
| `cargo test -p rock-lib --lib mir::borrowck::callable_results::bindings` | 40 / 0; 2968 filtered | `79186fa337789c6a` | `61b3941ff15ad0091cb6a97f8e9d3dccaf2d15fdcea2426c98a5a678d4161b3e` |
| `cargo test -p rock-lib --lib mono::return_storage::tests::all_reachable_returns_are_checked_atomically_in_both_orders -- --exact` | 1 / 0; 3007 filtered | `1857c7aa7592ecfb` | `25b1ea21003de7cf255e566b9dbe10a058e542def95f9c45b41dfc4a422439c1` |
| `cargo test -p rock-lib --lib mir::borrowck::callable_results::bindings::tests::nominal_field_envelope_and_canonical_demand_check_actual_roles_jointly -- --exact` | 1 / 0; 3007 filtered | `80166a9727264d51` | `43aba6bc427feceb933991d1f82c6813c73e5d117aec5a2ecc6591d0b1676368` |
| `cargo test -p rock-lib --lib mir::borrowck::callable_results::bindings::forwarding::tests::overlapping_record_write_joins_are_order_independent -- --exact` | 1 / 0; 3007 filtered | `07f40928890ef3fd` | `185e01788cf6594e1b35a1e237853a1e6ed010bd6b961028435de4b06f9bbe7d` |

Archive files are `/home/champii/.local/share/lean-ctx/archives/<first-two-ID-characters>/<ID>.txt`; job aliases are not unique receipts. Tests were serialized; no ignored test, negative execution or filtered subset is counted as complete acceptance. `bd ready --json` still exits 127 (`command not found: bd`); no active issue/change URL is known to link.

### Final completion gates

All four commands are terminal **PASS** on the `4ab80428` / `3ff01f15` functional snapshot above. They ran serially, in the same Linux/static-LLVM-18/Cargo environment as the earlier gates, with a 3,600,000 ms job limit. No functional compiler/test edits occurred between them; both source hashes were rechecked after Clippy. Documentation-only evidence updates followed.

| Exact command | Exit | Archive ID | Output SHA256 |
| --- | ---: | --- | --- |
| `cargo test -p rock-lib` | 0 | `569c6d3fe50d0d28` | `ba21730d357034f528dcc0a137835f5469cf455b0db6e708945b0942657fd1f8` |
| `cargo test -p rock-lib --release` | 0 | `02f6e0b3163fdbb6` | `6107e309e424fe1da680ec0499eb9ebf8e97a777098d703a8a6d6f585673c882` |
| `cargo build --release` | 0 | `483c6b5b58510449` | `138b75c78e3b1e16c3c2d3a4687440f2810e321548e60bf49a80c46ea27a9624` |
| `cargo clippy --workspace --all-targets` | 0 | `aef29ed06166b32f` | `0108f6f43739a197524a74d6aca1b6f52a2194e9047116f22d3deba01bd9f72c` |

The following counts were extracted independently from both complete test receipts. Every target has **zero failed, measured and filtered tests** in each gate, not just in the focused runs.

| Target | Debug passed / ignored | Release passed / ignored |
| --- | ---: | ---: |
| Library units | 2999 / 9 | 2999 / 9 |
| `allocator_owner_execution` | 8 / 0 | 8 / 0 |
| `allocator_pointer_origins` | 12 / 0 | 12 / 0 |
| `callable_result_transport` | 32 / 0 | 32 / 0 |
| `copy_environment_reclamation` | 31 / 0 | 31 / 0 |
| `dynamic_completion` | 7 / 0 | 7 / 0 |
| `erased_objects` | 11 / 0 | 11 / 0 |
| `erased_scope_cleanup` | 4 / 0 | 4 / 0 |
| `existential_objects` | 16 / 0 | 16 / 0 |
| `existential_packages` | 19 / 0 | 19 / 0 |
| `integration` | 718 / 0 | 718 / 0 |
| `object_artifacts` | 5 / 0 | 5 / 0 |
| `object_contextual_syntax` | 2 / 0 | 2 / 0 |
| `object_execution` | 5 / 0 | 5 / 0 |
| `object_owners` | 13 / 0 | 13 / 0 |
| `object_types` | 29 / 0 | 29 / 0 |
| `owned_generic_objects` | 4 / 0 | 4 / 0 |
| `owned_object_calls` | 23 / 0 | 23 / 0 |
| `owner_cancellation` | 5 / 0 | 5 / 0 |
| `owner_constructor_statics` | 5 / 0 | 5 / 0 |
| `partial_owner_coercion` | 8 / 0 | 8 / 0 |
| `raw_helper_inference` | 5 / 0 | 5 / 0 |
| `runtime_dictionary_factories` | 2 / 0 | 2 / 0 |
| `runtime_nominal_factories` | 3 / 0 | 3 / 0 |
| `runtime_polymorphic_dictionaries` | 5 / 0 | 5 / 0 |
| `runtime_static_dictionaries` | 5 / 0 | 5 / 0 |
| `runtime_tuple_factories` | 2 / 0 | 2 / 0 |
| `test_parse_struct_with_fields` | 1 / 0 | 1 / 0 |
| Doctests | 0 / 1 | 0 / 1 |

The O0/O3 runtime/source-free cases in the family index execute in both gates: deleted provider sources stay absent, nongeneric native declarations are checked as bodyless, retained adapters remain serialized bodies, real imported crate identities change under dependency order, and the original emitted product/object bytes remain frozen. Borrow/Mono/corruption negatives do not link or execute. Ignored tests are not treated as executed acceptance.

Both test builds retain seven library warnings. Release build completes in 1m 35s with those seven warnings and one `rockup` dead-code warning. Clippy completes in 20.40s **with warnings**: library 226; library tests 338 (217 duplicates); main integration 5; allocator-owner integration 1; `rock` library tests 3; `rock-shared` library tests 1; `rock-lsp` binary 2 and binary tests 2 duplicates; `rockc` binary tests 1; `rockup` binary 1. These are per-target reports, not a sum of unique findings. The final lossless Clippy output was copied to `.agent-work/logs/early-return-completion-clippy-a396ee88.log`, SHA256 `a396ee88feb61f55cdb27112868a38b6189b1140d11b3d1f30953c159ca349c7`; the tool's command-derived tee path is reusable and now holds this run, not the earlier run's bytes. No auto-fix or formatter was run.

Source/review guards were checked at consolidation acceptance, before the later removal of obsolete artifact-only observer guards:

| Guard | Unchanged SHA256 |
| --- | --- |
| Initial consolidation review | `f2e0f32d980a71a0450907962d5c75ecdcd94aaf2318a0287060d1e2939c3fa4` |
| Allocator observer | `7b34a40dbdab7436872d1cdd6836574a113a3862b310e564d4a53e304b289c0c` |
| HKT observer | `186acbec7355747c68dce323978157feceabf4293dfd471d1e63adee2bf737fc` |
| C copy-environment observer | `53438d867a67530d7d01ee22eb539b791965d389765b4ad7ff2f21a7737890f2` |

At consolidation acceptance there was no stdlib, main-integration, frozen-observer or proof-cap source diff, and the initial review was unchanged. No staging, commit, push or other VCS mutation occurred. Subsequent forward-only artifact cleanup retires obsolete schema57/58 snapshots and their hash-only observer guards; the current-format acceptance fixture, original source checks, proof caps and semantic assertions remain. Historical artifact preservation is not a continuing requirement.

### Scoped completion conclusion

The early-return storage/callable-loan completion contract is **complete on this snapshot**: shared target-sensitive atomic return agreement, independent continuation joins, declaration/producer/storage/loan-role boundaries, dedicated R01-R15/C01-C18 and source-free/corruption controls, complete debug/release/build/Clippy acceptance and frozen guards are all established above. This is not a claim of every conceivable permutation or whole-feature completion.

Existing unsupported combinations remain deliberately rejected: non-scalar native callback certification without admitted origin authority, opaque/cyclic/missing producer recipes, invalid environment-storage escapes, borrowed nominal moves, explicit protocol/safety widening and second use of consuming callables. Direct checked nominal calls and retained generic value transport remain separate accepted positives, not certificates for those rejected combinations. No format59/ABI change, added proof budget, caller annotation workaround or formatting work was used to reach completion. The historical macro-flake root cause remains unknown and separate; its passing controls in both final gates do not diagnose the old failure.
