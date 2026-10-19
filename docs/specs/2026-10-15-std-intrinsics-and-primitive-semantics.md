# Standard-library intrinsics and primitive semantics

Date: 2026-10-15

Status: implemented and verified, 2026-10-15.

## Purpose and baseline

Make the boundary between Rock language mechanisms and standard-library policy
explicit, complete meaningful primitive operations, and adopt Rust's comparison
and integer-overflow model. This compiler-contributor specification records the
implemented boundary and its validation requirements.

The initial inventory of `std/src/**/*.rk` found 121 direct intrinsic calls,
113 distinct names, and 14 calling files. Consuming-method syntax such as
`~@drop`, libc externs, and indirect calls through std were excluded.

The inventory comprises 100 primitive-operator calls, 17 additional
array/string/pointer/memory calls, and four atomic calls. The pointer-subtraction
call reuses `I64Sub`, so those groups contain 113 distinct names in total.

Compiler logic remains in `lib/`; operator definitions and user-facing
abstractions remain in std. Std is an explicitly supplied external dependency,
not a compiler-owned sysroot or source of hardcoded operator fallbacks.

## Raw pointer construction and borrowing

Replace `BorrowSlice`, `BorrowSliceMut`, and `BorrowStr` with the bounded raw
wide-pointer mechanism `PtrFromParts`.

- An element pointer and element count construct `*[T]`.
- A byte pointer and byte count can construct `*Str` when the expected target
  type selects a string pointer.
- Type checking must reject incompatible data-pointer/pointee combinations and
  ambiguous result types; it must not silently default to an arbitrary type.
- Initially support only length metadata for slices and strings, not general
  object/vtable metadata or arbitrary representation reinterpretation.
- Constructing a raw pointer does not establish reference validity, a loan,
  exclusive access, initialized elements, allocation ownership, or valid UTF-8.

Use ordinary unsafe raw-pointer borrowing (`&*` and `&mut *`) to establish shared
and exclusive references. Expose named unsafe slice constructors in std, and
checked UTF-8 conversion plus an explicitly unsafe unchecked string conversion.
The constructor and wrappers must retain source origins; they must not invent
lifetimes or erase existing aliasing restrictions.

Safety contracts cover nonnegative lengths, representable byte extents,
alignment, initialization, allocation extent, non-null reference requirements,
exclusive access for mutable references, and valid UTF-8 for string references.
Raw pointers may describe invalid storage; creating or using references cannot.
Runtime scalar checks cannot prove the full memory-validity contract.

The std entry points are `slice_from_raw_parts`, `slice_from_raw_parts_mut`,
`str_from_utf8`, and `str_from_utf8_unchecked`. The slice constructors are unsafe
and reject negative lengths, null pointers, and byte extents above `I64::MAX`.
Alignment, allocation validity, initialization, and exclusivity remain caller
obligations. The checked UTF-8 conversion returns `Option &Str`; the unchecked
conversion is unsafe and preserves the byte slice's origin.

Existing special handling in inference, MIR construction, borrow collection,
return-origin propagation, and LLVM lowering must be replaced consistently.
Establish positive and negative borrow regressions before removing existing
paths. In particular, the initial unsafe-intrinsic classification omitted
`BorrowSliceMut`; both the interim implementation and its replacement must
require unsafe borrowing.

## Pointer subtraction

Implement pointer-minus-offset through the existing pointer offset operation
and explicitly wrapping integer negation. Remove the exceptional spelling
`I64Sub 0, offset` from std pointer subtraction. Do not add a pointer-subtraction
intrinsic or depend on profile-sensitive ordinary unary negation.

`I64::MIN` has no positive mathematical counterpart in `I64`; its wrapping
negation is itself. This supports the current modular address calculation on
64-bit targets, not permission to dereference the resulting address. Preserve
existing pointer behavior during this cleanup; distinguish in-bounds and
wrapping pointer contracts before introducing stronger LLVM assumptions.
Do not add `inbounds` or no-overflow flags without the required proof.

## Atomics

Retain LLVM atomic instructions and the four existing U64 operations:
exchange, fetch-add, fetch-subtract, and store. AtomicU64 remains a std-owned
abstraction over appropriately aligned storage, not an LLVM-owned language type.
Retain sequential consistency and document that policy.

C `_Atomic` and C atomic operations are compiler/language facilities, not a
portable set of ordinary libc types and functions. A C shim or libatomic ABI
would add maintenance and dependencies without improving this design.
Additional atomic widths, explicit memory orderings, and lock-free guarantees
are outside this change; target lowering must not assume every atomic is
lock-free on every architecture.

## Remove MakeArr

Delete the unused `MakeArr` intrinsic completely: source recognition, inference,
unsafe classification, MIR identity and string mappings, classification, LLVM
generation, and any associated references found during implementation.
It must not remain as a compatibility alias. Add a source-rejection regression.
This removes an obsolete bare-slice construction path, not normal array/slice
support.

## One trait per bitwise operation

Replace the combined Bitwise trait with BitAnd, BitOr, BitXor, Shl, and Shr.
Keep the existing Not trait. Match the dedicated arithmetic traits' operand
parameters and associated Output model.

All eight integer types implement all six applicable operations. Bool implements
BitAnd, BitOr, BitXor, and Not, but never shifts. Unsupported operations fail
during compilation, not through `unimplemented()` or panicking placeholder
implementations. Remove the old trait rather than preserving an alias.

## Partial and total comparisons

Introduce Ordering with Less, Equal, and Greater variants and these protocols:

| Trait | Contract |
| --- | --- |
| PartialEq | Owns `==` and default `!=` as the negation of equality. |
| Eq | Extends PartialEq with the reflexive-equality guarantee; no new methods. |
| PartialOrd | Extends PartialEq; partial_cmp returns Option Ordering and owns comparison operators. |
| Ord | Extends Eq and PartialOrd; cmp returns Ordering and establishes a total order. |

All integer types, Bool, Char, unit, and borrowed strings receive meaningful
equality and ordering implementations. Bool orders false before true; Char
orders by Unicode scalar value; strings order lexicographically; unit has one
value. Generic containers/algorithms request only the guarantees they require;
hash-map keys retain Eq, while ordinary comparisons use the partial traits.

F32 and F64 implement only PartialEq and PartialOrd. A partial comparison with
NaN returns None. Equality is ordered equality; inequality is its logical
negation, including for NaN. Direct float Ne intrinsics must use LLVM UNE rather
than ONE so direct intrinsic calls and std's default inequality agree.

Keep IEEE-754 NaNs, infinities, and signed zeros. Provide std is_nan conveniences.
A separate total_cmp-style float ordering can be added independently; do not
claim that ordinary float comparison is a total order.

## Meaningful primitive coverage

Provide +, -, *, /, and % for I8/I16/I32/I64, U8/U16/U32/U64, and F32/F64.
Provide unary negation for signed integers and floats, not unsigned integers.
Provide integer ordering at every width and signedness, including unsigned
LLVM comparison/division semantics. Integer bitwise coverage must preserve
width and signed-versus-unsigned right-shift behavior.

Do not invent arithmetic for Bool, Char, unit, or strings. Existing string
concatenation remains a std abstraction. Do not hide inference regressions by
adding local type annotations to existing signatureless programs.

## Explicit integer arithmetic families

Expose Rust-style checked_*, wrapping_*, saturating_*, and overflowing_* methods
for the applicable integer operations. Their behavior is independent of build
profile. Addition examples for U8 operands 255 and 1:

| Method | Result |
| --- | --- |
| checked_add | Option::None |
| wrapping_add | 0 |
| saturating_add | 255 |
| overflowing_add | (0, true) |

Apply corresponding contracts to subtraction/multiplication and applicable
negation, division, remainder, and shifts. Do not assume every family has the
same meaning for every operation: division by zero, signed MIN / -1, MIN % -1,
and out-of-range shift counts require explicit Rust-compatible treatment.

For signed minimum negation, checked_neg returns None, wrapping_neg returns the
minimum, saturating_neg returns the maximum, and overflowing_neg returns the
minimum with overflow=true. A separate integer abs audit must not accidentally
assume MIN has a representable positive value.

The existing free `abs` utility retains its historical wrapping-at-MIN contract
through explicit `wrapping_neg`; it is not a checked absolute-value API.

Use LLVM typed overflow operations where needed. Keep option construction,
saturation policy, and ordinary operator implementations in std where practical.
Do not expose LLVM poison or undefined behavior as safe Rock arithmetic.

`Integer` exposes checked, wrapping, and overflowing add/sub/mul/div/rem/neg/shl/shr,
plus saturating add/sub/mul/div/neg. Explicit shift methods accept `U32` counts;
out-of-range counts wrap modulo the bit width in wrapping/overflowing methods,
and the overflowing flag reports a count error, not discarded value bits.
There is no saturating remainder or saturating shift family.

Checked division and remainder return `None` for zero divisors and signed
`MIN / -1` or `MIN % -1`. Wrapping/overflowing division still fails on zero;
the signed exceptional pair returns `MIN` or zero respectively, with the
overflowing flag set. Saturating division returns the signed maximum for
`MIN / -1`, but still fails on zero. Integer-to-string conversion keeps signed
magnitudes negative internally so formatting `I64::MIN` never negates it.

## Ordinary integer operators and build policy

Adopt Rust-style overflow-check defaults: development builds check overflow and
fail deterministically; release builds wrap for ordinary addition, subtraction,
multiplication, and negation. Expose an explicit overflow-check option separately
from optimization level. Integer division by zero and signed MIN / -1 still
fail when overflow checks are disabled; cover remainder and shift exceptions
explicitly rather than applying one blanket rule.

Use the compiler's runtime failure mechanism (or establish a minimal consistent
one if absent); this specification does not require Rust-style stack unwinding.
Compile-time erroneous constant arithmetic receives structured diagnostics.

Constant checking follows resolved scalar method bodies, not operator spellings.
Literal/cast operands and nested constant applications, including
`(127 as I8) + (1 as I8)`, receive source-span-aware type diagnostics when the
selected arithmetic traps. Ordinary local variables and function calls remain
runtime values; this bounded analysis does not execute arbitrary programs.
Concrete dependency methods retain validated, identity-free arithmetic summaries
for source-free diagnostics, including producer-owned policy branches and
discarded intermediate computations. Summaries are diagnostic metadata, not
permission to recompile or reinterpret a dependency's native implementation.

Each dependency retains its compiled arithmetic policy across artifact
boundaries. Optimizing or linking a consumer must not reinterpret an already
compiled dependency's operations. Explicit wrapping operations used inside
pointer arithmetic must never become profile-dependent checks.

The compiler configuration uses `overflow_checks: Option<bool>` and `rockc`
accepts `--overflow-checks true|false`, separately from `--debug`/`--release`.
Std queries `~OverflowChecks!`, which lowers to a producer-owned Boolean literal
before serialization; the consumer cannot re-select that policy. This applies
to std as an ordinary explicitly supplied dependency too: changing only the
application profile does not rebuild or reinterpret a supplied std artifact.
Arithmetic failure uses LLVM's trap/unreachable path, with no unwinding.

## Artifacts, migration, and validation

Intrinsic-identity and serialized policy changes require the current artifact
schema to be updated as needed and dependencies rebuilt. Use forward-only
artifacts, no legacy readers or transitional intrinsic/trait aliases. Preserve
semantic fixtures and negative controls; do not edit frozen observer sources
merely to accommodate an implementation regression.

Verify each stage with focused compiler and end-to-end tests, then serialized
broader suites. Coverage includes all numeric widths, unsigned comparisons and
division, signed shifts, NaN/infinity/signed zero, Boolean operator rejection,
partial/total trait bounds, pointer boundaries and zero-sized types, borrow
escape/alias negatives, and debug/release/explicit-overflow policy combinations.
Exercise rebuilt and source-free dependency artifacts where contracts cross
crate boundaries.

The current artifact format is 49 in both compiler and shared tooling. Verification
covers all eight integer widths at generated O0/O3, runtime arithmetic failures,
constant-operand diagnostics, development/release/explicit check selection, and
deleted-source dependencies with opposite consumer policies. A separately rebuilt
release std artifact also retains wrapping operators in checked consumers.

The linked release test suite passes 2,361 unit tests, 756 main integration tests,
and the parser integration test; the compiler CLI overflow-option regression,
workspace/all-targets Clippy, and release workspace build pass as well. Clippy
reports warnings; formatting is not a completion gate.

The original constant-overflow reproducer now receives a type diagnostic
highlighting the arithmetic expression, even in an unchecked release consumer
using checked std. Rebuilt format-49 development and release std artifacts retain
their respective checked/wrapping policies; the wrapping consumer prints `false`,
and the native arithmetic/NaN/raw-pointer control prints `0`, `checked`, `true`,
and `20`.

Formatting work remains paused. Do not commit, stage, push, or modify unrelated
working files as part of implementation unless separately requested.
