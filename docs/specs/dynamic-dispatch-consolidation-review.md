# Initial dynamic-dispatch consolidation review

This is the original review supplied by the user, retained as the consolidation baseline. Its line references, measurements, and test results describe the reviewed checkout, not the current implementation. Subsequent fixes and verification receipts are recorded separately in `MEMORY.md`; do not rewrite this review as those results change.

---

## Verdict

The foundations are sound, but I would not merge this branch yet—or keep expanding it without consolidating the ownership and callable systems.

This is not simply “AI-generated overengineering”: the specification deliberately includes runtime generics, HKT, ownership, and existentials, which genuinely require substantial compiler work. However, I found reproducible correctness defects, an unnecessarily exponential representation, and increasingly patch-driven handling of callable borrowing.

I compared against develop at 5cd2d81. The branch contains roughly 51,000 committed additions, plus 18,000 tracked uncommitted additions and further untracked modules; tests below exercised the current checkout, not a clean HEAD.

## Main findings

### 1. High: erased execution changes destructor timing

Locations: lib/src/mono/erased.rs:2321, lib/src/codegen/mir_llvm/erased.rs:1825.

• Ordinary MIR explicitly drops block-local values at scope exit.
• Erased lowering restores its binding map after a block but does not emit equivalent scope cleanup; LLVM instead drops remaining locals when the function returns.
• I reproduced this with a block-local object whose destructor increments a counter, followed by reading that counter.

| Call path | O0 result | O3 result |
| --- | --- | --- |
| Concrete receiver | 1 | 1 |
| Erased trait-object receiver | 0 | 0 |

Dispatch changes observable program semantics. Exactly-once destruction tests do not catch this; destruction must also occur at the correct time, especially for locks and resource guards.

This cleanup mechanism is already present in the committed branch.

### 2. High: erased MIR has a demonstrated borrow-validation hole

Location: lib/src/mir/erased.rs:3114.

• Dictionary calls check conflicting loans for mutable receivers, but lack the corresponding shared-receiver check against live mutable loans.
• The nearby opened-object call paths do perform that check, demonstrating inconsistent ownership rules between dispatch forms.
• A standalone MIR probe returned:

```text
shared_dictionary_while_mut_borrow_live=Ok(())
mutable_dictionary_control=Err(BorrowConflict)
```

The mutable reference was used after the shared call, so this was not merely an unused, expired borrow.

This proves a validator defect, not an end-to-end unsafe-source exploit: my source-level probe was rejected earlier for a missing layout.

### 3. High: existing HKT behavior is regressed

Unchanged integration tests now fail:

• test_hkt_standalone_and_receiver_operations
• test_hkt_receiver_failure_paths_and_constructor_sections
• test_hkt_foldable_and_traversable

The callable machinery increasingly depends on bounded recognition rules:

• lib/src/infer/generalize.rs:107 recognizes selected structural call trees and rewrites callable requirements.
• lib/src/mir/borrowck/callable_results/bindings/forwarding.rs:465 admits adapters only under restrictions including exactly one call.

These restrictions preserve conservative rejection, which is preferable to fabricating borrow authority, but they are not yet a general contract for the language’s existing callable/HKT abstractions.

The required refactor is definition-owned callable requirements and compositional borrow/effect transport—not another growing collection of accepted expression shapes.

### 4. Medium: object views are unnecessarily exponential

Locations: lib/src/mono/object_schema.rs:28, lib/src/mono/object.rs:439.

The implementation enumerates every guarantee subset, chooses each possible principal, and recursively materializes those views.

I measured a tiny program that performs one read and no upcasts:

| Guaranteed traits | Schemas | View edges | Object size |
| --- | --- | --- | --- |
| 1 | 1 | 0 | 4 KB |
| 3 | 12 | 51 | 7 KB |
| 5 | 80 | 1,405 | 57 KB |
| 7 | 448 | 25,067 | 852 KB |

This is concrete overengineering with a scalability cost. Separate compilation needs portable upcast evidence, but does not inherently require a precomputed power set of complete object schemas.

A factored per-trait view/evidence representation deserves redesign before this ABI becomes entrenched.

### 5. Architectural debt: critical semantics are being implemented twice

The erased path has its own:

• HIR-to-body lowering;
• initialization, move, origin, and loan analysis;
• CFG joins;
• destruction scheduling.

Distinct concrete and descriptor-backed layout representations are justified; duplicated ownership semantics are much harder to justify, and findings 1–2 show that they have already diverged.

Production functions are also becoming difficult to audit: erased expression lowering, erased validation, and LLVM body emission each contain roughly 900–1,100 lines.

The new C runtime additionally interprets type/schema and borrow-shape encodings; this is a maintenance concern rather than a demonstrated correctness bug, but every extension now risks requiring synchronized semantic changes across Rust and C.

## What is genuinely good

I would preserve these foundations:

• Shared sizedness and instantiated-supertrait queries replace duplicated logic.
• Canonical trait/member identities and producer-owned slot order support separate compilation correctly.
• Object operations are explicit in HIR/MIR rather than disguised as casts.
• Concrete MIR’s unresolved-type barrier remains strict.
• Owner protocols use canonical language-item identities rather than Box/Arc name exceptions.
• Tests include meaningful source-free artifact execution, unchanged producer objects, malformed metadata rejection, and O0/O3 coverage.

This is substantial useful work—not something I would discard wholesale.

## Verification

| Check | Result |
| --- | --- |
| Library unit tests | 2,908 passed; 4 failed; 9 ignored |
| Main integration suite | 715 passed; 3 failed |
| Six selected dispatch/artifact/ownership suites | 55 passed |
| cargo fmt --all --check | Failed |
| cargo clippy -p rock-lib --all-targets | Completed with warnings |
| Custom probes | Confirmed destructor mismatch, validator hole, and view explosion |

## Recommendation

Keep the foundations, but pause feature expansion. Fix scope cleanup and borrow validation through shared ownership machinery, restore the unchanged HKT tests, and redesign view representation before adding more runtime-generic cases.

Borrowed dispatch is progressing credibly; the complete feature is not yet on a sufficiently coherent implementation path. Passing narrowly scoped fixtures currently overstates how well the critical systems compose.

No repository source or VCS changes were made; probe artifacts are retained under .agent-work/.
