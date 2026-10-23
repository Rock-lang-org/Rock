# Second review: `better_intrinsics` relative to `develop`

Date: 2026-10-17

Follow-up: [implementation and verification of R8–R10](2026-10-17-better-intrinsics-second-review-implementation.md). The findings below preserve the reviewed baseline.

**Verdict: do not merge yet. Three reproduced defects remain, including two loan-tracking holes.**

## Scope

- Reviewed the seven commits and 105 changed files in `develop...HEAD`, including the R1–R7 repairs, compiler phases, std, artifact interfaces, CLI changes, tests, and documentation.
- HEAD: `c3eaddb137d84c4bd09c9cd4b7c9e4c68450382a`.
- `develop`: `ff6262fdea5de69028f0fe7b0d213cd8e5257578`.
- Pre-repair branch snapshot: `e4af46b31cc738f2149977fda3021f18b942ffab`.
- Previous records: [initial review](2026-10-17-better-intrinsics-branch-review.md) and [R1–R7 implementation](2026-10-17-better-intrinsics-implementation.md).

R8–R10 continue the historical review references; they are not a separate issue tracker. Source line numbers refer to HEAD above. No compiler or permanent test files were changed during this review.

## Findings

| Reference | Priority | Finding | Attribution |
| --- | --- | --- | --- |
| R8 | P1 | Pointer-containing arguments can return references without caller-side storage loans | Newly accepted by `c3eaddb`; both baselines reject |
| R9 | P1 | Reference-containing aggregate results lose raw-input loans | `c3eaddb` regresses the pre-repair branch; `develop` also has the hole |
| R10 | P2 | Mutually recursive nominal fields cannot use omitted generic defaults | Remaining incomplete path in the branch's new defaults feature |

The borrowing reproducers use explicit unsafe reference formation. They demonstrate failures of the branch's storage-origin/loan-preservation contract, rather than safe-source-only memory-safety exploits. The compiler accepts a later mutation while the derived reference is still used.

### R8 — Preserve raw origins through aggregate arguments

**Locations:** `lib/src/mir/borrowck/mod.rs:200–207,316–319`; `lib/src/mir/borrowck/borrows.rs:174–193,212–219`.

The repair permits references derived from a pointer field of an aggregate parameter to escape with that parameter's origin. At the caller, however, `collect_terminator_borrows_with_context` only recovers raw storage when the selected argument itself is pointer-typed. Its reference fallback also ignores a by-value `Holder`. The new accepted callee therefore has no corresponding caller-side loan.

```rock
struct Holder
    < ptr: *I64

unsafe borrow: Holder -> &I64
unsafe borrow = holder -> unsafe &*holder.ptr

main = ->
    mut value = 1
    holder = Holder
        ptr: &mut value as *I64
    view = unsafe borrow holder
    value = 9
    *view
```

**Observed:** HEAD accepts. Both `develop` and the freshly rebuilt `e4af46b` reject the callee as returning a reference to a local value. The direct-pointer equivalent at HEAD correctly rejects the mutation with a borrow conflict.

**Required correction:** when allowing aggregate parameters to supply return origins, preserve the corresponding pointed-to storage loans at calls, including the appropriate projected source authority. Merely changing escape classification is insufficient.

**Evidence:** `.agent-work/branch-rereview-20261017/raw_struct_argument.rk`, `raw_struct_argument.log`, `develop-raw_struct_argument.log`, `e4af46b-fresh-raw_struct_argument.log`, and `raw_direct_control.log`.

### R9 — Preserve loans for aggregate return values

**Locations:** `lib/src/mir/borrowck/liveness.rs:332–338`; `lib/src/mir/borrowck/borrows.rs:169–172`.

The source-specific authority repair stops generic loan forwarding from raw-pointer arguments. Its replacement loan recovery only runs when the destination is a top-level `Ty::Reference`; a tuple containing a reference returns before recovering any loan. Once the raw input dies, the returned tuple no longer keeps the pointed-to storage borrowed.

```rock
unsafe borrow: *I64 -> (&I64, I64)
unsafe borrow = ptr -> (unsafe &*ptr, 0)

main = ->
    mut value = 1
    ptr = &mut value as *I64
    pair = unsafe borrow ptr
    value = 9
    *pair.0
```

**Observed:** HEAD accepts. Fresh `e4af46b` rejects the mutation with a borrow conflict. `develop` accepts, so this is specifically a regression of the branch's earlier hardening, not a newly introduced behavior relative to `develop`. Returning `&I64` directly instead of a tuple correctly rejects at HEAD.

**Required correction:** preserve selected raw-input loans for reference-bearing result shapes as well as direct references, retaining source-specific authority rather than restoring unconditional first-argument forwarding.

**Evidence:** `.agent-work/branch-rereview-20261017/raw_tuple_return.rk`, `raw_tuple_return.log`, `develop-raw_tuple_return.log`, `e4af46b-fresh-raw_tuple_return.log`, and `raw_direct_control.log`.

### R10 — Publish defaults across mutually recursive declarations

**Locations:** `lib/src/collect/collector.rs:55–60`; `lib/src/collect/headers.rs:501–515` (analogous enum publication at `627–629`).

Dependency ordering deliberately permits declaration cycles, but defaults are only published while building each individual declaration. Within a mutually recursive group, the first member lowered still sees another member's predeclared descriptor without its defaults. An omitted nominal application remains an unapplied constructor in a field type and reaches MIR/backend validation.

```rock
struct First {T = I64}
    value: T
    other: *Second

struct Second {T = I64}
    value: T
    other: *First

main = !-> return
```

**Observed:** HEAD and fresh `e4af46b` fail with `MIR/codegen agreement failed`, reporting `invalid_type_ids: 1`. Replacing the two pointer types with `*(Second I64)` and `*(First I64)` succeeds. The accepted-HIR dump contains `Second.other: Pointer(Constructor(First))`, confirming incomplete default application before MIR. `develop` predates brace-default syntax and is not a semantic control for this feature.

This is a legal recursive layout through pointers; the defaults themselves are just `I64` and are not cyclic expansions.

**Required correction:** make completed generic-default descriptors available across recursive nominal declaration groups before lowering their fields; retain diagnostics for actual default-expansion cycles.

**Evidence:** `.agent-work/branch-rereview-20261017/mutual_defaults.rk`, `mutual_defaults.log`, `e4af46b-fresh-mutual_defaults.log`, `mutual_explicit_control.rk`, `mutual_explicit_control.log`, and `mutual-hir.log`.

## Other investigated behavior

- An aggregate containing both a raw pointer and an existing mutable reference to the same storage permits another mutable reference through the pointer field, while the equivalent unaggregated locals reject. This reproduces on `develop`, `e4af46b`, and HEAD (`raw_aggregate_authority.rk` and its control), so it is an additional pre-existing aggregate-authority limitation, not counted as a branch-introduced defect.
- A factory returning a lambda that constructs a generic wrapper fails when separate calls are used with `I64` and `Bool`. Removing the default produces the same error, including on `develop`; this probe does not establish a default-argument regression (`default_lambda.rk` and `default_lambda_control.rk`).
- No additional reproduced blocker was established in integer LLVM lowering, std checked/wrapping/overflowing/saturating arithmetic, equality/ordering changes, bounded selected-method arithmetic summaries, artifact remapping, or selection-search optimization. Passing existing coverage does not cover the aggregate and recursive-group gaps above.

## Verification

Fresh runs at reviewed HEAD:

| Command | Result |
| --- | --- |
| `cargo test -p rock-lib --release` | 2,373 unit tests, 776 main integration tests, one parser integration passed; one doctest ignored |
| `cargo test -p rock -p rockc --release` | 72 Rock CLI and 48 compiler CLI tests passed |
| `cargo build --release -p rockc` | Passed |

Total: **3,270 tests passed**, with no failures. Logs are `rock-lib-tests.log`, `cli-tests.log`, and `head-build.log` under `.agent-work/branch-rereview-20261017/`.

The earlier implementation's full workspace build and Clippy receipts remain historical; this review did not rerun those broader checks. Formatting remains deferred.

### Reproduce the findings and controls

The scratch runner invokes `rockc` with `--no-std --no-prelude --no-link` and separate output directories; no std artifact is needed.

```sh
python3 .agent-work/branch-rereview-20261017/run_probes.py raw_struct_argument raw_tuple_return raw_direct_control mutual_defaults mutual_explicit_control
```

Expected current results, in order: accept, accept, borrow rejection, internal error, accept.

Baseline source snapshots were extracted with `git archive`, without switching the checkout. A clean `develop` compiler was built in scratch and preserved as `rockc-develop`. The initial attempt to reuse its Cargo target directory for `e4af46b` incorrectly reused the old binary; those `e4af46b-*` probe logs are invalid attribution evidence. After cleaning the scratch target's `rock-lib`, `rockc`, and `rock-shared` packages and rebuilding, the authoritative logs are named **`e4af46b-fresh-*`**. The rebuild log is `e4af46b-build-fresh.log`.

## Review state

- Findings are reproduced and remain unfixed in this review snapshot.
- `bd ready --json` failed because `bd` is unavailable; no issue records were created.
- `gh pr view --json url,title,number,baseRefName` found no associated PR; no issue/PR URL was available to link to the session.
- Tracked source remains unchanged; scratch evidence and this report are untracked. No commit, merge, or push was performed during this review.
