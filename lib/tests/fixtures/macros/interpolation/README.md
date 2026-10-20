# External interpolation fixture

This is a deliberately small library-owned interpolation proof, not a production
formatting library. `provider` is the ordinary procedural package `weave`;
`runtime` is the ordinary target library `ink_runtime`. Neither the macro name
nor the runtime's `Paint` trait has compiler-recognized meaning.

The complete consumer is:

```rock
> weave::weave
main = !->
    name = String::from "Ada"
    count = 2
    (%weave "Hello {name}: {count + 1}").println!
```

The application declares `weave`, `ink_runtime`, and `std` dependencies in
`rock.toml`. From this directory, with the compiler binaries on PATH:

```sh
rock run --debug
```

The program prints `Hello Ada: 3` and exits successfully. The procedural provider
uses `rock_macro` on the host; the application does not depend on that SDK.

## Boundaries

- The input must be one quoted literal, recognized by the ordinary Rock parser.
  The provider reads the compiler-prepared canonical literal view; it does not
  implement another Rock literal decoder or make compiler callbacks.
- Non-nested `{expression}` holes become deferred literal fragments. Ordinary
  generated parsing checks their Rock syntax and resolves them in caller scope.
  The fixture deliberately rejects nested braces, unmatched braces, and empty
  holes; it is not a full expression-aware interpolation scanner.
- Only new literal text is quoted into new source fragments. Hole syntax is
  never printed/stringified into generated source. Unicode and decoded escapes
  retain the compiler-owned byte mappings used for precise diagnostics.
- Definition-side helper paths use declared dependency handles. A consumer must
  explicitly supply the matching `ink_runtime` library; host dependencies do
  not silently become application dependencies.
- The helper's ordinary `Paint` bound determines which types can be formatted.
  Its initial implementations cover I64, String and borrowed Str. Missing names,
  unsupported values and absent runtime dependencies produce normal compiler
  errors, not interpolation-specific compiler fallbacks.

Contributor acceptance commands, run serially from the repository root:

```sh
cargo test -p rock macro_b_21 -- --nocapture
cargo test -p rock-lib --test macro_sdk -- --nocapture
```

B.21 establishes interpolation, binding, dependency and diagnostic boundaries.
Evaluation order, borrowing/consumption/destruction, hygiene collisions and
relocated source-free consumers are the separate B.22 acceptance step.
