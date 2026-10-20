# Rock macro worker wire: protocol 7

This is the compiler/SDK contract for the current one-shot worker, not a stable
cross-version protocol. Rebuild workers and dependencies after a format change.
The normative Rust records are `lib/src/macro_expansion/proc_macro/schema.rs`
and `proc_macro.rs`; the checked lossless transport is
`lib/src/lexer/lossless/wire.rs`. Product artifacts are format 53. No older
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
Every request and response begins with protocol version 7, validated before
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

The input retains `RKLOSS04` snapshot tables, run kinds, byte ranges, syntax
contexts, boundaries, provenance/projections, and opaque expression ranges.
Aliases reference the same snapshot table row. Snapshot handles must be nonzero
and unique; source adjacency is valid only between contiguous ranges of that
same row. Invalid ranges, run coverage/kinds, UTF-8, projection arithmetic, or
crossing/overdeep expression ranges reject. Input cannot contain unresolved
construction requests. All offsets in metadata are logical UTF-8 byte offsets
in the concatenated retained spelling; diagnostic projections never prove
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

Each diagnostic contains severity (Error=0, Warning=1, Info=2, Hint=3), message,
primary location, secondary labels, notes, and help. Locations are Invocation=0,
Input=1 (range), or Output=2 (range). There are no worker-provided filesystem
paths or unchecked compiler IDs in diagnostic locations. Output locations require
output and must fit its expanded spelling; labels use the same validation.

Source/literal descriptors are schema-level output data in B.01. Their deferred
compiler scanning is B.19, and canonical literal-view preparation is B.18;
before those steps, executable invocations using deferred descriptors fail
explicitly rather than silently dropping or prematurely interpreting them.

## Bounds and golden messages

In addition to the envelope limit, metadata lists have at most 16,384 entries;
declared sequence counts reject before element allocation. Lossless tables/runs
and opaque captures retain their 1,048,576-entry and 16 MiB spelling limits.
Group/capture nesting is at most 128. Expanded fragment spelling is limited to
1 MiB for a response; checked lengths precede concatenation.

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
`lib/tests/fixtures/macros/protocol-7/`: `request.hex`, `response.hex`, and
`error.hex`. They cover the `("é\n", value)` input (backslash-n source spelling),
an opaque capture, a group, argument ranges, escape-aware literal mappings,
forwarded spelling, all construction kinds, a literal fragment, and structured
warning/error results. Invocation and snapshot handles are fixed to 1 solely
for these test messages. The future Rock SDK must consume/produce these same
bytes; compiler-only tests do not establish Rock-authored worker usability.
