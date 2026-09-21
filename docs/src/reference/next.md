# Where to Go Next

The language chapters cover expressions, ownership, functions, control flow,
data declarations, traits, modules, and the standard-library boundaries. The
next step depends on which boundary you want to practice:

1. Use the FizzBuzz checkpoint below if you want a short review of core syntax with deterministic output.
2. Combine owned buffers and `Result` in [Input, Output, and Files](../stdlib/io-and-files.md), then compare direct error propagation with the combinators in [Option, Result, and `?`](../functional/error-handling.md).
3. Move on to the [HTTP server guide](../stdlib/http.md) for a library-backed application that brings together requests, responses, configuration, and concurrent connection handling. Read [Threads and Synchronization](../stdlib/concurrency.md) before changing task lifetimes or shutdown behavior.
4. Use [Editors and Diagnostics](../getting-started/editor-and-diagnostics.md) to keep editor feedback and project builds aligned, then reduce unexpected behavior to a focused regression.

## Checkpoint: a complete FizzBuzz executable

This optional checkpoint separates calculation from output. It exercises an enum,
function signatures, guarded `match` arms, and the prelude's `for_each` and
pipeline operator.

### Outline

1. Define `FizzBuzzValue` with text and numeric variants.
2. Define `fizzbuzz_value` with guarded match arms, checking divisibility by 15 before 3 or 5 and ending with an unguarded numeric fallback.
3. Define `print_value` to consume one enum and print both variants.
4. Define `main` with `for_each` over `1..=30` and an inline action.
5. Run the project with `rock`.
6. Run the executable and compare the final lines with the expected output.

Save this manifest as `rock.toml` in a new project directory:

```toml
[crate]
name = "fizzbuzz"
version = "0.1.0"

[lib]
path = "main.rk"
```

Save this complete source beside it as `main.rk`:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

fizzbuzz_value: I64 -> FizzBuzzValue
fizzbuzz_value = number ->
    match number
        n if n % 15 == 0 => FizzBuzzValue::Text "FizzBuzz"
        n if n % 3 == 0 => FizzBuzzValue::Text "Fizz"
        n if n % 5 == 0 => FizzBuzzValue::Text "Buzz"
        n => FizzBuzzValue::Number n

print_value: FizzBuzzValue -> I32
print_value = value ->
    match value
        FizzBuzzValue::Text text => text.println!
        FizzBuzzValue::Number number => number.println!

main = !->
    for_each 1..=30, number !->
        number
            |> fizzbuzz_value
            |> print_value
```

Run it from the project directory:

```console
$ rock run
```

The expected output starts with `1`, `2`, `Fizz`, `4`, and `Buzz`, contains
`FizzBuzz` at 15, and ends with `Fizz`, `28`, `29`, and `FizzBuzz` at 30.

The final pipeline step moves the enum produced by `fizzbuzz_value` into
`print_value`. Its `&Str` payloads refer to static string literals and the
numeric payload is copyable. The inline action returns unit after printing;
`for_each` visits all 30 integers without creating an intermediate collection.

## Read concrete source

The repository's current examples and tests are more reliable than old design
notes. Start with these exact files:

```text
examples/fizzbuzz.rk
examples/structs.rk
examples/enums.rk
examples/enum_match.rk
examples/test_mod.rk
examples/extern_test.rk
stdlib/io.rk
stdlib/fs.rk
```

Read public signatures first, then inspect the implementation body that owns
allocation, borrowing, or FFI cleanup. The systems chapters explain why that
order matters.

## Run focused tests

The integration suite has concrete tests for the same features as the
checkpoint. Run one exact test while investigating compiler behavior, then run the
library suite:

```console
$ cargo test -p rock-lib --test integration test_functions -- --exact
$ cargo test -p rock-lib --test integration test_enum_match -- --exact
$ cargo test -p rock-lib --test integration test_for_loop -- --exact
$ cargo test -p rock-lib --test integration test_stdlib_program_args_are_owned_and_available_outside_main -- --exact
$ cargo test -p rock-lib
```

For unsafe boundaries, use the exact diagnostic test that checks the contract:

```console
$ cargo test -p rock-lib --test integration test_unsafe_operator_function_requires_unsafe -- --exact
$ cargo test -p rock-lib --test integration test_unsafe_ampersand_function_reference_call_requires_unsafe -- --exact
```

There is no application-level `rock test` command yet. Keep executable
regressions as complete Rock sources and use Cargo's integration harness for
compiler changes.

## Contribute a language example

A useful contribution has a parser test for new syntax, an integration test for
user-visible behavior, a small complete example, and a book update when the
public surface changes. Keep the example's declarations and imports in the
same source fence, place it in a project, and record the exact target and
`rock` command used.

Prefer the simplest abstraction that communicates intent. A `match` can be
clearer than a combinator chain, a concrete function can be clearer than a
higher-kinded trait, and a named method can be clearer than a custom operator.
Add an abstraction when it captures a repeated law or ownership contract.

## Keep feedback reproducible

When a program exposes a compiler limitation, reduce it to one complete source
file, state the command that compiles it, and include the observed diagnostic
or output. When it exposes a cleanup or FFI problem, state who owns every
allocation and which target ABI is involved. Precise examples help both book
readers and compiler contributors more than broad feature requests.
