# Rock macro SDK

`rock_macro` is an ordinary Rock library, supplied explicitly like any other
dependency. It contains owned data types for protocol 7:
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

## Local operations

- `TokenStream` implements explicit deep `Clone`, retaining snapshot identities,
  contexts, origins, projections and opaque captures.
- `len_bytes!`, `position offset`, `slice range`, `spelling!` and
  `join &right, boundary` return `Result ..., operations::SdkError`; invalid
  sources, byte ranges, UTF-8 cuts and capture cuts are structured failures.
- Joins remap/deduplicate source indices by retained identity, reject conflicting
  snapshots, and restore `SourceAdjacent` only for contiguous ranges of the same
  snapshot with a retained adjacent right boundary; otherwise use `Separate`.
- `cursor!` owns a copy; `peek!`/`consume!` expose a UTF-8 scalar or an entire
  opaque expression as the same `TokenStream`. `group view` reads a supplied
  group range and checks its delimiter endpoints, without parsing its contents.
- Create **one** `MacroBuilder` using `context.builder &input` per invocation.
  `fresh_ident spelling`, `call_site_ident spelling`,
  `dependency_path dependency_index, components` and `group delimiter, &body`
  build known lexical leaves/groups with symbolic construction requests.
  Clone a `construction::Identifier` to reuse its fresh identity; a second
  `fresh_ident` call gets a different handle. `input.forwarded_ident range`
  preserves existing context/provenance and rejects mixed or separated names.
- The small identifier/path helpers currently accept ASCII letters, digits and
  underscores (not a leading digit); use preserved stream slices for Unicode
  names or arbitrary DSL text. Compiler parsing remains authoritative.
- `context.error message`, `input.error range, message` and `Diagnostic::new`
  construct structured reports; `label`, `note` and `suggest` add details.
  Diagnostic locations never grant source or binding authority.

Local limits match lossless transport bounds (16 MiB retained/logical source,
1,048,576 sources/segments/captures and capture nesting 128). No operation reads
files, invokes a parser/compiler callback, evaluates captured expressions, or
materializes arbitrary deferred source/literal fragments. Dependency constructors
check the supplied index/spelling locally; only the compiler can validate public
export authority. Public owned data and symbolic requests cannot grant authority.

This is not yet a runnable procedural macro package workflow. Binary IO/codecs,
generated runners and source export discovery remain staged implementation work.
The Rust wire specification is
[`docs/specs/2026-10-19-macro-worker-wire.md`](../docs/specs/2026-10-19-macro-worker-wire.md).
