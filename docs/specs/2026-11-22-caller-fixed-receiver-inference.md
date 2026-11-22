# Caller-fixed receivers for undecided method calls

Date: 2026-11-22

Status: implemented 2026-11-22, except the dedicated conflicting-callers
diagnostic (see Diagnostics). Found while implementing std string methods
(issue #200).

## Behavior before this change (measured)

A method call whose receiver is still a type variable is not decided inside its
own function. Inference defers it (non-strict passes of `select_deferred_method`,
`infer/authority/deferred_method.rs`) and lets callers fix the receiver. Only
the final strict pass reports an error.

Probes compiled with `rockc --no-std`, with `area` defined on `Circle` and
`Square`:

| Program | Result |
|---|---|
| `f = x -> x.area!`, called once with a `Circle` | compiles; `x: Circle`, decided by the caller |
| same, `f` passed to `apply = g, v -> g v` | compiles; the receiver is fixed through the higher-order call |
| `f` called with a `Circle` and with a `Square` | `Ambiguous selection for 'area' on type _` |
| `f` never called, `area` only on `Circle` | compiles; the by-name guess silently picks `Circle` |

There are two by-name guesses, both through
`select_inferred_method_candidates` (`selection/service/concrete_methods.rs`):
the deferred-method fallback in `deferred_method.rs`, and the method-as-value
path in `lower/control_flow/secondary/dot.rs`. With both disabled, the whole
workspace test suite (`cargo test --workspace --release`) still passes. The
integration std cache is keyed by a compiler stamp, so the suite run after the
removal also rebuilt std without the guess.

The guess therefore only matters for functions no caller fixes, where it picks
an arbitrary but unique type. That is the non-modular part: adding a same-named
method elsewhere turns such a function into an error.

### The advent1 failure

```rock
process_file = content ->
    arr = content.split "\n"
    ...
    res

read_file = ->
    read_to_string "input"
        >>= process_file
```

`process_file` returns `Vec Turn`, but `>>=` on `Result` needs a function
returning a `Result`. `read_file` fails to type-check, so no caller fixes
`content`, and the strict pass reports `Ambiguous selection for 'split' on type
_` in `process_file`. With `read_file` corrected, `content` is fixed to `String`
and `split` resolves. The real bug was in `read_file`; the reported error was a
consequence.

## Decision

Keep caller-fixed receivers as the language rule, make it explicit, and remove
the by-name guess.

## Rules

1. **Trait path first.** Bound lookup and `select_shared_trait_member` keep
   running first. `show = x -> x.println!` generalizes to `x: Show`.
2. **Callers fix undecided receivers.** A receiver whose method is undecided
   when its function is inferred is fixed by the function's uses in the crate,
   as today.
3. **No by-name selection.** `select_inferred_method_candidates` no longer
   fixes a free receiver in either path. Adding an impl never changes whether
   unrelated code compiles.
4. **One receiver type per crate.** A function whose receiver is fixed this way
   accepts one receiver type per crate. Other type variables in the function
   stay generic.
5. **Unused functions.** Dead code is still type-checked
   (`docs/plans/unused-items-and-demand-driven-mono.md`): an unused function
   whose receiver nothing fixes is an error, like any other undetermined type.
6. **Exported functions.** A `pub` function whose receiver is still undecided
   after the crate is inferred is an error asking for an annotation. This holds
   by construction: nothing outside the crate can fix the receiver, so the
   rule 5 error fires. Undecided receivers never reach `.rkca` artifacts.
7. **Annotated functions are unaffected.**

## Diagnostics

Implemented:

- A receiver nothing fixed reports `cannot infer the receiver type of 'area': no
  caller fixes it; add a type annotation`, instead of `Ambiguous selection for
  'area' on type _`. Two callers with different receiver types currently get
  the same message.
- Undetermined-receiver errors (including the strict pass's "receiver remains
  unresolved" errors) are consequences. The strict pass holds them back, keeps
  checking the remaining components, and reports a definite failure found
  anywhere (strict pass or final solve) instead.
- A callable bound that no choice of its free variables can satisfy is a
  definite failure, not a deferred where clause
  (`callable_trait_impossible`, `infer/solve/impl_lookup.rs`). For advent1 this
  reports the real error:

  ```text
  closure `(_) -> Vec Turn` does not implement trait
  `FnMut String, (Result _, IoError)` (required by call to generic function '>>=')
  ```

  When the callback's return type itself depends on the undetermined call
  (`parse = content -> content.split "\n"`), nothing is definitely wrong yet,
  and the receiver error is reported.

Not implemented: a dedicated conflicting-callers message naming both call
sites. Caller propagation does not currently keep which call sites tried to fix
a receiver.

## Implementation

1. `deferred_method.rs`, `dot.rs`: the by-name branches are gone;
   `select_inferred_method_candidates` now only feeds
   `select_shared_trait_member` and the multi-candidate ambiguity check.
2. `lower/error.rs`: `ResolveError` carries an undetermined-receiver flag;
   `pending_authority_error` sets it.
3. `infer/quiescence.rs`: the strict pass and final solve hold undetermined
   errors back behind definite ones.
4. `infer/solve/worklist.rs` + `impl_lookup.rs`: impossible callable bounds fail
   immediately, for concrete and partly inferred callables.
5. Tests: `lib/tests/integration/receiver_inference.rs`.

## Open questions

- Conflicting-callers diagnostic: record the call sites caller propagation
  (`propagate_function_instances_for_owners_with_progress`) unifies into a
  receiver, so the error can name both.
- Chained undecided calls (`x.foo!.bar!`) and undecided receivers inside
  closures: add tests.

## Validation

- The probe programs above give the results in the Rules (two-impl and
  higher-order cases compile; never-fixed and two-type cases fail).
- advent1 with the original `read_file` reports the `>>=` mismatch, not
  `split`.
- Adding an unrelated impl with the same method name changes no outcome.
- Full workspace suite plus std built from a clean cache.
