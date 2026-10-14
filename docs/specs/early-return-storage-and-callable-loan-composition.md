# Early-return storage agreement and callable/loan composition completion

Date: 2026-10-09

Status: implementation specification, not an implementation or acceptance claim.

Scope: finish these two audits within dynamic-dispatch consolidation. Feature expansion and all formatting work remain paused.

## 1. Purpose and relationship to existing work

This specification defines the completion contract for the remaining return-storage and callable/loan composition obligations of the consolidation. It does not define a new callable ABI or broaden dynamic-dispatch language features.

The baseline is [the initial consolidation review](dynamic-dispatch-consolidation-review.md). That review is historical and must remain byte-for-byte unchanged. The overall language specification is [dynamic-dispatch.md](dynamic-dispatch.md); implementation and verification history is in `MEMORY.md`.

The required outcome is:

1. Every normal return from a callable uses one producer-validated storage representation compatible with its independently established semantic contract.
2. Return agreement is independent of which return appears first, which branch is written first, whether the tail has captures, or whether a return occurs in a nested expression.
3. Callable producer facts and loan dependencies compose through the supported value, aggregate, projection, capture and call paths without manufacturing authority.
4. Every accepted result keeps its actual ownership, access restrictions, invocation protocol, loans and cleanup responsibility.
5. Unsupported or incompatible storage is rejected before LLVM emission with the appropriate source-backed diagnostic.
6. Existing supported programs and all frozen positive and negative controls remain intact.

The implementation must repair the responsible compiler phases. Adding local type annotations to regression programs, relaxing callable restrictions, raising observer caps, editing stdlib abstractions to evade the defect, or rejecting previously supported compatible compositions is not an acceptable substitute.

## 2. Current implementation and evidence boundary

These observations describe the inspected checkout, not a proof that every listed path is defective.

| Area | Current implementation | Completion obligation |
| --- | --- | --- |
| Storage detection | `has_measured_storage` recognizes represented nominals and functions with captures | A diagnostic must not depend solely on the selected candidate having such a marker |
| Block result refinement | `process_block` derives a candidate from its last expression/return | Account for all relevant exits, including all-explicit-return and non-returning-tail bodies |
| Early-return check | `block_returns_agree` visits nested expressions and skips nested lambda bodies | Establish exact return ownership, reachability and target-sensitive block/loop behavior |
| Function processing | `process_function` refines a return when the candidate agrees | Never silently keep a canonical header that loses incompatible returned storage |
| Generic specialization | `create_specialization_from_signature` repeats candidate/agreement handling | Use the same agreement decision as ordinary functions |
| Method specialization | `process_impl_method_signature` uses `declared_value_representation` and agreement | Preserve declaration-owned slots and generic substitution authority |
| Lambda processing | Lambda return refinement follows processed block type | Validate the lambda's own returns independently of its enclosing callable |
| Conditional/match results | Equal processed branch types refine the result; measured disagreement rejects | Exclude non-continuing branches and validate every continuing producer without order dependence |
| Nominal result storage | Canonical local MIR results have separate `native_result_storage` | Keep the semantic/storage split and validate storage against every actual return |
| Producer transport | `Recipe` models arguments, calls, aggregates, projections, alternatives and field overlays | Preserve only justified facts at each path and join |
| Callable input demands | `callable_payload_requirements` pairs actual fields with canonical requirements | The enclosing requirement and declared field envelope remain joint obligations |
| Loan measurement | Lifetime and ValueLoans summaries are measured separately | Never collapse distinct roles merely to obtain an invocation certificate |

In particular, `process_block` currently reports a failed `block_returns_agree` check only when `has_measured_storage(actual)` is true. A captured early return paired with an uncaptured tail is therefore a required regression case. The reverse ordering, failure of `value_representation` itself, and functions without a useful lexical tail are equally required. Their runtime or diagnostic outcomes must be measured before recording a confirmed defect.

Existing evidence includes a passing 19-control reclamation suite with O0/O3 execution, represented enum parameters, active-variant cleanup, consuming results and source-free producers. Earlier complete debug/release receipts precede the newest changes. None of those receipts establishes acceptance of this specification.

## 3. Scope boundaries

### Included

- Ordinary functions, impl methods, lambdas and existing generic specializations.
- Explicit returns, implicit tails, nested ordinary/unsafe blocks, conditionals, matches, guards, existing loop exits and returns nested in evaluated expressions.
- Direct callables, nested callable results, tuples, supported arrays, represented structs/enums, and existing safe-reference projections.
- Actual move/borrow capture storage and existing recursive owning callable copies.
- Local producer bodies, checked native producers and existing source-free generic adapter bodies.
- Canonical definition-owned requirements and existing Fn/FnMut/FnOnce/HKT callable input obligations.
- Producer/effect/loan invalidation across aliases, owned field writes, branches, loops, calls and consuming cleanup.
- MIR/result-storage, native publication/import and descriptor agreement relevant to these paths.
- Permanent positive, negative and corruption controls, followed by complete final-snapshot acceptance.

### Excluded

- New syntax, new owner protocols, new existential forms or additional erased/HKT execution features.
- A runtime union of incompatible concrete callable environments, a new enum-copy operation, or a new descriptor ABI.
- Synthesizing enum variant storage from unrelated producers to make incompatible return representations agree.
- New scalar callable demands or expansion of `UnknownExternal` to non-scalar native records.
- Relaxing explicit Fn/FnMut constraints, raw-pointer invariance, safe-reference mutability, existing unsupported-effect vetoes or consuming-use conservatism.
- Formatting, formatting checks, formatter configuration or exclusions, and reformatting frozen observers. A later user-authorized full pass owns that work.
- Diagnosing the historical macro flake as a separate feature of this specification. A flake in a required full gate still prevents claiming that gate passed.
- Commits, staging, pushes, or changes to `.sisyphus/`.

## 4. Separate authorities

The following facts have different owners and must not substitute for one another.

| Fact | Authority | What it does not prove |
| --- | --- | --- |
| Semantic result type | Accepted definition, inference and canonical substitution | Concrete environment layout or producer identity |
| Measured storage representation | Actual construction/producer and checked storage metadata | Invocation permission, borrowing contract or effects |
| Definition-owned callable requirement | Accepted definition and canonical input obligations | That a caller's supplied value has a checked producer |
| Producer evidence | Checked body/closure or exact validated portable native record | Permission to ignore another reachable alternative |
| Return dependencies | Independently measured Lifetime and ValueLoans roles | Mutation/retention independence or an empty effect set |
| Move/Copy/drop authority | Actual storage, existing ownership semantics and complete drop plans | That a same-sized representation can be substituted |
| Place access | Actual safe loan and existing access/conflict checks | Ownership of a borrowed nominal payload |

`value_representation` is a representation transport operation, not a borrowing or producer certificate. `declared_value_representation` additionally restricts refinement to slots owned by the declaration. Neither is an alternative to checking actual incoming producer facts.

`normalize_type` and `normalize_storage_type` have distinct jobs. Canonical nominal identity may forget represented fields; storage agreement must retain them. Neither nominal identity equality nor equal LLVM size/alignment establishes storage agreement.

An empty measured loan set remains an empty measurement. It must not be rewritten to Static. Unknown, missing evidence and unmeasured roles are not empty loan sets or source independence.

## 5. Return and continuation model

### 5.1 Return ownership

Each return belongs to the nearest enclosing callable body: an ordinary function, method or lambda.

- Ordinary and unsafe blocks do not create callable return targets.
- A nested lambda creates a new target; its internal returns must not be checked against the enclosing callable's result.
- The enclosing callable still validates the lambda value's actual storage if that value is itself returned.
- A return inside a branch, guard, call argument, initializer or other evaluated expression targets its enclosing callable.
- A `break` targets its actual loop and supplies that loop's result when the existing language supports a value-bearing break. It is not a function return.
- A `continue` does not supply a result. A return nested in a loop body remains a callable return.
- Compiler-generated cleanup and adapter bodies must not create accidental source-level return targets.

Return discovery must use existing typed HIR identities and traversal, not names, source text or a growing whitelist of accepted expression shapes.

### 5.2 Normal continuations

Distinguish:

1. A normal value-producing exit from an expression/block.
2. A return from the current callable.
3. A targeted break/continue.
4. A non-returning path.
5. An unavailable or incompatible representation on a value-producing path.

These states must not be conflated into a single boolean or ambiguous missing candidate.

Only normal value-producing exits contribute to the containing expression's storage join. Explicit callable returns contribute to that callable's return agreement, even when their surrounding expression does not continue.

A branch that returns from the callable, a never-returning call, or an established non-terminating path must not be treated as a competing unit/canonical-tail representation. A unit-valued normal return is a real return candidate when unit is the semantic result.

Discarded expression values, including the discarded tail of an effects-only body, do not become returned storage. They retain their ordinary cleanup obligations.

### 5.3 Reachability

Agreement is based on control flow, not a caller's current literal argument or an optimization level.

- No statement after an established unconditional terminator contributes a normal exit or a producer proof.
- Every structurally reachable branch/return remains relevant unless existing pre-emission analysis proves it unreachable.
- A constant actual argument such as `true` must not erase a definition's unsafe alternative or certify a universal contract.
- Missing reachability evidence is handled conservatively.
- Do not add a new value-range optimizer or arbitrary constant execution to this work.
- HIR agreement and MIR reachable-return validation must agree on the exits they admit; disagreement is a compiler validation failure, not permission to silently change storage.

### 5.4 Bodies without a lexical tail

An all-explicit-return body selects its representation from those returns. A body with no normal returns does not invent a concrete producer or a storage row from an unreachable tail.

A fallthrough body uses its actual normal tail only if that tail is returned. An effects-only fallthrough supplies unit through the existing semantics.

## 6. Storage agreement contract

### 6.1 Atomic, order-independent agreement

For a callable with canonical result contract `C`, collect every relevant returned value and its actual processed representation.

The agreement procedure is conceptually:

1. Determine which exits return values to this callable and which exits normally continue.
2. Resolve each returned value's producer-backed representation against `C`, preserving declaration/substitution restrictions.
3. Validate every represented nominal row and relevant projection against its declaration schema.
4. Establish one compatible storage representation for all normal returns.
5. Validate each return against that representation and the independent callable/loan obligations.
6. Commit the refined result/storage metadata only after the complete decision succeeds.

The first observed return may seed a candidate, but it has no special authority. Reversing the return/branch order must not change acceptance, emitted representation or diagnostic class.

There must be no partially committed return refinement after a later mismatch. An incompatible candidate, failed representation transport or unmeasured required storage must not silently fall back to a canonical arrow/nominal header that loses ownership or cleanup information.

The decision applies even when the chosen candidate has no captures or nominal representation marker. Examining all returned representations, rather than only `has_measured_storage(candidate)`, is mandatory.

### 6.2 Compatible storage

Use the existing checked storage normalization and representation services. Do not invent an alternative layout-equivalence system.

Compatibility requires agreement on all storage facts needed for correct access, copying, calling and destruction, including:

- Canonical nominal identity and instantiated semantic arguments.
- Ordered struct fields and enum variant identities/field schemas.
- Tuple positions and array element representation/length.
- Actual callable capture count, order, capture mode and recursively owned storage.
- Applicable invocation protocol and safety restrictions.
- Safe-reference mutability and checked pointee representation where already supported.
- The complete existing ownership/drop convention.

Different producer identities or different runtime captured values may share compatible storage. Identity equality is not required merely because both producers return closures.

Different borrowed origins may also share storage. They remain distinct dependencies for the independent loan analysis; storage agreement must not erase them.

Different callable borrowing metadata is not automatically a physical-layout mismatch. If existing services support their conservative composition, preserve role-separated measurements and validate every alternative. Otherwise reject explicitly. Never copy one branch's stronger header to another or union away an access restriction.

### 6.3 Incompatible storage

Reject cases that would require a new runtime representation, including uncaptured versus owning-captured arrows and incompatible scalar-versus-tuple capture environments, even if their outer function-value ABI has the same width.

Represented nominal disagreement must reject rather than select one branch's fields or fall back to the nominal declaration. In this scope, different enum variant storage tables do not gain a synthesized common environment table merely because the active variants differ.

This is a fail-closed boundary for genuinely incompatible storage, not authorization to reject existing compatible callable, borrowing or aggregate compositions.

### 6.4 Generic and method constraints

- A declaration-owned arrow slot may preserve producer-measured environment storage under the existing refinement rules.
- A generic binder's selected representation remains authoritative; a use-site destination or actual closure cannot redefine that binder's substitution.
- Impl-first and method-local generic ordering remain canonical.
- Ordinary functions, methods, generic specializations and lambdas must use one shared agreement policy. Their representation emission may remain different.
- Parameter-layout specialization remains distinct from semantic substitutions and is used only for actual measured layout differences.
- Active recursive definitions cannot certify their unfinished result from themselves. Reuse existing recursion guards and conservative unresolved-evidence handling.

### 6.5 Canonical MIR results and native storage

For local represented nominal results, retain the existing separation:

- Callable semantic headers, MIR `ret_type` and canonical result locals keep nominal identity.
- Producer-validated `native_result_storage` supplies measured return storage.
- Construction, projection, origin/loan measurement, copying, cleanup and emission use the appropriate checked storage accessors.
- Every reachable return must match the published storage row; validating only the lexical tail is insufficient.
- Missing, stale, forged or invalid storage metadata rejects before codegen.

Do not spread represented fields into canonical dictionary witnesses or introduce a second storage side channel. Non-nominal callable results retain the existing valid representation convention rather than being forcibly canonicalized by this nominal rule.

## 7. Callable and loan composition contract

### 7.1 Definition-owned requirements

Requirements originate in the accepted definition before caller specialization. Retained requirements must survive concrete-definition removal, method instantiation and existing source-free body transport.

For each admitted callable payload path:

1. Obtain the canonical enclosing input requirement.
2. Instantiate the declaration-owned field envelope independently.
3. Match the actual storage/path against both obligations using existing services.
4. Require actual producer measurement at every binding/invocation that discharges the obligation.

A declared field envelope may restrict the accepted source set; it does not become a producer certificate. An Unknown field header cannot strengthen a canonical demand. Without an enclosing requirement, a field's physical representation does not seed a new universal demand.

Demand propagation must be atomic over the definition-use graph. Existing repeated-use, borrowing, capture/exposure and unsupported-effect vetoes remain in force. A failed path cannot leave valid-looking partial requirements behind.

Scalar measurements may be transported where the current implementation already admits them, but scalar result types do not manufacture new owned callable input demands.

### 7.2 Producer facts and value recipes

Reuse the existing `Recipe` machinery rather than implementing return-specific producer recognition.

| Recipe/path | Required behavior |
| --- | --- |
| Argument | Forward actual supplied facts; an argument type/header alone is not a fact |
| Opaque | Preserve uncertainty, not an empty effect/source certificate |
| Callable/Measured | Use the exact checked callable/body/native identity and actual measurements |
| Project/Reference | Preserve full typed projection/access paths and the correct source roles |
| Aggregate | Attach facts to actual field/element/variant paths without flattening |
| Call | Compose actual callee and argument facts only under validated target/contract authority |
| Alternatives | Check all relevant producer alternatives; retain only justified path evidence |
| Overlay | Replace facts at the exact owned write path, invalidating overlaps and preserving proven disjoint fields |

Recipes transport producer facts. They do not establish Copy, ownership, capture permission, storage layout, effects or borrowing authority by themselves.

### 7.3 Joins and deterministic projection identity

Evidence at a CFG join is must-evidence: it is retained only where justified on every relevant incoming path. Possible dependencies are conservatively combined within each independently measured origin role. These are different joins and must not be conflated.

- A producer missing on one applicable predecessor is not supplied by a producer on another.
- Different checked native function alternatives may remain usable when the existing alternative validation proves each compatible with the requirement.
- A known, dominating enum discriminant may exclude an impossible variant path. A represented enum schema alone does not prove which variant is active.
- Distinct enum variants, field identities, dereferences and element paths must remain distinguishable during ordering, deduplication, overwrites and equality.
- Do not use field indices alone as a complete identity for projections that also contain downcasts or other access steps.
- Joins, overlay application and diagnostics must be deterministic under map insertion order and imported ID renumbering.
- A one-predecessor write does not establish an initialized field or producer on all predecessors.

Reuse shared place-conflict, initialization, origin and CFG machinery. Do not introduce a second ownership validator in the return agreement code.

### 7.4 Alias, write, call and loop invalidation

- Immutable aliases transport supported facts without creating fresh authority.
- An owned field overwrite invalidates facts for that field and overlapping paths, including aliases, while keeping independently proven disjoint paths.
- A parent overwrite invalidates descendants; a child overwrite does not certify the remainder of an opaque parent.
- Borrowed/unknown writes cannot be treated as uniquely owned overlays.
- Calls invalidate or preserve facts according to independently established mutation, retention and exposure information. An unrelated call is neither a blanket proof of independence nor automatically a reason to erase a disjoint checked producer.
- Moving a carrier invalidates its old owner; a later use or second consuming invocation remains rejected.
- Loop/back-edge evidence must converge through the existing fixpoint rules. Initial-iteration facts cannot certify every later iteration.
- Existing proof/resource caps remain unchanged. A cycle or missing producer fails closed rather than self-certifying or increasing the budget.

### 7.5 Role-separated return dependencies

Measure and transport Lifetime and ValueLoans separately. Preserve argument ordinals, output/projection paths and environment roles through every supported composition.

Required distinctions include:

- References carried by an argument value versus references obtained from its contents.
- Actual borrowed contents versus authority to borrow the storage holding those contents.
- Environment value/contents versus environment storage.
- Safe references, raw-memory views and scalar dependency graphs.
- Static versus Unknown/UnknownExternal.

If one role is missing or the roles disagree in a way that prevents the existing certificate from being formed, no invocation certificate is fabricated. Keep the conservative measurements and reject the obligation that requires unavailable proof.

`UnknownExternal` remains limited to the current checked scalar native-record admission. It is not source independence, mutation/retention knowledge or permission to externalize local/temporary origins.

### 7.6 Invocation protocol and access

- Actual environments continue to determine inferred callable kind under the existing rules.
- Explicit Fn/FnMut contracts are not inference wildcards and must not be widened to accept incompatible FnOnce environments.
- Repeated Fn/FnMut invocation requires the existing real access/effect proof; an owned-result demand does not imply reusable invocation.
- FnMut requires writable access and remains incompatible with conflicting live shared loans.
- Shared invocation remains incompatible with a live conflicting unique loan.
- Exposed or opaque callable storage does not supply empty effects.
- Borrowed represented struct/enum patterns preserve measured payload storage but remain borrowed bindings. They cannot move a non-Copy nominal payload out of a safe reference.

### 7.7 Result-before-cleanup ownership

A consuming call/return must:

1. Read and validate actual callee/argument facts while the original carriers are still available.
2. Initialize the result and transport its producer/origin/loan facts.
3. Transfer move and initialization responsibility exactly once.
4. Release consumed carriers/environments using complete `PayloadDrop` plans at the existing lexical cleanup point.

An output borrowed from consumed environment storage still rejects. An output carrying an independently valid argument/payload loan may survive consumption when the existing ownership contract permits it.

Owning copies recursively duplicate concrete callable environments using the existing operation. Borrowed invocation reads do not become owning copies. Nominal enums and arrays remain non-Copy.

Active-variant cleanup destroys only initialized, owned fields of the active variant. Actual user destructors and structural field cleanup retain their correct order and timing; matching allocation/release counts alone is insufficient.

## 8. Compiler responsibilities

| Phase/files | Responsibility |
| --- | --- |
| `lib/src/infer/generalize.rs` | Preserve definition-owned, atomic callable demands and all existing vetoes; do not infer storage from caller wishes |
| `lib/src/type_services/callable.rs` | Shared representation transport, declared-slot restrictions, protocol/safety compatibility and conditional demand construction |
| `lib/src/mono/process.rs` | Target-sensitive exits, normal-result/return agreement, actual patterns/captures and no silent canonical fallback |
| `lib/src/mono/specialize.rs`, `lib/src/mono/methods.rs` | Reuse agreement for functions/methods with canonical substitutions and measured parameter layouts |
| `lib/src/hir/visit.rs` | Reuse typed traversal; callable/loop ownership must be explicit where generic child traversal is insufficient |
| `lib/src/mir/builder/` | Canonical result/storage separation, correct evaluated temporaries, branch bindings and result-before-cleanup scheduling |
| `lib/src/mir/backend_contract.rs`, `lib/src/mir/agreement.rs` | Validate actual return storage, schemas, signatures, result locals, projections and every relevant MIR return |
| `lib/src/mir/backend_contract/callable_inputs.rs` | Pair actual payloads with enclosing canonical and declared field obligations |
| `lib/src/mir/borrowck/callable_results/` | Producer binding/recipe composition and independent discharge |
| `lib/src/mir/borrowck/return_sources.rs`, `liveness.rs`, `provenance.rs`, `borrows.rs` | Independent source-role measurement, loan transport, invalidation and cleanup liveness |
| `lib/src/mir/object.rs`, `object_adapters.rs`, `erased.rs` | Exact descriptor/drop and existing native/erased boundary validation |
| `lib/src/products/native_borrowing.rs`, `type_table.rs`, `lib/src/crate_artifact/load.rs` | Checked portable storage/source publication, decoding, declaration validation and canonical remapping |
| `lib/src/codegen/mir_llvm/` | Emit only validated storage/ownership decisions; no layout or producer repair during LLVM emission |

Prefer one small shared internal agreement result over duplicated booleans and fallback rules if existing APIs cannot express the required states. It must distinguish non-returning flow from missing/incompatible proof, remain phase-owned, and retain enough exact source provenance for diagnostics. No new persistent registry or serialized exit-analysis format is required.

The implementation order is semantic exit/agreement repair, producer/loan composition validation, MIR/native boundary validation, then serialized acceptance. This order is dependency guidance, not a separate issue-tracking system.

## 9. Diagnostics

User-source storage disagreements are structured source-backed Mono diagnostics before backend emission. Typing errors remain Type/Selection diagnostics; actual ownership, escape and access violations remain Borrow diagnostics. Corrupt artifacts are rejected through existing artifact-validation diagnostics.

For a return-storage conflict:

- Identify the conflicting return/tail/branch value with its real span.
- When available, label another actual return that established the incompatible candidate.
- Describe the relevant storage path and mismatch, such as an owning capture versus no environment or incompatible represented fields.
- Do not require identical wording across every control-flow form, but keep diagnostic class and failure ordering deterministic.
- Do not expose opaque compiler debug identities as a substitute for a meaningful source diagnosis.
- Do not invent a return span from the function header, caller destination or unrelated declaration.

Source-free retained bodies use their existing producer-owned source/local metadata. Missing producer locations remain missing and must not be repaired by an invented caller span. Impossible post-validation disagreements remain Internal compiler errors; tests must distinguish those corruption/invariant cases from valid user-source rejection.

## 10. Representative behavior

The following are intended regression programs, not claims of current execution. Each fence is self-contained and uses no stdlib mutation or workaround.

### 10.1 Compatible early and tail returns

Both producers capture an I64 using the same valid environment convention; different values and code identities do not create a storage mismatch.

```rock
make = seed -> -> seed

choose = flag, seed ->
    if flag
        return (make seed)
    make seed

main = ->
    callback = choose true, 42
    callback!
```

This must compile and return 42 at O0/O3. A permanent observation control must additionally verify owning copies and reclamation, not just the integer result.

### 10.2 Captured early return, uncaptured tail

```rock
make = seed -> -> seed

choose = flag, seed ->
    if flag
        return (make seed)
    -> 42

main = ->
    callback = choose true, 42
    callback!
```

This must reject before codegen because a canonical/plain tail cannot describe the other return's owning environment. Changing the caller's flag to `false` does not certify the definition. Reversing the two return representations must also reject.

### 10.3 Represented nominal return

```rock
struct Holder
    < value: () -> I64

make = seed ->
    Holder
        value: -> seed

choose = flag, seed ->
    if flag
        return (make seed)
    make seed

main = ->
    held = choose false, 42
    held.value!
```

This must retain measured field storage while preserving canonical local nominal result identity. Its explicit-result-signature and source-free bodyless producer variants are separate required controls.

### 10.4 Shared storage does not erase different loans

```rock
choose: Bool -> &I64 -> &I64 -> &I64
choose = flag, left, right ->
    if flag
        return left
    right

main = ->
    left = 40
    right = 42
    result = choose false, &left, &right
    *result
```

This must retain conservative return dependencies on both inputs, without treating the caller's literal flag as a universal producer certificate. A paired control whose applicable result can refer to dead local storage must reject. The signatureless supported form must also remain accepted without an annotation workaround.

## 11. Required regression coverage

The IDs below identify specification acceptance cases, not implementation progress or replacement issue tracking. Add missing controls in non-frozen test modules; preserve original tests, source bytes and caps.

### 11.1 Return/storage cases

| ID | Case | Required outcome |
| --- | --- | --- |
| R01 | Same storage on an early return and tail, with two distinct producers | Accept; preserve actual captures and cleanup |
| R02 | Captured early return/plain tail and its reverse | Source-span storage rejection in both orders |
| R03 | Same outer-ABI-width callable values with incompatible environments, including scalar versus tuple captures | Reject; ABI width is not agreement |
| R04 | All-explicit-return function with no normal lexical tail | Derive agreement from returns; no canonical fallback |
| R05 | Returning branch combined with a continuing branch | Join only normal continuations; check the explicit return against the callable result |
| R06 | Non-returning tail/branch and code after an unconditional terminator | No fictitious unit candidate or producer proof; reachable alternatives remain conservative |
| R07 | Return in nested blocks, unsafe blocks, initializer, guard and evaluated call argument | Correct callable target and real source provenance |
| R08 | Nested lambda with its own early returns, including a returned lambda value | Independent inner agreement; outer callable validates the lambda value, not its internal return type |
| R09 | Loop break/continue versus function return; nested loop targets | Correct exit owner, loop result and cleanup |
| R10 | Tuples, nested tuples, homogeneous arrays/repeats and represented struct/enum payload returns | Preserve recursive storage and exact active paths; incompatible representations reject |
| R11 | Explicit and inferred results in ordinary functions, generic specializations and methods | Same shared policy; declaration-owned slots and selected substitutions preserved |
| R12 | Mutability mismatch/raw-pointer mismatch and numeric-literal array controls | Existing restrictions/coercions unchanged; no blanket storage rejection |
| R13 | Different origins with compatible returned-reference/callable storage | Preserve independent role-specific dependencies; no arbitrary branch selection |
| R14 | Direct/indirect recursion with missing producer evidence | No self-certification; deterministic fail-closed behavior within unchanged caps |
| R15 | Reversed branch order and reordered producer registration | Identical acceptance and storage decision, deterministic diagnostic class |

R02 must be tested with the candidate both measured and unmeasured, and with representation transport failing outright. A single nominal tail regression is insufficient.

### 11.2 Callable/loan composition cases

| ID | Case | Required outcome |
| --- | --- | --- |
| C01 | Input invocation through immutable callback/data/result aliases and multiple call sites | Each canonical requirement survives; every actual producer is checked |
| C02 | Canonical demand plus narrower declared field envelope, and Unknown field header | Both obligations enforced; Unknown cannot strengthen the enclosing demand |
| C03 | Callable through tuple/record/array/enum projection and a captured relay | Full paths and actual producers survive supported transport |
| C04 | Multiple early-return producer alternatives | Every applicable alternative independently validated |
| C05 | Producer missing on one branch or malformed on one return | Reject; another branch cannot fill the hole |
| C06 | Field replacement on both branches versus only one branch | Exact overlap invalidation; only justified common facts survive |
| C07 | Parent/child writes, disjoint fields, aliases and different enum downcasts | Preserve typed identity and shared conflict semantics; no stale or falsely disjoint evidence |
| C08 | Incidental call with known disjoint facts versus exposed/opaque storage | Preserve proven disjoint facts; do not fabricate empty effects |
| C09 | Loop/back-edge producer replacement and recursion | Existing fixpoint converges; no first-iteration or cyclic proof |
| C10 | Lifetime/ValueLoans disagreement, missing role, EnvironmentStorage and borrowed contents | Keep roles distinct; no forged invocation/escape certificate |
| C11 | Safe/raw returned views and existing scalar UnknownExternal transport | Correct dependency graph; no scalar independence or widened native admission |
| C12 | Reusable Fn/FnMut positives and explicit protocol negatives | Positive reuse preserved; mutable access, unsafe and consuming restrictions unchanged |
| C13 | Stored FnOnce with a real non-Copy capture, one use and second-use negative | One invocation valid; old carrier/second invocation rejected |
| C14 | Borrowed represented struct/enum pattern, callable field copy and nominal payload move | Valid field copy preserves storage; illegal borrowed move rejects |
| C15 | Consuming return of a nested callable/nominal/aggregate carrying a valid loan | Result facts and initialization precede carrier/environment cleanup |
| C16 | Return borrowing consumed environment storage or a dead local/capture | Borrow rejection before execution |
| C17 | Owned user destructor containing loans, unused result and partial moves | Real destructor timing/order, complete drop plan and exactly-once responsibility |
| C18 | Map insertion order, active variants and imported ID renumbering | Deterministic producer/path joins without identity collisions |

Conservative veto tests must use real ownership/capture/effect conditions, not numeric values that happen to be Copy. No negative is linked or executed merely to inspect its behavior.

### 11.3 Source-free and corruption cases

The source-free fixture must publish a checked producer before compiling the consumer, delete provider sources, and preserve artifact/object bytes throughout later O0/O3 consumption.

Cover both a nongeneric linked/bodyless native producer and an existing serialized generic adapter body. A locally specialized function is not evidence of bodyless native correctness.

Required artifact checks include:

- Compatible early-return producer layouts and role-separated dependencies survive emission/import.
- Exact closed producer signature, selected callable identity, object/link path and result storage agree after remapping.
- Removing the real producer/native record does not turn a header or physical field into a certificate.
- Missing, forged, stale or invalid result-storage TypeIds reject without panicking.
- Extra enum variants, missing fields, forged captures and declaration/schema mismatch reject.
- Bodyless owned and borrowed nominal inputs with incompatible measured storage reject at their actual binding spans.
- Corrupt input obligations, origin roles, source paths or a single return alternative cannot certify the remaining program.
- Retained producer source spans remain correct; missing locations do not acquire fabricated caller provenance.
- Current-format source-free fixtures remain intact; original source guards are preserved. Obsolete compiled artifacts and their hash-preservation bookkeeping are not required.

This completed consolidation used existing format59 rows and validation. Subsequent feature work may advance the artifact schema and rebuild dependencies; old schemas must reject before decoding, without compatibility readers or migration shims.

## 12. Runtime observations and immutable controls

Preserve:

- The initial consolidation review.
- `lib/src/mir/borrowck/allocator_trace_tests.rs` and `hkt_return_origin_tests.rs`, including original proof caps and sources.
- `lib/tests/fixtures/copy_environment_observer.c`.
- Existing HKT integration sources, stdlib sources and producer/role/exposure/ownership negatives.
- Current-format product/object acceptance fixtures and historical diagnostic logs; obsolete compiled snapshots need not be retained.

New positive controls must observe effects, not just successful compilation:

- Correct result values at O0/O3.
- Distinct simultaneously live environments for owning callable copies via the unchanged C observer.
- Balanced allocations/releases and no surviving moved carrier.
- User destructor counters, order and lexical timing after early exits.
- Active-variant-only cleanup and initialized-field responsibility.
- Valid retained loans after consuming transport, alongside paired mutation/escape negatives.

Optimization may eliminate unobserved allocations. Allocation assertions must correspond to actually C-observed live environments rather than incidental temporaries; optimization is not permission to weaken environment independence or omit the O0 leak oracle.

A test repair for malformed fixture syntax/declaration order must be recorded separately from a compiler defect. Never rewrite a semantic positive as a smaller passing program to claim its original behavior passed.

## 13. Verification and evidence

### 13.1 Serialized verification

Before running tests, establish whether an existing suite is active and recover its exact result where possible. Do not cancel an unknown actor's run or start a concurrent suite. Builds do not replace tests.

Use the smallest relevant registered tests first, then the complete suites. The following commands illustrate the required gates; run them serially and retain terminal output:

```bash
cargo test -p rock-lib --test copy_environment_reclamation
cargo test -p rock-lib --test callable_result_transport
cargo test -p rock-lib
cargo test -p rock-lib --release
cargo build --release
cargo clippy --workspace --all-targets
```

Add exact fully qualified unit-test invocations for the new exit/agreement and role/join regressions. A filter that runs zero tests is not a pass. Integration positives that require O0/O3 must actually execute both configurations, including source-free cases.

Do not run formatters or formatting checks. Formatting is explicitly outside current acceptance, not silently waived after a failure.

### 13.2 Final-snapshot evidence

For each gate, record:

- Exact command, relevant environment and source snapshot identity, including relevant uncommitted/untracked compiler/test inputs without mutating VCS.
- Terminal exit status and every target's pass/fail/ignored counts.
- O0/O3 execution outcomes and source-free producer state.
- Saved output and content identity, not just a reusable job alias or command-derived filename.
- Actual failing diagnostics when a failure occurs.
- Frozen source/review/artifact/object guard comparisons.

Pending jobs, expired handles, stale alias expansions, historical receipts and passing subsets do not establish complete acceptance. A macro or other unrelated failure in a full gate must be recorded as a failed gate, not removed from its totals.

Any functional compiler/test change after a gate invalidates that gate's final-snapshot claim. Documentation-only evidence updates do not authorize changing the code snapshot underneath a running test.

### 13.3 Audit record

Record implementation and verification separately in `MEMORY.md` and the relevant specification history; leave the initial review untouched.

The audit record must explain the shared agreement owner, normal/return exit handling, declaration-versus-storage boundary, producer/loan joins, source-free admission and any remaining unsupported combinations. Each acceptance family must have concrete test evidence; a code inspection alone is not a runtime pass.

Use `bd` for implementation issue tracking when available, not this document as a progress checklist. If it remains unavailable, report that fact rather than inventing another tracker. Link an actively worked issue/PR to the session as soon as its URL is known; no such URL is established by writing this specification.

## 14. Completion boundary

This scoped work is complete only when:

1. Return agreement is shared, target-sensitive, atomic and independent of lexical tail/candidate order, with no ownership-erasing fallback.
2. Every required return/storage and callable/loan acceptance family has appropriate permanent positives, negatives and corruption controls.
3. Compatible existing compositions, strict protocol/access restrictions, original HKT controls and unchanged proof caps pass without annotations or source workarounds.
4. Native/erased result storage, dictionary/descriptor identity, source-free publication/loading and cleanup agree on the checked producer facts.
5. Complete debug/release library gates and refreshed release-build/Clippy gates pass on the final functional snapshot; warnings and ignores remain explicitly reported.
6. Frozen guards pass, exact terminal evidence is retained, and the implementation/verification records do not overstate coverage.

Formatting is not part of that boundary. The historical macro-flake investigation and any other outstanding consolidation obligations remain separately visible; completing this scoped specification does not automatically declare the entire dynamic-dispatch feature complete.
