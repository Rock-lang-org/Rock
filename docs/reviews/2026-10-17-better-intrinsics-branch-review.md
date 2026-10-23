# Review: `better_intrinsics` relative to `develop`

Date: 2026-10-17

Status: review snapshot; findings reproduced, fixes not implemented by this review.

Follow-up: [implementation and verification of R1–R7](2026-10-17-better-intrinsics-implementation.md).

## Scope and verdict

**Do not merge yet: seven reproduced issues, including two borrow-tracking holes.**

The review covered the six commits and 104 changed files in `develop...HEAD`, including compiler changes, std, artifact handling, tooling, documentation, and tests. It examined correctness, borrowing, incomplete implementation paths, consistency, code quality, and refactoring opportunities.

| Reference | Reviewed value |
| --- | --- |
| Branch | `better_intrinsics` |
| HEAD | `e4af46b31cc738f2149977fda3021f18b942ffab` |
| `develop` | `ff6262fdea5de69028f0fe7b0d213cd8e5257578` |
| Merge base | `ff6262fdea5de69028f0fe7b0d213cd8e5257578` |

Source line numbers below refer to this snapshot. Finding IDs R1–R7 are stable references for a subsequent specification, not a separate issue-tracking system. Implementation work should use the repository's issue tracker.

The borrowing findings concern Rock's stated requirement to preserve storage origins and aliasing restrictions across unsafe raw-reference formation. They are not a claim that unsafe code generally removes the caller's memory-validity obligations. Not every limitation in the underlying borrow machinery was established to originate on this branch; the new raw-wide-pointer paths and the branch's promised borrowing contract remain incomplete.

## Findings at a glance

| ID | Priority | Finding |
| --- | --- | --- |
| R1 | P1 | Borrowing through a pointer stored in a field loses the storage loan |
| R2 | P1 | Mixed reference/raw-pointer arguments discard required loans |
| R3 | P2 | Reborrow authorization uses the first raw-pointer argument regardless of origin |
| R4 | P2 | Recursive declarations cannot use their own generic defaults |
| R5 | P2 | Default resolution remains declaration-order-dependent inside file modules |
| R6 | P2 | Ordinary static constructors do not receive generic-default fallback |
| R7 | P2 | Constant-arithmetic diagnostics disappear across artifact boundaries for simple wrappers |

## R1 — Pointer fields lose the storage loan

**Priority:** P1

**Location:** `lib/src/mir/borrowck/borrows.rs:84–89`

The new recovery logic requires the local itself to be a pointer and its first projection to be `Deref`. A field access such as `&*holder.ptr` satisfies neither condition.

### Reproducer

Compile with the rebuilt std artifact:

```rock
struct Holder
    < ptr: *[I64]

main = ->
    mut values = [1, 2]
    raw: *[I64] = ~PtrFromParts (~ArrPtr (&mut values)), 2
    holder = Holder
        ptr: raw
    view = unsafe &*holder.ptr
    values[0] = 9
    view[0]
```

**Observed:** compilation succeeds despite mutation of the array while the shared slice reference remains in use. Keeping the pointer in a plain local and borrowing with `unsafe &*raw` correctly rejects the mutation.

**Impact:** the new wide-pointer path violates its stated alias-tracking contract when the pointer is stored in an aggregate.

**Fix direction:** resolve pointer types and origins through projected places rather than testing only the root local.

Evidence: `.agent-work/branch-review-20261017/raw_wide_field.rk` and `raw_wide_local_control.rk`.

## R2 — Mixed reference/raw-pointer arguments discard required loans

**Priority:** P1

**Location:** `lib/src/mir/borrowck/borrows.rs:121–128`

Raw-origin recovery runs only when `call_borrows.is_empty()`. If a function can return either a reference argument or a reference formed from a raw-pointer argument, collecting the first loan suppresses recovery of the second.

### Reproducer

This probe requires no std:

```rock
unsafe choose: &I64 -> *I64 -> Bool -> &I64
unsafe choose = first, second, flag ->
    if flag
        first
    else
        unsafe &*second

main = ->
    mut a = 1
    mut b = 2
    data = &mut b as *I64
    view = unsafe choose &a, data, false
    b = 9
    *view
```

**Observed:** assigning to `b` while using the returned reference is accepted. The equivalent single-pointer function correctly rejects that assignment.

**Impact:** a reference-returning call can lose one of its required storage loans merely because another argument already contributed a loan.

**Fix direction:** combine all return-origin loans, preserving their respective source arguments. Raw-origin recovery cannot be a fallback for an empty result.

Evidence: `.agent-work/branch-review-20261017/raw_alias.rk` and `raw_alias_control.rk`.

## R3 — Reborrow authorization uses the first pointer argument

**Priority:** P2

**Location:** `lib/src/mir/borrowck/mod.rs:1683–1690`

The conflict checker uses `args.iter().find_map(...)` to select the first pointer and applies its reborrow authority to every recovered loan.

### Reproducer

```rock
unsafe second: *I64 -> *I64 -> &mut I64
unsafe second = first, second -> unsafe &mut *second

main = ->
    mut a = 1
    mut b = 2
    first_ptr = &mut a as *I64
    second_ptr = &mut b as *I64
    view = unsafe second first_ptr, second_ptr
    *view = 9
    *view
```

**Observed:** the call is rejected with “borrow conflict.” Changing the function to return `&mut *first` makes the otherwise equivalent program compile.

**Impact:** valid reborrows depend on argument position rather than the actual return origin.

**Fix direction:** retain the originating argument/place alongside each recovered loan instead of reconstructing it by scanning for an arbitrary pointer.

Evidence: `.agent-work/branch-review-20261017/raw_two_params.rk` and `raw_two_params_control.rk`.

## R4 — Recursive declarations cannot use their own defaults

**Priority:** P2

**Location:** `lib/src/collect/headers.rs:501–509`

Defaults are resolved into a local `generic_params` vector but are not published to the collection context before field types are lowered.

### Reproducer

This probe requires no std:

```rock
struct Node {T = I64}
    value: T
    next: *Node

main = !-> return
```

**Observed:** this valid declaration reaches an internal MIR/codegen agreement error, with `invalid_type_ids: 1`, even with no instances constructed. Writing `next: *(Node I64)` succeeds.

**Impact:** recursive nominal types cannot use the advertised omitted-argument syntax, and an unresolved declaration reaches a backend contract failure instead of being handled by the frontend.

**Fix direction:** make completed parameter descriptors available before resolving fields, including recursive references, and reject noncanonical types before the backend.

Evidence: `.agent-work/branch-review-20261017/self_default.rk` and `self_explicit.rk`.

## R5 — File modules remain declaration-order-dependent

**Priority:** P2

**Locations:** `lib/src/collect/collector.rs:17` and `lib/src/collect/collector.rs:1313`

The new `ordered_headers` traversal is used in several collection paths, but `collect_source_backed_qualified_declarations` still processes declarations in source order.

### Reproducer

`main.rk`:

```rock
mod models

main = !-> return
```

`models.rk`:

```rock
read: Outer -> I64
< read = value -> value.value.value

< struct Outer {T = Inner}
    < value: T

< struct Inner {T = I64}
    < value: T
```

**Observed:** compilation fails with:

```text
default for 'T' has kind Type -> Type, expected Type
```

The corresponding declarations at the root compile successfully.

**Impact:** moving valid declarations into a file module changes default-type resolution.

**Fix direction:** use one declaration/default-resolution pipeline for root, inline, and file-backed modules.

Evidence: `.agent-work/branch-review-20261017/modules/main.rk`, `modules/models.rk`, and `top_level.rk`.

## R6 — Static constructors do not receive default fallback

**Priority:** P2

**Locations:** `lib/src/lower/paths.rs:693–704` and `lib/src/lower/paths.rs:983–989`

Default registration was added to `instantiate_selected_constructor_member`, but the ordinary static-method instantiation path does not perform equivalent registration.

### Reproducer

```rock
struct Phantom {T = I64}

impl Phantom T
    new: () -> Phantom T
    new = -> Phantom

main = !->
    value = Phantom::new!
    ()
```

**Observed:** this reports multiple ambiguous-type diagnostics. Adding `value: Phantom` succeeds.

**Impact:** an unconstrained construction through an ordinary static factory cannot use its owner's declared fallback.

**Fix direction:** associate defaults with instantiated owner arguments centrally, so construction and static factories share the same fallback behavior.

Evidence: `.agent-work/branch-review-20261017/static_defaults.rk` and `static_default_control.rk`.

## R7 — Scalar wrappers lose constant diagnostics across artifacts

**Priority:** P2

**Locations:** `lib/src/infer/constants.rs:194–224` and `lib/src/infer/constants.rs:386–409`

The local evaluator follows selected method calls, but the artifact-summary builder cannot summarize them. A pure scalar method that delegates to another scalar method consequently loses its arithmetic summary.

### Local reproducer

This probe requires no std:

```rock
trait Divide
    @divide: I8 -> I8
    @wrapped: I8 -> I8

impl Divide for I8
    @divide = other -> ~IntCheckedDiv *self, other
    @wrapped = other -> self.divide other

main = !->
    (1 as I8).wrapped (0 as I8)
```

**Observed:** the local call produces the expected constant division-by-zero diagnostic.

### Artifact reproducer

Compile the following dependency as crate `arithmetic`, emitting an object and artifact:

```rock
< trait Divide
    @divide: I8 -> I8
    @wrapped: I8 -> I8

impl Divide for I8
    @divide = other -> ~IntCheckedDiv *self, other
    @wrapped = other -> self.divide other
```

Remove its original source path, then compile this consumer against the artifact:

```rock
> arithmetic::Divide

main = !->
    (1 as I8).wrapped (0 as I8)
```

**Observed:** the consumer compiles successfully. The shipped `(1 as I8).wrapping_div (0 as I8)` also escapes the diagnostic when using the rebuilt std artifact, whereas ordinary `(1 as I8) / (0 as I8)` is diagnosed.

**Impact:** compile-time diagnostics depend on whether an implementation is local or imported. Runtime trapping remains implemented; the missing part is the promised source-free constant diagnostic.

**Fix direction:** compose bounded summaries through selected scalar calls and use the same supported-expression model for local and imported analysis.

Evidence: `.agent-work/branch-review-20261017/nested_arithmetic/`, `constant_wrapping_zero.rk`, and `constant_direct_zero.rk`. The dependency's original `library.rk` was moved to `library.source` after artifact generation to remove the source path used during compilation.

## Code quality and refactoring assessment

The main concern is special-case accumulation around existing compiler paths:

- **Borrow checking:** recovery, loan construction, liveness transfer, and authorization make separate assumptions about pointer locals and argument positions. R1–R3 call for one provenance-driven representation shared across these steps.
- **Generic defaults:** name-based header sorting, selective constructor registration, and additional backend normalization distribute responsibility across phases. R4–R6 show why default resolution needs a coherent declaration/instantiation design.
- **Constant analysis:** the HIR evaluator and serialized-summary builder support different expression sets. Their divergence already causes observable behavior differences in R7.

The branch also contains worthwhile refactors and improvements:

- Reusing the common dataflow worklist instead of retaining a separate active-loan sweep.
- Reducing repeated implementation-registry searches during selection.
- Centralizing exceptional integer code generation, including division and shift handling.
- Isolating environment-mutating tests in child processes.

The findings support targeted architectural repairs rather than treating the entire branch as over-engineered. The proposed directions above are review recommendations; a later specification should define exact invariants and acceptance requirements before implementation.

## Verification and evidence

The existing release checks all passed at the reviewed snapshot:

| Command | Result |
| --- | --- |
| `cargo build -p rockc --release` | Passed |
| `cargo test -p rock-lib --release` | 2,373 unit tests and 769 integration tests passed; one doctest ignored |
| `cargo test -p rock --release` | 72 tests passed |
| `cargo test -p rockc --release` | 48 tests passed |
| `cargo clippy --workspace --all-targets --release` | Passed with warnings |

The library integration count consists of 768 main integration tests and one parser integration test. Passing existing suites does not cover the seven reproduced failures above. Clippy success is not a warning-free result.

Logs are retained locally under `.agent-work/branch-review-20261017/`:

- `rock-lib-tests.log`
- `rock-tests.log`
- `rockc-tests.log`
- `clippy.log`

The same directory contains the individual probes, control programs, and rebuilt artifacts. Its local files are supplementary evidence; the source reproducers are embedded above so this review remains useful independently of scratch-directory retention.

### Compiler-contributor reproduction workflow

These probes use `rockc` directly because they exercise compiler diagnostics and explicit artifact boundaries. Build the compiler, then compile a no-std probe with:

```sh
cargo build -p rockc --release
target/release/rockc --entry-file path/to/probe.rk --output-dir path/to/output --no-std --no-prelude --no-link
```

For probes using std, first build a current-format artifact:

```sh
target/release/rockc --entry-file std/src/lib.rk --crate-name std --no-std --no-prelude --no-link --emit-artifact .agent-work/branch-review-20261017/std.rkca --emit-object .agent-work/branch-review-20261017/std.o --output-dir .agent-work/branch-review-20261017/std-build
```

Then compile with that explicit dependency:

```sh
target/release/rockc --entry-file path/to/probe.rk --output-dir path/to/output --extern-artifact std=.agent-work/branch-review-20261017/std.rkca --no-link
```

The original review modified no tracked source files and performed no VCS mutations. This document preserves the review for future specification work; it does not record completed fixes or constitute an implementation specification.
