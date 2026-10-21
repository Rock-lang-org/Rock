# Compiler testing

Run the ordinary regression suite during compiler development:

```sh
cargo test -p rock-lib
cargo test -p rock -p rockc -p rock-shared
```

Run suites serially. The default library suite retains lexer, parser, expansion,
hygiene, diagnostic, inference and artifact regressions, bounded worker transport
tests, and a small native SDK smoke test through the production process runner.
It does not rebuild complete macro application/package acceptance scenarios.

## Explicit macro acceptance

```sh
cargo macro-acceptance
```

This alias runs the ignored macro acceptance groups serially in `rock-lib` and
`rock`. Run it manually when validating macro changes. It includes the unchanged cold CLI
package builds, freshness invalidation, worker lifecycle, source-free relocation,
DSL, interpolation and ownership scenarios.

`lib/tests/macro_sdk.rs` exposes independent native groups. Select one with its
full test name, for example:

```sh
cargo test -p rock-lib --test macro_sdk macro_acceptance_sdk_request_codec -- --ignored --exact --nocapture
cargo test -p rock-lib --test macro_sdk macro_acceptance_sdk_interpolation_ownership -- --ignored --exact --nocapture
cargo test -p rock-lib --test macro_sdk macro_acceptance_sdk_authoring -- --ignored --exact --nocapture
cargo test -p rock macro_b_20 -- --ignored --nocapture --test-threads=1
```

Request and result codec controls run in two test-only batch clients, rather than
launching a process per case. Each response frame retains its individual success
or rejection status and byte-exact assertions. This driver is not a worker
protocol: production macro execution and lifecycle tests still start a fresh,
bounded, one-shot process for every invocation.

## Shared prerequisites and compiler optimization

The native SDK groups and smoke test reuse immutable, explicitly built `std` and
`rock_macro` artifacts under the active Cargo profile's
`test-fixtures/macro-sdk/` directory. Keys include the compiler executable,
source/manifest contents, target and build policy. An OS file lock serializes
construction, and a completion digest validates artifacts and objects before
reuse. Incomplete or changed outputs are rebuilt. Mutable consumers, providers
and outputs are isolated per test; failing runs retain their directories for
inspection. The cold CLI acceptance scenarios do not use this fixture cache.

To force cold native prerequisites, remove only that profile's
`test-fixtures/macro-sdk/` cache while no tests are running. Rebuilding the test
executable also selects a new cache key. This is a local cache, not a portable
artifact bundle or a compiler-owned standard-library discovery mechanism.

The `rock-lib` package uses optimization level 2 in the test and dev profiles,
including the compiler linked into CLI subprocesses. Debug assertions and
overflow checks retain their normal profile settings. Other test harnesses keep
their inexpensive default optimization setting. The first optimized Rust build
costs more; subsequent compiler executions avoid the old unoptimized hot path.

These changes accelerate test orchestration; they do not establish expansion
phase timings or resolve the separately identified hashing, snapshot-copying,
stream-joining or mapping-scan scaling concerns.
