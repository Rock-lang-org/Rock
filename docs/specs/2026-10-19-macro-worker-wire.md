# Rock macro worker wire: protocol 8

This is the compiler/SDK contract for the current one-shot worker, not a stable
cross-version protocol. Rebuild workers and dependencies after a format change.
The normative Rust records are `lib/src/macro_expansion/proc_macro/schema.rs`
and `proc_macro.rs`; the checked lossless transport is
`lib/src/lexer/lossless/wire.rs`. Product artifacts are format 56. No older
request, response, or artifact reader is provided.

## Encoding and framing

Use bincode 1 `DefaultOptions::new().with_limit(1048576)` on both sides:
little-endian, variable-width unsigned integers, sequence/string lengths as
variable-width `u64`, declaration-order struct fields, declaration-order enum
tags, and UTF-8 strings. Values below 251 occupy one byte; marker 251 introduces
a little-endian `u16`, 252 a `u32`, and 253 a `u64`. Options use one byte, 0 for
None and 1 for Some, followed by the payload. An enum tag uses the same unsigned
integer encoding. A Rust/LLVM in-memory struct is never a wire record.

There is no extra length prefix: compiler stdin contains one complete request,
then EOF; stdout contains one complete response, then EOF. Reject trailing data.
Every request and response begins with protocol version 8, validated before
decoding its payload. Requests also require the exact compiler package version.
The 1 MiB limit covers the whole encoded envelope, not just spelling. Nested
lossless byte payloads use these same bincode options, not `bincode::serialize`'s
fixed-integer defaults. Stdout contains protocol data only; stderr is not syntax.

## Request order and authority

`ProcMacroRequest` fields, in order:

1. Protocol version (`u32`) and compiler version (`String`).
2. Export display name, canonical export identity, optional source-module name.
3. Optional expansion number, invocation handle, optional parent expansion number.
4. Output category (Items=0, Expression=1).
5. Macro kind (FunctionLike=0, AttributeLike=1, DeriveLike=2).
6. Physical header indentation and optional block-body margin (`usize` as
   variable-width `u64`, checked when decoding on the receiving host).
7. Encoded lossless input (`Vec<u8>`), never a raw-string input variant.
8. Declared definition dependencies (`Vec<ProductCrateIdentity>`).
9. `ProcMacroViews`: optional argument-range record, group records, literal records.

The input retains `RKLOSS05` snapshot tables, run kinds, byte ranges, syntax
contexts, boundaries, provenance/projections, and opaque expression ranges.
Aliases reference the same snapshot table row. Snapshot handles must be nonzero
and unique; source adjacency is valid only between contiguous ranges of that
same row. Invalid ranges, run coverage/kinds, UTF-8, projection arithmetic, or
crossing/overdeep expression ranges reject. Input cannot contain unresolved
construction requests. Snapshot rows contain identity, file path, physical window
offset, and UTF-8 window text, in that order. Writers retain only the minimum
window covering that snapshot's referenced runs and anchor; unrelated file
prefixes/suffixes do not count toward the envelope. Internal gaps within that
window remain retained. Run ranges and anchor offsets stay physical: subtract the
window offset before indexing text. Window ends must not overflow; ranges and
anchors must lie within the window on UTF-8 boundaries. Returned windows must
match the corresponding substring of the compiler-owned snapshot.

Argument, group, literal-view and opaque-expression ranges are logical UTF-8 byte
offsets in concatenated retained spelling. Diagnostic projections never prove
physical adjacency or binding authority.

An argument view is an ordered list of disjoint, nonempty ranges. Absence means
no ordinary argument interpretation was supplied, not an empty argument list.
Group records contain a delimiter (Paren=0, Bracket=1, Brace=2) and a complete
opening-through-closing range; order is preorder with bounded, noncrossing
nesting. Literal records contain an input source range, decoded UTF-8, and
ordered mappings `(decoded range, input source range)`. Mappings cover every
decoded byte exactly once, on character boundaries, and remain within the
literal's source range. Escape spelling and decoded widths may differ. The
compiler prepares canonical views only when that interpretation was requested;
arbitrary DSL quote characters do not force Rock literal interpretation.

B.18 prepares these records from parser-cached string/character ranges, including
ordinary inline arguments, without a second lexical pass. Indented block inputs
receive no automatic literal views, even for valid-looking quoted text. The
shared compiler decoder preserves the existing escapes (`\n`, `\t`, `\r`, `\0`,
`\\`, `\"`, `\'`) and leaves unknown string escapes verbatim; characters must
decode to exactly one Unicode scalar. Each mapping covers one decoded scalar,
with the complete escape spelling or the original UTF-8 scalar as its source
range. Empty strings have an empty mapping. Source ranges include the enclosing
quotes, but individual mappings exclude them. Mapping preparation obeys the
metadata entry limit before appending an entry; workers only read the supplied
records and never request decoding callbacks.

## Final response

`ProcMacroResponse` contains protocol version, optional output, and diagnostics.
`Some(ProcMacroOutput { parts: [] })` is intentional empty expansion. `None`
requires an Error diagnostic. Errors may accompany output, but prevent parsing
or executing it. Warnings, notes and help may accompany successful output and
are retained by expansion/analysis APIs and reported by compiler/expansion CLIs.

Each ordered output part is exactly one of:

- Stream=0: encoded shared lossless stream, with forwarded metadata revalidated
  against the original compiler-owned request, not a worker's replacement input.
- Source=1: generated UTF-8 plus an invocation-owned symbolic construction context.
- Literal=2: literal-view index plus a decoded UTF-8 substring range; no worker
  copy of the literal can replace the compiler's canonical text/mapping.

Each part boundary is an emission splice, not permission to glue tokens.
Intentional adjacency within an emitted stream remains explicit in its runs.

Construction contexts contain an invocation handle and Fresh=0 (`u32` slot),
CallSite=1, or Dependency=2 (`u32` index). A dependency index selects a declared
product identity; it does not name a caller alias or grant private exports.
Compiler-owned freshness and public export/environment resolution remain the
only source of binding authority. Cross-invocation requests, invalid dependency
or literal handles, mutated forwarding snapshots/ranges/contexts/provenance,
and invalid diagnostic ranges reject before generated parsing.

An opaque expression capture is an indivisible forwarded atom. Output may omit,
duplicate or reorder whole captures, but cannot remove or invent their metadata,
split a capture, change its layout margin, or change nested capture metadata.
Validation compares retained source ranges, contexts, provenance, and internal
splice boundaries against the original input before reconstructing output.

Each diagnostic contains severity (Error=0, Warning=1, Info=2, Hint=3), message,
primary location, secondary labels, notes, and help. Locations are Invocation=0,
Input=1 (range), or Output=2 (range). There are no worker-provided filesystem
paths or unchecked compiler IDs in diagnostic locations. Output locations require
output and must fit its expanded spelling; labels use the same validation.

Source/literal descriptors are materialized only after worker exit. Source parts
undergo neutral scanning and normal construction-context authorization; literal
parts select the compiler-owned decoded view and receive invocation-site context.
Captured Stream parts retain their snapshots, contexts and opaque expression
ranges without printing/re-lexing. Every part is joined with an emission boundary.
Normal capture-aware generated parsing diagnoses invalid Rock afterward.

Decoded literal scalars retain their original input extents using existing
Generated provenance records. The compiler alone registers their fresh snapshot
identities for literal diagnostic mapping; this set is neither worker-controlled
nor binding/adjacency authority. Equal-width mappings are linear; an escaped
scalar maps to its complete spelling. Mixed extents map back only when their
source pieces are contiguous. Forwarded projected spans retain ancestry, so a
second expansion does not turn decoded byte indices into guessed source offsets.
No wire layout or additional SDK decoding service is needed for this policy.
Fragment scans, mapping/slice walks and splice work use the shared expansion work
budget; output bytes/units, opaque nesting and invocation ancestry retain their
normal aggregate limits. The response envelope/output spelling bounds still
apply before reconstruction.

## Bounds and golden messages

In addition to the envelope limit, metadata lists have at most 16,384 entries;
declared sequence counts reject before element allocation. Lossless tables/runs
and opaque captures retain their 1,048,576-entry and 16 MiB spelling limits.
Group/capture nesting is at most 128. Expanded fragment spelling is limited to
1 MiB for a response; checked lengths precede concatenation.

Normal source and generated parsing share a sticky parser-work/depth budget,
including speculative alternatives and nested opaque fragments. Parser calls
and token lookahead charge work; direct recursive type paths also carry depth
guards. The limits are 512 guarded frames and at most 8,388,608 work units per
parse. On Linux x86-64/AArch64, guards also reserve 256 KiB of the current native
thread stack for diagnostics and unguarded frames; unavailable stack bounds use
a conservative 128-frame cap. Nested parsers share the root guard. Generated parsing and
matcher expression/operand probes charge the aggregate expansion matching-work
budget; resource exhaustion cannot select a later matcher arm. Nested declarative
repetition rejects at the second opening, before recursive descent.

## Process IO

`Config::no_proc_execution` and `--no-proc-execution` on `rockc` or `rock`
disable procedural worker execution without disabling metadata inspection or
declarative macros. `rock` accepts the flag before or after `build`, `run` and
`expand`, and propagates it to root and dependency compiler invocations. Arguments
after `rock run --` still belong to the program. Protected builds may reuse
prebuilt artifacts; they do not implicitly run the separate developer std
packager without the policy, and request a prebuilt `ROCK_SYSROOT` if needed.

The expansion engine diagnoses a disabled procedural use at its invocation;
the native runner and final spawn boundary enforce the policy independently of
protocol validity, host metadata, executable existence or OS permissions. Imports,
artifact validation and declarative-only expansion execute no workers even when
execution is enabled. This is a macro-execution policy, not a sandbox or a ban
on compiler/linker processes or on running the requested application. Enabled
workers are launched as native executables directly, never through a fallback
shell; OS permission and executable-format failures remain visible.

The Unix execution host pumps all three pipes nonblockingly under one deadline,
starting before request encoding and continuing through writes, bounded stdout
and stderr reads, response validation, and successful worker termination. A
valid response alone is not success: nonzero exit, crash, unfinished shutdown,
broken request IO, malformed response, and oversized streams fail. No transport
reader/writer threads or blocking IO joins remain. Error reports retain the
selected export identity and at most 4 KiB of stderr bytes, using lossy UTF-8
only for that developer context, never for syntax or wire input.

Every invocation has a private process group. The runner observes leader exit
without reaping it, terminates its group while the group ID is still reserved,
then reaps the leader and closes the pipes. Unsupported hosts reject execution
before spawning, instead of using an unbounded fallback. The native runner accepts
an optional host-owned atomic cancellation flag; expansion contexts accept the
same flag through `with_cancellation`. Pre-cancelled requests spawn nothing.
Cancellation is checked before spawning and throughout IO/validation, and uses
the same group termination, leader reaping and pipe closure as other failures.
Dedicated native process integration checks exercise descendants retaining pipes
on success, malformed response, timeout, broken request and cancellation. A
separate-process Linux repetition check measures stable descriptor/thread counts
without racing unrelated compiler tests. Each failing fixture has an independent
watchdog and worker alarm, and checks that no recorded process survives.
Native macro code remains
trusted host code, not sandboxed code; intentionally escaping the process group
is not a supported worker lifecycle.

Byte-exact lowercase hex fixtures live in
`lib/tests/fixtures/macros/protocol-8/`: `request.hex`, `response.hex`, and
`error.hex`. They cover the `("é\n", value)` input (backslash-n source spelling),
an opaque capture, a group, argument ranges, escape-aware literal mappings,
forwarded spelling, all construction kinds, a literal fragment, and structured
warning/error results. Invocation and snapshot handles are fixed to 1 solely
for these test messages. B.08's Rock SDK request acceptance consumes `request.hex`
and Rust-produced enriched/malformed controls; B.09 produces the exact
`response.hex`/`error.hex` bytes and verifies result variants through Rust decoding
and request-specific validation. Compiler-only tests do not establish
Rock-authored worker usability.

## Ordinary Rock data model

`rock_macro/src/lib.rk` supplies the owned SDK data types as an ordinary library
with an explicit `std` dependency. `TokenStream` retains source snapshots,
segments, anchor and opaque expression ranges; it is not printed source or a
compiler AST. `MacroContext` contains the remaining request facts, so an ordinary
entry has the `MacroContext, TokenStream -> MacroResult` contract. UTF-8 byte
ranges and host-sized transport indices use `U64`; context/version handles use
their declared `U32` widths. Struct layout is not the wire encoding.

`MacroResult::Expand` and `MacroResult::Error` are authoring conveniences;
`MacroResult::Report` carries optional ordered output parts and diagnostics
together. `encode::encode_result` normalizes all three into the single B.01 response
record. All provenance, optional views, dependency identities, construction
requests and fragment descriptors are ordinary typed data. They grant no compiler
authority merely by being constructed locally. Local operations and checked IO/
serialization are B.06–B.10; the data-model runtime test is not worker or binary
interoperability acceptance.

`rock_macro::decode::decode_request` accepts owned request bytes and an explicitly
supplied expected compiler version. It checks the envelope and declared lengths
before payload allocation, uses checked UTF-8 conversion, decodes the nested
snapshot table, and validates the request's local structural invariants. Errors
contain a typed kind and byte offset (nested-stream-relative for nested failures,
end-of-envelope for whole-request semantic failures). No parser callbacks,
filesystem loading, macro execution or construction authority are introduced.

`rock_macro::encode::encode_result` takes the request and an owned authoring
result, validates local structure/ranges/limits, and returns a bounded owned byte
vector or typed `EncodeError`. It never writes a partial response or invokes a
parser. Request/input stream validation shares `validation.rk`; output streams
may retain symbolic construction requests, checked for invocation/dependency
ownership locally and reauthorized by the compiler on return. The local writer
does not certify forwarding metadata or interpret deferred fragments. Canonical
literals remain B.18 and materialization B.19.

`runtime::run_worker` reads one bounded EOF-terminated request, invokes a supplied
ordinary entry once with deep-owned context/input copies, writes a final encoded
response and exits. It retains the original request for local output checks.
Valid structured macro failures are protocol responses; IO/codec failures exit
nonzero with phase-only stderr messages and no incidental stdout. Concatenation
rejects as trailing data before entry execution. The native compiler host owns
process-per-invocation isolation, deadlines and final-response validation. The
bootstrap runtime is not generated export discovery/dispatch or package building;
those remain B.11–B.17. It does not normalize preserved DSL/block input into Rock.
