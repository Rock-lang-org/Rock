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

Run manually with:

```bash
cargo test -p rock-lib --lib macro_acceptance_derive_struct -- --ignored --nocapture
```
