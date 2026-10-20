# Rock macro SDK

`rock_macro` is an ordinary Rock library, supplied explicitly like any other
dependency. Its current delivery contains owned data types for protocol 7:
lossless source snapshots/segments, contextual handles, optional input views,
output fragments, and structured results/diagnostics.

- Input is always `TokenStream`; there is no raw-string or compiler-AST variant.
- Ranges are UTF-8 byte offsets, not character indices; `U64` transport values
  are checked against host limits by the staged codecs.
- Snapshot indices preserve aliases and explicit splice boundaries. Diagnostic
  projections do not grant adjacency or binding authority.
- Context/identity values are symbolic handles revalidated by the compiler.
  Public struct construction cannot grant private exports or forge captures.
- `MacroResult` is the authoring result: `Expand` forwards a stream, `Error`
  reports failure, and `Report` permits structured diagnostics alongside output.
  All forms normalize to the same optional-output/diagnostics wire contract.

`std` is declared as an ordinary path dependency. Direct compiler contributors
must build and pass both dependencies as artifacts; the compiler never discovers
or injects this SDK by name. Host-role package planning is separate staged work.

This is not yet a runnable procedural macro package workflow. Local operations,
binary IO/codecs, generated runners and source export discovery are the following
implementation steps. The Rust wire specification is
[`docs/specs/2026-10-19-macro-worker-wire.md`](../docs/specs/2026-10-19-macro-worker-wire.md).
