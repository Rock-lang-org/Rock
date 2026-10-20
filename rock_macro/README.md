# Rock macro SDK

`rock_macro` is an ordinary Rock library, supplied explicitly like any other
dependency. It contains owned data types for protocol 7:
lossless source snapshots/segments, contextual handles, optional input views,
output fragments, and structured results/diagnostics.

- Input is always `TokenStream`; there is no raw-string or compiler-AST variant.
- Ranges are UTF-8 byte offsets, not character indices; `U64` transport values
  are checked against host limits by the bounded codecs.
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

The binary IO seam is tested through ordinary `std::io::traits` APIs, including
`read_to_end_limited` (EOF-terminated bytes with a cap) and `write_all` (short-write
handling). Input/output are byte collections, never strings or compiler callbacks.
Negative limits, excess input and invalid read counts yield `IoError::InvalidInput`;
source IO errors propagate unchanged.

## Request decoding

`decode::decode_request bytes, expected_compiler_version` returns
`Result MacroRequest, decode::DecodeError`. Supply the runner's compiler package
version explicitly; the SDK does not discover a compiler or contact one.

- Only protocol 7 and `RKLOSS04` are accepted, with the exact field order and
  bincode 1 variable-width little-endian encoding; trailing bytes reject.
- The whole encoded request is capped at 1 MiB, including its nested lossless
  stream. Declared lengths are checked against schema caps and remaining bytes
  before strings or collections are allocated. UTF-8 conversion is checked.
- Validation checks source tables, byte boundaries, run kinds, adjacency,
  projections, opaque captures, layout, handles, unique dependencies and views.
  An input construction request is rejected; no bindings are minted by decoding.
- Failures carry a typed `DecodeErrorKind` and byte offset. Nested-stream errors
  use an offset within that payload; whole-request semantic failures point at the
  end of the envelope. Offsets do not designate filesystem locations.

## Result encoding

`encode::encode_result &request, result` returns `Result (Vec U8), encode::EncodeError`.
It consumes `MacroResult::Expand`, `Error` or `Report`, normalizes to one optional
output/diagnostics record, and writes protocol 7 bytes into an owned buffer.

- `None` output requires an Error diagnostic; `Some` with zero parts is an
  intentional empty expansion. Warnings and errors may accompany output.
- Stream, generated-source and literal-view fragments retain their explicit
  discriminants, construction contexts and ranges. No fragments are parsed or
  materialized by the encoder.
- Local checks reject invalid streams, cross-invocation/dependency construction
  requests, invalid UTF-8 diagnostic/literal ranges, absent-output locations,
  oversized lists, expanded spelling and encoded envelopes. All buffers are
  bounded; failure returns no partial response bytes.
- The compiler independently reauthorizes forwarded snapshots/contexts/origins
  and canonical literal views against its original request. Encoding locally
  constructed data does not grant authority or validate Rock syntax.

The native acceptance compares exact response/error goldens, verifies all result,
severity and fragment forms through Rust decoding/validation, and exercises
large/wide values and failure paths. Binary stdout is drained concurrently with
stdin/stderr under the fixture deadline, including output above pipe capacity.

## One-shot runtime

`runtime::run_worker expected_compiler_version, entry` reads one EOF-terminated
request, calls an ordinary `MacroContext -> TokenStream -> MacroResult` function,
writes one final response and exits the process. A bootstrap executable can use:

```rock
> rock_macro::*
> rock_macro::runtime::run_worker

identity: MacroContext -> TokenStream -> MacroResult
identity = context, input -> MacroResult::Expand input

main = !->
    run_worker "0.7.0", identity
```

The compiler-generated wrapper owns this bootstrap code; macro authors do not
implement transport or a dispatcher. The version is supplied explicitly,
not discovered by the SDK. The runner gives the entry deep-owned context/input
copies and retains the original request for output validation. It does not
normalize block indentation or interpret DSL text.

- Concatenated requests, truncation, invalid versions and excess input reject
  before the entry is called. There is no second-request loop or persistent state.
- A structured `MacroResult::Error` is a complete protocol response with normal
  worker exit. Invalid results, request/response IO and codec failures exit nonzero;
  static phase messages go only to ordinary `std::io::traits::stderr`.
- The native compiler host still validates the final response and clean exit.
  Native acceptance uses parser-produced inline-expression/block invocation data
  through that host; it does not claim procedural expression dispatch, block
  margin rewriting, or the complete package-build workflow.

## Source declarations and discovery

Proc-library source declares entries explicitly in the macro namespace:

```rock
> rock_macro::*

helper = context, input -> MacroResult::Expand input

< macro identity = context, input -> helper context, input
```

Only macro declarations exported through `< macro name = ...` or a separate
`< name` enter the proc export metadata. Ordinary functions, including exported
ones, are not implicit entries. Helpers/private macros stay private, and exported
members of private modules do not become public package entries. `derive` is
reserved for the later derive declaration syntax.

The compiler library's `discover_proc_macros` uses an explicit proc target role,
expands source declarative macros, then returns `Expansion::proc_exports` with
source definition identities, public names and function-like entry kinds. It
disables workers even if the caller's configuration permits execution. A provider
cannot invoke its own source proc entries while building itself. Proc facades and
runtime-target proc declarations diagnose instead of inventing runtime exports.

## Generated host compilation

The compiler library's `compile_proc_macro(config, sdk)` builds a native host from
source declarations, with `Config::target_role = TargetRole::ProcMacro` and
`ProcMacroSdk { dependency: "rock_macro".into() }`. The SDK is an explicitly supplied
ordinary artifact, never loaded by package spelling. A differently named SDK
implementing the same contract works too; artifact names must still match their
declared crate identities.

The compiler generates a private dispatcher and `main`, preserves each source
implementation AST in its original module, and checks it against
`MacroContext -> TokenStream -> MacroResult` using ordinary compiler phases.
Provider-local names cannot shadow the generated SDK bindings. Only discovered
public identities dispatch; unknown identities/kinds return a structured error.
Source entries need no handwritten table, bootstrap, or signature annotation.

The SDK's root-level ordinary bootstrap contract exports `MacroContext`,
`TokenStream`, `MacroResult`, `run_worker`, `is_function_like` and `unknown_entry`.
The compiler supplies its version to the runner. The root `run_worker` delegates
to the same one-shot runtime described above; no second transport path exists.

Compiler contributors can build a host directly with `rockc --proc-macro-sdk
rock_macro`, supplying std and SDK using ordinary `--extern-artifact name=path`
arguments. Add `--emit-artifact provider.rkca` to publish a proc-only artifact
using `compile_proc_macro_with_products`; do not pass `--no-link`/`--emit-object`.
The product contains host/export metadata, not ordinary functions, HIR bodies or
a dummy runtime object. Its SDK/std objects are not consumer dependencies.

Artifact format 54 records the host/compiler/protocol and bounded SHA-256 content
identities for the executable/required host files. Relative host paths are anchored
to the artifact directory. Loading or `rockc --validate-artifact` checks metadata
and files without running the worker; invocation checks them again before spawn.
Ordinary artifacts still require their ordinary runtime object during validation.
Application package workflows will use `rock build`/`rock run` once host-aware
planning and package integration land in B.15–B.17.

The Rust wire specification is
[`docs/specs/2026-10-19-macro-worker-wire.md`](../docs/specs/2026-10-19-macro-worker-wire.md).
