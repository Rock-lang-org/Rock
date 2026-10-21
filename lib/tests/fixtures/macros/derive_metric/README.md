# External struct derive proof

`provider.rk` is an ordinary proc-library provider depending on `rock_macro` and
`rock_syntax`. `consumer.rk` defines the ordinary `Metric` trait; the compiler has
no knowledge of this trait or `SumMetric`.

`SumMetric` parses `DeriveInput`, copies target/field/generic syntax, strips
declaration-only generic defaults through `GenericSyntax.type_arguments`, and
uses `ImplBuilder` to emit an implementation. Scalar parameters receive a caller
`Metric` bound. A fresh generic helper with the same explicit bound dispatches
field measurements without accidentally selecting an unrelated same-named
trait. Reference arguments are grouped (`helper (&self.field)`) because Rock's
call/dot precedence would otherwise apply the field access to the call result.

The test intentionally collides provider/caller types, traits, functions and
fields, plus distinct root/nested `Metric` implementations. The output is
`0 / 18 / 18 / 11 / 19 / 30`, one number per line. Private fields are read only
inside their owner implementation; the caller uses an owner constructor.

The Rust driver checks retained field-default ASTs, defaulted generic arguments,
missing concrete/generic trait obligations, private construction/access, duplicate
and overlapping implementations, and the library's explicit unsupported-enum
diagnostic. Missing generic obligations are rejected during specialization, not
by metadata-only analysis. `UnboundedMetric` is a deliberate negative provider.

Field defaults are a syntax-retention control, not a runtime default-evaluation
claim: the executable supplies all fields explicitly. A macro-free program with
an omitted defaulted field independently reproduces an existing MIR panic.
General higher-kinded field derivation and production derive policy are outside
this fixture.

## Packaged enum proof

`EnumMetric` extends the same policy to nullary, positional and named variants.
Per-variant pattern construction copies declaration/variant/field names and uses
fresh payload bindings. `OutputBuffer` accumulates arms before `ImplBuilder`
emits the ordinary implementation. The multiline definition supplies its own
continuation margins within the builder's four-column member layout.

`enum-consumer.rk` uses defaulted generic arguments, public declarations/fields,
and a retained named-field default. The driver sends CRLF source with Unicode
trivia and checks the original stream after real worker transport. As with the
struct proof, runtime construction supplies every field explicitly.

`Cascade` emits a fresh enum with another `EnumMetric` attachment; `After` follows
it on the same original declaration. The driver checks expansion order and the
generated attachment's parent, then verifies `0 / 3 / 12 / 24 / 0 / 7` at runtime.
`Loop` deliberately emits another attachment to test the shared depth limit;
`WrongCategory` and `ordinary` provide item-category and kind negatives.

Before consuming the enum program, the test copies only the proc artifact and
its worker to a package, relocates that directory, and invalidates the original
host/syntax build locations. There is no provider source file, SDK dependency or
syntax-library dependency in the consumer configuration. Missing field impls,
overlapping enum impls, invalid targets/categories/kinds, canonical duplicates,
unbounded recursive attachments and unsupported attribute syntax all reject.

Run manually with:

```bash
cargo test -p rock-lib --lib macro_acceptance_derive_struct -- --ignored --nocapture
cargo test -p rock-lib --lib macro_acceptance_derive_enums_packaged -- --ignored --nocapture
```
